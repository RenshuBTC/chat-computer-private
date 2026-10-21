//! Restarting the PC. Chat Computer's own system is started once from Windows' power-on menu, so a restart brings the
//! PC back to what it normally starts (Windows). Ways to restart, tried in turn until one works: the register the
//! firmware names for it (ACPI's FADT), the chipset's reset control register (port 0xCF9, on every PC chipset since
//! the 1990s), the keyboard controller's old reset line, and last a processor reset by a fault it can't handle.
//!
//! (Shutting down needs the firmware's ACPI programs, which Chat Computer doesn't run yet.)

use crate::acpi::{Acpi, ResetRegister};
use crate::cpu;
use crate::paging::{self, Cache};
use crate::sync::SpinLock;
use crate::time;
use core::sync::atomic::{AtomicU64, Ordering};

static RESET: SpinLock<Option<ResetRegister>> = SpinLock::new(None);
/// When to restart (uptime in ms; 0: not asked).
static RESTART_AT: AtomicU64 = AtomicU64::new(0);

pub fn init(acpi: &Acpi) {
    *RESET.lock() = acpi.reset;
}

/// Restarts the PC in `ms` milliseconds (from the main loop, `due`).
pub fn restart_in(ms: u64) {
    RESTART_AT.store(time::uptime_ms() + ms.max(1), Ordering::Relaxed);
}

/// From the main loop: restarts when it's time.
pub fn due() {
    let at = RESTART_AT.load(Ordering::Relaxed);
    if at != 0 && time::uptime_ms() >= at {
        restart();
    }
}

pub fn restart() -> ! {
    crate::log!("Chat Computer kernel: restarting");
    cpu::disable_interrupts();
    crate::keep::shut_down();
    let reset = *RESET.lock();
    if let Some(r) = reset {
        match r.space {
            1 if r.address <= 0xFFFF => unsafe { cpu::outb(r.address as u16, r.value) },
            0 => {
                let at = paging::map_device(r.address & !0xFFF, 0x1000, Cache::Uncached) + (r.address & 0xFFF);
                unsafe { core::ptr::write_volatile(at as *mut u8, r.value) };
            }
            _ => {}
        }
        time::delay_ms(100);
    }
    unsafe {
        cpu::outb(0xCF9, 0x02);
        time::delay_us(50);
        cpu::outb(0xCF9, 0x06); // a full reset
    }
    time::delay_ms(100);
    for _ in 0..10_000 {
        if unsafe { cpu::inb(0x64) } & 2 == 0 {
            break;
        }
        time::delay_us(10);
    }
    unsafe { cpu::outb(0x64, 0xFE) };
    time::delay_ms(100);
    // An empty interrupt table, then an interrupt: the processor can't handle it, and resets.
    unsafe {
        let empty: [u8; 10] = [0; 10];
        core::arch::asm!("lidt [{}]", "int3", in(reg) &empty, options(noreturn));
    }
}
