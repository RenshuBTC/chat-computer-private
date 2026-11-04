//! The processor core's own interrupt controller (local APIC), in either of its two modes: registers
//! in memory (xAPIC) or registers as MSRs (x2APIC, if the firmware turned that on). The two old 8259
//! controllers are moved out of the way and switched off.

use crate::acpi::Acpi;
use crate::cpu::{self, outb, rdmsr, wrmsr};
use crate::idt::{APIC_ERROR_VECTOR, SPURIOUS_VECTOR};
use crate::paging::{self, Cache};
use crate::time;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

const IA32_APIC_BASE: u32 = 0x1B;

pub const ID: u32 = 0x20;
pub const TASK_PRIORITY: u32 = 0x80;
pub const EOI: u32 = 0xB0;
pub const SPURIOUS: u32 = 0xF0;
pub const ERROR_STATUS: u32 = 0x280;
/// The interrupt command register: where a core sends interrupt messages to other cores (two 32-bit registers in
/// xAPIC mode, the low one sending; one 64-bit MSR in x2APIC mode).
const ICR_LOW: u32 = 0x300;
const ICR_HIGH: u32 = 0x310;
const X2APIC_ICR: u32 = 0x830;
/// In xAPIC mode: the message is still being sent.
const SEND_PENDING: u32 = 1 << 12;
/// The messages Chat Computer sends: INIT (a core reset to wait for a start-up message; level asserted, as Intel
/// says), start-up (with the number of the page below 1 MiB the core starts in), and a fixed vector.
pub const INIT: u32 = 0x4500;
pub const STARTUP: u32 = 0x4600;
pub const FIXED: u32 = 0x4000;
pub const LVT_TIMER: u32 = 0x320;
pub const LVT_LINT0: u32 = 0x350;
pub const LVT_ERROR: u32 = 0x370;
pub const TIMER_INITIAL: u32 = 0x380;
pub const TIMER_CURRENT: u32 = 0x390;
pub const TIMER_DIVIDE: u32 = 0x3E0;

pub const MASKED: u32 = 1 << 16;

static BASE: AtomicU64 = AtomicU64::new(0);
/// The registers' physical address, in xAPIC mode.
static BASE_PHYS: AtomicU64 = AtomicU64::new(0);
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
/// until a Chat Computer driver asks for it.
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
/// needed past 255 cores; the other cores' follow this one's: `init_core`). Returns the core's APIC id and whether
/// x2APIC is on.
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
        BASE_PHYS.store(phys, Ordering::Relaxed);
        BASE.store(paging::map_device(phys, 4096, Cache::Uncached), Ordering::Relaxed);
    }
    write(SPURIOUS, 0x100 | SPURIOUS_VECTOR as u32); // software-enable, spurious vector
    write(TASK_PRIORITY, 0);
    write(LVT_LINT0, MASKED);
    write(LVT_ERROR, APIC_ERROR_VECTOR as u32);
    write(ERROR_STATUS, 0);
    write(ERROR_STATUS, 0);
    end_of_interrupt();
    let id = live_id();
    OWN_ID.store(id, Ordering::Relaxed);
    (id, x2)
}

/// Whether the controllers are in x2APIC mode (the first core's, which the others follow).
pub fn x2apic() -> bool {
    X2APIC.load(Ordering::Relaxed)
}

/// The id of the controller of the core this runs on, read from the controller itself (`id` is the first core's).
pub fn live_id() -> u32 {
    if X2APIC.load(Ordering::Relaxed) { read(ID) } else { read(ID) >> 24 }
}

/// Sends an interrupt message (`command`: INIT, STARTUP or FIXED, with its page or vector) to the core whose
/// controller id is `to`. False if, in xAPIC mode, the controller hadn't sent it a millisecond later.
pub fn send(to: u32, command: u32) -> bool {
    let on = cpu::interrupts_enabled();
    cpu::disable_interrupts();
    let sent = if X2APIC.load(Ordering::Relaxed) {
        // (Writing this MSR isn't ordered after earlier writes to memory, as writing the xAPIC's registers is: a
        // core woken must see what it's woken for.)
        cpu::memory_fence();
        unsafe { wrmsr(X2APIC_ICR, (to as u64) << 32 | command as u64) };
        true
    } else {
        write(ICR_HIGH, to << 24);
        write(ICR_LOW, command);
        let until = time::now_us() + 1000;
        loop {
            if read(ICR_LOW) & SEND_PENDING == 0 {
                break true;
            }
            if time::now_us() > until {
                break false;
            }
            core::hint::spin_loop();
        }
    };
    if on {
        cpu::enable_interrupts();
    }
    sent
}

/// Sets up the interrupt controller of a core other than the first, in the mode the first one's is in, at the
/// same address: so it can be woken by the first core's messages.
pub fn init_core() {
    let mut base = unsafe { rdmsr(IA32_APIC_BASE) };
    let first = BASE_PHYS.load(Ordering::Relaxed);
    if !X2APIC.load(Ordering::Relaxed) && base & (1 << 10) != 0 {
        // In x2APIC mode where the first core's isn't: back to xAPIC mode, which goes by way of off.
        base &= !(1 << 11 | 1 << 10);
        unsafe { wrmsr(IA32_APIC_BASE, base) };
    }
    if !X2APIC.load(Ordering::Relaxed) && first != 0 && base & 0x000F_FFFF_FFFF_F000 != first {
        // (Firmwares keep every core's at one address; one that moved this core's: back where the first's is
        // mapped.)
        base = (base & !0x000F_FFFF_FFFF_F000) | first;
    }
    if base & (1 << 11) == 0 {
        base |= 1 << 11;
        unsafe { wrmsr(IA32_APIC_BASE, base) };
    }
    if X2APIC.load(Ordering::Relaxed) && base & (1 << 10) == 0 {
        // From xAPIC mode to x2APIC: enabled already, then extended.
        base |= 1 << 10;
    }
    unsafe { wrmsr(IA32_APIC_BASE, base) };
    write(SPURIOUS, 0x100 | SPURIOUS_VECTOR as u32);
    write(TASK_PRIORITY, 0);
    write(LVT_TIMER, MASKED);
    write(LVT_LINT0, MASKED);
    write(LVT_ERROR, APIC_ERROR_VECTOR as u32);
    write(ERROR_STATUS, 0);
    write(ERROR_STATUS, 0);
    end_of_interrupt();
}
