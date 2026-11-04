//! The few parts of the UEFI interface the start-up stage uses, written from the UEFI 2.10
//! specification. Only the start-up stage talks to the firmware; the kernel never does.
#![allow(dead_code)]

use core::ffi::c_void;
use core::mem::offset_of;

pub type Handle = *mut c_void;
pub type Status = usize;

pub const SUCCESS: Status = 0;
const ERROR: usize = 1 << 63;
pub const INVALID_PARAMETER: Status = ERROR | 2;
pub const BUFFER_TOO_SMALL: Status = ERROR | 5;
pub const ABORTED: Status = ERROR | 21;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Guid(pub u32, pub u16, pub u16, pub [u8; 8]);

pub const GRAPHICS_OUTPUT: Guid =
    Guid(0x9042a9de, 0x23dc, 0x4a38, [0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a]);
/// Real devices have a device path; the firmware's virtual console devices don't.
pub const DEVICE_PATH: Guid = Guid(0x09576e91, 0x6d3f, 0x11d2, [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b]);
pub const EDID_ACTIVE: Guid = Guid(0xbd8c1056, 0x9f36, 0x44ec, [0x92, 0xa8, 0xa6, 0x33, 0x7f, 0x81, 0x79, 0x86]);
pub const EDID_DISCOVERED: Guid =
    Guid(0x1c0c34f6, 0xd380, 0x41fa, [0xa0, 0x49, 0x8a, 0xd0, 0x6c, 0x1a, 0x66, 0xaa]);
pub const ACPI_20_TABLE: Guid = Guid(0x8868e871, 0xe4f1, 0x11d3, [0xbc, 0x22, 0x00, 0x80, 0xc7, 0x3c, 0x88, 0x81]);
pub const ACPI_10_TABLE: Guid = Guid(0xeb9d2d30, 0x2d88, 0x11d3, [0x9a, 0x16, 0x00, 0x90, 0x27, 0x3f, 0xc1, 0x4d]);

#[repr(C)]
pub struct TableHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
}

#[repr(C)]
pub struct SystemTable {
    pub hdr: TableHeader,
    pub firmware_vendor: *const u16,
    pub firmware_revision: u32,
    pub console_in_handle: Handle,
    pub con_in: *mut c_void,
    pub console_out_handle: Handle,
    pub con_out: *mut SimpleTextOutput,
    pub standard_error_handle: Handle,
    pub std_err: *mut SimpleTextOutput,
    pub runtime_services: *mut RuntimeServices,
    pub boot_services: *mut BootServices,
    pub number_of_table_entries: usize,
    pub configuration_table: *const ConfigurationTable,
}

/// Only the first two members are used (the rest of the table is never touched).
#[repr(C)]
pub struct SimpleTextOutput {
    pub reset: unsafe extern "efiapi" fn(*mut SimpleTextOutput, bool) -> Status,
    pub output_string: unsafe extern "efiapi" fn(*mut SimpleTextOutput, *const u16) -> Status,
}

#[repr(C)]
pub struct ConfigurationTable {
    pub vendor_guid: Guid,
    pub vendor_table: *mut c_void,
}

pub const ALLOCATE_ANY_PAGES: u32 = 0;
pub const LOADER_DATA: u32 = 2;
pub const BY_PROTOCOL: u32 = 2;

/// EFI_BOOT_SERVICES. Entries Chat Computer doesn't call are left as plain pointers.
#[repr(C)]
pub struct BootServices {
    pub hdr: TableHeader,
    raise_tpl: usize,
    restore_tpl: usize,
    pub allocate_pages: unsafe extern "efiapi" fn(u32, u32, usize, *mut u64) -> Status,
    pub free_pages: unsafe extern "efiapi" fn(u64, usize) -> Status,
    pub get_memory_map: unsafe extern "efiapi" fn(*mut usize, *mut u8, *mut usize, *mut usize, *mut u32) -> Status,
    pub allocate_pool: unsafe extern "efiapi" fn(u32, usize, *mut *mut u8) -> Status,
    pub free_pool: unsafe extern "efiapi" fn(*mut c_void) -> Status,
    create_event: usize,
    set_timer: usize,
    wait_for_event: usize,
    signal_event: usize,
    close_event: usize,
    check_event: usize,
    install_protocol_interface: usize,
    reinstall_protocol_interface: usize,
    uninstall_protocol_interface: usize,
    pub handle_protocol: unsafe extern "efiapi" fn(Handle, *const Guid, *mut *mut c_void) -> Status,
    reserved: usize,
    register_protocol_notify: usize,
    locate_handle: usize,
    locate_device_path: usize,
    install_configuration_table: usize,
    load_image: usize,
    start_image: usize,
    exit: usize,
    unload_image: usize,
    pub exit_boot_services: unsafe extern "efiapi" fn(Handle, usize) -> Status,
    get_next_monotonic_count: usize,
    pub stall: unsafe extern "efiapi" fn(usize) -> Status,
    pub set_watchdog_timer: unsafe extern "efiapi" fn(usize, u64, usize, *const u16) -> Status,
    connect_controller: usize,
    disconnect_controller: usize,
    open_protocol: usize,
    close_protocol: usize,
    open_protocol_information: usize,
    protocols_per_handle: usize,
    pub locate_handle_buffer: unsafe extern "efiapi" fn(u32, *const Guid, *mut c_void, *mut usize, *mut *mut Handle) -> Status,
    pub locate_protocol: unsafe extern "efiapi" fn(*const Guid, *mut c_void, *mut *mut c_void) -> Status,
}

