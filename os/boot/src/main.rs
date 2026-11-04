//! Chat Computer's start-up stage: the UEFI program the PC's firmware runs (\EFI\BOOT\BOOTX64.EFI).
//! It carries the kernel inside it.
//!
//! It asks the firmware, one last time, for what the kernel needs to know (the screen, the memory map,
//! where the ACPI tables are), copies the kernel into memory, builds the kernel's first page tables,
//! tells the firmware it is finished with it (ExitBootServices) and jumps to the kernel. Nothing in the
//! firmware is called after that.
#![no_std]
#![no_main]

mod elf;
mod paging;
mod uefi;
#[path = "../../shared/mem.rs"]
mod mem;

use bootinfo::{BootInfo, Framebuffer, KernelSegment, MemoryRegion, StartDisk, MAX_SEGMENTS, PAGE_SIZE, PHYS_OFFSET};
use core::arch::asm;
use core::ffi::c_void;
use core::mem::size_of;
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicPtr, Ordering};
use uefi::*;

/// The kernel, put here by build.sh.
static KERNEL: &[u8] = include_bytes!(env!("CHATCOMPUTER_KERNEL_ELF"));

/// The firmware's table, while its services may still be used (null after ExitBootServices).
static SYSTEM_TABLE: AtomicPtr<SystemTable> = AtomicPtr::new(null_mut());

const STACK_PAGES: usize = 64; // the kernel's first stack: 256 KiB

/// # Safety
/// Only the firmware calls this, with its own image handle and system table.
#[unsafe(no_mangle)]
pub unsafe extern "efiapi" fn efi_main(image: Handle, st: *mut SystemTable) -> Status {
    SYSTEM_TABLE.store(st, Ordering::Release);
    // `start` only comes back if something stopped it.
    let reason = unsafe { start(image, st) };
    say("\r\nChat Computer couldn't start: ");
    say(reason);
    say(".\r\n");
    unsafe { ((*(*st).boot_services).stall)(15_000_000) };
    ABORTED
}

/// Writes on the firmware's text screen (only possible before ExitBootServices).
fn say(text: &str) {
    let st = SYSTEM_TABLE.load(Ordering::Acquire);
    if st.is_null() {
        return;
    }
    unsafe {
        let out = (*st).con_out;
        if out.is_null() {
            return;
        }
        let mut buf = [0u16; 80];
        let mut n = 0;
        for unit in text.encode_utf16() {
            buf[n] = unit;
            n += 1;
            if n == buf.len() - 1 {
                buf[n] = 0;
                ((*out).output_string)(out, buf.as_ptr());
                n = 0;
            }
        }
        buf[n] = 0;
        ((*out).output_string)(out, buf.as_ptr());
    }
}

/// Zeroed pages from the firmware, marked as the loader's data.
unsafe fn pages(bs: &BootServices, count: usize) -> Result<u64, &'static str> {
    let mut addr = 0u64;
    if unsafe { (bs.allocate_pages)(ALLOCATE_ANY_PAGES, LOADER_DATA, count, &mut addr) } != SUCCESS {
        return Err("the firmware has no free memory");
    }
    unsafe { core::ptr::write_bytes(addr as *mut u8, 0, count * PAGE_SIZE as usize) };
    Ok(addr)
}

unsafe fn start(image: Handle, st: *mut SystemTable) -> &'static str {
    match unsafe { prepare(image, st) } {
        Err(reason) => reason,
    }
}

enum Never {}

