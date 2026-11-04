//! A pool of threads for the model's arithmetic, where there's an operating system to have threads (the desktop
//! app, Chat Computer on Linux): each big step's jobs are shared among every core, the caller's own included.

extern crate std;

use crate::net::Pool;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread::JoinHandle;

/// One step's jobs: the function (on the caller's stack, which waits until every job is done), how many, which is
/// next, and how many are left.
struct Job {
    f: *const (dyn Fn(usize) + Sync),
    total: usize,
    next: AtomicUsize,
    left: AtomicUsize,
}

unsafe impl Send for Job {}
unsafe impl Sync for Job {}

struct Shared {
    job: Mutex<(u64, Option<Arc<Job>>)>,
    wake: Condvar,
    generation: AtomicUsize,
    stop: AtomicBool,
}

pub struct Threads {
    shared: Arc<Shared>,
    handles: Vec<JoinHandle<()>>,
}

impl Threads {
    /// A pool of `n` threads besides the caller's (n = the cores there are, less one).
    pub fn new(n: usize) -> Threads {
        let shared = Arc::new(Shared {
            job: Mutex::new((0, None)),
            wake: Condvar::new(),
            generation: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
        });
        let handles = (0..n)
            .map(|_| {
                let s = shared.clone();
                std::thread::spawn(move || worker(&s))
            })
            .collect();
        Threads { shared, handles }
    }

    /// As many threads as the computer has cores (less the caller's).
    pub fn for_this_computer() -> Threads {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        Threads::new(cores.saturating_sub(1))
    }
}

fn work(job: &Job) {
    loop {
        let i = job.next.fetch_add(1, Ordering::Relaxed);
        if i >= job.total {
            return;
        }
        // The caller waits for every job before its function goes.
        unsafe { (*job.f)(i) };
        job.left.fetch_sub(1, Ordering::Release);
    }
}

fn worker(s: &Shared) {
    let mut seen = 0usize;
    loop {
        // A moment's spinning for the next step (they come fast), then sleep until woken.
        let mut spins = 0u32;
        while s.generation.load(Ordering::Acquire) == seen && !s.stop.load(Ordering::Relaxed) {
            if spins < 20_000 {
                core::hint::spin_loop();
                spins += 1;
            } else {
                let g = s.job.lock().unwrap();
                if s.generation.load(Ordering::Acquire) == seen && !s.stop.load(Ordering::Relaxed) {
                    let _g = s.wake.wait(g).unwrap();
                }
                spins = 0;
            }
        }
        if s.stop.load(Ordering::Relaxed) {
            return;
        }
        let job = {
            let g = s.job.lock().unwrap();
            seen = g.0 as usize;
            g.1.clone()
        };
        if let Some(job) = job {
            work(&job);
        }
    }
}

impl Pool for Threads {
    fn run(&self, jobs: usize, f: &(dyn Fn(usize) + Sync)) {
        if jobs <= 1 || self.handles.is_empty() {
            for i in 0..jobs {
                f(i);
            }
            return;
        }
        // (The function's lifetime is the caller's: it's only used until every job is done, below.)
        let f: *const (dyn Fn(usize) + Sync) = unsafe { core::mem::transmute(f) };
        let job = Arc::new(Job { f, total: jobs, next: AtomicUsize::new(0), left: AtomicUsize::new(jobs) });
        {
            let mut g = self.shared.job.lock().unwrap();
            g.0 += 1;
            g.1 = Some(job.clone());
            self.shared.generation.store(g.0 as usize, Ordering::Release);
        }
        self.shared.wake.notify_all();
        work(&job);
        while job.left.load(Ordering::Acquire) != 0 {
            core::hint::spin_loop();
        }
        // (No worker can start this job's function again: every index is taken.)
        self.shared.job.lock().unwrap().1 = None;
    }
}

impl Drop for Threads {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        {
            let _g = self.shared.job.lock().unwrap();
        }
        self.shared.wake.notify_all();
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
    }
}
