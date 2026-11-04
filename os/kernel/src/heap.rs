//! The kernel's heap: memory for `Box`, `Vec`, `String` and the like. One block of RAM to start with, handed out
//! first-fit from a list of free pieces kept in address order; freed pieces join their neighbours. When nothing
//! free is big enough, the heap grows: more RAM, mapped on at the end of its own range of addresses (paging's
//! HEAP_START on), so even the model's file (gigabytes) is one piece, however scattered the RAM behind it. It grows
//! only while enough RAM is left for everything else (devices' memory, page tables), and never shrinks.

use crate::memory;
use crate::paging;
use crate::sync::SpinLock;
use bootinfo::{PAGE_SIZE, PHYS_OFFSET};
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;

const UNIT: usize = 16;
/// The heap grows by at least this much at a time, in whole blocks of 2 MiB.
const GROW_MIN: usize = 32 << 20;
const BLOCK: usize = 2 << 20;
/// The RAM growing leaves free for everything else.
pub const LEAVE: u64 = 64 << 20;

struct Piece {
    size: usize,
    next: *mut Piece,
}

struct Heap {
    first: *mut Piece,
    size: usize,
    /// Where the part it grew into ends (0: it hasn't grown).
    end: usize,
}

unsafe impl Send for Heap {}

static HEAP: SpinLock<Heap> = SpinLock::new(Heap { first: null_mut(), size: 0, end: 0 });

pub fn init() -> usize {
    // A quarter of free memory, between 16 and 256 MiB.
    let free = memory::stats().free as usize;
    let bytes = (free / 4).clamp(16 << 20, 256 << 20) / (2 << 20) * (2 << 20);
    let phys = memory::alloc_contiguous(bytes / PAGE_SIZE as usize).expect("no memory for the kernel heap");
    let mut heap = HEAP.lock();
    unsafe { heap.release((PHYS_OFFSET + phys) as usize, bytes) };
    heap.size = bytes;
    bytes
}

/// The biggest piece of the heap free now, in bytes: the most one allocation could have.
pub fn largest_free() -> usize {
    let heap = HEAP.lock();
    let mut most = 0;
    let mut p = heap.first;
    while !p.is_null() {
        unsafe {
            most = most.max((*p).size);
            p = (*p).next;
        }
    }
    most
}

/// The most it could still grow by, in bytes: the RAM free beyond what's left for everything else.
pub fn could_grow() -> u64 {
    memory::stats().free.saturating_sub(LEAVE) / BLOCK as u64 * BLOCK as u64
}

/// Whether the heap is in use right now (for the crash path, which must not wait for it).
pub fn busy() -> bool {
    HEAP.is_locked()
}

fn round(n: usize) -> usize {
    n.max(UNIT).div_ceil(UNIT) * UNIT
}

impl Heap {
    /// Grows by enough for `layout`: RAM mapped on at the end of the part it grew into (less what's free at its end
    /// already, which the new part joins). False if there isn't the RAM.
    unsafe fn grow(&mut self, layout: Layout) -> bool {
        let start = if self.end == 0 { paging::HEAP_START as usize } else { self.end };
        // What's free where the grown part ends: the last piece, if it reaches that far.
        let mut tail = 0;
        let mut p = self.first;
        while !p.is_null() {
            unsafe {
                if p as usize + (*p).size == start {
                    tail = (*p).size;
                }
                p = (*p).next;
            }
        }
        let need = (round(layout.size()) + layout.align().max(UNIT)).saturating_sub(tail);
        let bytes = need.max(GROW_MIN).div_ceil(BLOCK) * BLOCK;
        if bytes as u64 > could_grow() || (start + bytes) as u64 > paging::HEAP_END {
            return false;
        }
        paging::map_heap(start as u64, bytes as u64);
        self.end = start + bytes;
        self.size += bytes;
        unsafe { self.release(start, bytes) };
        true
    }

    unsafe fn take(&mut self, layout: Layout) -> *mut u8 {
        let size = round(layout.size());
        let align = layout.align().max(UNIT);
        let mut link: *mut *mut Piece = &mut self.first;
        unsafe {
            while !(*link).is_null() {
                let piece = *link;
                let (base, len, next) = (piece as usize, (*piece).size, (*piece).next);
                let start = base.div_ceil(align) * align;
                let end = start + size;
                if end <= base + len {
                    // Take [start, end); what's left before and after stays free, in order.
                    *link = next;
                    if end < base + len {
                        let after = end as *mut Piece;
                        (*after).size = base + len - end;
                        (*after).next = *link;
                        *link = after;
                    }
                    if start > base {
                        (*piece).size = start - base;
                        (*piece).next = *link;
                        *link = piece;
                    }
                    return start as *mut u8;
                }
                link = &mut (*piece).next;
            }
        }
        null_mut()
    }

    unsafe fn release(&mut self, at: usize, size: usize) {
        unsafe {
            let mut before: *mut Piece = null_mut();
            let mut after = self.first;
            while !after.is_null() && (after as usize) < at {
                before = after;
                after = (*after).next;
            }
            let piece = at as *mut Piece;
            (*piece).size = size;
            (*piece).next = after;
            if !after.is_null() && at + size == after as usize {
                (*piece).size += (*after).size;
                (*piece).next = (*after).next;
            }
            if before.is_null() {
                self.first = piece;
            } else if before as usize + (*before).size == at {
                (*before).size += (*piece).size;
                (*before).next = (*piece).next;
            } else {
                (*before).next = piece;
            }
        }
    }
}

struct KernelHeap;

unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut heap = HEAP.lock();
        let p = unsafe { heap.take(layout) };
        if !p.is_null() {
            return p;
        }
        if unsafe { heap.grow(layout) } { unsafe { heap.take(layout) } } else { null_mut() }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { HEAP.lock().release(ptr as usize, round(layout.size())) }
    }
}

#[global_allocator]
static ALLOCATOR: KernelHeap = KernelHeap;