unsafe fn prepare(image: Handle, st: *mut SystemTable) -> Result<Never, &'static str> {
    let bs = unsafe { &*(*st).boot_services };
    // The firmware restarts the PC if a boot program takes more than 5 minutes; not this one's job.
    unsafe { (bs.set_watchdog_timer)(0, 0, 0, null()) };

    let cpu = paging::Features::detect();
    if cpu.five_level {
        return Err("the firmware turned on 5-level paging, which Chat Computer can't use yet");
    }

    let (framebuffer, edid, edid_len) = unsafe { screen(bs, st)? };
    let rsdp = unsafe { find_rsdp(st) };

    // Copy each part of the kernel to its own pages.
    let kernel = elf::Elf::parse(KERNEL)?;
    let mut segments = [KernelSegment::default(); MAX_SEGMENTS];
    let mut count = 0;
    for load in kernel.loads() {
        let load = load?;
        if count == MAX_SEGMENTS {
            return Err("the kernel has too many parts");
        }
        let page_offset = load.vaddr % PAGE_SIZE;
        let size = (page_offset + load.memsz).div_ceil(PAGE_SIZE) * PAGE_SIZE;
        let virt = load.vaddr - page_offset;
        for s in &segments[..count] {
            if virt < s.virt + s.size && s.virt < virt + size {
                return Err("two parts of the kernel share a page");
            }
        }
        let phys = unsafe { pages(bs, (size / PAGE_SIZE) as usize)? };
        let bytes = kernel.bytes(&load);
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), (phys + page_offset) as *mut u8, bytes.len()) };
        let mut flags = 0;
        if load.flags & elf::PF_X != 0 {
            flags |= bootinfo::SEGMENT_EXECUTE;
        }
        if load.flags & elf::PF_W != 0 {
            flags |= bootinfo::SEGMENT_WRITE;
        }
        segments[count] = KernelSegment { virt, phys, size, flags, _reserved: 0 };
        count += 1;
    }
    if count == 0 {
        return Err("the kernel is empty");
    }

    let stack = unsafe { pages(bs, STACK_PAGES)? };

    // Room for the memory map, twice: the firmware's form, then Chat Computer's. Allocating changes the map,
    // so leave room for more entries than there are now.
    let (raw_size, desc_size) = unsafe { map_size(bs)? };
    let raw_pages = ((raw_size / desc_size + 64) * desc_size).div_ceil(PAGE_SIZE as usize);
    let raw = unsafe { pages(bs, raw_pages)? };
    let max_entries = raw_pages * PAGE_SIZE as usize / desc_size; // as many as the firmware's form can hold
    let info_bytes = size_of::<BootInfo>() + max_entries * size_of::<MemoryRegion>();
    let info_phys = unsafe { pages(bs, info_bytes.div_ceil(PAGE_SIZE as usize))? };

    // Page tables: enough for all of RAM and the kernel. Where RAM ends is known from the current map.
    let mut size = raw_pages * PAGE_SIZE as usize;
    let mut key = 0usize;
    let mut dsize = desc_size;
    let mut version = 0u32;
    if unsafe { (bs.get_memory_map)(&mut size, raw as *mut u8, &mut key, &mut dsize, &mut version) } != SUCCESS {
        return Err("the firmware wouldn't give its memory map");
    }
    let ram_end = unsafe { regions(raw, size, dsize) }
        .filter(|r| r.is_ram())
        .map(|r| r.end())
        .max()
        .unwrap_or(0);
    let kernel_low = segments[..count].iter().map(|s| s.virt).min().unwrap_or(0);
    let kernel_high = segments[..count].iter().map(|s| s.virt + s.size).max().unwrap_or(0);
    let table_pages = 1 // the top-level table
        + paging::direct_map_tables(ram_end, &cpu)
        + 2 + ((kernel_high - kernel_low).div_ceil(paging::MIB2) as usize + 1) // the kernel's
        + 8;
    let pool_phys = unsafe { pages(bs, table_pages)? };

    // The firmware's name, for the record.
    let info = unsafe { &mut *(info_phys as *mut BootInfo) };
    unsafe {
        let vendor = (*st).firmware_vendor;
        if !vendor.is_null() {
            for i in 0..info.firmware_vendor.len() - 1 {
                let c = *vendor.add(i);
                if c == 0 {
                    break;
                }
                info.firmware_vendor[i] = c;
            }
        }
        info.uefi_revision = (*st).hdr.revision;
        info.secure_boot = secure_boot(st);
        (info.time, info.time_zone) = clock(st);
        info.settings_len = settings(bs, image, &mut info.settings) as u32;
        info.start_disk = start_disk(bs, image);
    }

    // Last call: the final memory map, then ExitBootServices. If something (a timer event, say) changed
    // the map in between, the firmware refuses; get the map again and retry. After the first attempt,
    // even a failed one, the firmware may already have shut part of itself down, and only the memory
    // map may still be asked for: no more messages, so a failure from then on can only stop here.
    let mut tried = false;
    let mut exited = false;
    for _ in 0..8 {
        size = raw_pages * PAGE_SIZE as usize;
        if unsafe { (bs.get_memory_map)(&mut size, raw as *mut u8, &mut key, &mut dsize, &mut version) } != SUCCESS {
            if !tried {
                return Err("the firmware wouldn't give its memory map");
            }
            break;
        }
        tried = true;
        if unsafe { (bs.exit_boot_services)(image, key) } == SUCCESS {
            exited = true;
            break;
        }
    }
    SYSTEM_TABLE.store(null_mut(), Ordering::Release); // no more messages either way
    unsafe { asm!("cli", options(nomem, nostack)) };
    if !exited {
        halt();
    }
    // From here on the firmware's services are gone: no allocations.

    let region_phys = info_phys + size_of::<BootInfo>() as u64;
    let mut n = 0;
    for r in unsafe { regions(raw, size, dsize) } {
        if n == max_entries {
            break;
        }
        unsafe { *(region_phys as *mut MemoryRegion).add(n) = r };
        n += 1;
    }
    info.magic = bootinfo::MAGIC;
    info.version = bootinfo::VERSION;
    info.framebuffer = framebuffer;
    info.memory_map = PHYS_OFFSET + region_phys;
    info.memory_map_len = n as u64;
    info.rsdp = rsdp;
    info.kernel_segments = segments;
    info.kernel_segment_count = count as u32;
    info.edid = edid;
    info.edid_len = edid_len;
    info.stack_phys = stack;
    info.stack_size = STACK_PAGES as u64 * PAGE_SIZE;

    // The kernel's first page tables. The pool's first page is the top-level table.
    let pml4 = pool_phys;
    let mut pool = paging::Pool::new(pool_phys + PAGE_SIZE, table_pages - 1);
    unsafe {
        let firmware = paging::read_cr3() as *const u64;
        for i in 0..256 {
            *(pml4 as *mut u64).add(i) = *firmware.add(i);
        }
    }
    let nx = if cpu.no_execute { paging::NO_EXECUTE } else { 0 };
    let ok = unsafe { paging::map_direct(&mut pool, pml4, PHYS_OFFSET, ram_end, &cpu) }.is_some()
        && segments[..count].iter().all(|s| {
            let mut flags = paging::PRESENT;
            if s.flags & bootinfo::SEGMENT_WRITE != 0 {
                flags |= paging::WRITABLE;
            }
            if s.flags & bootinfo::SEGMENT_EXECUTE == 0 {
                flags |= nx;
            }
            (0..s.size / PAGE_SIZE).all(|i| {
                let off = i * PAGE_SIZE;
                unsafe { paging::map(&mut pool, pml4, s.virt + off, s.phys + off, paging::KIB4, flags) }
                    .is_some()
            })
        });
    if !ok {
        halt(); // nothing can be shown any more; this can only be a bug in Chat Computer
    }

    if cpu.no_execute {
        unsafe { paging::enable_no_execute() };
    }
    let stack_top = PHYS_OFFSET + stack + STACK_PAGES as u64 * PAGE_SIZE;
    let info_virt = PHYS_OFFSET + info_phys;
    unsafe {
        asm!(
            "mov cr3, {pml4}",
            "mov rsp, {stack}",
            "xor ebp, ebp",
            "call {entry}",
            "2:",
            "hlt",
            "jmp 2b",
            pml4 = in(reg) pml4,
            stack = in(reg) stack_top,
            entry = in(reg) kernel.entry,
            in("rdi") info_virt,
            options(noreturn)
        );
    }
}

