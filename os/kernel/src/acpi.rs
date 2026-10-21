//! Reads the firmware's ACPI tables: how many processor cores there are, where the interrupt
//! controllers are, which timers exist, where PCI Express devices' configuration can be read, and how to
//! restart and turn off the PC. Only the tables (plain data) are read; the small programs (AML) some tables
//! contain are not run (the one value needed from them, S5's sleep type, is plain data in them).

use crate::paging;

#[derive(Clone, Copy, Default, Debug)]
pub struct IoApic {
    pub id: u8,
    pub address: u64,
    pub first_interrupt: u32,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct PmTimer {
    pub port: u16,
    /// 32 bits wide (otherwise 24).
    pub wide: bool,
}

/// Where PCI Express configuration space is in memory (the MCFG table): 4 KiB for each function of
/// buses `first_bus` to `last_bus`, starting at `base`.
#[derive(Clone, Copy, Default, Debug)]
pub struct PciWindow {
    pub base: u64,
    pub first_bus: u8,
    pub last_bus: u8,
}

#[derive(Default)]
pub struct Acpi {
    pub revision: u8,
    pub oem: [u8; 6],
    pub local_apic: u64,
    /// Processor cores the firmware says are present and enabled.
    pub cores: u32,
    pub io_apics: [IoApic; 8],
    pub io_apic_count: usize,
    pub has_legacy_pics: bool,
    pub pm_timer: Option<PmTimer>,
    pub hpet: Option<u64>,
    /// Segment group 0's window (PCs have no other).
    pub pci_window: Option<PciWindow>,
    /// The firmware says this is a laptop or a tablet (the FADT's preferred power profile).
    pub portable: bool,
    /// The IOMMUs' register addresses: Intel's (DMAR table) and AMD's (IVRS table).
    pub intel_iommus: [u64; 8],
    pub intel_iommu_count: usize,
    pub amd_iommus: [u64; 8],
    pub amd_iommu_count: usize,
    /// How the firmware says to restart the PC (the FADT's reset register), if it says.
    pub reset: Option<ResetRegister>,
    /// How to turn the PC off ("soft off", ACPI's S5), if the firmware says enough.
    pub soft_off: Option<SoftOff>,
}

/// Turning the PC off: the sleep type the firmware gives for S5 (the `\_S5` package in its DSDT), written with "sleep
/// enable" to the PM1 control registers (or, on a "hardware-reduced" PC, the sleep control register).
#[derive(Clone, Copy, Debug)]
pub struct SoftOff {
    /// The PM1a and PM1b control registers' I/O ports (PM1b: 0, there's none).
    pub pm1a: u16,
    pub pm1b: u16,
    /// A hardware-reduced PC's sleep control register: its generic address (space, address).
    pub sleep_control: Option<(u8, u64)>,
    /// S5's sleep type values, for PM1a and PM1b.
    pub typ_a: u8,
    pub typ_b: u8,
    /// To switch the PC into ACPI mode, if the firmware hasn't: the SMI command port, and the value.
    pub smi_cmd: u16,
    pub acpi_enable: u8,
}

/// What the FADT says about turning off, until the `\_S5` package is found.
#[derive(Default)]
struct Fadt {
    dsdt: u64,
    pm1a: u16,
    pm1b: u16,
    sleep_control: Option<(u8, u64)>,
    hardware_reduced: bool,
    smi_cmd: u16,
    acpi_enable: u8,
}

/// The `\_S5` package's first two values (the sleep types for PM1a and PM1b), found by its name in AML: "_S5_" given a
/// package by a Name, the form firmware uses. (One worked out by a method would need the AML run.)
fn s5_in(aml: &[u8]) -> Option<(u8, u8)> {
    let mut from = 0;
    while let Some(found) = aml[from..].windows(4).position(|w| w == b"_S5_") {
        let at = from + found;
        from = at + 4;
        // NameOp (0x08) just before the name, or before its root (\) or parent (^) prefix.
        let name = match at.checked_sub(1).map(|i| aml[i]) {
            Some(0x08) => true,
            Some(b'\\' | b'^') => at >= 2 && aml[at - 2] == 0x08,
            _ => false,
        };
        if !name || aml.get(at + 4) != Some(&0x12) {
            continue; // not a Name with a Package (PackageOp 0x12)
        }
        // The package's length (its first byte's top two bits say how many more bytes it has), then how many
        // elements it has, then the elements.
        let mut j = at + 5;
        j += 1 + (*aml.get(j)? >> 6) as usize;
        let count = *aml.get(j)?;
        j += 1;
        let mut values = [0u8; 2];
        for v in values.iter_mut().take(count.min(2) as usize) {
            let (value, len) = match *aml.get(j)? {
                0x00 => (0, 1),                // Zero
                0x01 => (1, 1),                // One
                0x0A => (*aml.get(j + 1)?, 2), // a byte
                0x0B => (*aml.get(j + 1)?, 3), // a word (the values are 3 bits)
                0x0C => (*aml.get(j + 1)?, 5), // a double word
                _ => return None,
            };
            *v = value;
            j += len;
        }
        return Some((values[0], values[1]));
    }
    None
}

/// A register to write a value to, to restart the PC.
#[derive(Clone, Copy, Debug)]
pub struct ResetRegister {
    /// 0: memory, 1: I/O port.
    pub space: u8,
    pub address: u64,
    pub value: u8,
}

fn u8_at(t: &[u8], o: usize) -> u8 {
    t.get(o).copied().unwrap_or(0)
}
fn u32_at(t: &[u8], o: usize) -> u32 {
    t.get(o..o + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn u64_at(t: &[u8], o: usize) -> u64 {
    t.get(o..o + 8).map_or(0, |b| u64::from_le_bytes(b.try_into().unwrap()))
}

fn sum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |a, b| a.wrapping_add(*b))
}

/// A firmware table at `phys`, as bytes. Tables say their own length at offset 4.
fn table(phys: u64) -> Option<&'static [u8]> {
    if phys == 0 {
        return None;
    }
    let head = paging::map_firmware_table(phys, 36);
    let len = unsafe { core::ptr::read_unaligned((head + 4) as *const u32) } as u64;
    if !(36..=16 << 20).contains(&len) {
        return None;
    }
    let at = paging::map_firmware_table(phys, len);
    Some(unsafe { core::slice::from_raw_parts(at as *const u8, len as usize) })
}

