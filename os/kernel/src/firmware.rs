//! The firmware's own programs: the ACPI Machine Language in its DSDT and SSDTs, run by Chat Computer's own
//! interpreter (the aml crate), the way Windows' ACPI driver runs them. On most PCs only these programs know about the
//! battery, the AC adapter, the lid, the temperature, and a laptop's power button, because they talk to the embedded
//! controller (the small computer on the mainboard that charges the battery, turns the fans and reads the keyboard)
//! and to the chipset in each PC's own way. So, at start-up:
//!
//! * the PC is put in ACPI mode (as every operating system does), and the tables are loaded: the DSDT, then the SSDTs;
//! * the embedded controller's driver is set up (its ports from its device's _CRS) and the firmware told it's there
//!   (_REG); then the devices' _INI methods run, as ACPICA (Linux's interpreter) runs them, and PCI configuration
//!   space is announced (_REG) as ACPICA does;
//! * the firmware's events are watched: the power button (its fixed register bits, or a device the firmware notifies),
//!   the general-purpose events (GPEs) its _Lxx and _Exx methods handle, and the embedded controller's queries (its
//!   _Qxx methods). They're checked 20 times a second rather than by the system control interrupt, which is left off:
//!   checking the status bits is all that interrupt would lead to;
//! * the batteries (_BIX or _BIF, and _BST), AC adapters (_PSR), lids (_LID) and thermal zones (_TMP, _CRT) are read
//!   when the firmware says they changed (Notify), and every so often anyway.
//!
//! What comes of it: pressing the power button turns the PC off, the way Windows does by default on a desktop (with
//! the conversation kept); the battery and the rest are in the computer's status the model reads; a note says when the PC
//! is plugged in or out, when its battery is low, and when it's nearly empty or the PC too hot (it's turned off then,
//! as Windows would); and turning off runs the firmware's _TTS and _PTS first, as it expects.
//!
//! None of it can stop the PC: the interpreter gives errors, never panics, and gives up on a method that runs too long;
//! an embedded controller that doesn't answer is left alone for a while. The settings file can turn all of it off
//! (`firmware-programs = off`), for a PC whose firmware does something odd.

use crate::acpi::{Acpi, Events, Register};
use crate::paging::{self, Cache};
use crate::{chat, cpu, pci, power, time};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use aml::{Aml, Host, PciAddress, Resource, Space, Value};
use bootinfo::PHYS_OFFSET;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// The firmware's state. Only the kernel's main loop uses it, never an interrupt handler, so it needs no lock that
/// turns interrupts off (which a method's Sleep would keep off for its length): it's just never entered twice.
struct MainLoop<T> {
    busy: AtomicBool,
    value: UnsafeCell<Option<T>>,
}

unsafe impl<T> Sync for MainLoop<T> {}

impl<T> MainLoop<T> {
    const fn new() -> Self {
        MainLoop { busy: AtomicBool::new(false), value: UnsafeCell::new(None) }
    }

    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        if self.busy.swap(true, Ordering::Acquire) {
            return None;
        }
        let r = unsafe { (*self.value.get()).as_mut() }.map(f);
        self.busy.store(false, Ordering::Release);
        r
    }

    fn set(&self, value: T) {
        if !self.busy.swap(true, Ordering::Acquire) {
            unsafe { *self.value.get() = Some(value) };
            self.busy.store(false, Ordering::Release);
        }
    }
}

static FIRMWARE: MainLoop<Firmware> = MainLoop::new();
/// When the events are next checked (uptime, ms).
static NEXT_CHECK: AtomicU64 = AtomicU64::new(0);
/// S5's sleep types as the firmware's \_S5 gives them (bits 0-2 and 8-10; u32::MAX: not known).
static S5: AtomicU32 = AtomicU32::new(u32::MAX);

/// How often the events are checked, and the batteries, AC adapters, lids and temperatures read anyway (ms).
const CHECK_MS: u64 = 50;
const BATTERY_MS: u64 = 30_000;
const ADAPTER_MS: u64 = 10_000;
const LID_MS: u64 = 10_000;
const THERMAL_MS: u64 = 30_000;

/// The most steps and sleeping time one evaluation gets here (less than the interpreter's own default: the main loop
/// waits for it).
const MOST_STEPS: u64 = 16_000_000;
const MOST_SLEEP_US: u64 = 3_000_000;

/// PM1 status and enable bits (ACPI 6.5, 4.8.3.1).
const PWRBTN: u16 = 1 << 8;
/// Every fixed event's status bit: timer, bus master, global lock, power button, sleep button, real-time clock, wake.
const ALL_FIXED_STATUS: u16 = 0x8731;
/// PM1 control: ACPI mode (SCI_EN), and the global lock's release (GBL_RLS).
const SCI_EN: u16 = 1;
const GBL_RLS: u16 = 1 << 2;

/// The embedded controller's status bits and commands (ACPI 6.5, 12.2).
const OBF: u8 = 1;
const IBF: u8 = 2;
const SCI_EVT: u8 = 0x20;
const RD_EC: u8 = 0x80;
const WR_EC: u8 = 0x81;
const QR_EC: u8 = 0x84;

/// A register of the events' hardware, ready to read: an I/O port or mapped memory.
#[derive(Clone, Copy)]
enum Reg {
    Port(u16),
    Memory(u64),
}

impl Reg {
    fn of(r: Register, offset: u64) -> Reg {
        if r.memory {
            Reg::Memory(paging::map_device(r.address + offset, 2, Cache::Uncached))
        } else {
            Reg::Port((r.address + offset) as u16)
        }
    }
    fn read16(self) -> u16 {
        match self {
            Reg::Port(p) => unsafe { cpu::inw(p) },
            Reg::Memory(v) => unsafe { core::ptr::read_volatile(v as *const u16) },
        }
    }
    fn write16(self, value: u16) {
        match self {
            Reg::Port(p) => unsafe { cpu::outw(p, value) },
            Reg::Memory(v) => unsafe { core::ptr::write_volatile(v as *mut u16, value) },
        }
    }
    fn read8(self) -> u8 {
        match self {
            Reg::Port(p) => unsafe { cpu::inb(p) },
            Reg::Memory(v) => unsafe { core::ptr::read_volatile(v as *const u8) },
        }
    }
    fn write8(self, value: u8) {
        match self {
            Reg::Port(p) => unsafe { cpu::outb(p, value) },
            Reg::Memory(v) => unsafe { core::ptr::write_volatile(v as *mut u8, value) },
        }
    }
}

/// A block of general-purpose events: its status bytes and its enable bytes, and its first event's number.
struct GpeBlock {
    status: Vec<Reg>,
    enable: Vec<Reg>,
    first: u32,
}

