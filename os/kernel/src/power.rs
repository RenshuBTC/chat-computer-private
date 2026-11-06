//! Restarting and turning off the PC. Chat Computer's own system is started once from Windows' power-on menu, so a
//! restart brings the PC back to what it normally starts (Windows), and so does turning it on again after it's off.
//! A restart into Chat Computer again asks the firmware first to start the power-on entry it started from next
//! (BootNext = BootCurrent), through the firmware's runtime services: their code and data mapped where they are (at
//! their physical addresses: Chat Computer never moves them), the call made with interrupts off, just before the
//! reset. If the firmware faults in it, the PC is reset anyway (to what it normally starts).
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
use bootinfo::{BootInfo, MemoryRegion, MEMORY_RUNTIME, NO_BOOT_ENTRY};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

static RESET: SpinLock<Option<ResetRegister>> = SpinLock::new(None);
static SOFT_OFF: SpinLock<Option<SoftOff>> = SpinLock::new(None);
/// When to restart, and when to turn off (uptime in ms; 0: not asked).
static RESTART_AT: AtomicU64 = AtomicU64::new(0);
static OFF_AT: AtomicU64 = AtomicU64::new(0);
/// The firmware's runtime services table (physical address; 0: none), the power-on entry the PC started from, the
/// firmware's memory its runtime services use ((start, bytes, code or device or data)), and whether the restart
/// asked for starts that entry again.
static RUNTIME: AtomicU64 = AtomicU64::new(0);
static BOOT_CURRENT: AtomicU32 = AtomicU32::new(NO_BOOT_ENTRY);
const MOST_RUNTIME_REGIONS: usize = 64;
static RUNTIME_REGIONS: SpinLock<[(u64, u64, u8); MOST_RUNTIME_REGIONS]> = SpinLock::new([(0, 0, 0); MOST_RUNTIME_REGIONS]);
const REGION_DATA: u8 = 1;
const REGION_CODE: u8 = 2;
const REGION_DEVICE: u8 = 3;
static HERE_NEXT: AtomicBool = AtomicBool::new(false);
/// The firmware's runtime code is running (a fault in it resets the PC: `in_firmware`).
static IN_FIRMWARE: AtomicBool = AtomicBool::new(false);

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

/// What the start-up stage found of the firmware: its runtime services, the power-on entry the PC started from, and
/// the memory the runtime services use (from the firmware's memory map).
pub fn init_firmware(info: &BootInfo) {
    let mut regions = RUNTIME_REGIONS.lock();
    let mut n = 0;
    let map: &[MemoryRegion] = unsafe { info.memory_map() };
    for r in map.iter().filter(|r| r.attribute & MEMORY_RUNTIME != 0 && r.pages > 0) {
        if n == MOST_RUNTIME_REGIONS {
            break;
        }
        let kind = match r.kind {
            bootinfo::memtype::RUNTIME_SERVICES_CODE => REGION_CODE,
            bootinfo::memtype::MMIO | bootinfo::memtype::MMIO_PORT_SPACE => REGION_DEVICE,
            _ => REGION_DATA,
        };
        regions[n] = (r.start, r.pages * bootinfo::PAGE_SIZE, kind);
        n += 1;
    }
    drop(regions);
    RUNTIME.store(info.runtime_services, Ordering::Relaxed);
    BOOT_CURRENT.store(info.boot_current, Ordering::Relaxed);
    if info.boot_current != NO_BOOT_ENTRY {
        crate::log!(
            "Chat Computer kernel: started from the power-on menu's entry Boot{:04X}; the firmware's runtime services: {}",
            info.boot_current,
            if info.runtime_services != 0 && n > 0 { "there" } else { "none" }
        );
    }
}

/// Whether a restart can start Chat Computer again (the firmware said which entry started it, and has runtime
/// services to ask with).
pub fn can_restart_here() -> bool {
    RUNTIME.load(Ordering::Relaxed) != 0 && BOOT_CURRENT.load(Ordering::Relaxed) != NO_BOOT_ENTRY
}

/// Restarts the PC in `ms` milliseconds into Chat Computer again (if it can: `can_restart_here`).
pub fn restart_here_in(ms: u64) {
    HERE_NEXT.store(can_restart_here(), Ordering::Relaxed);
    restart_in(ms);
}

/// Whether the firmware's runtime code is running now: a fault then is the firmware's, and the PC is reset.
pub fn in_firmware() -> bool {
    IN_FIRMWARE.load(Ordering::Relaxed)
}

/// Asks the firmware to start the power-on entry Chat Computer started from, next time (BootNext): through its runtime
/// services' SetVariable, their code and data mapped where they are. Returns its status (0: done).
fn start_here_next() -> u64 {
    let (rt, entry) = (RUNTIME.load(Ordering::Relaxed), BOOT_CURRENT.load(Ordering::Relaxed));
    if rt == 0 || entry == NO_BOOT_ENTRY {
        return u64::MAX;
    }
    let regions = *RUNTIME_REGIONS.lock();
    for &(start, bytes, kind) in regions.iter().filter(|r| r.1 > 0) {
        paging::map_identity(start, bytes, kind == REGION_CODE, kind == REGION_DEVICE);
    }
    // EFI_RUNTIME_SERVICES: SetVariable is at 88 (its header, then nine functions before it).
    let set_variable = unsafe { core::ptr::read_volatile((rt + 88) as *const u64) };
    if set_variable == 0 {
        return u64::MAX;
    }
    type SetVariable = unsafe extern "efiapi" fn(*const u16, *const [u8; 16], u32, usize, *const u8) -> u64;
    let f: SetVariable = unsafe { core::mem::transmute::<u64, SetVariable>(set_variable) };
    const NAME: [u16; 9] = [b'B' as u16, b'o' as u16, b'o' as u16, b't' as u16, b'N' as u16, b'e' as u16, b'x' as u16, b't' as u16, 0];
    // EFI_GLOBAL_VARIABLE (8be4df61-93ca-11d2-aa0d-00e098032b8c), as an EFI_GUID lies in memory.
    const GLOBAL: [u8; 16] = [0x61, 0xdf, 0xe4, 0x8b, 0xca, 0x93, 0xd2, 0x11, 0xaa, 0x0d, 0x00, 0xe0, 0x98, 0x03, 0x2b, 0x8c];
    let value = (entry as u16).to_le_bytes();
    IN_FIRMWARE.store(true, Ordering::SeqCst);
    // (Non-volatile, and seen at boot and at runtime: 7, as the firmware keeps BootNext.)
    let status = unsafe { f(NAME.as_ptr(), &GLOBAL, 7, value.len(), value.as_ptr()) };
    IN_FIRMWARE.store(false, Ordering::SeqCst);
    status
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
    let here = HERE_NEXT.load(Ordering::Relaxed);
    crate::log!("Chat Computer kernel: restarting{}", if here { ", into Chat Computer again" } else { "" });
    crate::journal::save();
    cpu::disable_interrupts();
    crate::cores::park();
    crate::keep::shut_down();
    if here {
        let status = start_here_next();
        crate::serial::write_anyway(format_args!(
            "Chat Computer kernel: the firmware {} to start Boot{:04X} next{}\n",
            if status == 0 { "was asked" } else { "couldn't be asked" },
            BOOT_CURRENT.load(Ordering::Relaxed),
            if status == 0 { alloc::string::String::new() } else { alloc::format!(" (status {status:#x})") }
        ));
    }
    reset()
}

/// The PC reset: by the reset register the firmware names, else the chipset's reset control, else the keyboard
/// controller's reset line, else a fault the processor can't handle.
pub fn reset() -> ! {
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
