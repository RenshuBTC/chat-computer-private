//! The USB controller: xHCI, the "eXtensible Host Controller Interface" every PC since about 2015 has
//! (Chat Computer follows version 1.2 of Intel's specification).
//!
//! The controller works from memory Chat Computer gives it. Requests go on rings of 16-byte records
//! (TRBs): one ring for commands to the controller, one for each channel to a device (an endpoint).
//! Ringing a doorbell tells the controller to look at a ring. Its answers come back on the event
//! ring. Each device gets a slot: the controller's record of its address and endpoints (its device
//! context), which Chat Computer changes by handing it an input context with a command.
//!
//! This file is the controller itself: taking it over from the firmware, resetting it, its rings,
//! its ports and the commands. What is plugged into it is in `device.rs`.

use super::Speed;
use crate::paging::{self, Cache};
use crate::{memory, pci, time};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use bootinfo::{PAGE_SIZE, PHYS_OFFSET};
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

// Capability registers.
const CAPLENGTH: u64 = 0x00;
const HCSPARAMS1: u64 = 0x04;
const HCSPARAMS2: u64 = 0x08;
const HCCPARAMS1: u64 = 0x10;
const DBOFF: u64 = 0x14;
const RTSOFF: u64 = 0x18;
// Operational registers.
const USBCMD: u64 = 0x00;
const USBSTS: u64 = 0x04;
const PAGESIZE: u64 = 0x08;
const CRCR: u64 = 0x18;
const DCBAAP: u64 = 0x30;
const CONFIG: u64 = 0x38;
const PORTSC: u64 = 0x400;
// The first interrupter's registers, in the runtime registers.
const IMAN: u64 = 0x20;
const IMOD: u64 = 0x24;
const ERSTSZ: u64 = 0x28;
const ERSTBA: u64 = 0x30;
const ERDP: u64 = 0x38;

const CMD_RUN: u32 = 1 << 0;
const CMD_RESET: u32 = 1 << 1;
const CMD_INTERRUPTS: u32 = 1 << 2;
const STS_HALTED: u32 = 1 << 0;
const STS_HOST_ERROR: u32 = 1 << 2;
const STS_EVENT: u32 = 1 << 3;
const STS_NOT_READY: u32 = 1 << 11;
const STS_CONTROLLER_ERROR: u32 = 1 << 12;
const IMAN_PENDING: u32 = 1 << 0;
const IMAN_ENABLE: u32 = 1 << 1;
const ERDP_BUSY: u64 = 1 << 3;

// A port's status and control register.
pub const PORT_CONNECTED: u32 = 1 << 0;
pub const PORT_ENABLED: u32 = 1 << 1;
const PORT_RESET: u32 = 1 << 4;
const PORT_POWER: u32 = 1 << 9;
const PORT_CONNECT_CHANGE: u32 = 1 << 17;
const PORT_RESET_CHANGE: u32 = 1 << 21;
const PORT_WARM_RESET_CHANGE: u32 = 1 << 19;
const PORT_WARM_RESET: u32 = 1 << 31;
/// The change bits, which a write of 1 clears.
const PORT_CHANGES: u32 = 0x7F << 17;
/// The bits a write must repeat to leave them as they are (power, indicator, wake-up settings).
const PORT_KEEP: u32 = PORT_POWER | 3 << 14 | 7 << 25;

// TRB types.
pub const TRB_NORMAL: u32 = 1;
pub const TRB_SETUP: u32 = 2;
pub const TRB_DATA: u32 = 3;
pub const TRB_STATUS: u32 = 4;
pub const TRB_ISOCH: u32 = 5;
const TRB_LINK: u32 = 6;
const TRB_ENABLE_SLOT: u32 = 9;
const TRB_DISABLE_SLOT: u32 = 10;
const TRB_ADDRESS_DEVICE: u32 = 11;
const TRB_CONFIGURE_ENDPOINT: u32 = 12;
const TRB_EVALUATE_CONTEXT: u32 = 13;
const TRB_RESET_ENDPOINT: u32 = 14;
const TRB_STOP_ENDPOINT: u32 = 15;
const TRB_SET_DEQUEUE: u32 = 16;
pub const EVENT_TRANSFER: u32 = 32;
pub const EVENT_COMMAND: u32 = 33;
pub const EVENT_PORT: u32 = 34;
pub const EVENT_HOST: u32 = 37;

