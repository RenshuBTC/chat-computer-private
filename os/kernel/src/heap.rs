//! The kernel's heap: memory for `Box`, `Vec`, `String` and the like. One block of RAM, handed out
//! first-fit from a list of free pieces kept in address order; freed pieces join their neighbours.

use crate::memory;
use crate::sync::SpinLock;
use bootinfo::{PAGE_SIZE, PHYS_OFFSET};
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;

const UNIT: usize = 16;

struct Piece {
    size: usize,
    next: *mut Piece,
}

struct Heap {
    first: *mut Piece,
    size: usize,
}

unsafe impl Send for Heap {}

static HEAP: SpinLock<Heap> = SpinLock::new(Heap { first: null_mut(), size: 0 });

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

/// Whether the heap is in use right now (for the crash path, which must not wait for it).
pub fn busy() -> bool {
    HEAP.is_locked()
}

fn round(n: usize) -> usize {
    n.max(UNIT).div_ceil(UNIT) * UNIT
}

impl Heap {
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
        unsafe { HEAP.lock().take(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { HEAP.lock().release(ptr as usize, round(layout.size())) }
    }
}

#[global_allocator]
static ALLOCATOR: KernelHeap = KernelHeap;