/// The PC's clock, in seconds since 1970 as if it were UTC, and its time zone as the firmware gives it; 0 if it
/// can't be read.
unsafe fn clock(st: *mut SystemTable) -> (i64, i32) {
    let rt = unsafe { (*st).runtime_services };
    if rt.is_null() {
        return (0, uefi::UNSPECIFIED_TIMEZONE as i32);
    }
    let mut t = uefi::Time::default();
    if unsafe { ((*rt).get_time)(&mut t, null_mut()) } != SUCCESS
        || t.year < 1970
        || !(1..=12).contains(&t.month)
        || !(1..=31).contains(&t.day)
        || t.hour > 23
        || t.minute > 59
        || t.second > 59
    {
        return (0, uefi::UNSPECIFIED_TIMEZONE as i32);
    }
    let (y, m, d) = (t.year as i64, t.month as i64, t.day as i64);
    // Days from 1970 (days_from_civil).
    let yy = if m <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    (days * 86_400 + t.hour as i64 * 3600 + t.minute as i64 * 60 + t.second as i64, t.time_zone as i32)
}

/// Chat Computer's settings file, from the partition this program was started from: how many bytes went into
/// `out` (0 if there's no such file).
unsafe fn settings(bs: &BootServices, image: Handle, out: &mut [u8]) -> usize {
    const PATH: &str = "\\EFI\\chatcomputer-own\\settings.txt";
    let mut loaded: *mut core::ffi::c_void = null_mut();
    if unsafe { (bs.handle_protocol)(image, &uefi::LOADED_IMAGE, &mut loaded) } != SUCCESS || loaded.is_null() {
        return 0;
    }
    let device = unsafe { (*(loaded as *mut uefi::LoadedImage)).device_handle };
    let mut fs: *mut core::ffi::c_void = null_mut();
    if unsafe { (bs.handle_protocol)(device, &uefi::SIMPLE_FILE_SYSTEM, &mut fs) } != SUCCESS || fs.is_null() {
        return 0;
    }
    let fs = fs as *mut uefi::SimpleFileSystem;
    let mut root: *mut uefi::File = null_mut();
    if unsafe { ((*fs).open_volume)(fs, &mut root) } != SUCCESS || root.is_null() {
        return 0;
    }
    let mut name = [0u16; 64];
    for (i, c) in PATH.encode_utf16().enumerate() {
        name[i] = c;
    }
    let mut file: *mut uefi::File = null_mut();
    let mut got = 0;
    if unsafe { ((*root).open)(root, &mut file, name.as_ptr(), uefi::FILE_MODE_READ, 0) } == SUCCESS && !file.is_null() {
        let mut size = out.len();
        if unsafe { ((*file).read)(file, &mut size, out.as_mut_ptr() as *mut core::ffi::c_void) } == SUCCESS {
            got = size.min(out.len());
        }
        unsafe { ((*file).close)(file) };
    }
    unsafe { ((*root).close)(root) };
    got
}

