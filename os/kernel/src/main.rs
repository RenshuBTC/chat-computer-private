//! The Chat Computer kernel.
//!
//! The start-up stage jumps to `kernel_entry` with interrupts off, on a fresh stack, with temporary
//! page tables. From there the kernel sets up everything itself, in this order: its log, its own
//! segment and interrupt tables, its record of free memory, its own page tables, its heap, the screen,
//! the ACPI tables, its interrupt controller and timer, the PCI devices, and the USB controllers (taken
//! from the firmware before the firmware's memory is reused, started once interrupts are on).
#![no_std]
#![no_main]

extern crate alloc;

#[macro_use]
mod serial;
mod acpi;
mod apic;
mod assistant;
mod chat;
mod clock;
mod cpu;
mod dma;
mod draw;
mod gdt;
mod heap;
mod idt;
mod iommu;
mod keep;
mod keyboard;
mod layout;
mod memory;
mod mouse;
mod netcard;
mod network;
mod paging;
mod pci;
mod power;
mod ps2;
mod random;
mod rich;
mod screen;
mod settings;
mod storage;
mod sync;
mod system;
mod text;
mod time;
mod usb;
#[path = "../../shared/mem.rs"]
mod mem;

use alloc::string::String;
use bootinfo::BootInfo;

/// # Safety
/// Only the start-up stage calls this, once, with what it prepared.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel_entry(info: *const BootInfo) -> ! {
    serial::init();
    log!("Chat Computer kernel: started");
    let info = unsafe { &*info };
    if info.magic != bootinfo::MAGIC || info.version != bootinfo::VERSION {
        panic!("the start-up stage passed something this kernel doesn't understand");
    }
    gdt::init();
    idt::init();
    log!("Chat Computer kernel: own segment and interrupt tables loaded");
    log!(
        "Chat Computer kernel: Secure Boot {}",
        match info.secure_boot {
            bootinfo::SECURE_BOOT_ON => "on",
            bootinfo::SECURE_BOOT_OFF => "off",
            _ => "unknown",
        }
    );

    memory::init(info);
    paging::init(info);
    log!("Chat Computer kernel: own page tables in use");
    let heap = heap::init();
    log!("Chat Computer kernel: heap of {} MiB", heap >> 20);
    // The settings file and the clock, as the start-up stage read them (the key in the settings is never logged).
    let settings = settings::parse(info.settings());
    log!(
        "Chat Computer kernel: settings: {}{}",
        match (info.settings_len, &settings.claude_api_key, settings.key_problem) {
            (0, _, _) => "none",
            (_, Some(_), _) => "a Claude API key",
            (_, None, Some(_)) => "a Claude API key that can't be used",
            _ => "no Claude API key",
        },
        if settings.not_understood > 0 { alloc::format!(", {} lines not understood", settings.not_understood) } else { String::new() }
    );
    log!("Chat Computer kernel: clock: {}", clock::init(info.time, info.time_zone, settings.clock_offset_minutes));

    // The screen comes next, so that if anything after this stops the kernel, the screen says so.
    let brand = cpu::brand();
    let cpu_name: String = String::from_utf8_lossy(&brand).split_whitespace().collect::<alloc::vec::Vec<_>>().join(" ");
    let size_mm = bootinfo::edid::parse(info.edid()).and_then(|e| e.size_mm());
    let firmware: String = char::decode_utf16(info.firmware_vendor.iter().copied().take_while(|&c| c != 0))
        .map(|c| c.unwrap_or('\u{FFFD}'))
        .collect();
    let mut facts = system::Facts {
        cpu: cpu_name.clone(),
        screen: (info.framebuffer.width as usize, info.framebuffer.height as usize),
        diagonal_in: None,
        secure_boot: match info.secure_boot {
            bootinfo::SECURE_BOOT_ON => Some(true),
            bootinfo::SECURE_BOOT_OFF => Some(false),
            _ => None,
        },
        firmware,
        ps2_keyboard: None,
    };
    match screen::Screen::new(&info.framebuffer) {
        Some(screen) => {
            let (w, h) = (screen.width, screen.height);
            let layout = chat::Chat::show(screen, size_mm, &cpu_name, info.secure_boot);
            if layout.size_known {
                facts.diagonal_in = Some((layout.diagonal_in + 0.5) as u32);
            }
            log!(
                "Chat Computer kernel: screen {w}x{h}, monitor size {:?} mm, viewed from about {} cm{}: drawn at {}%, text {}.{} px",
                size_mm,
                (layout.distance_mm / 10.0) as u32,
                if layout.size_known { "" } else { " (size assumed)" },
                (layout.scale * 100.0) as u32,
                layout.font_px as u32,
                (layout.font_px * 10.0) as u32 % 10
            );
        }
        None => log!("Chat Computer kernel: no screen to draw on"),
    }
    system::set(facts);

    let acpi = acpi::Acpi::read(info.rsdp);
    power::init(&acpi);
    log!(
        "Chat Computer kernel: ACPI {} from {}: {} processor cores, {} I/O APICs, timer {:?}",
        acpi.revision,
        core::str::from_utf8(&acpi.oem).unwrap_or("?").trim_end(),
        acpi.cores,
        acpi.io_apic_count,
        acpi.pm_timer.map(|t| t.port)
    );
    for io in &acpi.io_apics[..acpi.io_apic_count] {
        log!("Chat Computer kernel: I/O APIC {} at {:#x}, inputs from {}", io.id, io.address, io.first_interrupt);
    }

    let (apic_id, x2apic) = apic::init(&acpi);
    log!("Chat Computer kernel: interrupt controller {} ({} mode)", apic_id, if x2apic { "x2APIC" } else { "xAPIC" });
    let (speed, how) = time::init(&acpi);
    log!("Chat Computer kernel: timer speed {how}: {speed} counts a second, {} ticks a second", time::HZ);
    let (tsc, measured) = time::tsc_mhz();
    log!(
        "Chat Computer kernel: time-stamp counter {tsc} MHz{}; time since start-up counted by {}",
        if measured { "" } else { " (assumed)" },
        time::uptime_by()
    );

    let (iommus, left_on) = iommu::take_over(&acpi);
    if iommus > 0 {
        let s = if iommus == 1 { "" } else { "s" };
        log!("Chat Computer kernel: {iommus} IOMMU{s} (not used yet), {left_on} of them left on by the firmware");
    }
    let (functions, how) = pci::init(&acpi);
    log!("Chat Computer kernel: {functions} PCI device functions, configuration read {how}");
    // The firmware may still be using its USB controllers, with memory it was given: take them over and
    // stop them before that memory is reused.
    usb::take_over();
    let freed = memory::reclaim_firmware_memory(unsafe { info.memory_map() });
    log!("Chat Computer kernel: {} MiB the firmware used is free again", freed >> 20);
    // After the USB controllers: a firmware that made a USB keyboard look like a PS/2 one has stopped.
    let ps2_name = if acpi.portable { "built-in keyboard" } else { "PS/2 keyboard" };
    let ps2 = ps2::init();
    system::set_ps2(ps2.then_some(ps2_name));
    log!("Chat Computer kernel: PS/2 keyboard {}", if ps2 { "found" } else { "not found" });

    cpu::enable_interrupts();
    usb::start();
    let cards = network::init();
    if cards == 0 {
        log!("Chat Computer kernel: network: no card Chat Computer has a driver for");
    }
    // The memory file on the start-up drive, and the conversation it kept.
    log!("Chat Computer kernel: {}", keep::init(&info.start_disk, settings.memory_id));
    assistant::init(settings.claude_api_key, settings.key_problem);
    chat::restore(&assistant::conversation());
    log!("Chat Computer: ready");

    // Tests of the crash path, built in only when asked for (CHATCOMPUTER_TEST=page-fault ./build.sh).
    match option_env!("CHATCOMPUTER_TEST") {
        Some("page-fault") => {
            log!("Chat Computer kernel: test: reading from an address nothing is mapped at");
            let _ = unsafe { core::ptr::read_volatile(0x10 as *const u64) };
        }
        Some("panic") => panic!("a test of what happens when the kernel stops"),
        _ => {}
    }

    let mut keyboards_shown = u64::MAX;
    loop {
        cpu::wait_for_interrupt();
        network::work();
        for event in assistant::work() {
            chat::claude(event);
        }
        assistant::keep();
        usb::work();
        ps2::poll();
        keyboard::repeat();
        while let Some(press) = keyboard::take() {
            chat::typed(press);
        }
        let turned = mouse::take_wheel();
        if turned != 0 {
            chat::wheel(turned);
        }
        if usb::changes() != keyboards_shown {
            keyboards_shown = usb::changes();
            chat::keyboards(&usb::status(), ps2::present().then_some(ps2_name));
        }
        chat::tick();
        power::due();
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    cpu::disable_interrupts();
    serial::write_anyway(format_args!("\nChat Computer kernel stopped: {info}\n"));
    let place = info.location().map_or(("?", 0), |l| (l.file(), l.line()));
    chat::show_stop("an internal error", format_args!("{} ({}, line {}).", info.message(), place.0, place.1));
    cpu::stop()
}