impl Acpi {
    pub fn read(rsdp_phys: u64) -> Acpi {
        let mut acpi = Acpi::default();
        if rsdp_phys == 0 {
            crate::log!("Chat Computer kernel: the firmware gave no ACPI tables");
            return acpi;
        }
        let rsdp = unsafe { core::slice::from_raw_parts(paging::map_firmware_table(rsdp_phys, 36) as *const u8, 36) };
        if &rsdp[..8] != b"RSD PTR " || sum(&rsdp[..20]) != 0 {
            crate::log!("Chat Computer kernel: the ACPI tables' starting point is damaged");
            return acpi;
        }
        acpi.revision = rsdp[15];
        acpi.oem.copy_from_slice(&rsdp[9..15]);
        // ACPI 2.0+: the XSDT (64-bit pointers); before that, the RSDT (32-bit pointers).
        let (root, wide) = if acpi.revision >= 2 && u64_at(rsdp, 24) != 0 {
            (table(u64_at(rsdp, 24)), true)
        } else {
            (table(u32_at(rsdp, 16) as u64), false)
        };
        let Some(root) = root else {
            crate::log!("Chat Computer kernel: the ACPI root table is missing");
            return acpi;
        };
        let step = if wide { 8 } else { 4 };
        let mut at = 36;
        let mut fadt = Fadt::default();
        let (mut ssdts, mut ssdt_count) = ([0u64; 32], 0);
        while at + step <= root.len() {
            let phys = if wide { u64_at(root, at) } else { u32_at(root, at) as u64 };
            at += step;
            let Some(t) = table(phys) else { continue };
            if sum(t) != 0 {
                crate::log!("Chat Computer kernel: ACPI table {} has a bad checksum; using it anyway",
                            core::str::from_utf8(&t[..4]).unwrap_or("????"));
            }
            match &t[..4] {
                b"APIC" => acpi.read_madt(t),
                b"FACP" => fadt = acpi.read_fadt(t),
                // Its registers' "generic address": usable when it's in memory (address space 0).
                b"HPET" if u8_at(t, 40) == 0 => acpi.hpet = Some(u64_at(t, 44)).filter(|a| *a != 0),
                b"MCFG" => acpi.read_mcfg(t),
                b"DMAR" => acpi.read_dmar(t),
                b"IVRS" => acpi.read_ivrs(t),
                b"SSDT" if ssdt_count < ssdts.len() => {
                    ssdts[ssdt_count] = phys;
                    ssdt_count += 1;
                }
                _ => {}
            }
        }
        acpi.soft_off = fadt.soft_off(&ssdts[..ssdt_count]);
        acpi
    }

