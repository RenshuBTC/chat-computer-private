//! Memory that devices read and write themselves (DMA): whole pages, reached by the kernel through its map of
//! all RAM and by the device at their physical address.

use crate::memory;
use bootinfo::{PAGE_SIZE, PHYS_OFFSET};

pub struct Page {
    pub phys: u64,
}

impl Page {
    /// A zeroed page; `low`: below 4 GiB, for devices that can't reach higher.
    pub fn new(low: bool) -> Option<Page> {
        let phys = if low { memory::alloc_zeroed_below(1 << 32)? } else { memory::alloc_zeroed()? };
        Some(Page { phys })
    }

    pub fn at<T>(&self, offset: usize) -> *mut T {
        debug_assert!(offset < PAGE_SIZE as usize);
        (PHYS_OFFSET + self.phys + offset as u64) as *mut T
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        memory::free(self.phys);
    }
}