/// The drive and partition this program was started from: the firmware's device path for its partition, read node
/// by node (UEFI 2.10, 10.3): the PCI root bridge (ACPI, PNP0A03 or PNP0A08), the PCI devices on the way, the NVMe
/// namespace or SATA port, and the partition (a hard drive media node). Anything else leaves the kind unknown.
unsafe fn start_disk(bs: &BootServices, image: Handle) -> StartDisk {
    let mut d = StartDisk { sata_multiplier: 0xFFFF, ..StartDisk::default() };
    let mut loaded: *mut core::ffi::c_void = null_mut();
    if unsafe { (bs.handle_protocol)(image, &uefi::LOADED_IMAGE, &mut loaded) } != SUCCESS || loaded.is_null() {
        return d;
    }
    let device = unsafe { (*(loaded as *mut uefi::LoadedImage)).device_handle };
    let mut path: *mut core::ffi::c_void = null_mut();
    if unsafe { (bs.handle_protocol)(device, &uefi::DEVICE_PATH, &mut path) } != SUCCESS || path.is_null() {
        return d;
    }
    let mut at = path as *const u8;
    let (mut root, mut kind) = (false, bootinfo::DISK_UNKNOWN);
    // (A path of more than 64 nodes isn't a real one.)
    for _ in 0..64 {
        let (t, sub) = unsafe { (*at, *at.add(1)) };
        let len = unsafe { (at.add(2) as *const u16).read_unaligned() } as usize;
        if (t, sub) == (0x7F, 0xFF) || len < 4 {
            break;
        }
        let u32_at = |o: usize| unsafe { (at.add(o) as *const u32).read_unaligned() };
        let u64_at = |o: usize| unsafe { (at.add(o) as *const u64).read_unaligned() };
        let u16_at = |o: usize| unsafe { (at.add(o) as *const u16).read_unaligned() };
        match (t, sub) {
            (2, 1) if len >= 12 && matches!(u32_at(4), 0x0A03_41D0 | 0x0A08_41D0) => {
                root = true;
                d.pci_root = u32_at(8);
            }
            (1, 1) if len >= 6 && root && (d.path_len as usize) < d.path.len() => {
                d.path[d.path_len as usize] = unsafe { [*at.add(5), *at.add(4)] }; // (device, function)
                d.path_len += 1;
            }
            (3, 0x17) if len >= 8 => {
                kind = bootinfo::DISK_NVME;
                d.nvme_namespace = u32_at(4);
            }
            (3, 0x12) if len >= 10 => {
                kind = bootinfo::DISK_SATA;
                d.sata_port = u16_at(4);
                d.sata_multiplier = u16_at(6);
            }
            (4, 1) if len >= 24 => {
                d.partition_start = u64_at(8);
                d.partition_blocks = u64_at(16);
            }
            // Anything else on the way (USB, SCSI...): not a drive this kernel has a driver for.
            (3, _) => kind = bootinfo::DISK_UNKNOWN,
            _ => {}
        }
        at = unsafe { at.add(len) };
    }
    if root && d.path_len > 0 && d.partition_start != 0 {
        d.kind = kind;
    }
    d
}