    /// The MADT: interrupt controllers and processor cores.
    fn read_madt(&mut self, t: &[u8]) {
        self.local_apic = u32_at(t, 36) as u64;
        self.has_legacy_pics = u32_at(t, 40) & 1 != 0;
        let mut at = 44;
        while at + 2 <= t.len() {
            let (kind, len) = (u8_at(t, at), u8_at(t, at + 1) as usize);
            if len < 2 || at + len > t.len() {
                break;
            }
            let e = &t[at..at + len];
            match kind {
                0 if u32_at(e, 4) & 1 != 0 => self.cores += 1, // processor local APIC, enabled
                9 if u32_at(e, 8) & 1 != 0 => self.cores += 1, // processor local x2APIC, enabled
                1 if self.io_apic_count < self.io_apics.len() => {
                    self.io_apics[self.io_apic_count] =
                        IoApic { id: u8_at(e, 2), address: u32_at(e, 4) as u64, first_interrupt: u32_at(e, 8) };
                    self.io_apic_count += 1;
                }
                5 => self.local_apic = u64_at(e, 4), // 64-bit address override
                _ => {}
            }
            at += len;
        }
    }

    /// The MCFG: after its header and 8 reserved bytes, 16-byte entries (base address, segment group,
    /// first bus, last bus).
    fn read_mcfg(&mut self, t: &[u8]) {
        let mut at = 44;
        while at + 16 <= t.len() {
            let (base, segment) = (u64_at(t, at), u16::from_le_bytes([u8_at(t, at + 8), u8_at(t, at + 9)]));
            let (first_bus, last_bus) = (u8_at(t, at + 10), u8_at(t, at + 11));
            if segment == 0 && base != 0 && first_bus <= last_bus && self.pci_window.is_none() {
                self.pci_window = Some(PciWindow { base, first_bus, last_bus });
            }
            at += 16;
        }
    }

    /// The DMAR: Intel's IOMMUs ("DMA remapping hardware units", structures of type 0 after 48 bytes).
    fn read_dmar(&mut self, t: &[u8]) {
        let mut at = 48;
        while at + 4 <= t.len() {
            let kind = u16::from_le_bytes([u8_at(t, at), u8_at(t, at + 1)]);
            let len = u16::from_le_bytes([u8_at(t, at + 2), u8_at(t, at + 3)]) as usize;
            if len < 4 || at + len > t.len() {
                break;
            }
            let base = u64_at(t, at + 8);
            if kind == 0 && len >= 16 && base != 0 && self.intel_iommu_count < self.intel_iommus.len() {
                self.intel_iommus[self.intel_iommu_count] = base;
                self.intel_iommu_count += 1;
            }
            at += len;
        }
    }

    /// The IVRS: AMD's IOMMUs (hardware definition blocks of types 0x10, 0x11 and 0x40 after 48 bytes;
    /// one IOMMU may be described more than once).
    fn read_ivrs(&mut self, t: &[u8]) {
        let mut at = 48;
        while at + 4 <= t.len() {
            let kind = u8_at(t, at);
            let len = u16::from_le_bytes([u8_at(t, at + 2), u8_at(t, at + 3)]) as usize;
            if len < 4 || at + len > t.len() {
                break;
            }
            let base = u64_at(t, at + 8);
            let known = self.amd_iommus[..self.amd_iommu_count].contains(&base);
            if matches!(kind, 0x10 | 0x11 | 0x40) && len >= 16 && base != 0 && !known && self.amd_iommu_count < self.amd_iommus.len() {
                self.amd_iommus[self.amd_iommu_count] = base;
                self.amd_iommu_count += 1;
            }
            at += len;
        }
    }