/// A general-purpose event the firmware has a method for.
struct Gpe {
    number: u32,
    level: bool,
    method: usize,
    /// How many checks in a row have found it fired (its method doesn't end what sets it: after 10 seconds of that,
    /// it's switched off).
    streak: u32,
}

/// How many checks in a row an event may fire before it's switched off (10 seconds).
const MOST_STREAK: u32 = 200;

/// The embedded controller: its ports (data, and command/status), its device, whether the global lock guards it,
/// and how it's been answering.
struct Ec {
    data: u16,
    command: u16,
    node: usize,
    lock: bool,
    gpe: Option<u32>,
    failures: u32,
    resting_until: u64,
    said_resting: bool,
}

impl Ec {
    /// Waits (up to 100 ms) for the status to be `ready`.
    fn wait(&self, ready: impl Fn(u8) -> bool) -> bool {
        let end = time::now_us() + 100_000;
        loop {
            if ready(unsafe { cpu::inb(self.command) }) {
                return true;
            }
            if time::now_us() > end {
                return false;
            }
            time::delay_us(5);
        }
    }

    fn send(&self, port: u16, value: u8) -> bool {
        if !self.wait(|s| s & IBF == 0) {
            return false;
        }
        unsafe { cpu::outb(port, value) };
        true
    }

    fn receive(&self) -> Option<u8> {
        self.wait(|s| s & OBF != 0).then(|| unsafe { cpu::inb(self.data) })
    }

    /// One transaction: a byte left over from one that failed read and dropped first.
    fn transaction(&mut self, run: impl Fn(&Ec) -> Option<u8>) -> Option<u8> {
        let now = time::uptime_ms();
        if now < self.resting_until {
            return None;
        }
        if unsafe { cpu::inb(self.command) } & OBF != 0 {
            unsafe { cpu::inb(self.data) };
        }
        let r = run(self);
        if r.is_some() {
            self.failures = 0;
            self.said_resting = false;
        } else {
            self.failures += 1;
            if self.failures >= 3 {
                // Not answering: left alone for half a minute, so the firmware's methods fail fast meanwhile.
                self.resting_until = now + 30_000;
                self.failures = 0;
                if !self.said_resting {
                    crate::log!("Chat Computer kernel: firmware: the embedded controller isn't answering; trying again in 30 s");
                    self.said_resting = true;
                }
            }
        }
        r
    }

    fn read(&mut self, address: u8) -> Option<u8> {
        let (data, command) = (self.data, self.command);
        self.transaction(|ec| (ec.send(command, RD_EC) && ec.send(data, address)).then(|| ec.receive()).flatten())
    }

    fn write(&mut self, address: u8, value: u8) -> bool {
        let (data, command) = (self.data, self.command);
        self.transaction(|ec| {
            (ec.send(command, WR_EC) && ec.send(data, address) && ec.send(data, value) && ec.wait(|s| s & IBF == 0)).then_some(0)
        })
        .is_some()
    }

    fn query(&mut self) -> Option<u8> {
        let command = self.command;
        self.transaction(|ec| ec.send(command, QR_EC).then(|| ec.receive()).flatten())
    }
}

/// The machine, as the firmware's programs reach it.
struct Machine {
    ec: Option<Ec>,
    /// What the programs notified (Notify(object, value)), by path.
    notified: Vec<(String, u64)>,
    /// Device memory mapped for them, by its 4 KiB page.
    mapped: BTreeMap<u64, u64>,
    /// The global lock (in the FACS: its virtual address, 0: none), how many times it's held, and the PM1 control
    /// ports (to tell the firmware it's released).
    lock: u64,
    held: u32,
    pm1_control: (u16, u16),
    /// The tables, by signature (LoadTable), and how many lines of the firmware's Debug output have been logged.
    tables: Vec<([u8; 4], u64)>,
    said: u32,
}

impl Machine {
    /// A virtual address for `bytes` at physical `address`: RAM (the firmware's own NVS memory, mostly) where the
    /// kernel reaches all RAM; device memory mapped uncached, a page (and the next) at a time.
    fn memory(&mut self, address: u64, bytes: u64) -> Option<u64> {
        if address.checked_add(bytes)? > 1 << 52 {
            return None;
        }
        if paging::in_ram(address, bytes) {
            return Some(PHYS_OFFSET + address);
        }
        let page = address & !0xFFF;
        if self.mapped.len() > 4096 && !self.mapped.contains_key(&page) {
            return None;
        }
        let at = *self.mapped.entry(page).or_insert_with(|| paging::map_device(page, 0x2000, Cache::Uncached));
        Some(at + (address - page))
    }

    fn take_lock(&mut self, timeout_ms: u64) -> bool {
        if self.held > 0 || self.lock == 0 {
            self.held += 1;
            return true;
        }
        let lock = unsafe { &*(self.lock as *const AtomicU32) };
        let end = time::now_us() + timeout_ms.min(1000) * 1000;
        loop {
            // ACPI 6.5, 5.2.10.1: set "owned", and "pending" if it was owned already; it's ours if it wasn't.
            let mut old = lock.load(Ordering::Acquire);
            loop {
                let new = (old & !3) | 2 | (old >> 1 & 1);
                match lock.compare_exchange(old, new, Ordering::AcqRel, Ordering::Acquire) {
                    Ok(_) => break,
                    Err(now) => old = now,
                }
            }
            if old & 2 == 0 {
                self.held = 1;
                return true;
            }
            if time::now_us() >= end {
                return false;
            }
            time::delay_us(50);
        }
    }

    fn give_lock(&mut self) {
        if self.held == 0 {
            return;
        }
        self.held -= 1;
        if self.held > 0 || self.lock == 0 {
            return;
        }
        let lock = unsafe { &*(self.lock as *const AtomicU32) };
        let mut old = lock.load(Ordering::Acquire);
        loop {
            match lock.compare_exchange(old, old & !3, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => break,
                Err(now) => old = now,
            }
        }
        // The firmware was waiting for it: tell it.
        if old & 1 != 0 {
            for port in [self.pm1_control.0, self.pm1_control.1] {
                if port != 0 {
                    unsafe { cpu::outw(port, cpu::inw(port) | GBL_RLS) };
                }
            }
        }
    }

