//! Locks for data shared between the kernel and its interrupt handlers.

use crate::cpu;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A spin lock that also turns interrupts off while held, so an interrupt handler can't wait forever
/// for a lock the code it interrupted is holding.
pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for SpinLock<T> {}
unsafe impl<T: Send> Send for SpinLock<T> {}

pub struct Guard<'a, T> {
    lock: &'a SpinLock<T>,
    interrupts_were_on: bool,
}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        SpinLock { locked: AtomicBool::new(false), value: UnsafeCell::new(value) }
    }

    pub fn lock(&self) -> Guard<'_, T> {
        let on = cpu::interrupts_enabled();
        cpu::disable_interrupts();
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        Guard { lock: self, interrupts_were_on: on }
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    pub fn try_lock(&self) -> Option<Guard<'_, T>> {
        let on = cpu::interrupts_enabled();
        cpu::disable_interrupts();
        if self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            Some(Guard { lock: self, interrupts_were_on: on })
        } else {
            if on {
                cpu::enable_interrupts();
            }
            None
        }
    }
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        if self.interrupts_were_on {
            cpu::enable_interrupts();
        }
    }
}

/// Memory the processor itself reads (descriptor tables, interrupt stacks), set up once at start.
#[repr(transparent)]
pub struct Static<T>(UnsafeCell<T>);

unsafe impl<T> Sync for Static<T> {}

impl<T> Static<T> {
    pub const fn new(value: T) -> Self {
        Static(UnsafeCell::new(value))
    }
    pub fn get(&self) -> *mut T {
        self.0.get()
    }
}
