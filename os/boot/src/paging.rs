//! The kernel's first page tables, built by the start-up stage just before it hands over.
//!
//! The lower half keeps the firmware's identity map (so the start-up stage can keep running for the
//! few instructions after the switch). The upper half is new: all of RAM at `PHYS_OFFSET`, and the
//! kernel's parts at the addresses it was linked for. The kernel replaces all of this with its own
//! tables as one of the first things it does.

use core::arch::asm;

pub const PRESENT: u64 = 1 << 0;
pub const WRITABLE: u64 = 1 << 1;
pub const HUGE: u64 = 1 << 7;
pub const NO_EXECUTE: u64 = 1 << 63;
const ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;

pub const KIB4: u64 = 4096;
pub const MIB2: u64 = 2 << 20;
pub const GIB1: u64 = 1 << 30;

/// What this processor's paging can do.
pub struct Features {
    pub no_execute: bool,
    pub gib_pages: bool,
    pub five_level: bool,
}

fn cpuid(leaf: u32) -> (u32, u32, u32, u32) {
    let (a, b, c, d): (u32, u64, u32, u32);
    unsafe {
        asm!("mov {t}, rbx", "cpuid", "xchg {t}, rbx", t = out(reg) b, inout("eax") leaf => a,
             inout("ecx") 0 => c, out("edx") d, options(nomem, nostack, preserves_flags));
    }
    (a, b as u32, c, d)
}

impl Features {
    pub fn detect() -> Features {
        let max_ext = cpuid(0x8000_0000).0;
        let edx = if max_ext >= 0x8000_0001 { cpuid(0x8000_0001).3 } else { 0 };
        let cr4: u64;
        unsafe { asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags)) };
        Features { no_execute: edx & (1 << 20) != 0, gib_pages: edx & (1 << 26) != 0, five_level: cr4 & (1 << 12) != 0 }
    }
}

/// Zeroed pages set aside (before ExitBootServices) for page tables.
pub struct Pool {
    next: u64,
    end: u64,
}

impl Pool {
    pub fn new(start: u64, pages: usize) -> Pool {
        Pool { next: start, end: start + pages as u64 * KIB4 }
    }

    fn take(&mut self) -> Option<u64> {
        if self.next >= self.end {
            return None;
        }
        let page = self.next;
        self.next += KIB4;
        Some(page)
    }
}

fn slot(virt: u64, shift: u32) -> usize {
    ((virt >> shift) & 0x1FF) as usize
}

/// The table one level down from `table[index]`, made if it doesn't exist yet.
unsafe fn down(pool: &mut Pool, table: u64, index: usize) -> Option<u64> {
    let entry = unsafe { (table as *mut u64).add(index) };
    let value = unsafe { *entry };
    if value & PRESENT != 0 {
        return if value & HUGE != 0 { None } else { Some(value & ADDRESS) };
    }
    let next = pool.take()?;
    unsafe { *entry = next | PRESENT | WRITABLE };
    Some(next)
}

/// Maps one page of `size` (4 KiB, 2 MiB or 1 GiB). Tables are reached through the identity map.
pub unsafe fn map(pool: &mut Pool, pml4: u64, virt: u64, phys: u64, size: u64, flags: u64) -> Option<()> {
    unsafe {
        let pdpt = down(pool, pml4, slot(virt, 39))?;
        if size == GIB1 {
            *(pdpt as *mut u64).add(slot(virt, 30)) = phys | flags | HUGE;
            return Some(());
        }
        let pd = down(pool, pdpt, slot(virt, 30))?;
        if size == MIB2 {
            *(pd as *mut u64).add(slot(virt, 21)) = phys | flags | HUGE;
            return Some(());
        }
        let pt = down(pool, pd, slot(virt, 21))?;
        *(pt as *mut u64).add(slot(virt, 12)) = phys | flags;
    }
    Some(())
}

/// How many table pages mapping `[0, ram_end)` at `PHYS_OFFSET` takes.
pub fn direct_map_tables(ram_end: u64, f: &Features) -> usize {
    let gib = ram_end.div_ceil(GIB1) as usize;
    let pdpts = gib.div_ceil(512);
    if f.gib_pages { pdpts } else { pdpts + gib }
}

pub unsafe fn map_direct(pool: &mut Pool, pml4: u64, offset: u64, ram_end: u64, f: &Features) -> Option<()> {
    let nx = if f.no_execute { NO_EXECUTE } else { 0 };
    let size = if f.gib_pages { GIB1 } else { MIB2 };
    let mut p = 0;
    while p < ram_end {
        unsafe { map(pool, pml4, offset + p, p, size, PRESENT | WRITABLE | nx)? };
        p += size;
    }
    Some(())
}

pub fn read_cr3() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr3", out(reg) v, options(nomem, nostack, preserves_flags)) };
    v & ADDRESS
}

pub unsafe fn enable_no_execute() {
    const EFER: u32 = 0xC000_0080;
    unsafe {
        let (lo, hi): (u32, u32);
        asm!("rdmsr", in("ecx") EFER, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags));
        let v = ((hi as u64) << 32 | lo as u64) | (1 << 11);
        asm!("wrmsr", in("ecx") EFER, in("eax") v as u32, in("edx") (v >> 32) as u32, options(nostack, preserves_flags));
    }
}
