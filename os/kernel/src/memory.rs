//! Physical memory: which 4 KiB frames of RAM are free. One bit per frame (set = in use).

use crate::sync::SpinLock;
use bootinfo::{memtype, BootInfo, MemoryRegion, PAGE_SIZE, PHYS_OFFSET};

struct Frames {
    bits: *mut u64,
    words: usize,
    frames: usize,
    free: usize,
    hint: usize,
}

unsafe impl Send for Frames {}

static FRAMES: SpinLock<Option<Frames>> = SpinLock::new(None);

/// The first MiB is left alone: old BIOS data lives there, and starting the other processor cores
/// will need a page below 1 MiB.
const LOW_RESERVED: u64 = 0x10_0000;

pub struct Stats {
    pub total_ram: u64,
    pub free: u64,
}

static TOTAL_RAM: SpinLock<u64> = SpinLock::new(0);

pub fn init(info: &BootInfo) {
    let map = unsafe { info.memory_map() };
    let top = map.iter().filter(|r| r.is_ram()).map(|r| r.end()).max().unwrap_or(0);
    *TOTAL_RAM.lock() = map.iter().filter(|r| r.is_ram()).map(|r| r.pages * PAGE_SIZE).sum();
    let frames = (top / PAGE_SIZE) as usize;
    let words = frames.div_ceil(64);
    let bytes = (words * 8) as u64;
    let home = map
        .iter()
        .filter(|r| r.kind == memtype::CONVENTIONAL && r.start >= LOW_RESERVED && r.pages * PAGE_SIZE >= bytes)
        .map(|r| r.start)
        .next()
        .expect("no room for the list of free memory");
    let bits = (PHYS_OFFSET + home) as *mut u64;
    unsafe { core::ptr::write_bytes(bits, 0xFF, words) };
    let mut f = Frames { bits, words, frames, free: 0, hint: 0 };
    for r in map.iter().filter(|r| r.kind == memtype::CONVENTIONAL) {
        f.set_free(r.start, r.end());
    }
    f.set_used(0, LOW_RESERVED);
    f.set_used(home, home + bytes.div_ceil(PAGE_SIZE) * PAGE_SIZE);
    *FRAMES.lock() = Some(f);
}

/// After the kernel runs on its own page tables, the memory the firmware used while it was in charge
/// is free.
pub fn reclaim_firmware_memory(map: &[MemoryRegion]) -> u64 {
    let mut guard = FRAMES.lock();
    let f = guard.as_mut().expect("memory::init first");
    let before = f.free;
    for r in map.iter().filter(|r| matches!(r.kind, memtype::BOOT_SERVICES_CODE | memtype::BOOT_SERVICES_DATA)) {
        f.set_free(r.start.max(LOW_RESERVED), r.end());
    }
    (f.free - before) as u64 * PAGE_SIZE
}

pub fn stats() -> Stats {
    let free = FRAMES.lock().as_ref().map_or(0, |f| f.free) as u64 * PAGE_SIZE;
    Stats { total_ram: *TOTAL_RAM.lock(), free }
}

/// One free frame, filled with zeros.
pub fn alloc_zeroed() -> Option<u64> {
    let frame = FRAMES.lock().as_mut()?.alloc(1)?;
    unsafe { core::ptr::write_bytes((PHYS_OFFSET + frame) as *mut u8, 0, PAGE_SIZE as usize) };
    Some(frame)
}

/// `count` frames in a row (not zeroed).
pub fn alloc_contiguous(count: usize) -> Option<u64> {
    FRAMES.lock().as_mut()?.alloc(count)
}

/// Frames in a block of 2 MiB.
pub const BLOCK_FRAMES: usize = 512;

/// One free block of 2 MiB, starting on a 2 MiB boundary (so it can be mapped as one big page), not zeroed. The
/// highest free one: memory below 4 GiB is left for devices that can't reach higher.
pub fn alloc_block() -> Option<u64> {
    FRAMES.lock().as_mut()?.alloc_block()
}