// The table is read by position, so check every position against the specification.
const _: () = {
    assert!(offset_of!(BootServices, allocate_pages) == 40);
    assert!(offset_of!(BootServices, get_memory_map) == 56);
    assert!(offset_of!(BootServices, free_pool) == 72);
    assert!(offset_of!(BootServices, handle_protocol) == 152);
    assert!(offset_of!(BootServices, exit_boot_services) == 232);
    assert!(offset_of!(BootServices, stall) == 248);
    assert!(offset_of!(BootServices, set_watchdog_timer) == 256);
    assert!(offset_of!(BootServices, locate_handle_buffer) == 312);
    assert!(offset_of!(BootServices, locate_protocol) == 320);
    assert!(offset_of!(SystemTable, boot_services) == 96);
    assert!(offset_of!(SystemTable, configuration_table) == 112);
    assert!(core::mem::size_of::<ConfigurationTable>() == 24);
};

/// EFI_RUNTIME_SERVICES: only GetTime and GetVariable are used, and only before ExitBootServices.
#[repr(C)]
pub struct RuntimeServices {
    pub hdr: TableHeader,
    pub get_time: unsafe extern "efiapi" fn(*mut Time, *mut c_void) -> Status,
    set_time: usize,
    get_wakeup_time: usize,
    set_wakeup_time: usize,
    set_virtual_address_map: usize,
    convert_pointer: usize,
    pub get_variable: unsafe extern "efiapi" fn(*const u16, *const Guid, *mut u32, *mut usize, *mut c_void) -> Status,
}

const _: () = assert!(offset_of!(RuntimeServices, get_variable) == 72);

/// EFI_TIME: the PC's clock.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Time {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub _pad1: u8,
    pub nanosecond: u32,
    /// Minutes from UTC (the specification's versions disagree on which way), or 2047 when the clock doesn't know.
    pub time_zone: i16,
    pub daylight: u8,
    pub _pad2: u8,
}

pub const UNSPECIFIED_TIMEZONE: i16 = 0x07FF;

pub const LOADED_IMAGE: Guid = Guid(0x5B1B31A1, 0x9562, 0x11d2, [0x8E, 0x3F, 0x00, 0xA0, 0xC9, 0x69, 0x72, 0x3B]);
pub const SIMPLE_FILE_SYSTEM: Guid = Guid(0x964e5b22, 0x6459, 0x11d2, [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b]);

/// EFI_LOADED_IMAGE_PROTOCOL: which disk this start-up program was read from.
#[repr(C)]
pub struct LoadedImage {
    pub revision: u32,
    pub parent_handle: Handle,
    pub system_table: *mut SystemTable,
    pub device_handle: Handle,
    pub file_path: *mut c_void,
    reserved: *mut c_void,
    pub load_options_size: u32,
    pub load_options: *mut c_void,
    pub image_base: *mut c_void,
    pub image_size: u64,
    pub image_code_type: u32,
    pub image_data_type: u32,
    unload: usize,
}

const _: () = assert!(offset_of!(LoadedImage, device_handle) == 24);
const _: () = assert!(offset_of!(LoadedImage, image_size) == 72);

/// EFI_SIMPLE_FILE_SYSTEM_PROTOCOL.
#[repr(C)]
pub struct SimpleFileSystem {
    pub revision: u64,
    pub open_volume: unsafe extern "efiapi" fn(*mut SimpleFileSystem, *mut *mut File) -> Status,
}

/// EFI_FILE_PROTOCOL: only opening, reading and closing.
#[repr(C)]
pub struct File {
    pub revision: u64,
    pub open: unsafe extern "efiapi" fn(*mut File, *mut *mut File, *const u16, u64, u64) -> Status,
    pub close: unsafe extern "efiapi" fn(*mut File) -> Status,
    delete: usize,
    pub read: unsafe extern "efiapi" fn(*mut File, *mut usize, *mut c_void) -> Status,
}

pub const FILE_MODE_READ: u64 = 1;

/// The UEFI global variables' owner (EFI_GLOBAL_VARIABLE), e.g. "SecureBoot".
pub const GLOBAL_VARIABLE: Guid = Guid(0x8be4df61, 0x93ca, 0x11d2, [0xaa, 0x0d, 0x00, 0xe0, 0x98, 0x03, 0x2b, 0x8c]);

#[repr(C)]
pub struct GraphicsOutput {
    pub query_mode: unsafe extern "efiapi" fn(*mut GraphicsOutput, u32, *mut usize, *mut *mut ModeInfo) -> Status,
    pub set_mode: unsafe extern "efiapi" fn(*mut GraphicsOutput, u32) -> Status,
    blt: usize,
    pub mode: *mut GraphicsMode,
}

#[repr(C)]
pub struct GraphicsMode {
    pub max_mode: u32,
    pub mode: u32,
    pub info: *mut ModeInfo,
    pub size_of_info: usize,
    pub frame_buffer_base: u64,
    pub frame_buffer_size: usize,
}

pub const PIXEL_RGB_RESERVED: u32 = 0;
pub const PIXEL_BGR_RESERVED: u32 = 1;
pub const PIXEL_BIT_MASK: u32 = 2;
pub const PIXEL_BLT_ONLY: u32 = 3;

#[repr(C)]
pub struct ModeInfo {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub pixel_format: u32,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub reserved_mask: u32,
    pub pixels_per_scan_line: u32,
}

/// EFI_EDID_ACTIVE_PROTOCOL and EFI_EDID_DISCOVERED_PROTOCOL share this shape.
#[repr(C)]
pub struct EdidProtocol {
    pub size: u32,
    pub edid: *const u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryDescriptor {
    pub kind: u32,
    pub _pad: u32,
    pub phys_start: u64,
    pub virt_start: u64,
    pub pages: u64,
    pub attribute: u64,
}