// TRB control bits.
const CYCLE: u32 = 1 << 0;
const TOGGLE_CYCLE: u32 = 1 << 1;
pub const CHAIN: u32 = 1 << 4;
pub const SHORT_OK: u32 = 1 << 2; // ISP: tell about a short packet
pub const ON_COMPLETION: u32 = 1 << 5; // IOC
pub const IMMEDIATE: u32 = 1 << 6; // IDT
pub const DIR_IN: u32 = 1 << 16;
/// An isochronous TRB's "start as soon as possible" (SIA): its packet goes in the first service interval it can.
pub const ISOCH_ASAP: u32 = 1 << 31;

// Completion codes.
pub const SUCCESS: u8 = 1;
pub const SHORT_PACKET: u8 = 13;
pub const WRONG_STATE: u8 = 19;

/// Endpoint types in an endpoint context.
pub const EP_ISOCH_OUT: u32 = 1;
pub const EP_BULK_OUT: u32 = 2;
pub const EP_INTERRUPT_OUT: u32 = 3;
pub const EP_CONTROL: u32 = 4;
pub const EP_ISOCH_IN: u32 = 5;
pub const EP_BULK_IN: u32 = 6;
pub const EP_INTERRUPT_IN: u32 = 7;

const RING_TRBS: usize = PAGE_SIZE as usize / 16;

/// A page of memory for the controller. It goes back to the free list when dropped.
pub struct Page {
    pub phys: u64,
}

impl Page {
    /// `low`: the controller can only reach the first 4 GiB.
    pub fn new(low: bool) -> Option<Page> {
        let phys = if low { memory::alloc_zeroed_below(1 << 32)? } else { memory::alloc_zeroed()? };
        Some(Page { phys })
    }

    pub fn at<T>(&self, offset: usize) -> *mut T {
        (PHYS_OFFSET + self.phys + offset as u64) as *mut T
    }

    pub fn zero(&self) {
        unsafe { core::ptr::write_bytes(self.at::<u8>(0), 0, PAGE_SIZE as usize) };
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        memory::free(self.phys);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Trb {
    pub param: u64,
    pub status: u32,
    pub control: u32,
}

impl Trb {
    pub fn new(kind: u32, param: u64, status: u32, flags: u32) -> Trb {
        Trb { param, status, control: kind << 10 | flags }
    }
    pub fn kind(&self) -> u32 {
        (self.control >> 10) & 0x3F
    }
    pub fn code(&self) -> u8 {
        (self.status >> 24) as u8
    }
    /// For a transfer event: the bytes that were not transferred.
    pub fn residual(&self) -> u32 {
        self.status & 0xFF_FFFF
    }
    pub fn slot(&self) -> u8 {
        (self.control >> 24) as u8
    }
    /// For a transfer event: the endpoint (device context index).
    pub fn endpoint(&self) -> u8 {
        ((self.control >> 16) & 0x1F) as u8
    }
}

/// Whether a completion code means the transfer worked (all of it, or less than asked for).
pub fn ok(code: u8) -> bool {
    code == SUCCESS || code == SHORT_PACKET
}

/// What a completion code means, for the log.
pub fn code_name(code: u8) -> &'static str {
    match code {
        1 => "done",
        2 => "data buffer error",
        3 => "babble",
        4 => "transaction error",
        5 => "bad request",
        6 => "stalled",
        7 => "out of resources",
        8 => "out of bandwidth",
        9 => "no free slot",
        11 => "slot not enabled",
        12 => "endpoint not enabled",
        13 => "short packet",
        17 => "bad parameter",
        19 => "context in the wrong state",
        21 => "command aborted",
        26 => "command stopped",
        _ => "error",
    }
}

/// A ring of requests: one page, the last entry leading back to the first.
pub struct Ring {
    page: Page,
    enqueue: usize,
    cycle: u32,
}

impl Ring {
    pub fn new(low: bool) -> Option<Ring> {
        Some(Ring { page: Page::new(low)?, enqueue: 0, cycle: 1 })
    }

