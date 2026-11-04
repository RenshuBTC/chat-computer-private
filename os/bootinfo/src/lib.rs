//! What Chat Computer's start-up stage hands to the kernel. Both sides use this crate, so the layout is
//! written down once.
#![no_std]

pub mod edid;

/// "ChatComp" in ASCII: the first field of [`BootInfo`], so the kernel can tell it got the real thing.
pub const MAGIC: u64 = u64::from_le_bytes(*b"ChatComp");
/// Raised whenever [`BootInfo`] changes shape.
pub const VERSION: u32 = 4;

/// All of physical memory is visible to the kernel at this virtual address: physical address `p` is at
/// `PHYS_OFFSET + p`. It is the start of the upper half of the address space (page-map slot 256).
pub const PHYS_OFFSET: u64 = 0xFFFF_8000_0000_0000;

pub const PAGE_SIZE: u64 = 4096;

/// How a framebuffer pixel is laid out (UEFI's EFI_GRAPHICS_PIXEL_FORMAT).
pub const PIXEL_RGB: u32 = 0; // 32 bits: red in the lowest byte
pub const PIXEL_BGR: u32 = 1; // 32 bits: blue in the lowest byte
pub const PIXEL_BITMASK: u32 = 2; // 32 bits: see the masks

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Framebuffer {
    pub phys: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    /// Pixels per row in memory (at least `width`).
    pub stride: u32,
    pub format: u32,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub reserved_mask: u32,
}

/// UEFI memory types, exactly as the firmware reports them.
pub mod memtype {
    pub const RESERVED: u32 = 0;
    pub const LOADER_CODE: u32 = 1;
    pub const LOADER_DATA: u32 = 2;
    pub const BOOT_SERVICES_CODE: u32 = 3;
    pub const BOOT_SERVICES_DATA: u32 = 4;
    pub const RUNTIME_SERVICES_CODE: u32 = 5;
    pub const RUNTIME_SERVICES_DATA: u32 = 6;
    pub const CONVENTIONAL: u32 = 7;
    pub const UNUSABLE: u32 = 8;
    pub const ACPI_RECLAIM: u32 = 9;
    pub const ACPI_NVS: u32 = 10;
    pub const MMIO: u32 = 11;
    pub const MMIO_PORT_SPACE: u32 = 12;
    pub const PAL_CODE: u32 = 13;
    pub const PERSISTENT: u32 = 14;
    pub const UNACCEPTED: u32 = 15;
}

/// One entry of the firmware's memory map, converted when the start-up stage took over.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryRegion {
    pub start: u64,
    pub pages: u64,
    /// A [`memtype`] value.
    pub kind: u32,
    pub _reserved: u32,
    pub attribute: u64,
}

impl MemoryRegion {
    pub fn end(&self) -> u64 {
        self.start.saturating_add(self.pages.saturating_mul(PAGE_SIZE))
    }

    /// Whether this is RAM (as opposed to device registers or holes), whatever it holds right now.
    pub fn is_ram(&self) -> bool {
        use memtype::*;
        matches!(
            self.kind,
            LOADER_CODE
                | LOADER_DATA
                | BOOT_SERVICES_CODE
                | BOOT_SERVICES_DATA
                | RUNTIME_SERVICES_CODE
                | RUNTIME_SERVICES_DATA
                | CONVENTIONAL
                | ACPI_RECLAIM
                | ACPI_NVS
                | PERSISTENT
        )
    }
}

pub const SEGMENT_EXECUTE: u32 = 1;
pub const SEGMENT_WRITE: u32 = 2;

/// Where one part of the kernel (code, read-only data, data) was put.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct KernelSegment {
    pub virt: u64,
    pub phys: u64,
    /// Bytes, a whole number of pages.
    pub size: u64,
    pub flags: u32,
    pub _reserved: u32,
}

pub const MAX_SEGMENTS: usize = 8;

