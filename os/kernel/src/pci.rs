//! PCI and PCI Express: finding the PC's devices, reading and writing their configuration, finding
//! their registers, and having them interrupt by message (MSI-X, else MSI).
//!
//! Configuration is read through the memory window the ACPI MCFG table describes (PCI Express), else
//! through the old ports 0xCF8 and 0xCFC (which reach each device's first 256 bytes; that is all
//! Chat Computer needs). The firmware has already numbered the buses, set up the bridges and placed every
//! device's registers; Chat Computer keeps all of that. Devices are found by trying every bus, device and
//! function number in the window, so a device behind a host bridge that isn't bus 0's is found too.

use crate::acpi::Acpi;
use crate::cpu::{inb, inl, inw, outb, outl, outw};
use crate::paging::{self, Cache};
use crate::sync::SpinLock;
use alloc::vec::Vec;
use core::fmt;

pub const COMMAND: u16 = 0x04;
pub const STATUS: u16 = 0x06;
const CAPABILITIES: u16 = 0x34;

const COMMAND_IO: u16 = 1 << 0;
const COMMAND_MEMORY: u16 = 1 << 1;
const COMMAND_BUS_MASTER: u16 = 1 << 2;
const COMMAND_NO_INTX: u16 = 1 << 10;
const STATUS_CAPABILITIES: u16 = 1 << 4;

const CAP_POWER: u8 = 0x01;
const CAP_EXPRESS: u8 = 0x10;
/// PCI Express's extended capability: L1 PM Substates (its control register 1 is 8 bytes in).
const EXT_CAP_L1_SUBSTATES: u16 = 0x1E;
const CAP_MSI: u8 = 0x05;
const CAP_MSIX: u8 = 0x11;

/// Where configuration is read from.
struct Config {
    /// The MCFG window, mapped: (virtual address, first bus, last bus).
    window: Option<(u64, u8, u8)>,
}

static CONFIG: SpinLock<Config> = SpinLock::new(Config { window: None });
static FUNCTIONS: SpinLock<Vec<Function>> = SpinLock::new(Vec::new());

/// A device function's place: bus, device, function.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Address {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:02x}:{:02x}.{}", self.bus, self.device, self.function)
    }
}

/// One device function the firmware set up.
#[derive(Clone, Copy)]
pub struct Function {
    pub at: Address,
    pub vendor: u16,
    pub device: u16,
    pub class: u8,
    pub subclass: u8,
    pub interface: u8,
}

/// A memory region a device's registers are in.
#[derive(Clone, Copy, Debug)]
pub struct Bar {
    pub phys: u64,
    pub size: u64,
}

/// How a device interrupts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Signal {
    MsiX,
    Msi,
    /// Not at all: its driver looks for work instead.
    None,
}

impl Config {
    /// The window's virtual address for `offset` of `at`, if the window covers its bus.
    fn in_window(&self, at: Address, offset: u16) -> Option<u64> {
        let (base, first, last) = self.window?;
        if at.bus < first || at.bus > last {
            return None;
        }
        let bus = (at.bus - first) as u64;
        Some(base + (bus << 20 | (at.device as u64) << 15 | (at.function as u64) << 12 | (offset & 0xFFF) as u64))
    }
}

fn port_address(at: Address, offset: u16) -> u32 {
    0x8000_0000 | (at.bus as u32) << 16 | (at.device as u32) << 11 | (at.function as u32) << 8 | (offset as u32 & 0xFC)
}

impl Address {
    pub fn read32(self, offset: u16) -> u32 {
        let config = CONFIG.lock();
        match config.in_window(self, offset) {
            Some(v) => unsafe { core::ptr::read_volatile(v as *const u32) },
            None if offset < 256 => unsafe {
                outl(0xCF8, port_address(self, offset));
                inl(0xCFC)
            },
            None => !0,
        }
    }

    pub fn write32(self, offset: u16, value: u32) {
        let config = CONFIG.lock();
        match config.in_window(self, offset) {
            Some(v) => unsafe { core::ptr::write_volatile(v as *mut u32, value) },
            None if offset < 256 => unsafe {
                outl(0xCF8, port_address(self, offset));
                outl(0xCFC, value);
            },
            None => {}
        }
    }