    /// The FADT: fixed hardware, including the power-management timer. Returns what it says about turning off.
    fn read_fadt(&mut self, t: &[u8]) -> Fadt {
        // Preferred power profile: 2 is "mobile", 8 "tablet".
        self.portable = matches!(u8_at(t, 45), 2 | 8);
        let flags = u32_at(t, 112);
        let wide = flags & (1 << 8) != 0;
        // ACPI 2.0+ gives the timer as a "generic address" at 208; use it when it's an I/O port.
        let mut port = u32_at(t, 76) as u64;
        if t.len() >= 220 && u8_at(t, 208) == 1 && u64_at(t, 212) != 0 {
            port = u64_at(t, 212);
        }
        if port != 0 && port <= 0xFFFF {
            self.pm_timer = Some(PmTimer { port: port as u16, wide });
        }
        // The reset register (ACPI 2.0+): a generic address at 116 and the value at 128, if flag 10 says so.
        if t.len() >= 129 && flags & (1 << 10) != 0 {
            let (space, address) = (u8_at(t, 116), u64_at(t, 120));
            if matches!(space, 0 | 1) && address != 0 {
                self.reset = Some(ResetRegister { space, address, value: u8_at(t, 128) });
            }
        }
        // For turning off: the DSDT (its 64-bit address at 140, ACPI 2.0+, else the 32-bit one at 40); the PM1 control
        // registers (I/O ports: the generic addresses at 172 and 184, else the old fields at 64 and 68); on a
        // hardware-reduced PC (flag 20), the sleep control register (a generic address at 244, ACPI 5.0+); and the
        // SMI command port (48) and value (52) that put the PC in ACPI mode.
        let port = |old: usize, generic: usize| -> u16 {
            let address = u64_at(t, generic + 4);
            if t.len() >= generic + 12 && u8_at(t, generic) == 1 && address != 0 && address <= 0xFFFF {
                return address as u16;
            }
            u16::try_from(u32_at(t, old)).unwrap_or(0)
        };
        let mut fadt = Fadt {
            dsdt: u32_at(t, 40) as u64,
            pm1a: port(64, 172),
            pm1b: port(68, 184),
            hardware_reduced: flags & (1 << 20) != 0,
            smi_cmd: u16::try_from(u32_at(t, 48)).unwrap_or(0),
            acpi_enable: u8_at(t, 52),
            ..Fadt::default()
        };
        if t.len() >= 148 && u64_at(t, 140) != 0 {
            fadt.dsdt = u64_at(t, 140);
        }
        if t.len() >= 256 && matches!(u8_at(t, 244), 0 | 1) && u64_at(t, 248) != 0 {
            fadt.sleep_control = Some((u8_at(t, 244), u64_at(t, 248)));
        }
        fadt
    }
}

impl Fadt {
    /// Turning off, if the FADT gives the registers and the DSDT (or an SSDT) the sleep types.
    fn soft_off(&self, ssdts: &[u64]) -> Option<SoftOff> {
        let sleep_control = if self.hardware_reduced { Some(self.sleep_control?) } else { None };
        if sleep_control.is_none() && self.pm1a == 0 {
            return None;
        }
        let (typ_a, typ_b) = core::iter::once(self.dsdt)
            .chain(ssdts.iter().copied())
            .filter_map(table)
            .find_map(|t| s5_in(t.get(36..)?))?;
        Some(SoftOff {
            pm1a: self.pm1a,
            pm1b: self.pm1b,
            sleep_control,
            typ_a,
            typ_b,
            smi_cmd: self.smi_cmd,
            acpi_enable: self.acpi_enable,
        })
    }
}
