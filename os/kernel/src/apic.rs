//! The processor core's own interrupt controller (local APIC), in either of its two modes: registers
//! in memory (xAPIC) or registers as MSRs (x2APIC, if the firmware turned that on). The two old 8259
//! controllers are moved out of the way and switched off.

use crate::acpi::Acpi;
use crate::cpu::{outb, rdmsr, wrmsr};
use crate::idt::{APIC_ERROR_VECTOR, SPURIOUS_VECTOR};
use crate::paging::{self, Cache};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

const IA32_APIC_BASE: u32 = 0x1B;

pub const ID: u32 = 0x20;
pub const TASK_PRIORITY: u32 = 0x80;
pub const EOI: u32 = 0xB0;
pub const SPURIOUS: u32 = 0xF0;
pub const ERROR_STATUS: u32 = 0x280;
pub const LVT_TIMER: u32 = 0x320;
pub const LVT_LINT0: u32 = 0x350;
pub const LVT_ERROR: u32 = 0x370;
pub const TIMER_INITIAL: u32 = 0x380;
pub const TIMER_CURRENT: u32 = 0x390;
pub const TIMER_DIVIDE: u32 = 0x3E0;

pub const MASKED: u32 = 1 << 16;

static BASE: AtomicU64 = AtomicU64::new(0);
static X2APIC: AtomicBool = AtomicBool::new(false);
static OWN_ID: AtomicU32 = AtomicU32::new(0);

/// This core's interrupt controller id: where devices send their interrupt messages.
pub fn id() -> u32 {
    OWN_ID.load(Ordering::Relaxed)
}

pub fn read(reg: u32) -> u32 {
    if X2APIC.load(Ordering::Relaxed) {
        unsafe { rdmsr(0x800 + (reg >> 4)) as u32 }
    } else {
        unsafe { core::ptr::read_volatile((BASE.load(Ordering::Relaxed) + reg as u64) as *const u32) }
    }
}

pub fn write(reg: u32, value: u32) {
    if X2APIC.load(Ordering::Relaxed) {
        unsafe { wrmsr(0x800 + (reg >> 4), value as u64) }
    } else {
        unsafe { core::ptr::write_volatile((BASE.load(Ordering::Relaxed) + reg as u64) as *mut u32, value) }
    }
}

pub fn end_of_interrupt() {
    write(EOI, 0);
}

pub fn error_status() -> u32 {
    write(ERROR_STATUS, 0);
    read(ERROR_STATUS)
}

/// Moves the 8259s' interrupts to 0x20-0x2F (away from the processor's exceptions) and masks them all.
fn retire_legacy_pics() {
    unsafe {
        outb(0x20, 0x11);
        outb(0xA0, 0x11);
        outb(0x21, 0x20);
        outb(0xA1, 0x28);
        outb(0x21, 0x04);
        outb(0xA1, 0x02);
        outb(0x21, 0x01);
        outb(0xA1, 0x01);
        outb(0x21, 0xFF);
        outb(0xA1, 0xFF);
    }
}

/// Masks every input of the I/O APICs, whatever the firmware left there: no device interrupt arrives
/// until a ClaudeOS driver asks for it.
fn mask_io_apics(acpi: &Acpi) {
    for io in &acpi.io_apics[..acpi.io_apic_count] {
        let base = paging::map_device(io.address, 0x20, Cache::Uncached);
        let select = base as *mut u32;
        let window = (base + 0x10) as *mut u32;
        unsafe {
            core::ptr::write_volatile(select, 1);
            let inputs = (core::ptr::read_volatile(window) >> 16 & 0xFF) + 1;
            for i in 0..inputs {
                core::ptr::write_volatile(select, 0x10 + 2 * i);
                let low = core::ptr::read_volatile(window);
                core::ptr::write_volatile(window, low | MASKED);
            }
        }
    }
}

/// Sets up this core's interrupt controller, in the mode the firmware left it in (x2APIC is only
/// needed past 255 cores; switching modes waits until ClaudeOS starts its other cores). Returns the
/// core's APIC id and whether x2APIC is on.
pub fn init(acpi: &Acpi) -> (u32, bool) {
    retire_legacy_pics();
    mask_io_apics(acpi);
    let mut base = unsafe { rdmsr(IA32_APIC_BASE) };
    if base & (1 << 11) == 0 {
        base |= 1 << 11;
        unsafe { wrmsr(IA32_APIC_BASE, base) };
    }
    let x2 = base & (1 << 10) != 0;
    X2APIC.store(x2, Ordering::Relaxed);
    if !x2 {
        let phys = base & 0x000F_FFFF_FFFF_F000;
        BASE.store(paging::map_device(phys, 4096, Cache::Uncached), Ordering::Relaxed);
    }
    write(SPURIOUS, 0x100 | SPURIOUS_VECTOR as u32); // software-enable, spurious vector
    write(TASK_PRIORITY, 0);
    write(LVT_LINT0, MASKED);
    write(LVT_ERROR, APIC_ERROR_VECTOR as u32);
    write(ERROR_STATUS, 0);
    write(ERROR_STATUS, 0);
    end_of_interrupt();
    let id = if x2 { read(ID) } else { read(ID) >> 24 };
    OWN_ID.store(id, Ordering::Relaxed);
    (id, x2)
}
