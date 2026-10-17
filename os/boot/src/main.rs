//! ClaudeOS's start-up stage: the UEFI program the PC's firmware runs (\EFI\BOOT\BOOTX64.EFI).
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

use bootinfo::{BootInfo, Framebuffer, KernelSegment, MemoryRegion, MAX_SEGMENTS, PAGE_SIZE, PHYS_OFFSET};
use core::arch::asm;
use core::ffi::c_void;
use core::mem::size_of;
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicPtr, Ordering};
use uefi::*;

/// The kernel, put here by build.sh.
static KERNEL: &[u8] = include_bytes!(env!("CLAUDEOS_KERNEL_ELF"));

/// The firmware's table, while its services may still be used (null after ExitBootServices).
static SYSTEM_TABLE: AtomicPtr<SystemTable> = AtomicPtr::new(null_mut());

const STACK_PAGES: usize = 64; // the kernel's first stack: 256 KiB

#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(image: Handle, st: *mut SystemTable) -> Status {
    SYSTEM_TABLE.store(st, Ordering::Release);
    // `start` only comes back if something stopped it.
    let reason = unsafe { start(image, st) };
    say("\r\nClaudeOS couldn't start: ");
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
        return Err("the firmware turned on 5-level paging, which ClaudeOS can't use yet");
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

    // Room for the memory map, twice: the firmware's form, then ClaudeOS's. Allocating changes the map,
    // so leave room for more entries than there are now.
    let (raw_size, desc_size) = unsafe { map_size(bs)? };
    let max_entries = raw_size / desc_size + 64;
    let raw_pages = (max_entries * desc_size).div_ceil(PAGE_SIZE as usize);
    let raw = unsafe { pages(bs, raw_pages)? };
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
    }

    // Last call: the final memory map, then ExitBootServices. If something changed the map in between,
    // the firmware refuses once; get the map again and retry.
    let mut exited = false;
    for _ in 0..2 {
        size = raw_pages * PAGE_SIZE as usize;
        if unsafe { (bs.get_memory_map)(&mut size, raw as *mut u8, &mut key, &mut dsize, &mut version) } != SUCCESS {
            break;
        }
        if unsafe { (bs.exit_boot_services)(image, key) } == SUCCESS {
            exited = true;
            break;
        }
    }
    if !exited {
        return Err("the firmware wouldn't hand over the PC (ExitBootServices failed)");
    }
    // From here on the firmware's services are gone: no more messages, no allocations.
    SYSTEM_TABLE.store(null_mut(), Ordering::Release);
    unsafe { asm!("cli", options(nomem, nostack)) };

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
        halt(); // nothing can be shown any more; this can only be a bug in ClaudeOS
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
    // Prefer the screen the firmware's own text goes to.
    let console = unsafe { (*st).console_out_handle };
    let mut chosen: *mut GraphicsOutput = null_mut();
    let mut chosen_handle: Handle = null_mut();
    for i in 0..count {
        let handle = unsafe { *handles.add(i) };
        let mut gop: *mut c_void = null_mut();
        if unsafe { (bs.handle_protocol)(handle, &GRAPHICS_OUTPUT, &mut gop) } != SUCCESS || gop.is_null() {
            continue;
        }
        let gop = gop as *mut GraphicsOutput;
        let format = unsafe { (*(*(*gop).mode).info).pixel_format };
        if format == PIXEL_BLT_ONLY {
            continue; // no framebuffer to draw into
        }
        if chosen.is_null() || handle == console {
            chosen = gop;
            chosen_handle = handle;
        }
    }
    unsafe { (bs.free_pool)(handles as *mut c_void) };
    if chosen.is_null() {
        return Err("the screen can't be drawn on directly (no linear framebuffer)");
    }

    // What the monitor says about itself.
    let mut edid = [0u8; 256];
    let mut edid_len = 0u32;
    for guid in [&EDID_ACTIVE, &EDID_DISCOVERED] {
        let mut p: *mut c_void = null_mut();
        if unsafe { (bs.handle_protocol)(chosen_handle, guid, &mut p) } == SUCCESS && !p.is_null() {
            let e = unsafe { &*(p as *const EdidProtocol) };
            if e.size >= 128 && !e.edid.is_null() {
                let n = (e.size as usize).min(edid.len());
                unsafe { core::ptr::copy_nonoverlapping(e.edid, edid.as_mut_ptr(), n) };
                edid_len = n as u32;
                break;
            }
        }
    }

    // The monitor's own resolution, if the firmware has a mode for it.
    if let Some((w, h)) = bootinfo::edid::parse(&edid[..edid_len as usize]).and_then(|e| e.preferred_mode()) {
        unsafe {
            let mode = &*(*chosen).mode;
            let current = &*mode.info;
            if current.width != w || current.height != h {
                for m in 0..mode.max_mode {
                    let mut size = 0usize;
                    let mut info: *mut ModeInfo = null_mut();
                    if ((*chosen).query_mode)(chosen, m, &mut size, &mut info) != SUCCESS || info.is_null() {
                        continue;
                    }
                    let fits = (*info).width == w && (*info).height == h && (*info).pixel_format != PIXEL_BLT_ONLY;
                    (bs.free_pool)(info as *mut c_void);
                    if fits {
                        let _ = ((*chosen).set_mode)(chosen, m);
                        break;
                    }
                }
            }
        }
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
    say("\r\nClaudeOS's start-up stage hit a bug.\r\n");
    halt()
}