/// One free frame below `limit` (for devices that can't reach all of memory), filled with zeros.
pub fn alloc_zeroed_below(limit: u64) -> Option<u64> {
    let frame = FRAMES.lock().as_mut()?.alloc_below(limit)?;
    unsafe { core::ptr::write_bytes((PHYS_OFFSET + frame) as *mut u8, 0, PAGE_SIZE as usize) };
    Some(frame)
}

/// Gives back the frame at `phys` (from `alloc_zeroed`, `alloc_zeroed_below` or one of
/// `alloc_contiguous`'s).
pub fn free(phys: u64) {
    if let Some(f) = FRAMES.lock().as_mut() {
        f.set_free(phys, phys + PAGE_SIZE);
    }
}

impl Frames {
    fn test(&self, frame: usize) -> bool {
        unsafe { *self.bits.add(frame / 64) & (1 << (frame % 64)) != 0 }
    }

    fn mark(&mut self, frame: usize, used: bool) {
        let word = unsafe { &mut *self.bits.add(frame / 64) };
        let bit = 1u64 << (frame % 64);
        if used && *word & bit == 0 {
            *word |= bit;
            self.free -= 1;
        } else if !used && *word & bit != 0 {
            *word &= !bit;
            self.free += 1;
        }
    }

    fn set_free(&mut self, start: u64, end: u64) {
        let first = start.div_ceil(PAGE_SIZE) as usize;
        let last = ((end / PAGE_SIZE) as usize).min(self.frames);
        for frame in first..last {
            self.mark(frame, false);
        }
    }

    fn set_used(&mut self, start: u64, end: u64) {
        let first = (start / PAGE_SIZE) as usize;
        let last = (end.div_ceil(PAGE_SIZE) as usize).min(self.frames);
        for frame in first..last {
            self.mark(frame, true);
        }
    }

    fn alloc_below(&mut self, limit: u64) -> Option<u64> {
        let frames = ((limit / PAGE_SIZE) as usize).min(self.frames);
        for i in 0..frames.div_ceil(64) {
            let word = unsafe { *self.bits.add(i) };
            if word == u64::MAX {
                continue;
            }
            let frame = i * 64 + (!word).trailing_zeros() as usize;
            if frame < frames {
                self.mark(frame, true);
                return Some(frame as u64 * PAGE_SIZE);
            }
        }
        None
    }

    /// A block's 512 frames are 8 whole words of the list: all clear, it's free.
    fn alloc_block(&mut self) -> Option<u64> {
        let words = BLOCK_FRAMES / 64;
        for b in (0..self.frames / BLOCK_FRAMES).rev() {
            let first = b * words;
            if (first..first + words).all(|w| unsafe { *self.bits.add(w) } == 0) {
                for w in first..first + words {
                    unsafe { *self.bits.add(w) = u64::MAX };
                }
                self.free -= BLOCK_FRAMES;
                return Some((b * BLOCK_FRAMES) as u64 * PAGE_SIZE);
            }
        }
        None
    }

    fn alloc(&mut self, count: usize) -> Option<u64> {
        if count == 0 || count > self.free {
            return None;
        }
        if count == 1 {
            for i in (self.hint..self.words).chain(0..self.hint) {
                let word = unsafe { *self.bits.add(i) };
                if word == u64::MAX {
                    continue;
                }
                let frame = i * 64 + (!word).trailing_zeros() as usize;
                if frame >= self.frames {
                    continue;
                }
                self.mark(frame, true);
                self.hint = i;
                return Some(frame as u64 * PAGE_SIZE);
            }
            return None;
        }
        // A run of free frames, first fit.
        let mut run = 0;
        let mut frame = 0;
        while frame < self.frames {
            if run == 0 && frame % 64 == 0 && unsafe { *self.bits.add(frame / 64) } == u64::MAX {
                frame += 64;
                continue;
            }
            if self.test(frame) {
                run = 0;
            } else {
                run += 1;
                if run == count {
                    let first = frame + 1 - count;
                    for f in first..=frame {
                        self.mark(f, true);
                    }
                    return Some(first as u64 * PAGE_SIZE);
                }
            }
            frame += 1;
        }
        None
    }
}