    /// Where the controller starts reading, with the cycle state it starts with (for a context).
    pub fn start(&self) -> u64 {
        self.page.phys | 1
    }

    /// Where the next request will go, with the cycle state then (to skip a failed request).
    pub fn next(&self) -> u64 {
        (self.page.phys + 16 * self.enqueue as u64) | self.cycle as u64
    }

    /// Puts a request on the ring and returns its address. The cycle bit, which hands the entry to the
    /// controller, is written last, so the controller never sees half an entry.
    pub fn push(&mut self, t: Trb) -> u64 {
        let at = self.enqueue;
        let phys = self.page.phys + 16 * at as u64;
        unsafe { write_trb(self.page.at(16 * at), t.param, t.status, (t.control & !CYCLE) | self.cycle) };
        self.enqueue += 1;
        if self.enqueue == RING_TRBS - 1 {
            // (A transfer whose TRBs are chained across the ring's end has the link in its chain too.)
            let link = TRB_LINK << 10 | TOGGLE_CYCLE | (t.control & CHAIN) | self.cycle;
            unsafe { write_trb(self.page.at(16 * self.enqueue), self.page.phys, 0, link) };
            self.enqueue = 0;
            self.cycle ^= 1;
        }
        phys
    }
}

unsafe fn write_trb(p: *mut Trb, param: u64, status: u32, control: u32) {
    unsafe {
        write_volatile(&raw mut (*p).param, param);
        write_volatile(&raw mut (*p).status, status);
        fence(Ordering::Release);
        write_volatile(&raw mut (*p).control, control);
    }
}

/// The ring the controller answers on, with its one-entry segment table.
struct EventRing {
    page: Page,
    table: Page,
    dequeue: usize,
    cycle: u32,
}

impl EventRing {
    fn new(low: bool) -> Option<EventRing> {
        let page = Page::new(low)?;
        let table = Page::new(low)?;
        unsafe {
            write_volatile(table.at::<u64>(0), page.phys);
            write_volatile(table.at::<u64>(8), RING_TRBS as u64);
        }
        Some(EventRing { page, table, dequeue: 0, cycle: 1 })
    }

    fn next(&mut self) -> Option<Trb> {
        let p: *const Trb = self.page.at(16 * self.dequeue);
        let control = unsafe { read_volatile(&raw const (*p).control) };
        if control & CYCLE != self.cycle {
            return None;
        }
        fence(Ordering::Acquire);
        let t = unsafe { Trb { param: read_volatile(&raw const (*p).param), status: read_volatile(&raw const (*p).status), control } };
        self.dequeue += 1;
        if self.dequeue == RING_TRBS {
            self.dequeue = 0;
            self.cycle ^= 1;
        }
        Some(t)
    }

