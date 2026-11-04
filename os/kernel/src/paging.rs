//! The kernel's own page tables, which replace the start-up stage's:
//!
//! * all RAM at `PHYS_OFFSET` (read and write, never run),
//! * the kernel at the addresses it was linked for, each part with only the access it needs
//!   (code: read and run; read-only data: read; data: read and write),
//! * device memory (screen, interrupt controller, ...) mapped on request, uncached or write-combined,
//! * the heap's room to grow into, RAM mapped there as the heap needs it (for the model's file and what it reads),
//! * the kernel's own stack, with nothing mapped below it.
//!
//! Nothing is mapped at low addresses, so a null pointer faults.

use crate::cpu;
use crate::memory;
use crate::sync::SpinLock;
use bootinfo::{BootInfo, PAGE_SIZE, PHYS_OFFSET, SEGMENT_EXECUTE, SEGMENT_WRITE};
use core::sync::atomic::{AtomicU64, Ordering};

const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const WRITE_THROUGH: u64 = 1 << 3; // with the PAT set up below: selects write-combining
const CACHE_DISABLE: u64 = 1 << 4;
const HUGE: u64 = 1 << 7;
const NO_EXECUTE: u64 = 1 << 63;
const ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;

const MIB2: u64 = 2 << 20;
const GIB1: u64 = 1 << 30;

/// Where device memory gets mapped (page-map slot 384).
const DEVICE_START: u64 = 0xFFFF_C000_0000_0000;
const DEVICE_END: u64 = 0xFFFF_E000_0000_0000;
/// Where the heap grows (page-map slots 448 to 479), RAM mapped there as it's needed: one range, however scattered
/// the RAM behind it (heap.rs).
pub const HEAP_START: u64 = 0xFFFF_E000_0000_0000;
pub const HEAP_END: u64 = 0xFFFF_F000_0000_0000;
/// Where the kernel's own stack ends: its pages are the last of page-map slot 480, which has nothing else in it, so
/// running past the stack's end faults at once (the start-up stage's stack is in the map of all RAM, with other
/// memory right below it, which an overflow would write over unseen).
const STACK_TOP: u64 = 0xFFFF_F080_0000_0000;
/// Where its pages start (0 until it's mapped).
static STACK_BOTTOM: AtomicU64 = AtomicU64::new(0);
/// The other processor cores' stacks: below the first's, in the same page-map slot, each in 2 MiB of its own with
/// nothing mapped below it, and where each one's pages start (0: not mapped).
const CORE_STACK_ROOM: u64 = 2 << 20;
pub const MOST_CORES: usize = 64;
static CORE_STACK_BOTTOMS: [AtomicU64; MOST_CORES] = [const { AtomicU64::new(0) }; MOST_CORES];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cache {
    /// Normal memory.
    WriteBack,
    /// A framebuffer: writes are gathered into bursts; reads are slow.
    WriteCombining,
    /// Device registers: every access goes to the device, in order.
    Uncached,
}

impl Cache {
    fn bits(self) -> u64 {
        // Index into the page attribute table (PAT): 0 = write-back, 1 = write-combining (set up in
        // `init`), 3 = uncached.
        match self {
            Cache::WriteBack => 0,
            Cache::WriteCombining => WRITE_THROUGH,
            Cache::Uncached => WRITE_THROUGH | CACHE_DISABLE,
        }
    }
}

struct Tables {
    pml4: u64,
    no_execute: u64,
    gib_pages: bool,
    next_device: u64,
    /// The RAM that is mapped at PHYS_OFFSET: sorted [start, end) physical ranges.
    spans: &'static [(u64, u64)],
}

static TABLES: SpinLock<Option<Tables>> = SpinLock::new(None);

fn table(phys: u64) -> *mut u64 {
    (PHYS_OFFSET + phys) as *mut u64
}

fn slot(virt: u64, shift: u32) -> usize {
    ((virt >> shift) & 0x1FF) as usize
}

impl Tables {
    fn down(&mut self, table_phys: u64, index: usize) -> u64 {
        let entry = unsafe { &mut *table(table_phys).add(index) };
        if *entry & PRESENT != 0 {
            assert!(*entry & HUGE == 0, "a large page is in the way of a new mapping");
            return *entry & ADDRESS;
        }
        let next = memory::alloc_zeroed().expect("out of memory for page tables");
        *entry = next | PRESENT | WRITABLE;
        next
    }

    fn map_one(&mut self, virt: u64, phys: u64, size: u64, flags: u64) {
        let pdpt = self.down(self.pml4, slot(virt, 39));
        if size == GIB1 {
            unsafe { *table(pdpt).add(slot(virt, 30)) = phys | flags | HUGE };
            return;
        }
        let pd = self.down(pdpt, slot(virt, 30));
        if size == MIB2 {
            unsafe { *table(pd).add(slot(virt, 21)) = phys | flags | HUGE };
            return;
        }
        let pt = self.down(pd, slot(virt, 21));
        unsafe { *table(pt).add(slot(virt, 12)) = phys | flags };
    }