    pub fn read16(self, offset: u16) -> u16 {
        let config = CONFIG.lock();
        match config.in_window(self, offset) {
            Some(v) => unsafe { core::ptr::read_volatile(v as *const u16) },
            None if offset < 256 => unsafe {
                outl(0xCF8, port_address(self, offset));
                inw(0xCFC + (offset & 2))
            },
            None => !0,
        }
    }

    pub fn write16(self, offset: u16, value: u16) {
        let config = CONFIG.lock();
        match config.in_window(self, offset) {
            Some(v) => unsafe { core::ptr::write_volatile(v as *mut u16, value) },
            None if offset < 256 => unsafe {
                outl(0xCF8, port_address(self, offset));
                outw(0xCFC + (offset & 2), value);
            },
            None => {}
        }
    }

    pub fn read8(self, offset: u16) -> u8 {
        let config = CONFIG.lock();
        match config.in_window(self, offset) {
            Some(v) => unsafe { core::ptr::read_volatile(v as *const u8) },
            None if offset < 256 => unsafe {
                outl(0xCF8, port_address(self, offset));
                inb(0xCFC + (offset & 3))
            },
            None => !0,
        }
    }

    #[allow(dead_code)]
    pub fn write8(self, offset: u16, value: u8) {
        let config = CONFIG.lock();
        match config.in_window(self, offset) {
            Some(v) => unsafe { core::ptr::write_volatile(v as *mut u8, value) },
            None if offset < 256 => unsafe {
                outl(0xCF8, port_address(self, offset));
                outb(0xCFC + (offset & 3), value);
            },
            None => {}
        }
    }

    /// Memory region number `index` (0-5): where the firmware put it and how big it is. None for an I/O
    /// port region or one the firmware left unplaced. Sizing it switches the device's memory decoding
    /// off for a moment, so only a driver that owns the device calls this.
    pub fn memory_bar(self, index: u8) -> Option<Bar> {
        let at = 0x10 + 4 * index as u16;
        let low = self.read32(at);
        if low & 1 != 0 {
            return None;
        }
        let wide = (low >> 1) & 3 == 2;
        let high = if wide { self.read32(at + 4) } else { 0 };
        let command = self.read16(COMMAND);
        self.write16(COMMAND, command & !(COMMAND_MEMORY | COMMAND_IO));
        self.write32(at, !0);
        let size_low = self.read32(at) & !0xF;
        self.write32(at, low);
        let size_high = if wide {
            self.write32(at + 4, !0);
            let v = self.read32(at + 4);
            self.write32(at + 4, high);
            v
        } else {
            !0
        };
        self.write16(COMMAND, command);
        let mask = (size_high as u64) << 32 | size_low as u64;
        let phys = (high as u64) << 32 | (low & !0xF) as u64;
        let size = (!mask).wrapping_add(1);
        (phys != 0 && size_low != 0 && size != 0).then_some(Bar { phys, size })
    }

    /// Lets the device answer at its memory registers and (if `dma`) read and write memory itself. Its
    /// old-style interrupt line is switched off: Chat Computer only takes interrupts by message.
    pub fn enable(self, dma: bool) {
        let command = self.read16(COMMAND);
        let mut wanted = command | COMMAND_MEMORY | COMMAND_NO_INTX;
        if dma {
            wanted |= COMMAND_BUS_MASTER;
        }
        if wanted != command {
            self.write16(COMMAND, wanted);
        }
    }

    /// Brings the device to full power (D0), if a firmware or the system before left it in a low-power state; true
    /// if it had to. A device needs 10 ms to come back from D3 (PCI Power Management 1.2, 5.6.1).
    pub fn power_on(self) -> bool {
        let Some(control) = self.power_control() else { return false };
        let state = self.read16(control);
        if state & 3 == 0 {
            return false;
        }
        self.write16(control, state & !3);
        crate::time::delay_ms(10);
        true
    }

    /// Where the device's power-management control and status register is, if it has one.
    pub fn power_control(self) -> Option<u16> {
        self.capability(CAP_POWER).map(|cap| cap + 4)
    }