fn halt() -> ! {
    loop {
        unsafe { asm!("cli", "hlt", options(nomem, nostack)) };
    }
}

/// How big the firmware's memory map is right now, and the size of one entry.
unsafe fn map_size(bs: &BootServices) -> Result<(usize, usize), &'static str> {
    let (mut size, mut key, mut dsize, mut version) = (0usize, 0usize, 0usize, 0u32);
    let status = unsafe { (bs.get_memory_map)(&mut size, null_mut(), &mut key, &mut dsize, &mut version) };
    if status != BUFFER_TOO_SMALL || dsize < size_of::<MemoryDescriptor>() {
        return Err("the firmware wouldn't give its memory map");
    }
    Ok((size, dsize))
}

unsafe fn regions(raw: u64, size: usize, dsize: usize) -> impl Iterator<Item = MemoryRegion> {
    (0..size / dsize).map(move |i| {
        let d = unsafe { ((raw as usize + i * dsize) as *const MemoryDescriptor).read_unaligned() };
        MemoryRegion { start: d.phys_start, pages: d.pages, kind: d.kind, _reserved: 0, attribute: d.attribute }
    })
}

/// The screen the firmware is showing, set to the monitor's own resolution when the firmware offers it.
unsafe fn screen(bs: &BootServices, st: *mut SystemTable) -> Result<(Framebuffer, [u8; 256], u32), &'static str> {
    let mut count = 0usize;
    let mut handles: *mut Handle = null_mut();
    let status = unsafe { (bs.locate_handle_buffer)(BY_PROTOCOL, &GRAPHICS_OUTPUT, null_mut(), &mut count, &mut handles) };
    if status != SUCCESS || count == 0 {
        return Err("the firmware offers no graphics screen");
    }
    // The firmware's console may be a virtual device that mirrors real ones (and has no EDID). Prefer a
    // real graphics device (it has a device path); of those, the one the console is showing on (the
    // same framebuffer), then one with a monitor that describes itself. Otherwise, any device with a
    // usable framebuffer.
    let console_base = unsafe { gop_on(bs, (*st).console_out_handle) }.map(|g| unsafe { (*(*g).mode).frame_buffer_base });
    let mut chosen: *mut GraphicsOutput = null_mut();
    let mut chosen_handle: Handle = null_mut();
    let mut chosen_rank = 0;
    for i in 0..count {
        let handle = unsafe { *handles.add(i) };
        let Some(gop) = (unsafe { gop_on(bs, handle) }) else { continue };
        let mut path: *mut c_void = null_mut();
        let real = unsafe { (bs.handle_protocol)(handle, &DEVICE_PATH, &mut path) } == SUCCESS && !path.is_null();
        let shows_console = Some(unsafe { (*(*gop).mode).frame_buffer_base }) == console_base;
        let has_monitor = unsafe { edid_on(bs, handle) }.is_some();
        let rank = 1 + real as u32 * 4 + shows_console as u32 * 2 + has_monitor as u32;
        if rank > chosen_rank {
            (chosen, chosen_handle, chosen_rank) = (gop, handle, rank);
        }
    }
    unsafe { (bs.free_pool)(handles as *mut c_void) };
    if chosen.is_null() {
        return Err("the screen can't be drawn on directly (no linear framebuffer)");
    }

    // What the monitor says about itself.
    let mut edid = [0u8; 256];
    let mut edid_len = 0u32;
    if let Some(e) = unsafe { edid_on(bs, chosen_handle) } {
        let n = (e.size as usize).min(edid.len());
        unsafe { core::ptr::copy_nonoverlapping(e.edid, edid.as_mut_ptr(), n) };
        edid_len = n as u32;
    }

    // The monitor's own resolution, if the firmware has a mode for it. If the new mode turns out not to
    // be drawable after all, go back to the one that was.
    if let Some((w, h)) = bootinfo::edid::parse(&edid[..edid_len as usize]).and_then(|e| e.preferred_mode()) {
        unsafe {
            let mode = &*(*chosen).mode;
            let current = &*mode.info;
            let was = mode.mode;
            if current.width != w || current.height != h {
                for m in 0..mode.max_mode {
                    let mut size = 0usize;
                    let mut info: *mut ModeInfo = null_mut();
                    if ((*chosen).query_mode)(chosen, m, &mut size, &mut info) != SUCCESS || info.is_null() {
                        continue;
                    }
                    let fits = (*info).width == w && (*info).height == h && drawable(&*info);
                    (bs.free_pool)(info as *mut c_void);
                    if fits {
                        if ((*chosen).set_mode)(chosen, m) == SUCCESS && gop_on(bs, chosen_handle).is_none() {
                            let _ = ((*chosen).set_mode)(chosen, was);
                        }
                        break;
                    }
                }
            }
        }
    }
    if unsafe { gop_on(bs, chosen_handle) }.is_none() {
        return Err("the screen can't be drawn on directly (no linear framebuffer)");
    }

    let (mode, info) = unsafe { (&*(*chosen).mode, &*(*(*chosen).mode).info) };
    let fb = Framebuffer {
        phys: mode.frame_buffer_base,
        size: mode.frame_buffer_size as u64,
        width: info.width,
        height: info.height,
        stride: info.pixels_per_scan_line,
        format: info.pixel_format,
        red_mask: info.red_mask,
        green_mask: info.green_mask,
        blue_mask: info.blue_mask,
        reserved_mask: info.reserved_mask,
    };
    Ok((fb, edid, edid_len))
}