    /// Maps `len` bytes with the biggest pages that fit (when `large` allows).
    fn map(&mut self, virt: u64, phys: u64, len: u64, flags: u64, large: bool) {
        let mut off = 0;
        while off < len {
            let (v, p, left) = (virt + off, phys + off, len - off);
            let size = if large && self.gib_pages && v % GIB1 == 0 && p % GIB1 == 0 && left >= GIB1 {
                GIB1
            } else if large && v % MIB2 == 0 && p % MIB2 == 0 && left >= MIB2 {
                MIB2
            } else {
                PAGE_SIZE
            };
            self.map_one(v, p, size, flags);
            cpu::invlpg(v);
            off += size;
        }
    }

    fn in_direct_map(&self, phys: u64, len: u64) -> bool {
        self.spans.iter().any(|&(s, e)| s <= phys && phys.saturating_add(len) <= e)
    }
}

/// All RAM as sorted [start, end) ranges, neighbours merged so that big pages can be used. The list
/// lives in frames of its own (there is no heap yet), reached through the RAM map: the start-up
/// stage's covers all RAM, and so does the kernel's, since the list is that map's own outline.
fn ram_spans(info: &BootInfo) -> &'static [(u64, u64)] {
    let map = unsafe { info.memory_map() };
    let room = map.len().max(1);
    let frames = (room * core::mem::size_of::<(u64, u64)>()).div_ceil(PAGE_SIZE as usize);
    let phys = memory::alloc_contiguous(frames).expect("no memory for the list of RAM");
    let all = unsafe { core::slice::from_raw_parts_mut((PHYS_OFFSET + phys) as *mut (u64, u64), room) };
    let mut n = 0;
    for r in map.iter().filter(|r| r.is_ram() && r.pages > 0) {
        all[n] = (r.start, r.end());
        n += 1;
    }
    all[..n].sort_unstable_by_key(|r| r.0);
    let mut merged = 0;
    for i in 0..n {
        let (start, end) = all[i];
        if merged > 0 && all[merged - 1].1 >= start {
            all[merged - 1].1 = all[merged - 1].1.max(end);
        } else {
            all[merged] = (start, end);
            merged += 1;
        }
    }
    &all[..merged]
}

/// Sets the page attribute table so that one of its entries means write-combining (entry 1, which
/// is write-through by default and unused by Chat Computer).
fn set_up_pat() {
    const IA32_PAT: u32 = 0x277;
    // Entries 0..7: write-back, write-combining, uncached-minus, uncached, then the same again.
    const PAT: u64 = 0x0007_0406_0007_0106;
    cpu::wbinvd();
    unsafe { cpu::wrmsr(IA32_PAT, PAT) };
    unsafe { cpu::set_cr3(cpu::cr3()) }; // flush the TLB
    cpu::wbinvd();
}

pub fn init(info: &BootInfo) {
    set_up_pat();
    let mut t = Tables {
        pml4: memory::alloc_zeroed().expect("no memory for page tables"),
        no_execute: if cpu::has_no_execute() { NO_EXECUTE } else { 0 },
        gib_pages: cpu::has_gib_pages(),
        next_device: DEVICE_START,
        spans: ram_spans(info),
    };

    // All RAM. Every frame the memory record hands out is in it.
    let data = PRESENT | WRITABLE | t.no_execute;
    for &(start, end) in t.spans {
        t.map(PHYS_OFFSET + start, start, end - start, data, true);
    }

    // The kernel's parts.
    for s in info.segments() {
        let mut flags = PRESENT;
        if s.flags & SEGMENT_WRITE != 0 {
            flags |= WRITABLE;
        }
        if s.flags & SEGMENT_EXECUTE == 0 {
            flags |= t.no_execute;
        }
        t.map(s.virt, s.phys, s.size, flags, false);
    }

    unsafe { cpu::set_cr3(t.pml4) };
    *TABLES.lock() = Some(t);
}

/// Maps device memory and returns where it can be reached.
pub fn map_device(phys: u64, len: u64, cache: Cache) -> u64 {
    let mut guard = TABLES.lock();
    let t = guard.as_mut().expect("paging::init first");
    let start = phys & !(PAGE_SIZE - 1);
    let end = (phys + len).div_ceil(PAGE_SIZE) * PAGE_SIZE;
    // Keep the same offset within 2 MiB as the physical address, so large pages can be used.
    let virt = t.next_device.div_ceil(MIB2) * MIB2 + start % MIB2;
    assert!(virt + (end - start) <= DEVICE_END, "out of room for device mappings");
    t.next_device = virt + (end - start);
    let flags = PRESENT | WRITABLE | t.no_execute | cache.bits();
    t.map(virt, start, end - start, flags, true);
    virt + (phys - start)
}

