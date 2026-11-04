//! Restarting and turning off the PC. Chat Computer's own system is started once from Windows' power-on menu, so a
//! restart brings the PC back to what it normally starts (Windows), and so does turning it on again after it's off.
//!
//! Ways to restart, tried in turn until one works: the register the firmware names for it (ACPI's FADT), the
//! chipset's reset control register (port 0xCF9, on every PC chipset since the 1990s), the keyboard controller's old
//! reset line, and last a processor reset by a fault it can't handle.
//!
//! Turning off is ACPI's S5 ("soft off"): S5's sleep type, which the firmware gives in its DSDT, written with "sleep
//! enable" to the PM1 control registers (acpi.rs). First the firmware's own preparation for it runs (its _TTS and _PTS
//! methods: firmware.rs), as it expects, and its \_S5 says the sleep type (or the DSDT's bytes do, when its programs
//! aren't run).

use crate::acpi::{Acpi, ResetRegister, SoftOff};
use crate::cpu;
use crate::paging::{self, Cache};
use crate::sync::SpinLock;
use crate::time;
use core::sync::atomic::{AtomicU64, Ordering};

static RESET: SpinLock<Option<ResetRegister>> = SpinLock::new(None);
static SOFT_OFF: SpinLock<Option<SoftOff>> = SpinLock::new(None);
/// When to restart, and when to turn off (uptime in ms; 0: not asked).
static RESTART_AT: AtomicU64 = AtomicU64::new(0);
static OFF_AT: AtomicU64 = AtomicU64::new(0);

pub fn init(acpi: &Acpi) {
    *RESET.lock() = acpi.reset;
    *SOFT_OFF.lock() = acpi.soft_off;
    match acpi.soft_off {
        Some(s) => crate::log!(
            "Chat Computer kernel: turning off: sleep type {} by {}",
            s.typ_a,
            if s.sleep_control.is_some() { "the sleep control register" } else { "the PM1 control registers" }
        ),
        None => crate::log!("Chat Computer kernel: turning off: the firmware doesn't say how"),
    }
}

/// Whether this PC can be turned off (the firmware said how).
pub fn can_turn_off() -> bool {
    SOFT_OFF.lock().is_some()
}

/// Once the firmware's programs have run: S5's sleep types as its \_S5 gives them (when the DSDT's bytes didn't say,
/// or said otherwise: the programs are the firmware's own word).
pub fn use_firmware_s5(acpi: &Acpi) {
    let Some((typ_a, typ_b)) = crate::firmware::s5_types() else { return };
    let mut soft_off = SOFT_OFF.lock();
    match soft_off.as_mut() {
        Some(s) => {
            if (s.typ_a, s.typ_b) != (typ_a, typ_b) {
                crate::log!("Chat Computer kernel: turning off: sleep type {typ_a}, as the firmware's \\_S5 says (its DSDT's bytes said {})", s.typ_a);
            }
            s.typ_a = typ_a;
            s.typ_b = typ_b;
        }
        None => {
            if let Some(e) = acpi.events
                && e.pm1a_control != 0
                && !e.hardware_reduced
            {
                *soft_off = Some(SoftOff {
                    pm1a: e.pm1a_control,
                    pm1b: e.pm1b_control,
                    sleep_control: None,
                    typ_a,
                    typ_b,
                    smi_cmd: e.smi_cmd,
                    acpi_enable: e.acpi_enable,
                });
                crate::log!("Chat Computer kernel: turning off: sleep type {typ_a}, as the firmware's \\_S5 says");
            }
        }
    }
}

/// Restarts the PC in `ms` milliseconds (from the main loop, `due`).
pub fn restart_in(ms: u64) {
    RESTART_AT.store(time::uptime_ms() + ms.max(1), Ordering::Relaxed);
}

/// Turns the PC off in `ms` milliseconds (from the main loop, `due`).
pub fn turn_off_in(ms: u64) {
    OFF_AT.store(time::uptime_ms() + ms.max(1), Ordering::Relaxed);
}

/// From the main loop: restarts, or turns off, when it's time.
pub fn due() {
    let now = time::uptime_ms();
    let at = RESTART_AT.load(Ordering::Relaxed);
    if at != 0 && now >= at {
        restart();
    }
    let off = OFF_AT.load(Ordering::Relaxed);
    if off != 0 && now >= off {
        turn_off();
    }
}

/// Turns the PC off. If it's still on 3 seconds later, the screen says so (and how to turn it off by hand).
pub fn turn_off() -> ! {
    crate::log!("Chat Computer kernel: turning off");
    crate::journal::save();
    cpu::disable_interrupts();
    crate::cores::park();
    crate::keep::shut_down();
    // The firmware's own preparation, which can say S5's sleep types too.
    let types = crate::firmware::before_turning_off();
    let soft_off = *SOFT_OFF.lock();
    if let Some(mut s) = soft_off {
        if let Some((a, b)) = types {
            (s.typ_a, s.typ_b) = (a, b);
        }
        unsafe { enter_s5(&s) };
        time::delay_ms(3000);
    }
    crate::log!("Chat Computer kernel: the PC didn't turn off");
    crate::chat::show_stop(
        "the PC didn't turn off",
        format_args!("the firmware's way to turn it off (ACPI's S5) didn't work on this PC"),
    );
    cpu::stop()
}

/// ACPI's sleep state S5: its sleep type in the PM1 control registers' SLP_TYP field, then again with SLP_EN (as ACPI
/// says, in two writes); or, on a hardware-reduced PC, both at once in the sleep control register.
unsafe fn enter_s5(s: &SoftOff) {
    const SLP_TYP: u16 = 7 << 10;
    const SLP_EN: u16 = 1 << 13;
    const SCI_EN: u16 = 1;
    if let Some((space, address)) = s.sleep_control {
        let value = (s.typ_a & 7) << 2 | 1 << 5;
        match space {
            1 if address <= 0xFFFF => unsafe { cpu::outb(address as u16, value) },
            0 => {
                let at = paging::map_device(address & !0xFFF, 0x1000, Cache::Uncached) + (address & 0xFFF);
                unsafe { core::ptr::write_volatile(at as *mut u8, value) };
            }
            _ => {}
        }
        return;
    }
    // In ACPI mode (SCI_EN), which the firmware is asked for if it hasn't switched to it.
    if unsafe { cpu::inw(s.pm1a) } & SCI_EN == 0 && s.smi_cmd != 0 && s.acpi_enable != 0 {
        unsafe { cpu::outb(s.smi_cmd, s.acpi_enable) };
        for _ in 0..300 {
            if unsafe { cpu::inw(s.pm1a) } & SCI_EN != 0 {
                break;
            }
            time::delay_ms(10);
        }
    }
    let base = unsafe { cpu::inw(s.pm1a) } & !(SLP_TYP | SLP_EN);
    let (a, b) = (base | (s.typ_a as u16 & 7) << 10, base | (s.typ_b as u16 & 7) << 10);
    unsafe {
        cpu::outw(s.pm1a, a);
        if s.pm1b != 0 {
            cpu::outw(s.pm1b, b);
        }
    }
    cpu::wbinvd();
    unsafe {
        cpu::outw(s.pm1a, a | SLP_EN);
        if s.pm1b != 0 {
            cpu::outw(s.pm1b, b | SLP_EN);
        }
    }
}

pub fn restart() -> ! {
    crate::log!("Chat Computer kernel: restarting");
    crate::journal::save();
    cpu::disable_interrupts();
    crate::cores::park();
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