    fn ec_read(&mut self, address: u64, bits: u8) -> Option<u64> {
        let lock = self.ec.as_ref()?.lock;
        if lock && !self.take_lock(1000) {
            return None;
        }
        let mut value = 0u64;
        let mut ok = true;
        for i in 0..bits as u64 / 8 {
            let a = address + i;
            match (a < 256).then(|| self.ec.as_mut().and_then(|ec| ec.read(a as u8))).flatten() {
                Some(b) => value |= (b as u64) << (8 * i),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if lock {
            self.give_lock();
        }
        ok.then_some(value)
    }

    fn ec_write(&mut self, address: u64, bits: u8, value: u64) -> bool {
        let Some(lock) = self.ec.as_ref().map(|ec| ec.lock) else { return false };
        if lock && !self.take_lock(1000) {
            return false;
        }
        let mut ok = true;
        for i in 0..bits as u64 / 8 {
            let a = address + i;
            if a >= 256 || !self.ec.as_mut().is_some_and(|ec| ec.write(a as u8, (value >> (8 * i)) as u8)) {
                ok = false;
                break;
            }
        }
        if lock {
            self.give_lock();
        }
        ok
    }
}

impl Host for Machine {
    fn read(&mut self, space: Space, address: u64, bits: u8) -> Option<u64> {
        let bytes = (bits / 8).max(1) as u64;
        match space {
            Space::Memory => {
                let at = self.memory(address, bytes)?;
                Some(unsafe {
                    match bits {
                        8 => core::ptr::read_volatile(at as *const u8) as u64,
                        16 => core::ptr::read_volatile(at as *const u16) as u64,
                        32 => core::ptr::read_volatile(at as *const u32) as u64,
                        _ => core::ptr::read_volatile(at as *const u64),
                    }
                })
            }
            Space::Io => {
                let port = u16::try_from(address).ok().filter(|p| (*p as u64) + bytes <= 0x1_0000)?;
                Some(unsafe {
                    match bits {
                        8 => cpu::inb(port) as u64,
                        16 => cpu::inw(port) as u64,
                        32 => cpu::inl(port) as u64,
                        _ => cpu::inl(port) as u64 | (cpu::inl(port + 4) as u64) << 32,
                    }
                })
            }
            Space::EmbeddedControl => self.ec_read(address, bits),
            Space::Cmos => {
                // The real-time clock's memory: an index port and a data port (0x70/0x71; the upper bank 0x72/0x73).
                let mut value = 0u64;
                for i in 0..bytes {
                    let a = address + i;
                    if a > 0xFF {
                        return None;
                    }
                    // (Bit 7 of 0x70's index masks the NMI, so the lower bank's are below 0x80; 0x72 ignores it.)
                    let (index, data) = if a < 0x80 { (0x70, 0x71) } else { (0x72, 0x73) };
                    value |= unsafe {
                        cpu::outb(index, a as u8);
                        cpu::inb(data) as u64
                    } << (8 * i);
                }
                Some(value)
            }
            _ => None,
        }
    }

    fn write(&mut self, space: Space, address: u64, bits: u8, value: u64) -> bool {
        let bytes = (bits / 8).max(1) as u64;
        match space {
            Space::Memory => {
                let Some(at) = self.memory(address, bytes) else { return false };
                unsafe {
                    match bits {
                        8 => core::ptr::write_volatile(at as *mut u8, value as u8),
                        16 => core::ptr::write_volatile(at as *mut u16, value as u16),
                        32 => core::ptr::write_volatile(at as *mut u32, value as u32),
                        _ => core::ptr::write_volatile(at as *mut u64, value),
                    }
                }
                true
            }
            Space::Io => {
                let Some(port) = u16::try_from(address).ok().filter(|p| (*p as u64) + bytes <= 0x1_0000) else { return false };
                unsafe {
                    match bits {
                        8 => cpu::outb(port, value as u8),
                        16 => cpu::outw(port, value as u16),
                        32 => cpu::outl(port, value as u32),
                        _ => {
                            cpu::outl(port, value as u32);
                            cpu::outl(port + 4, (value >> 32) as u32);
                        }
                    }
                }
                true
            }
            Space::EmbeddedControl => self.ec_write(address, bits, value),
            Space::Cmos => {
                for i in 0..bytes {
                    let a = address + i;
                    if a > 0xFF {
                        return false;
                    }
                    let (index, data) = if a < 0x80 { (0x70, 0x71) } else { (0x72, 0x73) };
                    unsafe {
                        cpu::outb(index, a as u8);
                        cpu::outb(data, (value >> (8 * i)) as u8);
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn read_pci(&mut self, at: PciAddress, offset: u16, bits: u8) -> Option<u64> {
        if at.segment != 0 || at.device > 31 || at.function > 7 || offset as u32 + (bits as u32 / 8) > 4096 {
            return None;
        }
        let a = pci::Address { bus: at.bus, device: at.device, function: at.function };
        Some(match bits {
            8 => a.read8(offset) as u64,
            16 => a.read16(offset) as u64,
            32 => a.read32(offset) as u64,
            _ => a.read32(offset) as u64 | (a.read32(offset + 4) as u64) << 32,
        })
    }

    fn write_pci(&mut self, at: PciAddress, offset: u16, bits: u8, value: u64) -> bool {
        if at.segment != 0 || at.device > 31 || at.function > 7 || offset as u32 + (bits as u32 / 8) > 4096 {
            return false;
        }
        let a = pci::Address { bus: at.bus, device: at.device, function: at.function };
        match bits {
            8 => a.write8(offset, value as u8),
            16 => a.write16(offset, value as u16),
            32 => a.write32(offset, value as u32),
            _ => {
                a.write32(offset, value as u32);
                a.write32(offset + 4, (value >> 32) as u32);
            }
        }
        true
    }

    fn sleep(&mut self, ms: u64) {
        time::delay_ms(ms);
    }

    fn stall(&mut self, us: u64) {
        time::delay_us(us);
    }

    fn timer(&mut self) -> u64 {
        time::now_us().saturating_mul(10)
    }

    fn notify(&mut self, path: &str, value: u64) {
        if self.notified.len() < 256 {
            self.notified.push((String::from(path), value));
        }
    }

    fn debug(&mut self, text: &str) {
        if self.said < 50 {
            self.said += 1;
            crate::log!("Chat Computer kernel: firmware says: {}", text.chars().take(200).collect::<String>());
        }
    }

    fn table(&mut self, signature: &[u8; 4], oem: &[u8], oem_table: &[u8]) -> Option<Vec<u8>> {
        for &(sig, phys) in &self.tables {
            if sig != *signature {
                continue;
            }
            let head = unsafe { core::slice::from_raw_parts(paging::map_firmware_table(phys, 36) as *const u8, 36) };
            let matches = |field: &[u8], want: &[u8]| want.is_empty() || field.iter().zip(want).all(|(a, b)| a == b);
            if !matches(&head[10..16], oem) || !matches(&head[16..24], oem_table) {
                continue;
            }
            let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as u64;
            if !(36..=4 << 20).contains(&len) {
                continue;
            }
            let at = paging::map_firmware_table(phys, len);
            return Some(unsafe { core::slice::from_raw_parts(at as *const u8, len as usize) }.to_vec());
        }
        None
    }

    fn acquire_global_lock(&mut self, timeout_ms: u16) -> bool {
        self.take_lock(timeout_ms as u64)
    }

    fn release_global_lock(&mut self) {
        self.give_lock();
    }
}

/// A battery: what it is (_BIX or _BIF), and how it is (_BST).
struct Battery {
    node: usize,
    path: String,
    present: bool,
    /// Capacities in mWh or mAh (the same unit in both, and in _BST's), as the firmware gives them.
    design: Option<u32>,
    full: Option<u32>,
    state: Option<BatteryState>,
}

#[derive(Clone, Copy)]
struct BatteryState {
    /// Bit 0 discharging, 1 charging, 2 critical.
    flags: u32,
    rate: Option<u32>,
    remaining: Option<u32>,
}

impl Battery {
    /// Its charge, in percent of what it holds full.
    fn percent(&self) -> Option<u32> {
        let remaining = self.state?.remaining?;
        let full = self.full.filter(|&f| f > 0).or(self.design.filter(|&d| d > 0))?;
        Some(((remaining as u64 * 100 + full as u64 / 2) / full as u64).min(100) as u32)
    }

    /// How long until it's empty (discharging) or full (charging), in minutes.
    fn minutes(&self) -> Option<u64> {
        let s = self.state?;
        let rate = s.rate.filter(|&r| r > 0)? as u64;
        let remaining = s.remaining? as u64;
        if s.flags & 1 != 0 {
            Some(remaining * 60 / rate)
        } else if s.flags & 2 != 0 {
            let full = self.full.or(self.design)? as u64;
            Some(full.saturating_sub(remaining) * 60 / rate)
        } else {
            None
        }
    }

    fn charging(&self) -> bool {
        self.state.is_some_and(|s| s.flags & 2 != 0)
    }

    fn discharging(&self) -> bool {
        self.state.is_some_and(|s| s.flags & 1 != 0)
    }

    fn critical(&self) -> bool {
        self.state.is_some_and(|s| s.flags & 4 != 0)
    }
}

/// A thermal zone: its critical temperature (_CRT), and its temperature (_TMP), in tenths of a kelvin.
struct Zone {
    node: usize,
    path: String,
    critical: Option<u32>,
    temperature: Option<u32>,
    /// How many readings in a row have been at or past the critical temperature.
    over: u32,
}

/// What the main loop does about what happened: a note in the conversation, turning off.
enum Said {
    Note(String),
    TurnOff(u64),
}

struct Firmware {
    aml: Aml,
    machine: Machine,
    pm1: Option<(Reg, Reg, Option<(Reg, Reg)>)>,
    fixed_power_button: bool,
    gpe_blocks: Vec<GpeBlock>,
    gpes: Vec<Gpe>,
    batteries: Vec<Battery>,
    adapters: Vec<(usize, String)>,
    lids: Vec<(usize, String)>,
    buttons: Vec<String>,
    zones: Vec<Zone>,
    plugged: Option<bool>,
    lid_open: Option<bool>,
    /// When each kind is next read anyway (uptime, ms).
    next_battery: u64,
    next_adapter: u64,
    next_lid: u64,
    next_thermal: u64,
    /// When the power button was last pressed, the last plugged-in-or-out note, whether the low battery's been said,
    /// and when the nearly empty battery turns the PC off.
    pressed_at: u64,
    plug_note_at: u64,
    low_said: bool,
    empty_at: Option<u64>,
    /// Notifications nothing here acts on, logged (each once).
    logged: Vec<(String, u64)>,
}

/// Ten times a temperature in kelvin, in degrees Celsius.
fn celsius(tenths_kelvin: u32) -> i64 {
    (tenths_kelvin as i64 - 2732 + 5).div_euclid(10)
}

impl Firmware {
    fn integer(&mut self, node: usize, name: &str) -> Option<u64> {
        match self.aml.evaluate_child(node, name, Vec::new(), &mut self.machine)? {
            Ok(Value::Integer(n)) => Some(n),
            _ => None,
        }
    }

    fn status(&mut self, node: usize) -> u64 {
        match self.aml.evaluate_child(node, "_STA", Vec::new(), &mut self.machine) {
            None => 0x1F,
            Some(Ok(Value::Integer(n))) => n,
            Some(_) => 0,
        }
    }

    /// A battery's information (_BIX, or _BIF on older firmware): its design and last full capacities.
    fn battery_info(&mut self, i: usize) {
        let node = self.batteries[i].node;
        let sta = self.status(node);
        self.batteries[i].present = sta & 0x10 != 0;
        if !self.batteries[i].present {
            self.batteries[i].state = None;
            return;
        }
        let known = |v: Option<&Value>| match v {
            Some(Value::Integer(n)) if *n != 0xFFFF_FFFF && *n < 0x8000_0000 => Some(*n as u32),
            _ => None,
        };
        let bix = self.aml.evaluate_child(node, "_BIX", Vec::new(), &mut self.machine);
        let items = match bix {
            Some(Ok(Value::Package(p))) if p.borrow().len() >= 4 => {
                let p = p.borrow();
                Some((p.get(2).cloned(), p.get(3).cloned()))
            }
            _ => match self.aml.evaluate_child(node, "_BIF", Vec::new(), &mut self.machine) {
                Some(Ok(Value::Package(p))) if p.borrow().len() >= 3 => {
                    let p = p.borrow();
                    Some((p.get(1).cloned(), p.get(2).cloned()))
                }
                _ => None,
            },
        };
        if let Some((design, full)) = items {
            let b = &mut self.batteries[i];
            b.design = known(design.as_ref());
            b.full = known(full.as_ref());
        }
    }

    /// A battery's state (_BST): charging or not, the rate, what's left.
    fn battery_state(&mut self, i: usize) {
        if !self.batteries[i].present {
            return;
        }
        let node = self.batteries[i].node;
        let known = |v: Option<&Value>| match v {
            Some(Value::Integer(n)) if *n != 0xFFFF_FFFF && *n < 0x8000_0000 => Some(*n as u32),
            _ => None,
        };
        if let Some(Ok(Value::Package(p))) = self.aml.evaluate_child(node, "_BST", Vec::new(), &mut self.machine) {
            let p = p.borrow();
            if p.len() >= 3 {
                self.batteries[i].state = Some(BatteryState {
                    flags: known(p.first()).unwrap_or(0),
                    rate: known(p.get(1)),
                    remaining: known(p.get(2)),
                });
            }
        }
    }

    fn read_adapters(&mut self) -> Option<bool> {
        let nodes: Vec<usize> = self.adapters.iter().map(|a| a.0).collect();
        let mut any = None;
        for node in nodes {
            if let Some(online) = self.integer(node, "_PSR") {
                any = Some(any.unwrap_or(false) || online != 0);
            }
        }
        any
    }

    fn read_lids(&mut self) -> Option<bool> {
        let nodes: Vec<usize> = self.lids.iter().map(|l| l.0).collect();
        nodes.into_iter().find_map(|node| self.integer(node, "_LID")).map(|v| v != 0)
    }

    fn read_zones(&mut self) {
        for i in 0..self.zones.len() {
            let node = self.zones[i].node;
            let plausible = |t: u64| (2732..=4232).contains(&t).then_some(t as u32);
            self.zones[i].temperature = self.integer(node, "_TMP").and_then(plausible);
            if self.zones[i].critical.is_none() {
                self.zones[i].critical = self.integer(node, "_CRT").and_then(|t| (3132..=4232).contains(&t).then_some(t as u32));
            }
        }
    }

    /// Reads the batteries and adapters again, and says what changed that matters.
    fn power_changed(&mut self, said: &mut Vec<Said>, now: u64, info: bool) {
        for i in 0..self.batteries.len() {
            if info {
                self.battery_info(i);
            }
            self.battery_state(i);
        }
        let plugged = self.read_adapters().or_else(|| {
            // No adapter device: a battery charging or full and not discharging means plugged in.
            self.batteries.iter().find(|b| b.state.is_some()).map(|b| !b.discharging())
        });
        let battery = self.batteries.iter().find(|b| b.present).map(|b| (b.percent(), b.minutes(), b.charging(), b.discharging(), b.critical()));
        if let (Some(was), Some(now_plugged)) = (self.plugged, plugged)
            && was != now_plugged
        {
            crate::log!("Chat Computer kernel: firmware: {}", if now_plugged { "plugged in" } else { "unplugged" });
            if now.saturating_sub(self.plug_note_at) >= 30_000 || self.plug_note_at == 0 {
                self.plug_note_at = now;
                let charge = battery.and_then(|b| b.0).map(|p| format!("{p}%")).unwrap_or_default();
                said.push(Said::Note(match (now_plugged, charge.is_empty()) {
                    (true, true) => String::from("Plugged in."),
                    (true, false) => format!("Plugged in: the battery is at {charge}."),
                    (false, true) => String::from("Unplugged: on battery power."),
                    (false, false) => format!("Unplugged: on battery power, {charge}."),
                }));
            }
            if now_plugged {
                if self.empty_at.take().is_some() {
                    said.push(Said::Note(String::from("Plugged in: the PC stays on.")));
                }
                self.low_said = false;
            }
        }
        self.plugged = plugged.or(self.plugged);
        // A low battery is said once a discharge; nearly empty, the PC is turned off in a minute unless plugged in.
        if let Some((Some(percent), _, _, true, critical)) = battery {
            if plugged != Some(true) {
                if (percent <= 5 || critical) && self.empty_at.is_none() {
                    self.empty_at = Some(now + 60_000);
                    said.push(Said::Note(format!(
                        "The battery is nearly empty ({percent}%): Chat Computer turns the PC off in a minute unless it's plugged in."
                    )));
                } else if percent <= 10 && !self.low_said {
                    self.low_said = true;
                    said.push(Said::Note(format!("The battery is low ({percent}%): plug the PC in.")));
                }
            }
        } else if let Some((_, _, true, _, _)) = battery {
            self.low_said = false;
        }
    }

    /// Something the firmware notified: what it's about, read again.
    fn notified(&mut self, said: &mut Vec<Said>, now: u64) {
        for _ in 0..8 {
            let notes = core::mem::take(&mut self.machine.notified);
            if notes.is_empty() {
                break;
            }
            let mut power = false;
            let mut info = false;
            for (path, value) in notes {
                if self.batteries.iter().any(|b| b.path == path) {
                    power = true;
                    info |= value == 0x81;
                } else if self.adapters.iter().any(|a| a.1 == path) {
                    power = true;
                } else if self.lids.iter().any(|l| l.1 == path) {
                    self.lid_changed();
                } else if self.buttons.iter().any(|b| *b == path) && value == 0x80 {
                    self.pressed(said, now);
                } else if self.zones.iter().any(|z| z.path == path) {
                    self.read_zones();
                } else if self.logged.len() < 64 && !self.logged.iter().any(|(p, v)| *p == path && *v == value) {
                    crate::log!("Chat Computer kernel: firmware: notified {path} ({value:#x})");
                    self.logged.push((path, value));
                }
            }
            if power {
                self.power_changed(said, now, info);
            }
        }
    }

    fn lid_changed(&mut self) {
        let open = self.read_lids();
        if open.is_some() && self.lid_open.is_some() && open != self.lid_open {
            crate::log!("Chat Computer kernel: firmware: lid {}", if open == Some(true) { "open" } else { "closed" });
        }
        self.lid_open = open.or(self.lid_open);
    }

    /// The power button: the PC turned off (twice within a second counts once).
    fn pressed(&mut self, said: &mut Vec<Said>, now: u64) {
        if self.pressed_at != 0 && now.saturating_sub(self.pressed_at) < 1000 {
            return;
        }
        self.pressed_at = now;
        crate::log!("Chat Computer kernel: firmware: the power button was pressed");
        said.push(Said::Note(String::from("The power button was pressed: Chat Computer is turning the PC off.")));
        said.push(Said::TurnOff(2000));
    }

    /// The embedded controller's queued events: each one's _Qxx method run, until it has none (16 at most at once).
    fn queries(&mut self) {
        for _ in 0..16 {
            let Some(ec) = self.machine.ec.as_mut() else { return };
            let node = ec.node;
            let q = ec.query();
            match q {
                None | Some(0) => return,
                Some(q) => {
                    let name = format!("_Q{q:02X}");
                    match self.aml.evaluate_child(node, &name, Vec::new(), &mut self.machine) {
                        Some(Err(e)) => crate::log!("Chat Computer kernel: firmware: {name}: {e:?}"),
                        None if self.logged.len() < 64 => {
                            let key = (name.clone(), 0);
                            if !self.logged.contains(&key) {
                                crate::log!("Chat Computer kernel: firmware: the embedded controller's event {q:#x} has no method");
                                self.logged.push(key);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Checks the events: the fixed power button, the general-purpose events, the embedded controller's.
    fn check(&mut self, now: u64) -> Vec<Said> {
        let mut said = Vec::new();
        if let Some((status_a, _, b)) = self.pm1
            && self.fixed_power_button
        {
            let status = status_a.read16() | b.map_or(0, |(s, _)| s.read16());
            if status & PWRBTN != 0 {
                status_a.write16(PWRBTN);
                if let Some((s, _)) = b {
                    s.write16(PWRBTN);
                }
                self.pressed(&mut said, now);
            }
        }
        let ec_gpe = self.machine.ec.as_ref().and_then(|ec| ec.gpe);
        let mut ec_event = false;
        let mut fired_now: Vec<u32> = Vec::new();
        for bi in 0..self.gpe_blocks.len() {
            for byte in 0..self.gpe_blocks[bi].status.len() {
                let (status, enable) = (self.gpe_blocks[bi].status[byte], self.gpe_blocks[bi].enable[byte]);
                let fired = status.read8() & enable.read8();
                if fired == 0 {
                    continue;
                }
                for bit in 0..8 {
                    if fired & 1 << bit == 0 {
                        continue;
                    }
                    let number = self.gpe_blocks[bi].first + byte as u32 * 8 + bit;
                    if Some(number) == ec_gpe {
                        status.write8(1 << bit);
                        ec_event = true;
                        continue;
                    }
                    let Some(g) = self.gpes.iter_mut().find(|g| g.number == number) else {
                        status.write8(1 << bit);
                        continue;
                    };
                    fired_now.push(number);
                    g.streak += 1;
                    if g.streak > MOST_STREAK {
                        enable.write8(enable.read8() & !(1 << bit));
                        status.write8(1 << bit);
                        crate::log!("Chat Computer kernel: firmware: event {number:#x} keeps firing: switched off");
                        continue;
                    }
                    let (level, method) = (g.level, g.method);
                    // Edge-triggered: cleared, then handled; level-triggered: handled (which ends what set it), then
                    // cleared.
                    if !level {
                        status.write8(1 << bit);
                    }
                    if let Err(e) = self.aml.evaluate_node(method, Vec::new(), &mut self.machine) {
                        crate::log!("Chat Computer kernel: firmware: event {number:#x}'s method: {e:?}");
                    }
                    if level {
                        status.write8(1 << bit);
                    }
                }
            }
        }
        for g in &mut self.gpes {
            if !fired_now.contains(&g.number) {
                g.streak = 0;
            }
        }
        // The embedded controller's events: by its GPE, or its status's own bit (in case its GPE isn't one here).
        if let Some(ec) = self.machine.ec.as_ref()
            && (ec_event || (now >= ec.resting_until && unsafe { cpu::inb(ec.command) } & SCI_EVT != 0))
        {
            self.queries();
        }
        self.notified(&mut said, now);
        if now >= self.next_adapter || now >= self.next_battery {
            let info = now >= self.next_battery && self.batteries.iter().any(|b| b.full.is_none());
            if now >= self.next_battery {
                self.next_battery = now + BATTERY_MS;
            }
            self.next_adapter = now + ADAPTER_MS;
            self.power_changed(&mut said, now, info);
        }
        if now >= self.next_lid {
            self.next_lid = now + LID_MS;
            self.lid_changed();
        }
        if now >= self.next_thermal && !self.zones.is_empty() {
            self.next_thermal = now + THERMAL_MS;
            self.read_zones();
            for z in &mut self.zones {
                match (z.temperature, z.critical) {
                    (Some(t), Some(c)) if t >= c => z.over += 1,
                    _ => z.over = 0,
                }
                // Twice in a row, half a minute apart: the PC is turned off, as Windows does at the critical
                // temperature.
                if z.over == 2 {
                    let (t, c) = (z.temperature.unwrap_or(0), z.critical.unwrap_or(0));
                    crate::log!("Chat Computer kernel: firmware: {} is at {} °C, its critical temperature {} °C", z.path, celsius(t), celsius(c));
                    said.push(Said::Note(format!(
                        "The PC is too hot ({} °C; its firmware's limit is {} °C): Chat Computer is turning it off.",
                        celsius(t),
                        celsius(c)
                    )));
                    said.push(Said::TurnOff(2000));
                }
            }
        }
        if let Some(at) = self.empty_at
            && now >= at
        {
            self.empty_at = None;
            said.push(Said::Note(String::from("The battery is empty: Chat Computer is turning the PC off.")));
            said.push(Said::TurnOff(1000));
        }
        said
    }

    fn status_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let present: Vec<&Battery> = self.batteries.iter().filter(|b| b.present).collect();
        if !self.batteries.is_empty() && present.is_empty() {
            lines.push(String::from("Battery: none in the PC's battery bay"));
        }
        for (i, b) in present.iter().enumerate() {
            let name = if present.len() > 1 { format!("Battery {}", i + 1) } else { String::from("Battery") };
            let Some(percent) = b.percent() else {
                lines.push(format!("{name}: there, its charge unknown"));
                continue;
            };
            let time = |m: u64| if m >= 60 { format!("{} h {} min", m / 60, m % 60) } else { format!("{m} min") };
            lines.push(if b.charging() {
                match b.minutes().filter(|&m| m < 24 * 60) {
                    Some(m) => format!("{name}: {percent}%, charging (plugged in), full in about {}", time(m)),
                    None => format!("{name}: {percent}%, charging (plugged in)"),
                }
            } else if b.discharging() && self.plugged != Some(true) {
                match b.minutes().filter(|&m| m < 48 * 60) {
                    Some(m) => format!("{name}: {percent}%, on battery power, about {} left", time(m)),
                    None => format!("{name}: {percent}%, on battery power"),
                }
            } else if self.plugged == Some(true) {
                format!("{name}: {percent}%, plugged in (not charging)")
            } else {
                format!("{name}: {percent}%")
            });
        }
        if present.is_empty() && !self.adapters.is_empty() {
            match self.plugged {
                Some(true) => lines.push(String::from("Power: plugged in")),
                Some(false) => lines.push(String::from("Power: not plugged in")),
                None => {}
            }
        }
        if let Some(open) = self.lid_open {
            lines.push(String::from(if open { "Lid: open" } else { "Lid: closed" }));
        }
        if let Some(z) = self.zones.iter().filter(|z| z.temperature.is_some()).max_by_key(|z| z.temperature) {
            let t = celsius(z.temperature.unwrap_or(0));
            lines.push(match z.critical {
                Some(c) => format!("Temperature, as the firmware reads it: {t} °C (it's turned off at {} °C)", celsius(c)),
                None => format!("Temperature, as the firmware reads it: {t} °C"),
            });
        }
        lines
    }

    /// Before turning off: the firmware's own preparation (_TTS(5), _PTS(5), the power light off with _SST(0)), its
    /// events switched off, and S5's sleep types as \_S5 gives them.
    fn before_off(&mut self) -> Option<(u8, u8)> {
        let five = || vec![Value::Integer(5)];
        for path in ["\\_TTS", "\\_PTS"] {
            if let Some(node) = self.aml.find(path)
                && let Err(e) = self.aml.evaluate_node(node, five(), &mut self.machine)
            {
                crate::log!("Chat Computer kernel: firmware: {path}(5): {e:?}");
            }
        }
        if let Some(node) = self.aml.find("\\_SI_._SST") {
            let _ = self.aml.evaluate_node(node, vec![Value::Integer(0)], &mut self.machine);
        }
        for block in &self.gpe_blocks {
            for (s, e) in block.status.iter().zip(&block.enable) {
                e.write8(0);
                s.write8(0xFF);
            }
        }
        if let Some((status, _, b)) = self.pm1 {
            status.write16(ALL_FIXED_STATUS);
            if let Some((s, _)) = b {
                s.write16(ALL_FIXED_STATUS);
            }
        }
        s5(&mut self.aml, &mut self.machine)
    }
}

/// \_S5's two sleep types (for PM1a and PM1b), if the firmware's \_S5 gives them.
fn s5(aml: &mut Aml, machine: &mut Machine) -> Option<(u8, u8)> {
    let Ok(Value::Package(p)) = aml.evaluate("\\_S5_", Vec::new(), machine) else { return None };
    let p = p.borrow();
    let a = p.first()?.as_integer()?;
    // (Some firmware puts both in the first element, PM1b's in its second byte.)
    let b = p.get(1).and_then(|v| v.as_integer()).unwrap_or(a >> 8);
    Some(((a & 7) as u8, (b & 7) as u8))
}

/// Puts the PC in ACPI mode, if the firmware hasn't (writing ACPI_ENABLE to its SMI command port, then waiting up to 3
/// seconds for SCI_EN): what happened.
fn acpi_mode(e: &Events) -> &'static str {
    if e.pm1a_control == 0 {
        return "no PM1 control register";
    }
    let on = || unsafe { cpu::inw(e.pm1a_control) } & SCI_EN != 0;
    if on() {
        return "already in ACPI mode";
    }
    if e.smi_cmd == 0 || e.acpi_enable == 0 {
        return "not in ACPI mode, and the firmware gives no way to switch";
    }
    unsafe { cpu::outb(e.smi_cmd, e.acpi_enable) };
    for _ in 0..300 {
        if on() {
            return "switched to ACPI mode";
        }
        time::delay_ms(10);
    }
    "asked for ACPI mode, which didn't come"
}

/// A table's bytes, whole (DSDT, SSDT), if it looks like one.
fn table_bytes(phys: u64) -> Option<&'static [u8]> {
    if phys == 0 {
        return None;
    }
    let head = unsafe { core::slice::from_raw_parts(paging::map_firmware_table(phys, 36) as *const u8, 36) };
    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as u64;
    if !(36..=16 << 20).contains(&len) {
        return None;
    }
    let at = paging::map_firmware_table(phys, len);
    Some(unsafe { core::slice::from_raw_parts(at as *const u8, len as usize) })
}

/// The embedded controller: the first present PNP0C09 device whose _CRS gives its two ports.
fn find_ec(aml: &mut Aml, machine: &mut Machine) -> Option<Ec> {
    for node in aml.present_devices("PNP0C09", machine) {
        let Some(Ok(Value::Buffer(b))) = aml.evaluate_child(node, "_CRS", Vec::new(), machine) else { continue };
        let ports: Vec<u16> = aml::resources(&b.borrow())
            .into_iter()
            .filter_map(|r| match r {
                Resource::Io { min, .. } if min != 0 => Some(min),
                _ => None,
            })
            .collect();
        if ports.len() < 2 {
            continue;
        }
        let gpe = match aml.evaluate_child(node, "_GPE", Vec::new(), machine) {
            Some(Ok(Value::Integer(n))) if n < 256 => Some(n as u32),
            _ => None,
        };
        let lock = matches!(aml.evaluate_child(node, "_GLK", Vec::new(), machine), Some(Ok(Value::Integer(1))));
        return Some(Ec { data: ports[0], command: ports[1], node, lock, gpe, failures: 0, resting_until: 0, said_resting: false });
    }
    None
}

/// Loads and starts the firmware's programs, and sets up their events: what it found, for the log.
pub fn init(acpi: &Acpi, off: bool) -> String {
    if off {
        return String::from("firmware: its programs (ACPI's AML) aren't run: the settings say so");
    }
    let Some(dsdt) = table_bytes(acpi.dsdt) else {
        return String::from("firmware: no DSDT, so no programs to run");
    };
    let events = acpi.events.unwrap_or_default();
    let mode = if events.hardware_reduced { "a hardware-reduced PC" } else { acpi_mode(&events) };
    // The global lock, in the FACS (checked to be one).
    let mut lock = 0;
    if events.facs != 0 {
        let at = paging::map_firmware_table(events.facs, 64);
        if unsafe { core::slice::from_raw_parts(at as *const u8, 4) } == b"FACS" {
            lock = at + 16;
        }
    }
    let mut machine = Machine {
        ec: None,
        notified: Vec::new(),
        mapped: BTreeMap::new(),
        lock,
        held: 0,
        pm1_control: (events.pm1a_control, events.pm1b_control),
        tables: acpi.tables.clone(),
        said: 0,
    };
    let mut aml = Aml::new();
    aml.limits(MOST_STEPS, MOST_SLEEP_US);
    let mut loaded = 0;
    for t in core::iter::once(dsdt).chain(acpi.ssdts.iter().filter_map(|&p| table_bytes(p))) {
        match aml.load(t, &mut machine) {
            Ok(()) => loaded += 1,
            Err(e) => crate::log!("Chat Computer kernel: firmware: a table didn't load: {e:?}"),
        }
    }
    // The embedded controller first (its _REG: the firmware's methods may use it from then on), then CMOS's space, then
    // the _INIs (and PCI configuration space's _REG, as ACPICA does it).
    machine.ec = find_ec(&mut aml, &mut machine);
    if let Some(node) = machine.ec.as_ref().map(|ec| ec.node) {
        aml.connect_under(node, Space::EmbeddedControl, &mut machine);
    }
    aml.connect(Space::Cmos, &mut machine);
    let present = aml.initialize(&[Space::Pci], &mut machine);
    let problems = core::mem::take(&mut aml.problems);
    for p in problems.iter().take(12) {
        crate::log!("Chat Computer kernel: firmware: problem: {}", p.chars().take(160).collect::<String>());
    }

    // The events' hardware: PM1's status and enable registers (their stale bits cleared: a power button press that
    // turned the PC on can still be set), and the fixed power button's enable bit set.
    let half = (events.pm1_length / 2).max(2) as u64;
    let pm1 = events.pm1a.map(|a| (Reg::of(a, 0), Reg::of(a, half), events.pm1b.map(|b| (Reg::of(b, 0), Reg::of(b, half)))));
    if let Some((status, enable, b)) = pm1 {
        status.write16(ALL_FIXED_STATUS);
        enable.write16(if events.fixed_power_button { PWRBTN } else { 0 });
        if let Some((status, enable)) = b {
            status.write16(ALL_FIXED_STATUS);
            enable.write16(if events.fixed_power_button { PWRBTN } else { 0 });
        }
    }
    // The GPE blocks: every event switched off and cleared, then those with methods switched on (not the ones devices
    // wake the PC with), and the embedded controller's.
    let mut gpe_blocks = Vec::new();
    let blocks = [events.gpe0.map(|(r, len)| (r, len, 0u32)), events.gpe1.map(|(r, len, base)| (r, len, base as u32))];
    for (r, len, first) in blocks.into_iter().flatten() {
        let half = (len / 2) as u64;
        let block = GpeBlock {
            status: (0..half).map(|i| Reg::of(r, i)).collect(),
            enable: (0..half).map(|i| Reg::of(r, half + i)).collect(),
            first,
        };
        for (s, e) in block.status.iter().zip(&block.enable) {
            e.write8(0);
            s.write8(0xFF);
        }
        gpe_blocks.push(block);
    }
    let wake = aml.wake_gpes(&mut machine);
    let mut gpes = Vec::new();
    for (number, level, method) in aml.gpe_methods() {
        if !wake.contains(&number) && !gpes.iter().any(|g: &Gpe| g.number == number) {
            gpes.push(Gpe { number, level, method, streak: 0 });
        }
    }
    let ec_gpe = machine.ec.as_ref().and_then(|ec| ec.gpe);
    let mut enabled = 0;
    for number in gpes.iter().map(|g| g.number).chain(ec_gpe) {
        for block in &gpe_blocks {
            let n = number.wrapping_sub(block.first) as usize;
            if number >= block.first && n / 8 < block.enable.len() {
                let e = block.enable[n / 8];
                e.write8(e.read8() | 1 << (n % 8));
                enabled += 1;
            }
        }
    }

    // The devices: batteries, AC adapters, lids, power buttons that are devices, thermal zones.
    let path_of = |aml: &Aml, n: usize| aml.path(n);
    let batteries: Vec<Battery> = aml
        .present_devices("PNP0C0A", &mut machine)
        .into_iter()
        .map(|node| Battery { node, path: path_of(&aml, node), present: false, design: None, full: None, state: None })
        .collect();
    let adapters: Vec<(usize, String)> = aml.present_devices("ACPI0003", &mut machine).into_iter().map(|n| (n, path_of(&aml, n))).collect();
    let lids: Vec<(usize, String)> = aml.present_devices("PNP0C0D", &mut machine).into_iter().map(|n| (n, path_of(&aml, n))).collect();
    let buttons: Vec<String> = aml.present_devices("PNP0C0C", &mut machine).into_iter().map(|n| path_of(&aml, n)).collect();
    let zones: Vec<Zone> = aml
        .thermal_zones()
        .into_iter()
        .map(|node| Zone { node, path: path_of(&aml, node), critical: None, temperature: None, over: 0 })
        .collect();
    if let Some((a, b)) = s5(&mut aml, &mut machine) {
        S5.store(a as u32 | (b as u32) << 8, Ordering::Relaxed);
    }

    let ec_said = match &machine.ec {
        Some(ec) => format!(
            "embedded controller at ports {:#x} and {:#x}{}{}",
            ec.data,
            ec.command,
            ec.gpe.map(|g| format!(", event {g:#x}")).unwrap_or_default(),
            if ec.lock { ", with the global lock" } else { "" }
        ),
        None => String::from("no embedded controller"),
    };
    let mut fw = Firmware {
        aml,
        machine,
        pm1,
        fixed_power_button: events.fixed_power_button && !events.hardware_reduced,
        gpe_blocks,
        gpes,
        batteries,
        adapters,
        lids,
        buttons,
        zones,
        plugged: None,
        lid_open: None,
        next_battery: 0,
        next_adapter: 0,
        next_lid: 0,
        next_thermal: 0,
        pressed_at: 0,
        plug_note_at: 0,
        low_said: false,
        empty_at: None,
        logged: Vec::new(),
    };
    // First readings (anything they notify is about them, and nothing to say yet).
    let now = time::uptime_ms();
    let mut said = Vec::new();
    fw.power_changed(&mut said, now, true);
    fw.lid_changed();
    fw.read_zones();
    fw.machine.notified.clear();
    let now = time::uptime_ms();
    fw.next_battery = now + BATTERY_MS;
    fw.next_adapter = now + ADAPTER_MS;
    fw.next_lid = now + LID_MS;
    fw.next_thermal = now + THERMAL_MS;
    let mut summary = format!(
        "firmware: {loaded} tables of programs, {} objects ({} problems), {present} devices present; {mode}; {ec_said}; {} events \
         switched on, checked 20 times a second (their interrupt, {}, left off; {} with methods, {} for waking left off); \
         power button: {}",
        fw.aml.objects(),
        problems.len(),
        enabled,
        events.sci,
        fw.gpes.len(),
        wake.len(),
        match (fw.fixed_power_button, fw.buttons.len()) {
            (true, 0) => String::from("the fixed one"),
            (true, n) => format!("the fixed one, and {n} the firmware notifies"),
            (false, 0) => String::from("none found"),
            (false, n) => format!("{n} the firmware notifies"),
        }
    );
    for line in fw.status_lines() {
        summary += &format!("; {}", line.to_lowercase());
    }
    FIRMWARE.set(fw);
    summary
}

/// From the main loop: checks the firmware's events (20 times a second), and acts on what they say.
pub fn work() {
    let now = time::uptime_ms();
    if now < NEXT_CHECK.load(Ordering::Relaxed) {
        return;
    }
    NEXT_CHECK.store(now + CHECK_MS, Ordering::Relaxed);
    let Some(said) = FIRMWARE.with(|fw| fw.check(now)) else { return };
    for s in said {
        match s {
            Said::Note(text) => {
                crate::log!("Chat Computer kernel: firmware: {text}");
                chat::note(text);
            }
            Said::TurnOff(ms) => power::turn_off_in(ms),
        }
    }
}

/// The battery, power, lid and temperature, for the computer's status (none if the firmware's programs aren't run or
/// said nothing of them).
pub fn status() -> Vec<String> {
    FIRMWARE.with(|fw| fw.status_lines()).unwrap_or_default()
}

/// Before the PC is turned off (interrupts off): the firmware's own preparation run; S5's sleep types, as its \_S5
/// gives them, if it does.
pub fn before_turning_off() -> Option<(u8, u8)> {
    FIRMWARE.with(|fw| fw.before_off()).flatten()
}

/// S5's sleep types as the firmware's \_S5 gave them at start-up, if it did.
pub fn s5_types() -> Option<(u8, u8)> {
    let v = S5.load(Ordering::Relaxed);
    (v != u32::MAX).then(|| ((v & 7) as u8, (v >> 8 & 7) as u8))
}