/// Maps free RAM at `virt` (on a 2 MiB boundary, between HEAP_START and HEAP_END) for the heap: `len` bytes (whole
/// 2 MiB blocks), each block one big page where a whole block of RAM is free, else its own 512 frames. The caller
/// has checked there's the memory.
pub fn map_heap(virt: u64, len: u64) {
    assert!(virt.is_multiple_of(MIB2) && len.is_multiple_of(MIB2) && HEAP_START <= virt && virt + len <= HEAP_END);
    let mut guard = TABLES.lock();
    let t = guard.as_mut().expect("paging::init first");
    let flags = PRESENT | WRITABLE | t.no_execute;
    for v in (virt..virt + len).step_by(MIB2 as usize) {
        match memory::alloc_block() {
            Some(phys) => t.map(v, phys, MIB2, flags, true),
            None => {
                for page in (v..v + MIB2).step_by(PAGE_SIZE as usize) {
                    let phys = memory::alloc_contiguous(1).expect("out of memory for the heap");
                    t.map(page, phys, PAGE_SIZE, flags, false);
                }
            }
        }
    }
}

/// Maps `pages` zeroed frames as the kernel's own stack, and returns its top (a stack grows down from there).
pub fn map_stack(pages: u64) -> u64 {
    let mut guard = TABLES.lock();
    let t = guard.as_mut().expect("paging::init first");
    let flags = PRESENT | WRITABLE | t.no_execute;
    let bottom = STACK_TOP - pages * PAGE_SIZE;
    for page in (bottom..STACK_TOP).step_by(PAGE_SIZE as usize) {
        let phys = memory::alloc_zeroed().expect("no memory for the kernel's stack");
        t.map(page, phys, PAGE_SIZE, flags, false);
    }
    STACK_BOTTOM.store(bottom, Ordering::Relaxed);
    STACK_TOP
}

/// Maps `pages` zeroed frames as the stack of processor core `core` (1 to MOST_CORES - 1: not the first, whose stack
/// is the kernel's own), and returns its top.
pub fn map_core_stack(core: usize, pages: u64) -> u64 {
    assert!((1..MOST_CORES).contains(&core) && pages * PAGE_SIZE <= CORE_STACK_ROOM / 2);
    let mut guard = TABLES.lock();
    let t = guard.as_mut().expect("paging::init first");
    let flags = PRESENT | WRITABLE | t.no_execute;
    let top = STACK_TOP - core as u64 * CORE_STACK_ROOM;
    let bottom = top - pages * PAGE_SIZE;
    for page in (bottom..top).step_by(PAGE_SIZE as usize) {
        let phys = memory::alloc_zeroed().expect("no memory for a processor core's stack");
        t.map(page, phys, PAGE_SIZE, flags, false);
    }
    CORE_STACK_BOTTOMS[core].store(bottom, Ordering::Relaxed);
    top
}

/// How deep the kernel's own stack has gone, at most, and how big it is, in bytes. Its pages started zeroed, so the
/// lowest word that isn't zero is as deep as anything has written (a little short if the deepest words written were
/// zeros); measured, where a count of nested calls would be only a guess at it.
pub fn stack_used() -> (u64, u64) {
    let bottom = STACK_BOTTOM.load(Ordering::Relaxed);
    if bottom == 0 {
        return (0, 0);
    }
    let mut at = bottom;
    while at < STACK_TOP && unsafe { core::ptr::read_volatile(at as *const u64) } == 0 {
        at += 8;
    }
    (STACK_TOP - at, STACK_TOP - bottom)
}

/// Whether `address` is just past the end of the kernel's own stack, or another core's: where a fault means it ran
/// out.
pub fn past_stack(address: u64) -> bool {
    let past = |bottom: u64| bottom != 0 && address < bottom && address >= bottom - (1 << 20);
    past(STACK_BOTTOM.load(Ordering::Relaxed)) || CORE_STACK_BOTTOMS.iter().any(|b| past(b.load(Ordering::Relaxed)))
}

/// Whether `phys` to `phys + len` is RAM, reached at PHYS_OFFSET (the firmware's own RAM too: its ACPI tables' and
/// ACPI NVS memory).
pub fn in_ram(phys: u64, len: u64) -> bool {
    TABLES.lock().as_ref().is_some_and(|t| t.in_direct_map(phys, len))
}

/// A virtual address for reading firmware tables at `phys`: in the RAM map if they're in RAM,
/// otherwise mapped like device memory.
pub fn map_firmware_table(phys: u64, len: u64) -> u64 {
    {
        let guard = TABLES.lock();
        let t = guard.as_ref().expect("paging::init first");
        if t.in_direct_map(phys, len) {
            return PHYS_OFFSET + phys;
        }
    }
    map_device(phys, len, Cache::WriteBack)
}