    fn dequeue_phys(&self) -> u64 {
        self.page.phys + 16 * self.dequeue as u64
    }
}

/// The controller's doorbells.
#[derive(Clone, Copy)]
pub struct Doorbell(u64);

impl Doorbell {
    /// Tells the controller to look at endpoint `target` of `slot` (slot 0, target 0: the command ring).
    pub fn ring(self, slot: u8, target: u8) {
        fence(Ordering::SeqCst);
        unsafe { write_volatile((self.0 + 4 * slot as u64) as *mut u32, target as u32) };
    }
}

pub struct Host {
    /// Its PCI address, for the log ("00:14.0").
    pub name: String,
    cap: u64,
    op: u64,
    rt: u64,
    db: u64,
    pub ports: u8,
    pub max_slots: u8,
    /// Bytes per context: 32 or 64.
    pub context_size: usize,
    /// The version of the specification it follows (0x0100: 1.0).
    pub version: u16,
    /// It can only reach the first 4 GiB of memory.
    pub low: bool,
    /// The USB major version (2 or 3) of each port, by port number.
    pub protocol: [u8; 256],
    pub signal: pci::Signal,
    pub running: bool,
    dcbaa: Option<Page>,
    scratchpad: Vec<Page>,
    commands: Option<Ring>,
    events: Option<EventRing>,
    /// The last command completion: (the command's address, completion code, slot).
    pub command_done: Option<(u64, u8, u8)>,
    /// Ports with changes to look at, one bit per port number.
    port_changes: [u64; 4],
}

/// Intel's 7, 8 and 9 series chipsets (2012-2015) have an older controller next to the xHCI one, and the
/// sockets start out wired to the older one. These registers move every socket the firmware allows to
/// the xHCI controller, as Windows and Linux do.
fn route_intel_ports(f: &pci::Function) {
    const SWITCHABLE: [u16; 5] = [0x1E31, 0x8C31, 0x9C31, 0x8CB1, 0x9CB1];
    const USB2_ROUTE: u16 = 0xD0;
    const USB2_ALLOWED: u16 = 0xD4;
    const USB3_ENABLE: u16 = 0xD8;
    const USB3_ALLOWED: u16 = 0xDC;
    if f.vendor != 0x8086 || !SWITCHABLE.contains(&f.device) {
        return;
    }
    f.at.write32(USB3_ENABLE, f.at.read32(USB3_ALLOWED));
    f.at.write32(USB2_ROUTE, f.at.read32(USB2_ALLOWED));
    crate::log!("Chat Computer kernel: USB {}: moved the chipset's sockets to this controller", f.at);
}

fn read32(at: u64) -> u32 {
    unsafe { read_volatile(at as *const u32) }
}

fn write32(at: u64, v: u32) {
    unsafe { write_volatile(at as *mut u32, v) }
}

/// 64-bit registers are written as two halves, low first, which every controller accepts.
fn write64(at: u64, v: u64) {
    write32(at, v as u32);
    write32(at + 4, (v >> 32) as u32);
}

/// Waits up to `ms` milliseconds for `done`. False if it never happened.
fn wait_for(ms: u64, mut done: impl FnMut() -> bool) -> bool {
    let end = time::now_us() + ms * 1000;
    loop {
        if done() {
            return true;
        }
        if time::now_us() > end {
            return done();
        }
        time::delay_us(20);
    }
}

impl Host {
    /// Takes a controller over from the firmware, which may still be using it (for a keyboard in its
    /// own menus, or to make a USB keyboard look like an old PS/2 one): asks for it the official way,
    /// stops it, and resets it. Until this is done the controller may still write to memory the
    /// firmware gave it, so this runs before that memory is reused.
    pub fn take_over(f: &pci::Function) -> Result<Host, String> {
        let bar = f.at.memory_bar(0).ok_or("its registers weren't placed")?;
        f.at.enable(true);
        let cap = paging::map_device(bar.phys, bar.size.clamp(0x1000, 1 << 20), Cache::Uncached);
        let caplength = read32(cap + CAPLENGTH) & 0xFF;
        let params1 = read32(cap + HCSPARAMS1);
        let cparams1 = read32(cap + HCCPARAMS1);
        if read32(cap + CAPLENGTH) == !0 {
            return Err("it doesn't answer".into());
        }
        let mut host = Host {
            name: format!("{}", f.at),
            cap,
            op: cap + caplength as u64,
            rt: cap + (read32(cap + RTSOFF) & !0x1F) as u64,
            db: cap + (read32(cap + DBOFF) & !0x3) as u64,
            ports: (params1 >> 24) as u8,
            max_slots: params1 as u8,
            context_size: if cparams1 & (1 << 2) != 0 { 64 } else { 32 },
            version: (read32(cap + CAPLENGTH) >> 16) as u16,
            low: cparams1 & 1 == 0,
            protocol: [0; 256],
            signal: pci::Signal::None,
            running: false,
            dcbaa: None,
            scratchpad: Vec::new(),
            commands: None,
            events: None,
            command_done: None,
            port_changes: [0; 4],
        };
        host.extended_capabilities(cparams1);
        // Stop it, then reset it.
        let cmd = read32(host.op + USBCMD);
        write32(host.op + USBCMD, cmd & !(CMD_RUN | CMD_INTERRUPTS));
        if !wait_for(50, || read32(host.op + USBSTS) & STS_HALTED != 0) {
            return Err("it didn't stop".into());
        }
        write32(host.op + USBCMD, CMD_RESET);
        time::delay_ms(1); // some controllers mustn't be read for a moment after a reset starts
        let reset = wait_for(1000, || {
            read32(host.op + USBCMD) & CMD_RESET == 0 && read32(host.op + USBSTS) & STS_NOT_READY == 0
        });
        if !reset {
            return Err("it didn't finish resetting".into());
        }
        route_intel_ports(f);
        Ok(host)
    }