/// The graphics output on `handle`, if it has a linear framebuffer Chat Computer can draw into: 32 bits a
/// pixel, at a real address, big enough for the picture.
unsafe fn gop_on(bs: &BootServices, handle: Handle) -> Option<*mut GraphicsOutput> {
    if handle.is_null() {
        return None;
    }
    let mut p: *mut c_void = null_mut();
    if unsafe { (bs.handle_protocol)(handle, &GRAPHICS_OUTPUT, &mut p) } != SUCCESS || p.is_null() {
        return None;
    }
    let gop = p as *mut GraphicsOutput;
    if unsafe { (*gop).mode.is_null() || (*(*gop).mode).info.is_null() } {
        return None;
    }
    let (mode, info) = unsafe { (&*(*gop).mode, &*(*(*gop).mode).info) };
    let needed = info.pixels_per_scan_line as u64 * info.height as u64 * 4;
    let usable = mode.frame_buffer_base != 0
        && (mode.frame_buffer_size == 0 || mode.frame_buffer_size as u64 >= needed)
        && drawable(info);
    if usable { Some(gop) } else { None }
}

/// The monitor's description (EDID) the firmware keeps on `handle`: the one in use, else the one the
/// monitor sent. At least one 128-byte block.
unsafe fn edid_on(bs: &BootServices, handle: Handle) -> Option<&'static EdidProtocol> {
    for guid in [&EDID_ACTIVE, &EDID_DISCOVERED] {
        let mut p: *mut c_void = null_mut();
        if unsafe { (bs.handle_protocol)(handle, guid, &mut p) } == SUCCESS && !p.is_null() {
            let e = unsafe { &*(p as *const EdidProtocol) };
            if e.size >= 128 && !e.edid.is_null() {
                return Some(e);
            }
        }
    }
    None
}