/// What kind of drive the start-up partition is on.
pub const DISK_UNKNOWN: u32 = 0;
pub const DISK_NVME: u32 = 1;
pub const DISK_SATA: u32 = 2;

/// The drive and partition the start-up stage was started from, from the firmware's device path for it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct StartDisk {
    /// A `DISK_` value.
    pub kind: u32,
    /// The PCI root bridge's number (ACPI _UID: 0 on nearly every PC).
    pub pci_root: u32,
    /// The PCI devices from the root to the drive's controller: (device, function) pairs, bridges first.
    pub path_len: u32,
    pub path: [[u8; 2]; 8],
    /// NVMe: the namespace; SATA: the controller's port and the port multiplier's (0xFFFF: none).
    pub nvme_namespace: u32,
    pub sata_port: u16,
    pub sata_multiplier: u16,
    /// The partition: its first block and its size, in the drive's blocks (0: not known).
    pub partition_start: u64,
    pub partition_blocks: u64,
}

#[repr(C)]
pub struct BootInfo {
    pub magic: u64,
    pub version: u32,
    /// The firmware's UEFI version, e.g. 0x0002_0046 for 2.70.
    pub uefi_revision: u32,
    pub framebuffer: Framebuffer,
    /// Virtual address (in the [`PHYS_OFFSET`] map) of `memory_map_len` [`MemoryRegion`]s.
    pub memory_map: u64,
    pub memory_map_len: u64,
    /// Physical address of the ACPI RSDP, or 0 if the firmware gave none.
    pub rsdp: u64,
    pub kernel_segments: [KernelSegment; MAX_SEGMENTS],
    pub kernel_segment_count: u32,
    /// How many bytes of `edid` the monitor sent (0 if none).
    pub edid_len: u32,
    pub edid: [u8; 256],
    pub stack_phys: u64,
    pub stack_size: u64,
    /// The firmware maker's name as UEFI gives it (UCS-2, zero-terminated).
    pub firmware_vendor: [u16; 64],
    /// Whether the firmware checked this start-up stage's signature before running it: one of the
    /// `SECURE_BOOT_` values.
    pub secure_boot: u32,
    pub _reserved: u32,
    /// The PC's clock when the start-up stage read it, in seconds since 1970 as if it were UTC (whether it is
    /// depends on the time zone; Windows keeps it in local time); 0 if it couldn't be read.
    pub time: i64,
    /// The clock's time zone as the firmware gave it: EFI_TIME's TimeZone, in minutes, or 2047 when the firmware
    /// doesn't know (most PCs). Versions of the UEFI specification disagree on its sign.
    pub time_zone: i32,
    /// How many bytes of `settings` there are: Chat Computer's settings file (\EFI\chatcomputer-own\settings.txt
    /// on the start-up partition), as it was; 0 if there's none.
    pub settings_len: u32,
    pub settings: [u8; SETTINGS_MAX],
    /// Where the start-up partition is.
    pub start_disk: StartDisk,
}

/// The most of the settings file that's passed on.
pub const SETTINGS_MAX: usize = 2048;

pub const SECURE_BOOT_OFF: u32 = 0;
pub const SECURE_BOOT_ON: u32 = 1;
pub const SECURE_BOOT_UNKNOWN: u32 = 2;

impl BootInfo {
    /// # Safety
    /// `memory_map` must point to `memory_map_len` regions, as the start-up stage leaves it.
    pub unsafe fn memory_map(&self) -> &[MemoryRegion] {
        unsafe {
            core::slice::from_raw_parts(self.memory_map as *const MemoryRegion, self.memory_map_len as usize)
        }
    }

    pub fn segments(&self) -> &[KernelSegment] {
        &self.kernel_segments[..(self.kernel_segment_count as usize).min(MAX_SEGMENTS)]
    }

    pub fn edid(&self) -> &[u8] {
        &self.edid[..(self.edid_len as usize).min(self.edid.len())]
    }

    pub fn settings(&self) -> &[u8] {
        &self.settings[..(self.settings_len as usize).min(SETTINGS_MAX)]
    }
}