    /// Where extended capability `id` is (PCI Express's, from offset 0x100), if the device has it. Only reachable
    /// through the PCI Express window: the old ports reach the first 256 bytes only.
    pub fn ext_capability(self, id: u16) -> Option<u16> {
        CONFIG.lock().in_window(self, 0x100)?;
        let mut at: u16 = 0x100;
        for _ in 0..64 {
            let header = self.read32(at);
            if header == 0 || header == 0xFFFF_FFFF {
                return None;
            }
            if header as u16 == id {
                return Some(at);
            }
            at = (header >> 20) as u16 & !3;
            if at < 0x100 {
                return None;
            }
        }
        None
    }

    /// The bridge (a root port or a switch's port) this device is behind, if any.
    pub fn upstream(self) -> Option<Address> {
        FUNCTIONS
            .lock()
            .iter()
            .find(|f| f.class == 0x06 && f.subclass == 0x04 && f.at.read8(0x19) == self.bus && self.bus != 0)
            .map(|f| f.at)
    }

    /// Switches off the L1.2 power-saving state of the device's PCI Express link (the L1 PM Substates capability's
    /// control register 1: ASPM L1.2, and with `pci_pm` the PCI-PM L1.2 as well), for devices that get it wrong: on
    /// the device first, then on the port above it, the order PCI Express asks for.
    pub fn disable_l1_2(self, pci_pm: bool) {
        let off = 1 << 2 | if pci_pm { 1 << 0 } else { 0 };
        for at in [Some(self), self.upstream()].into_iter().flatten() {
            if let Some(cap) = at.ext_capability(EXT_CAP_L1_SUBSTATES) {
                let control = at.read32(cap + 8);
                if control & off != 0 {
                    at.write32(cap + 8, control & !off);
                }
            }
        }
    }

    /// Switches off the power saving of the device's PCI Express link (ASPM's L0s and L1, and L1's substates), on
    /// the device and on the port above it, for devices that are unreliable with it.
    pub fn disable_aspm(self) {
        self.disable_l1_2(true);
        for at in [Some(self), self.upstream()].into_iter().flatten() {
            if let Some(cap) = at.capability(CAP_EXPRESS) {
                let control = at.read16(cap + 0x10); // Link Control: bits 1:0 are ASPM's
                if control & 3 != 0 {
                    at.write16(cap + 0x10, control & !3);
                }
            }
        }
    }

    /// Where capability `id` is in the configuration, if the device has it.
    pub fn capability(self, id: u8) -> Option<u16> {
        if self.read16(STATUS) & STATUS_CAPABILITIES == 0 {
            return None;
        }
        let mut at = (self.read8(CAPABILITIES) & !3) as u16;
        for _ in 0..48 {
            if at < 0x40 {
                return None;
            }
            if self.read8(at) == id {
                return Some(at);
            }
            at = (self.read8(at + 1) & !3) as u16;
        }
        None
    }

    /// Every capability of `id` in the configuration (a device can have several: virtio has one per region).
    pub fn capabilities(self, id: u8) -> Vec<u16> {
        let mut found = Vec::new();
        if self.read16(STATUS) & STATUS_CAPABILITIES == 0 {
            return found;
        }
        let mut at = (self.read8(CAPABILITIES) & !3) as u16;
        for _ in 0..48 {
            if at < 0x40 {
                break;
            }
            if self.read8(at) == id {
                found.push(at);
            }
            at = (self.read8(at + 1) & !3) as u16;
        }
        found
    }