/// Whether a mode's pixels are in memory in a form Chat Computer draws: 32 bits each, rows at least as long
/// as the picture is wide.
fn drawable(info: &ModeInfo) -> bool {
    info.width > 0
        && info.height > 0
        && info.pixels_per_scan_line >= info.width
        && match info.pixel_format {
            PIXEL_RGB_RESERVED | PIXEL_BGR_RESERVED => true,
            // Pixels described by masks: only 32-bit pixels (the top byte holds part of the pixel).
            PIXEL_BIT_MASK => (info.red_mask | info.green_mask | info.blue_mask | info.reserved_mask).leading_zeros() < 8,
            _ => false, // PIXEL_BLT_ONLY: no framebuffer to draw into
        }
}

/// Whether Secure Boot is on: the firmware's "SecureBoot" variable (1 = on), read while it can still be.
unsafe fn secure_boot(st: *mut SystemTable) -> u32 {
    let rt = unsafe { (*st).runtime_services };
    if rt.is_null() {
        return bootinfo::SECURE_BOOT_UNKNOWN;
    }
    let name: [u16; 11] = [b'S' as u16, b'e' as u16, b'c' as u16, b'u' as u16, b'r' as u16, b'e' as u16,
                           b'B' as u16, b'o' as u16, b'o' as u16, b't' as u16, 0];
    let mut value = 0u8;
    let mut size = 1usize;
    let status = unsafe {
        ((*rt).get_variable)(name.as_ptr(), &GLOBAL_VARIABLE, null_mut(), &mut size, &mut value as *mut u8 as *mut c_void)
    };
    match (status, value) {
        (SUCCESS, 1) => bootinfo::SECURE_BOOT_ON,
        (SUCCESS, 0) => bootinfo::SECURE_BOOT_OFF,
        _ => bootinfo::SECURE_BOOT_UNKNOWN,
    }
}

/// Where the ACPI tables start (ACPI 2.0 or later preferred), or 0.
unsafe fn find_rsdp(st: *mut SystemTable) -> u64 {
    let (n, tables) = unsafe { ((*st).number_of_table_entries, (*st).configuration_table) };
    let mut v1 = 0;
    for i in 0..n {
        let t = unsafe { &*tables.add(i) };
        if t.vendor_guid == ACPI_20_TABLE {
            return t.vendor_table as u64;
        }
        if t.vendor_guid == ACPI_10_TABLE {
            v1 = t.vendor_table as u64;
        }
    }
    v1
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    say("\r\nChat Computer's start-up stage hit a bug.\r\n");
    halt()
}
