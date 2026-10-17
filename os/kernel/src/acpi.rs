//! Reads the firmware's ACPI tables: how many processor cores there are, where the interrupt
//! controllers are, which timers exist, and where PCI Express devices' configuration can be read.
//! Only the tables (plain data) are read; the small programs (AML) some tables contain are not run.

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
            crate::log!("ClaudeOS kernel: the firmware gave no ACPI tables");
            return acpi;
        }
        let rsdp = unsafe { core::slice::from_raw_parts(paging::map_firmware_table(rsdp_phys, 36) as *const u8, 36) };
        if &rsdp[..8] != b"RSD PTR " || sum(&rsdp[..20]) != 0 {
            crate::log!("ClaudeOS kernel: the ACPI tables' starting point is damaged");
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
            crate::log!("ClaudeOS kernel: the ACPI root table is missing");
            return acpi;
        };
        let step = if wide { 8 } else { 4 };
        let mut at = 36;
        while at + step <= root.len() {
            let phys = if wide { u64_at(root, at) } else { u32_at(root, at) as u64 };
            at += step;
            let Some(t) = table(phys) else { continue };
            if sum(t) != 0 {
                crate::log!("ClaudeOS kernel: ACPI table {} has a bad checksum; using it anyway",
                            core::str::from_utf8(&t[..4]).unwrap_or("????"));
            }
            match &t[..4] {
                b"APIC" => acpi.read_madt(t),
                b"FACP" => acpi.read_fadt(t),
                // Its registers' "generic address": usable when it's in memory (address space 0).
                b"HPET" if u8_at(t, 40) == 0 => acpi.hpet = Some(u64_at(t, 44)).filter(|a| *a != 0),
                b"MCFG" => acpi.read_mcfg(t),
                b"DMAR" => acpi.read_dmar(t),
                b"IVRS" => acpi.read_ivrs(t),
                _ => {}
            }
        }
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

    /// The FADT: fixed hardware, including the power-management timer.
    fn read_fadt(&mut self, t: &[u8]) {
        // Preferred power profile: 2 is "mobile", 8 "tablet".
        self.portable = matches!(u8_at(t, 45), 2 | 8);
        let wide = u32_at(t, 112) & (1 << 8) != 0;
        // ACPI 2.0+ gives the timer as a "generic address" at 208; use it when it's an I/O port.
        let mut port = u32_at(t, 76) as u64;
        if t.len() >= 220 && u8_at(t, 208) == 1 && u64_at(t, 212) != 0 {
            port = u64_at(t, 212);
        }
        if port != 0 && port <= 0xFFFF {
            self.pm_timer = Some(PmTimer { port: port as u16, wide });
        }
    }
}