    /// Walks the extended capabilities: takes the controller from the firmware (USB legacy support)
    /// and notes which ports are USB 2 and which USB 3 (supported protocols).
    fn extended_capabilities(&mut self, cparams1: u32) {
        let mut at = ((cparams1 >> 16) as u64) << 2;
        for _ in 0..64 {
            if at == 0 {
                return;
            }
            let head = read32(self.cap + at);
            match head & 0xFF {
                1 => self.claim_from_firmware(self.cap + at),
                2 => {
                    let major = (head >> 24) as u8;
                    let ports = read32(self.cap + at + 8);
                    let (first, count) = ((ports & 0xFF) as usize, ((ports >> 8) & 0xFF) as usize);
                    for p in first..(first + count).min(256) {
                        self.protocol[p] = major;
                    }
                }
                _ => {}
            }
            let next = ((head >> 8) & 0xFF) as u64;
            if next == 0 {
                return;
            }
            at += next << 2;
        }
    }

    /// The firmware hand-off: set "the OS owns it", wait for the firmware to let go (it may take a
    /// moment to finish what it was doing), and switch off the firmware's interrupts from it.
    fn claim_from_firmware(&self, legacy: u64) {
        const FIRMWARE_OWNS: u32 = 1 << 16;
        const OS_OWNS: u32 = 1 << 24;
        let v = read32(legacy);
        if v & FIRMWARE_OWNS != 0 {
            write32(legacy, v | OS_OWNS);
            if !wait_for(1000, || read32(legacy) & FIRMWARE_OWNS == 0) {
                crate::log!("Chat Computer kernel: USB {}: the firmware didn't let go of it; taking it anyway", self.name);
                write32(legacy, (read32(legacy) & !FIRMWARE_OWNS) | OS_OWNS);
            }
        }
        // Keep the reserved bits, switch off the firmware's interrupts (SMIs), clear the ones pending.
        let control = read32(legacy + 4);
        write32(legacy + 4, (control & (0x7 << 1 | 0xFF << 5 | 0x7 << 17)) | 0x7 << 29);
    }