    /// Has the device interrupt at `vector` on the processor core whose interrupt controller is
    /// `apic_id`, by message: MSI-X (its first entry) if it can, else MSI. The device must not be
    /// running yet. Returns which, or `Signal::None` if it can do neither.
    pub fn signal_messages(self, vector: u8, apic_id: u32) -> Signal {
        if apic_id > 255 {
            return Signal::None; // would need interrupt remapping
        }
        let address = 0xFEE0_0000 | apic_id << 12;
        if let Some(cap) = self.capability(CAP_MSIX) {
            let control = self.read16(cap + 2);
            let table = self.read32(cap + 4);
            let entries = (control & 0x7FF) as u64 + 1;
            if let Some(bar) = self.memory_bar((table & 7) as u8) {
                let base = paging::map_device(bar.phys + (table & !7) as u64, entries * 16, Cache::Uncached);
                unsafe {
                    for i in 1..entries {
                        let ctl = (base + 16 * i + 12) as *mut u32;
                        core::ptr::write_volatile(ctl, core::ptr::read_volatile(ctl) | 1); // masked
                    }
                    core::ptr::write_volatile(base as *mut u32, address);
                    core::ptr::write_volatile((base + 4) as *mut u32, 0);
                    core::ptr::write_volatile((base + 8) as *mut u32, vector as u32);
                    core::ptr::write_volatile((base + 12) as *mut u32, 0); // not masked
                }
                self.write16(cap + 2, (control | 1 << 15) & !(1 << 14)); // on; the function not masked
                return Signal::MsiX;
            }
        }
        self.signal_msi(vector, apic_id)
    }

    /// Has the device interrupt at `vector` by MSI only (for devices whose MSI-X needs more setting up than
    /// one message for everything). Returns `Signal::Msi`, or `Signal::None` if it can't.
    pub fn signal_msi(self, vector: u8, apic_id: u32) -> Signal {
        if apic_id > 255 {
            return Signal::None;
        }
        let address = 0xFEE0_0000 | apic_id << 12;
        if let Some(cap) = self.capability(CAP_MSI) {
            let control = self.read16(cap + 2);
            let wide = control & (1 << 7) != 0;
            self.write32(cap + 4, address);
            let data = if wide {
                self.write32(cap + 8, 0);
                cap + 12
            } else {
                cap + 8
            };
            self.write16(data, vector as u16);
            if control & (1 << 8) != 0 {
                self.write32(data + 4, 0); // per-vector mask bits: none masked
            }
            self.write16(cap + 2, (control & !(0x7 << 4)) | 1); // one message, on
            return Signal::Msi;
        }
        Signal::None
    }
}

/// Finds every device function. Returns how many, and how configuration is reached.
pub fn init(acpi: &Acpi) -> (usize, &'static str) {
    let (first, last) = match acpi.pci_window {
        Some(w) => {
            let buses = (w.last_bus - w.first_bus) as u64 + 1;
            let base = paging::map_device(w.base + ((w.first_bus as u64) << 20), buses << 20, Cache::Uncached);
            CONFIG.lock().window = Some((base, w.first_bus, w.last_bus));
            (w.first_bus, w.last_bus)
        }
        None => (0, 255),
    };
    let mut found = Vec::new();
    for bus in first..=last {
        for device in 0..32u8 {
            let at = Address { bus, device, function: 0 };
            if at.read16(0) == 0xFFFF {
                continue;
            }
            let functions = if at.read8(0x0E) & 0x80 != 0 { 8 } else { 1 };
            for function in 0..functions {
                let at = Address { bus, device, function };
                let id = at.read32(0);
                if id & 0xFFFF == 0xFFFF {
                    continue;
                }
                let class = at.read32(0x08);
                found.push(Function {
                    at,
                    vendor: id as u16,
                    device: (id >> 16) as u16,
                    class: (class >> 24) as u8,
                    subclass: (class >> 16) as u8,
                    interface: (class >> 8) as u8,
                });
            }
        }
    }
    let count = found.len();
    *FUNCTIONS.lock() = found;
    (count, if acpi.pci_window.is_some() { "through the PCI Express window" } else { "through the old ports" })
}

/// The device function at bus, device, function, if there is one.
pub fn at(bus: u8, device: u8, function: u8) -> Option<Function> {
    FUNCTIONS.lock().iter().find(|f| f.at == Address { bus, device, function }).copied()
}

/// Every device function of a class (0x02: network controllers), whatever its subclass.
pub fn of_class(class: u8) -> Vec<Function> {
    FUNCTIONS.lock().iter().filter(|f| f.class == class).copied().collect()
}

/// The device functions of one kind (class, subclass, programming interface).
pub fn find(class: u8, subclass: u8, interface: u8) -> Vec<Function> {
    FUNCTIONS
        .lock()
        .iter()
        .filter(|f| f.class == class && f.subclass == subclass && f.interface == interface)
        .copied()
        .collect()
}