    /// Gives the controller its memory and starts it. `vector`: the interrupt to signal events with.
    pub fn start(&mut self, pci_at: pci::Address, vector: u8) -> Result<(), String> {
        if read32(self.op + PAGESIZE) & 1 == 0 {
            return Err("it doesn't work with 4 KiB pages".into());
        }
        let low = self.low;
        let page = || Page::new(low).ok_or_else(|| String::from("out of memory"));
        // The device context base address array: entry n is slot n's device context; entry 0 is the
        // list of pages the controller keeps for itself (scratchpad), if it wants any.
        let dcbaa = page()?;
        let params2 = read32(self.cap + HCSPARAMS2);
        let scratch = ((params2 >> 27) & 0x1F | ((params2 >> 21) & 0x1F) << 5) as usize;
        if scratch > 0 {
            let list = page()?;
            if scratch > 512 {
                return Err("it wants more memory of its own than Chat Computer can give".into());
            }
            for i in 0..scratch {
                let p = page()?;
                unsafe { write_volatile(list.at::<u64>(8 * i), p.phys) };
                self.scratchpad.push(p);
            }
            unsafe { write_volatile(dcbaa.at::<u64>(0), list.phys) };
            self.scratchpad.push(list);
        }
        let max_slots = self.max_slots.max(1);
        let config = read32(self.op + CONFIG);
        write32(self.op + CONFIG, (config & !0xFF) | max_slots as u32);
        write64(self.op + DCBAAP, dcbaa.phys);
        self.dcbaa = Some(dcbaa);

        let commands = Ring::new(low).ok_or("out of memory")?;
        write64(self.op + CRCR, commands.start());
        self.commands = Some(commands);

        let events = EventRing::new(low).ok_or("out of memory")?;
        write32(self.rt + ERSTSZ, 1);
        write64(self.rt + ERDP, events.page.phys);
        write64(self.rt + ERSTBA, events.table.phys);
        self.events = Some(events);

        self.signal = pci_at.signal_messages(vector, crate::apic::id());
        write32(self.rt + IMOD, 160); // at most one interrupt every 40 µs
        write32(self.rt + IMAN, IMAN_PENDING | if self.signal == pci::Signal::None { 0 } else { IMAN_ENABLE });
        let interrupts = if self.signal == pci::Signal::None { 0 } else { CMD_INTERRUPTS };
        write32(self.op + USBCMD, CMD_RUN | interrupts);
        if !wait_for(100, || read32(self.op + USBSTS) & STS_HALTED == 0) {
            return Err("it didn't start".into());
        }
        self.running = true;
        // Power every port (on controllers that switch ports' power), then give devices time to wake.
        if read32(self.cap + HCCPARAMS1) & (1 << 3) != 0 {
            for port in 1..=self.ports {
                let sc = self.port_status(port);
                if sc & PORT_POWER == 0 {
                    self.port_write(port, (sc & PORT_KEEP) | PORT_POWER);
                }
            }
            time::delay_ms(20);
        }
        for port in 1..=self.ports {
            self.mark_port(port);
        }
        Ok(())
    }

    /// Whether the controller stopped itself because something went badly wrong.
    pub fn failed(&self) -> bool {
        read32(self.op + USBSTS) & (STS_HOST_ERROR | STS_CONTROLLER_ERROR) != 0
    }

    pub fn doorbell(&self) -> Doorbell {
        Doorbell(self.db)
    }

    /// The next event, if there is one.
    pub fn next_event(&mut self) -> Option<Trb> {
        self.events.as_mut()?.next()
    }

    /// Tells the controller which events have been handled, and clears the pending interrupt.
    pub fn events_handled(&mut self) {
        let Some(events) = self.events.as_ref() else { return };
        if read32(self.op + USBSTS) & STS_EVENT != 0 {
            write32(self.op + USBSTS, STS_EVENT); // a write of 1 clears it; the other bits written are 0
        }
        write64(self.rt + ERDP, events.dequeue_phys() | ERDP_BUSY);
        let iman = read32(self.rt + IMAN);
        if iman & IMAN_PENDING != 0 {
            write32(self.rt + IMAN, iman); // a write of 1 clears it
        }
    }

    /// Puts a command on the command ring, rings the doorbell, and returns the command's address.
    pub fn command(&mut self, t: Trb) -> Option<u64> {
        self.command_done = None;
        let at = self.commands.as_mut()?.push(t);
        self.doorbell().ring(0, 0);
        Some(at)
    }

    pub fn enable_slot(&mut self) -> Option<u64> {
        self.command(Trb::new(TRB_ENABLE_SLOT, 0, 0, 0))
    }

    /// Frees a slot. Its device context entry is cleared once the command has finished.
    pub fn disable_slot(&mut self, slot: u8) -> Option<u64> {
        self.command(Trb::new(TRB_DISABLE_SLOT, 0, 0, (slot as u32) << 24))
    }

    pub fn address_device(&mut self, slot: u8, input: u64) -> Option<u64> {
        self.command(Trb::new(TRB_ADDRESS_DEVICE, input, 0, (slot as u32) << 24))
    }

    pub fn configure_endpoint(&mut self, slot: u8, input: u64) -> Option<u64> {
        self.command(Trb::new(TRB_CONFIGURE_ENDPOINT, input, 0, (slot as u32) << 24))
    }

    pub fn evaluate_context(&mut self, slot: u8, input: u64) -> Option<u64> {
        self.command(Trb::new(TRB_EVALUATE_CONTEXT, input, 0, (slot as u32) << 24))
    }

    pub fn reset_endpoint(&mut self, slot: u8, endpoint: u8) -> Option<u64> {
        self.command(Trb::new(TRB_RESET_ENDPOINT, 0, 0, (slot as u32) << 24 | (endpoint as u32) << 16))
    }

    pub fn stop_endpoint(&mut self, slot: u8, endpoint: u8) -> Option<u64> {
        self.command(Trb::new(TRB_STOP_ENDPOINT, 0, 0, (slot as u32) << 24 | (endpoint as u32) << 16))
    }

    /// Moves an endpoint's dequeue pointer to `to` (an address with the cycle state in bit 0).
    pub fn set_dequeue(&mut self, slot: u8, endpoint: u8, to: u64) -> Option<u64> {
        self.command(Trb::new(TRB_SET_DEQUEUE, to, 0, (slot as u32) << 24 | (endpoint as u32) << 16))
    }

    /// Points slot `slot`'s entry at a device context (or at nothing).
    pub fn set_context(&mut self, slot: u8, phys: Option<u64>) {
        if let Some(d) = self.dcbaa.as_ref() {
            unsafe { write_volatile(d.at::<u64>(8 * slot as usize), phys.unwrap_or(0)) };
        }
    }

    pub fn port_status(&self, port: u8) -> u32 {
        read32(self.op + PORTSC + 0x10 * (port as u64 - 1))
    }

    fn port_write(&self, port: u8, v: u32) {
        write32(self.op + PORTSC + 0x10 * (port as u64 - 1), v);
    }

    /// Clears a port's change bits and returns its status as it was.
    pub fn port_acknowledge(&self, port: u8) -> u32 {
        let sc = self.port_status(port);
        if sc & PORT_CHANGES != 0 {
            self.port_write(port, (sc & PORT_KEEP) | (sc & PORT_CHANGES));
        }
        sc
    }

    /// Starts a reset of a USB 2 port (USB 3 ports enable themselves), or a warm reset of a USB 3 port
    /// that didn't.
    pub fn port_reset(&self, port: u8, warm: bool) {
        let sc = self.port_status(port);
        self.port_write(port, (sc & PORT_KEEP) | if warm { PORT_WARM_RESET } else { PORT_RESET });
    }

    /// Whether a port reset has finished (and clears the bits that say so).
    pub fn port_reset_done(&self, port: u8) -> bool {
        let sc = self.port_status(port);
        let done = sc & (PORT_RESET_CHANGE | PORT_WARM_RESET_CHANGE) != 0 && sc & PORT_RESET == 0;
        if done {
            self.port_write(port, (sc & PORT_KEEP) | (sc & (PORT_RESET_CHANGE | PORT_WARM_RESET_CHANGE)));
        }
        done
    }

    /// The speed of what is connected to a port (by the controller's default speed numbers).
    pub fn port_speed(&self, port: u8) -> Option<Speed> {
        Speed::from_xhci((self.port_status(port) >> 10) & 0xF)
    }

    pub fn mark_port(&mut self, port: u8) {
        self.port_changes[port as usize / 64] |= 1 << (port % 64);
    }

    /// The ports marked as changed since the last call.
    pub fn take_port_changes(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        for port in 1..=self.ports {
            if self.port_changes[port as usize / 64] & (1 << (port % 64)) != 0 {
                out.push(port);
            }
        }
        self.port_changes = [0; 4];
        out
    }

    /// Whether a port change event is for a connection change (as opposed to, say, a reset finishing).
    pub fn connect_changed(sc: u32) -> bool {
        sc & PORT_CONNECT_CHANGE != 0
    }
}
