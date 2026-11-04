//! What is plugged into a USB controller. When something is plugged in, Chat Computer gives it a slot and
//! an address, asks it what it is (its descriptors), and sets it up if it is something Chat Computer uses:
//! a keyboard, a mouse, a hub with more devices behind it, a network adapter (net.rs), a drive (storage.rs: a USB
//! stick, a card reader), or a sound card (audio.rs: a headset, speakers). Unplugged devices (and everything behind an
//! unplugged hub) give their slots back.
//!
//! Requests to a device go on its control endpoint (endpoint 0) as a setup packet, an optional data
//! stage and a status stage. Keyboards, mice and hubs also send small reports on an interrupt endpoint,
//! which the controller collects from them a few hundred times a second.

use super::audio::{self, Audio};
use super::net::{self, Net};
use super::storage::{self, Storage};
use super::xhci::{self, Doorbell, Page, Ring, Trb};
use super::{changed, sleep, wait, with, Speed};
use crate::{keyboard, mouse};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};

/// The longest control transfer: one page.
const CONTROL_MAX: usize = 4096;
/// USB allows five hubs in a row.
const MAX_DEPTH: u8 = 5;

pub enum Role {
    /// Something Chat Computer doesn't use yet.
    Other,
    /// Something to type or point with: a keyboard, a mouse, or both (a wireless receiver), an interface each.
    Input(Vec<Input>),
    Hub(Hub),
    Net(alloc::boxed::Box<Net>),
    /// A drive: a USB stick, an external disk, a card reader.
    Storage(alloc::boxed::Box<Storage>),
    /// A sound card: a headset, speakers.
    Audio(alloc::boxed::Box<Audio>),
}

/// One of a device's interfaces for typing or pointing (HID's, the USB class for them).
pub struct Input {
    pub interface: u8,
    dci: u8,
    /// The longest report it sends.
    size: u16,
    /// Where its reports land in the device's data page.
    at: usize,
    pub kind: InputKind,
}

pub enum InputKind {
    /// A keyboard, in the boot protocol: its last report, and the lights last asked for (0xFF: not yet).
    Keyboard { last: [u8; 8], lights: u8 },
    /// A mouse, in the report protocol: where its controls are in its reports.
    Mouse(hid::Pointer),
}

/// A device's inputs share its data page, this much each, from the page's end down (so that a device with one input
/// reads its reports at an offset, as one with several does).
const INPUT_ROOM: usize = 512;
/// So at most this many of a device's interfaces are used.
const INPUTS_MAX: usize = 4096 / INPUT_ROOM;

pub struct Hub {
    ports: u8,
    dci: u8,
    size: u16,
    /// Ports with changes to look at (bit n: port n).
    pub changes: u32,
}

pub struct Device {
    pub slot: u8,
    speed: Speed,
    route: u32,
    root_port: u8,
    /// The hub (slot) and hub port it is plugged into; None for a port of the controller itself.
    parent: Option<(u8, u8)>,
    /// For a low- or full-speed device behind a high-speed hub: that hub's slot and port. The hub's
    /// "transaction translator" turns its fast signals into the device's slow ones.
    tt: (u8, u8),
    /// Hubs between it and the controller.
    depth: u8,
    output: Page,
    input: Page,
    /// Data for control transfers.
    buffer: Page,
    /// Data from its interrupt endpoint.
    data: Page,
    ep0: Ring,
    rings: Vec<Option<Ring>>,
    /// Events on endpoint 0 since its last request.
    events: Vec<Trb>,
    pub(super) low: bool,
    /// Endpoints that stopped after an error, to restart in the main loop (bit n: device context index n).
    pub stopped: u32,
    pub name: String,
    pub role: Role,
}

/// Where a new device is.
#[derive(Clone, Copy)]
struct Place {
    root_port: u8,
    route: u32,
    parent: Option<(u8, u8)>,
    tt: (u8, u8),
    depth: u8,
}

impl Place {
    /// "port 3", or "port 3.1.4" behind hubs.
    fn describe(&self) -> String {
        let mut s = format!("port {}", self.root_port);
        for level in 0..self.depth {
            s += &format!(".{}", (self.route >> (4 * level)) & 0xF);
        }
        s
    }
}

/// A request on the control endpoint.
#[derive(Clone, Copy)]
pub(super) struct Request {
    pub kind: u8,
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub length: u16,
}

impl Request {
    fn get_descriptor(kind: u8, index: u8, language: u16, length: u16) -> Request {
        Request { kind: 0x80, request: 6, value: (kind as u16) << 8 | index as u16, index: language, length }
    }
    pub(super) fn to_class(kind: u8, request: u8, value: u16, index: u16) -> Request {
        Request { kind, request, value, index, length: 0 }
    }
}

const SET_CONFIGURATION: u8 = 9;
pub(super) const SET_INTERFACE: u8 = 11;
const HID_SET_REPORT: u8 = 0x09;
const HID_SET_IDLE: u8 = 0x0A;
const HID_SET_PROTOCOL: u8 = 0x0B;
const HUB_GET_STATUS: u8 = 0;
const HUB_CLEAR_FEATURE: u8 = 1;
const HUB_SET_FEATURE: u8 = 3;
const PORT_RESET: u16 = 4;
const PORT_POWER: u16 = 8;
const C_PORT_CONNECTION: u16 = 16;
const C_PORT_RESET: u16 = 20;

#[derive(Debug)]
pub(super) enum Error {
    /// The device answered with this completion code.
    Code(u8),
    NoAnswer,
    Gone,
    Bad(&'static str),
}

impl Error {
    pub(super) fn say(&self) -> String {
        match self {
            Error::Code(c) => format!("it answered \"{}\"", xhci::code_name(*c)),
            Error::NoAnswer => "it didn't answer".to_string(),
            Error::Gone => "it was unplugged".to_string(),
            Error::Bad(what) => what.to_string(),
        }
    }
}

pub(super) fn dword(page: &Page, at: usize) -> u32 {
    unsafe { read_volatile(page.at::<u32>(at)) }
}

pub(super) fn set_dword(page: &Page, at: usize, v: u32) {
    unsafe { write_volatile(page.at::<u32>(at), v) }
}

impl Device {
    fn new(slot: u8, speed: Speed, place: Place, low: bool) -> Option<Device> {
        Some(Device {
            slot,
            speed,
            route: place.route,
            root_port: place.root_port,
            parent: place.parent,
            tt: place.tt,
            depth: place.depth,
            output: Page::new(low)?,
            input: Page::new(low)?,
            buffer: Page::new(low)?,
            data: Page::new(low)?,
            ep0: Ring::new(low)?,
            rings: (0..32).map(|_| None).collect(),
            events: Vec::new(),
            low,
            stopped: 0,
            name: String::new(),
            role: Role::Other,
        })
    }

    /// The input context for Address Device: a slot context and endpoint 0.
    fn address_input(&self, context: usize, mps0: u16) {
        self.input.zero();
        set_dword(&self.input, 4, 0b11); // add the slot context and endpoint 0
        let slot = context;
        set_dword(&self.input, slot, self.route & 0xF_FFFF | self.speed.xhci() << 20 | 1 << 27);
        set_dword(&self.input, slot + 4, (self.root_port as u32) << 16);
        set_dword(&self.input, slot + 8, self.tt.0 as u32 | (self.tt.1 as u32) << 8);
        self.ep0_context(context, mps0);
    }

    /// Endpoint 0's context in the input context: a control endpoint with packets of `mps0` bytes.
    fn ep0_context(&self, context: usize, mps0: u16) {
        let ep = 2 * context;
        let start = self.ep0.start();
        set_dword(&self.input, ep + 4, 3 << 1 | xhci::EP_CONTROL << 3 | (mps0 as u32) << 16);
        set_dword(&self.input, ep + 8, start as u32);
        set_dword(&self.input, ep + 12, (start >> 32) as u32);
        set_dword(&self.input, ep + 16, 8);
    }

    /// Asks again for what interrupt endpoint `dci` brings in (after it was set up, after its last report, or after
    /// it was restarted).
    fn queue_interrupt(&mut self, db: Doorbell, dci: u8) {
        let (size, at) = match &self.role {
            Role::Input(inputs) => match inputs.iter().find(|i| i.dci == dci) {
                Some(i) => (i.size, i.at),
                None => return,
            },
            Role::Hub(h) if h.dci == dci => (h.size, 0),
            _ => return,
        };
        if let Some(Some(ring)) = self.rings.get_mut(dci as usize) {
            let buffer = self.data.phys + at as u64;
            ring.push(Trb::new(xhci::TRB_NORMAL, buffer, size as u32, xhci::ON_COMPLETION | xhci::SHORT_OK));
            db.ring(self.slot, dci);
        }
    }

    /// For the network side: its rings and its role, at once.
    pub(super) fn rings_and_role(&mut self) -> (&mut Vec<Option<Ring>>, &mut Role) {
        (&mut self.rings, &mut self.role)
    }

    pub(super) fn speed_name(&self) -> &'static str {
        self.speed.name()
    }

    /// After endpoint `dci` was restarted: what it brings in is asked for again.
    fn endpoint_restarted(&mut self, db: Doorbell, dci: u8) {
        if let Role::Net(n) = &mut self.role {
            n.restarted(&mut self.rings, db, self.slot, dci);
        } else {
            self.queue_interrupt(db, dci);
        }
    }

    /// A transfer finished (called from the interrupt).
    pub fn transfer_event(&mut self, db: Doorbell, e: Trb) {
        let dci = e.endpoint();
        if dci == 1 {
            if self.events.len() < 16 {
                self.events.push(e);
            }
            return;
        }
        if let Role::Net(n) = &mut self.role {
            if !n.transfer_event(&mut self.rings, db, self.slot, e) {
                self.stopped |= 1 << dci;
            }
            return;
        }
        // (A drive's command takes its events itself, errors included.)
        if let Role::Storage(s) = &mut self.role {
            s.transfer_event(e);
            return;
        }
        if let Role::Audio(a) = &mut self.role {
            if !a.transfer_event(&mut self.rings, db, self.slot, e) {
                self.stopped |= 1 << dci;
            }
            return;
        }
        let ok = xhci::ok(e.code());
        let data = &self.data;
        let read = |at: usize, n: usize| -> Vec<u8> { (0..n).map(|i| unsafe { read_volatile(data.at::<u8>(at + i)) }).collect() };
        match &mut self.role {
            Role::Input(inputs) => {
                let Some(input) = inputs.iter_mut().find(|i| i.dci == dci) else { return };
                if ok {
                    let got = (input.size as usize).saturating_sub(e.residual() as usize);
                    match &mut input.kind {
                        InputKind::Keyboard { last, .. } => {
                            let got = got.min(8);
                            if got >= 3 {
                                let mut report = [0u8; 8];
                                report[..got].copy_from_slice(&read(input.at, got));
                                keyboard::report(last, &report);
                            }
                        }
                        InputKind::Mouse(layout) => mouse::report(layout, &read(input.at, got)),
                    }
                }
            }
            Role::Hub(h) if dci == h.dci => {
                if ok {
                    let got = (h.size as usize).saturating_sub(e.residual() as usize).min(4);
                    h.changes |= read(0, got).iter().enumerate().fold(0u32, |a, (i, b)| a | (*b as u32) << (8 * i));
                }
            }
            _ => return,
        }
        if ok {
            self.queue_interrupt(db, dci);
        } else {
            self.stopped |= 1 << dci;
        }
    }
}

/// Runs a command and waits for it. Returns the slot the completion names.
pub(super) fn command(hc: usize, issue: impl FnOnce(&mut xhci::Host) -> Option<u64>) -> Result<u8, Error> {
    let at = with(hc, |c| issue(&mut c.host)).ok_or(Error::Gone)?;
    match wait(hc, 5000, |c| match c.host.command_done {
        Some((p, code, slot)) if p == at => Some((code, slot)),
        _ => None,
    }) {
        Some((xhci::SUCCESS, slot)) => Ok(slot),
        Some((code, _)) => Err(Error::Code(code)),
        None => Err(Error::NoAnswer),
    }
}

/// A request on endpoint 0, with `out` as its data if it sends any. Returns the data received.
pub(super) fn control(hc: usize, slot: u8, r: Request, out: &[u8]) -> Result<Vec<u8>, Error> {
    let input = r.kind & 0x80 != 0;
    let len = (r.length as usize).min(CONTROL_MAX);
    let trbs = with(hc, |c| {
        let db = c.host.doorbell();
        let d = c.devices.get_mut(slot as usize)?.as_mut()?;
        d.events.clear();
        if !input {
            for (i, b) in out.iter().take(len).enumerate() {
                unsafe { write_volatile(d.buffer.at::<u8>(i), *b) };
            }
        }
        let setup = r.kind as u64 | (r.request as u64) << 8 | (r.value as u64) << 16 | (r.index as u64) << 32 | (len as u64) << 48;
        let transfer = if len == 0 { 0 } else if input { 3 } else { 2 };
        let a = d.ep0.push(Trb::new(xhci::TRB_SETUP, setup, 8, xhci::IMMEDIATE | transfer << 16));
        let b = if len > 0 {
            let dir = if input { xhci::DIR_IN | xhci::SHORT_OK } else { 0 };
            d.ep0.push(Trb::new(xhci::TRB_DATA, d.buffer.phys, len as u32, dir))
        } else {
            0
        };
        let status_dir = if len > 0 && input { 0 } else { xhci::DIR_IN };
        let s = d.ep0.push(Trb::new(xhci::TRB_STATUS, 0, 0, xhci::ON_COMPLETION | status_dir));
        db.ring(slot, 1);
        Some([a, b, s])
    })
    .ok_or(Error::Gone)?;
    let outcome = wait(hc, 5000, |c| {
        let Some(Some(d)) = c.devices.get(slot as usize) else { return Some(Err(Error::Gone)) };
        let mut residual = 0;
        for e in &d.events {
            if e.param == 0 || !trbs.contains(&e.param) {
                continue;
            }
            if !xhci::ok(e.code()) {
                return Some(Err(Error::Code(e.code())));
            }
            if e.param == trbs[1] && e.code() == xhci::SHORT_PACKET {
                residual = e.residual() as usize;
            }
            if e.param == trbs[2] {
                return Some(Ok(residual));
            }
        }
        None
    });
    match outcome {
        Some(Ok(residual)) => {
            let got = len.saturating_sub(residual);
            with(hc, |c| {
                let d = c.devices.get(slot as usize)?.as_ref()?;
                Some((0..got).map(|i| unsafe { read_volatile(d.buffer.at::<u8>(i)) }).collect())
            })
            .ok_or(Error::Gone)
        }
        Some(Err(e)) => {
            if let Error::Code(_) = e {
                restart_endpoint(hc, slot, 1);
            }
            Err(e)
        }
        None => {
            restart_endpoint(hc, slot, 1);
            Err(Error::NoAnswer)
        }
    }
}

/// After an endpoint stopped on an error, or a request on it never finished: resets it (or stops it, if
/// it is still running), skips what was queued on it, and for an interrupt endpoint asks for the next
/// report again.
pub fn restart_endpoint(hc: usize, slot: u8, dci: u8) {
    if let Err(Error::Code(xhci::WRONG_STATE)) = command(hc, |h| h.reset_endpoint(slot, dci)) {
        let _ = command(hc, |h| h.stop_endpoint(slot, dci));
    }
    let next = with(hc, |c| {
        let d = c.devices.get(slot as usize)?.as_ref()?;
        if dci == 1 { Some(d.ep0.next()) } else { d.rings.get(dci as usize)?.as_ref().map(|r| r.next()) }
    });
    let Some(next) = next else { return };
    let _ = command(hc, |h| h.set_dequeue(slot, dci, next));
    if dci != 1 {
        with(hc, |c| {
            let db = c.host.doorbell();
            if let Some(Some(d)) = c.devices.get_mut(slot as usize) {
                d.endpoint_restarted(db, dci);
            }
        });
    }
}

pub(super) fn descriptor(hc: usize, slot: u8, kind: u8, index: u8, language: u16, length: u16) -> Result<Vec<u8>, Error> {
    control(hc, slot, Request::get_descriptor(kind, index, language, length), &[])
}

/// A string descriptor, as text.
pub(super) fn string(hc: usize, slot: u8, index: u8, language: u16) -> Option<String> {
    if index == 0 {
        return None;
    }
    let d = descriptor(hc, slot, 3, index, language, 255).ok()?;
    let n = (*d.first()? as usize).min(d.len());
    if n < 4 {
        return None;
    }
    let units = d[2..n].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
    let s: String = char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).filter(|c| !c.is_control()).collect();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// One alternate setting of an interface, with its endpoints and its class's own descriptors (a network
/// adapter's say what it is and where its address is).
pub(super) struct Interface {
    pub number: u8,
    pub alt: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub endpoints: Vec<Endpoint>,
    pub extra: Vec<Vec<u8>>,
    /// A HID interface's report descriptor's length, from its HID descriptor.
    pub report_length: Option<u16>,
}

#[derive(Clone, Copy)]
pub(super) struct Endpoint {
    pub address: u8,
    pub attributes: u8,
    pub max_packet: u16,
    pub interval: u8,
    /// A USB 3 endpoint's burst (from its companion descriptor): packets sent back to back, less one.
    pub burst: u8,
}

impl Endpoint {
    pub(super) fn interrupt_in(&self) -> bool {
        self.address & 0x80 != 0 && self.attributes & 3 == 3
    }
    pub(super) fn bulk_in(&self) -> bool {
        self.address & 0x80 != 0 && self.attributes & 3 == 2
    }
    pub(super) fn bulk_out(&self) -> bool {
        self.address & 0x80 == 0 && self.attributes & 3 == 2
    }
    /// Its device context index.
    pub(super) fn dci(&self) -> u8 {
        (self.address & 0xF) * 2 + (self.address >> 7)
    }
}

/// A configuration descriptor's value and interfaces: every alternate setting, in order.
fn parse_configuration(d: &[u8]) -> Option<(u8, Vec<Interface>)> {
    if d.len() < 9 || d[1] != 2 {
        return None;
    }
    let mut interfaces: Vec<Interface> = Vec::new();
    let mut i = 0;
    while i + 2 <= d.len() {
        let len = d[i] as usize;
        if len < 2 || i + len > d.len() {
            break;
        }
        let e = &d[i..i + len];
        match e[1] {
            4 if len >= 9 => interfaces.push(Interface {
                number: e[2],
                alt: e[3],
                class: e[5],
                subclass: e[6],
                protocol: e[7],
                endpoints: Vec::new(),
                extra: Vec::new(),
                report_length: None,
            }),
            5 if len >= 7 => {
                if let Some(interface) = interfaces.last_mut() {
                    let max_packet = u16::from_le_bytes([e[4], e[5]]);
                    interface.endpoints.push(Endpoint { address: e[2], attributes: e[3], max_packet, interval: e[6], burst: 0 });
                }
            }
            0x30 if len >= 6 => {
                // A USB 3 endpoint's companion: its burst.
                if let Some(ep) = interfaces.last_mut().and_then(|f| f.endpoints.last_mut()) {
                    ep.burst = e[2].min(15);
                }
            }
            0x24 => {
                if let Some(interface) = interfaces.last_mut()
                    && interface.endpoints.is_empty()
                {
                    interface.extra.push(e.to_vec());
                }
            }
            // A HID interface's own descriptor: its first class descriptor is its report descriptor, usually.
            0x21 if len >= 9 => {
                if let Some(interface) = interfaces.last_mut()
                    && interface.class == 3
                    && e[6] == 0x22
                {
                    interface.report_length = Some(u16::from_le_bytes([e[7], e[8]]));
                }
            }
            _ => {}
        }
        i += len;
    }
    Some((d[5], interfaces))
}

/// Something changed on port `port` of the controller itself.
pub fn root_port(hc: usize, port: u8) {
    let (sc, usb3, existing) = with(hc, |c| {
        let sc = c.host.port_acknowledge(port);
        let existing = c.devices.iter().flatten().find(|d| d.parent.is_none() && d.root_port == port).map(|d| d.slot);
        (sc, c.host.protocol[port as usize] == 3, existing)
    });
    let connected = sc & xhci::PORT_CONNECTED != 0;
    if let Some(slot) = existing {
        if connected && !xhci::Host::connect_changed(sc) {
            return; // still there (a change of another kind)
        }
        detach(hc, slot);
    }
    if !connected {
        return;
    }
    if usb3 {
        // A USB 3 port trains its link and enables itself; one that doesn't gets a warm reset.
        let enabled = wait(hc, 300, |c| (c.host.port_status(port) & xhci::PORT_ENABLED != 0).then_some(()));
        if enabled.is_none() {
            with(hc, |c| c.host.port_reset(port, true));
            if wait(hc, 1000, |c| c.host.port_reset_done(port).then_some(())).is_none() {
                return;
            }
        }
    } else {
        sleep(hc, 100); // let the plug settle
        with(hc, |c| c.host.port_reset(port, false));
        if wait(hc, 1000, |c| c.host.port_reset_done(port).then_some(())).is_none() {
            log_port(hc, &format!("port {port}"), "the port didn't finish resetting");
            return;
        }
    }
    let (sc, speed) = with(hc, |c| (c.host.port_status(port), c.host.port_speed(port)));
    let (true, Some(speed)) = (sc & xhci::PORT_ENABLED != 0, speed) else {
        log_port(hc, &format!("port {port}"), "something is plugged in, but the port didn't come on");
        return;
    };
    sleep(hc, 10); // the device's recovery time after a reset
    attach(hc, Place { root_port: port, route: 0, parent: None, tt: (0, 0), depth: 0 }, speed);
}

fn log_port(hc: usize, place: &str, what: &str) {
    let name = with(hc, |c| c.host.name.clone());
    crate::log!("Chat Computer kernel: USB {name} {place}: {what}");
}

fn attach(hc: usize, place: Place, speed: Speed) {
    if let Err(why) = enumerate(hc, place, speed) {
        log_port(hc, &place.describe(), &format!("couldn't set up what is plugged in: {}", why.say()));
    }
}

/// Gives a new device a slot and an address, finds out what it is, and sets it up.
fn enumerate(hc: usize, place: Place, speed: Speed) -> Result<(), Error> {
    let slot = command(hc, |h| h.enable_slot())?;
    let (context, low) = with(hc, |c| (c.host.context_size, c.host.low));
    let Some(device) = Device::new(slot, speed, place, low) else {
        let _ = command(hc, |h| h.disable_slot(slot));
        return Err(Error::Bad("out of memory"));
    };
    let mps0: u16 = match speed {
        Speed::Low | Speed::Full => 8,
        Speed::High => 64,
        Speed::Super | Speed::SuperPlus => 512,
    };
    device.address_input(context, mps0);
    let input = device.input.phys;
    with(hc, |c| {
        c.host.set_context(slot, Some(device.output.phys));
        c.devices[slot as usize] = Some(device);
    });
    let result = set_up(hc, slot, place, speed, context, input);
    if result.is_err() {
        let _ = command(hc, |h| h.disable_slot(slot));
        with(hc, |c| {
            c.host.set_context(slot, None);
            c.devices[slot as usize] = None;
        });
    }
    result
}

fn set_up(hc: usize, slot: u8, place: Place, speed: Speed, context: usize, input: u64) -> Result<(), Error> {
    command(hc, |h| h.address_device(slot, input))?;
    sleep(hc, 2);
    if speed == Speed::Full {
        // A full-speed device's first packets may be 8, 16, 32 or 64 bytes; its descriptor's first 8
        // bytes say which.
        let d = descriptor(hc, slot, 1, 0, 0, 8)?;
        let mps0 = *d.get(7).ok_or(Error::Bad("its device descriptor was too short"))? as u16;
        if mps0 != 8 && matches!(mps0, 16 | 32 | 64) {
            let input = with(hc, |c| {
                let d = c.devices[slot as usize].as_ref()?;
                d.input.zero();
                set_dword(&d.input, 4, 0b10);
                d.ep0_context(context, mps0);
                Some(d.input.phys)
            })
            .ok_or(Error::Gone)?;
            command(hc, |h| h.evaluate_context(slot, input))?;
        }
    }
    let d = descriptor(hc, slot, 1, 0, 0, 18)?;
    if d.len() < 18 {
        return Err(Error::Bad("its device descriptor was too short"));
    }
    let (class, vendor, product) = (d[4], u16::from_le_bytes([d[8], d[9]]), u16::from_le_bytes([d[10], d[11]]));
    let (maker_index, product_index, serial_index) = (d[14], d[15], d[16]);
    let language = descriptor(hc, slot, 3, 0, 0, 4).ok().filter(|l| l.len() >= 4).map_or(0x0409, |l| u16::from_le_bytes([l[2], l[3]]));
    let name = string(hc, slot, product_index, language).or_else(|| string(hc, slot, maker_index, language));
    // Its configurations: the first, unless another is a network adapter's (adapters often offer a maker's own
    // way first and a standard one second; phones offer the standard ones only when sharing their connection).
    let count = d[17].clamp(1, 8);
    let mut chosen: Option<(u8, Vec<Interface>, Option<net::Kind>)> = None;
    for index in 0..count {
        let head = descriptor(hc, slot, 2, index, 0, 9)?;
        if head.len() < 9 {
            return Err(Error::Bad("its configuration descriptor was too short"));
        }
        let total = u16::from_le_bytes([head[2], head[3]]).max(9);
        let config = descriptor(hc, slot, 2, index, 0, total)?;
        let (value, interfaces) = parse_configuration(&config).ok_or(Error::Bad("its configuration descriptor didn't make sense"))?;
        let kind = net::kind(&interfaces);
        let better = match (&chosen, kind) {
            (None, _) => true,
            (Some((_, _, None)), Some(_)) => true,
            (Some((_, _, Some(old))), Some(new)) => net::preference(new) > net::preference(*old),
            _ => false,
        };
        if better {
            chosen = Some((value, interfaces, kind));
        }
    }
    let Some((value, interfaces, net_kind)) = chosen else { return Err(Error::Bad("it has no configuration")) };
    control(hc, slot, Request::to_class(0x00, SET_CONFIGURATION, value as u16, 0), &[])?;

    let first: Vec<&Interface> = interfaces.iter().filter(|i| i.alt == 0).collect();
    let hid: Vec<&Interface> = first.iter().filter(|i| i.class == 3).copied().collect();
    let hub = first.iter().find(|i| i.class == 9).filter(|_| class == 9 || class == 0).copied();
    // What it is, for the log, and what to call it if it gives no name.
    let (what, plain_name): (String, Option<&str>) = if let Some(kind) = net_kind {
        net::set_up(hc, slot, speed, kind, &interfaces)?;
        (String::from(net::describe(kind)), Some("USB network adapter"))
    } else if let Some(plan) = audio::plan(&interfaces) {
        let (what, a) = audio::set_up(hc, slot, speed, plan)?;
        crate::log!("Chat Computer kernel: USB {}: sound card in slot {slot}: {}", place.describe(), a.describe());
        with(hc, |c| {
            if let Some(Some(d)) = c.devices.get_mut(slot as usize) {
                d.role = Role::Audio(alloc::boxed::Box::new(a));
            }
        });
        (what, Some("USB sound card"))
    } else if let Some(i) = hub {
        if matches!(speed, Speed::Super | Speed::SuperPlus) {
            (String::from("a USB 3 hub (keyboards and mice behind it show up on its USB 2 side)"), Some("USB hub"))
        } else {
            set_up_hub(hc, slot, place, speed, i)?;
            (String::from("a hub"), Some("USB hub"))
        }
    } else if let Some((what, plain_name)) = set_up_inputs(hc, slot, speed, &hid)? {
        (what, Some(plain_name))
    } else if storage::interface(&interfaces).is_some() {
        // (Its serial number: Windows names a USB stick by it.)
        let serial = string(hc, slot, serial_index, language);
        (storage::set_up(hc, slot, speed, &interfaces, serial)?, Some("USB drive"))
    } else {
        (String::from("a device Chat Computer doesn't use yet"), None)
    };
    let name = name.unwrap_or_else(|| plain_name.map_or_else(|| format!("USB device {vendor:04x}:{product:04x}"), String::from));
    log_port(
        hc,
        &place.describe(),
        &format!("{what}, \"{name}\" ({vendor:04x}:{product:04x}, {}), in slot {slot}", speed.name()),
    );
    let used = with(hc, |c| match c.devices.get_mut(slot as usize) {
        Some(Some(d)) => {
            d.name = name;
            matches!(d.role, Role::Input(_) | Role::Net(_))
        }
        _ => false,
    });
    if used {
        changed();
    }
    Ok(())
}

/// Adds endpoints (bulk, interrupt or isochronous, either way) to a configured device, each with a ring of its own.
pub(super) fn add_endpoints(hc: usize, slot: u8, speed: Speed, eps: &[Endpoint]) -> Result<(), Error> {
    let input = with(hc, |c| {
        let (context, low) = (c.host.context_size, c.host.low);
        let d = c.devices.get_mut(slot as usize)?.as_mut()?;
        d.input.zero();
        let mut add = 1u32;
        let mut highest = 0u32;
        for ep in eps {
            add |= 1 << ep.dci();
            highest = highest.max(ep.dci() as u32);
        }
        set_dword(&d.input, 4, add);
        for i in 0..4 {
            set_dword(&d.input, context + 4 * i, dword(&d.output, 4 * i));
        }
        let entries = ((dword(&d.output, 0) >> 27) & 0x1F).max(highest);
        set_dword(&d.input, context, (dword(&d.output, 0) & !(0x1F << 27)) | entries << 27);
        set_dword(&d.input, context + 12, 0);
        for ep in eps {
            let dci = ep.dci();
            let ring = Ring::new(low)?;
            let mps = (ep.max_packet & 0x7FF) as u32;
            let (kind, high_burst) = match (ep.attributes & 3, ep.address & 0x80 != 0) {
                (2, false) => (xhci::EP_BULK_OUT, 0),
                (2, true) => (xhci::EP_BULK_IN, 0),
                (1, false) => (xhci::EP_ISOCH_OUT, ((ep.max_packet >> 11) & 3) as u32),
                (1, true) => (xhci::EP_ISOCH_IN, ((ep.max_packet >> 11) & 3) as u32),
                (_, true) => (xhci::EP_INTERRUPT_IN, ((ep.max_packet >> 11) & 3) as u32),
                (_, false) => (xhci::EP_INTERRUPT_OUT, ((ep.max_packet >> 11) & 3) as u32),
            };
            let burst = match speed {
                Speed::Super | Speed::SuperPlus => ep.burst as u32,
                Speed::High => high_burst,
                _ => 0,
            };
            let at = context * (dci as usize + 1);
            let start = ring.start();
            let periodic = matches!(ep.attributes & 3, 1 | 3);
            let interval = match ep.attributes & 3 {
                3 => interval(speed, ep.interval),
                // An isochronous endpoint's: 2^(bInterval - 1) frames at full speed (eight microframes each),
                // microframes faster.
                1 => (ep.interval.clamp(1, 16) - 1) as u32 + if matches!(speed, Speed::Low | Speed::Full) { 3 } else { 0 },
                _ => 0,
            };
            // (An isochronous endpoint isn't tried again after an error: its packet's time has gone.)
            let errors = if ep.attributes & 3 == 1 { 0 } else { 3 };
            set_dword(&d.input, at, interval << 16);
            set_dword(&d.input, at + 4, errors << 1 | kind << 3 | burst << 8 | mps << 16);
            set_dword(&d.input, at + 8, start as u32);
            set_dword(&d.input, at + 12, (start >> 32) as u32);
            let average = if ep.attributes & 3 == 2 { 1024 } else { mps };
            let esit = if periodic { mps * (burst + 1) } else { 0 };
            set_dword(&d.input, at + 16, average | esit << 16);
            d.rings[dci as usize] = Some(ring);
        }
        Some(d.input.phys)
    })
    .ok_or(Error::Bad("out of memory"))?;
    command(hc, |h| h.configure_endpoint(slot, input))?;
    Ok(())
}

/// How often an interrupt endpoint is asked for data, in the controller's terms: 2^n × 125 µs.
fn interval(speed: Speed, b_interval: u8) -> u32 {
    match speed {
        // Milliseconds, 1 to 255; rounded down to a power of two.
        Speed::Low | Speed::Full => {
            let micro = b_interval.max(1) as u32 * 8;
            (31 - micro.leading_zeros()).clamp(3, 10)
        }
        // Already a power of two: 2^(bInterval - 1) × 125 µs.
        _ => (b_interval.clamp(1, 16) - 1) as u32,
    }
}

/// Adds an interrupt endpoint (and, for a hub, says it is a hub). Returns its device context index.
fn add_interrupt_in(hc: usize, slot: u8, ep: Endpoint, speed: Speed, hub: Option<(u8, u32)>) -> Result<u8, Error> {
    let dci = (ep.address & 0xF) * 2 + 1;
    let mps = (ep.max_packet & 0x7FF) as u32;
    let burst = if speed == Speed::High { ((ep.max_packet >> 11) & 3) as u32 } else { 0 };
    let input = with(hc, |c| {
        let (context, low) = (c.host.context_size, c.host.low);
        let d = c.devices.get_mut(slot as usize)?.as_mut()?;
        let ring = Ring::new(low)?;
        d.input.zero();
        set_dword(&d.input, 4, 1 | 1 << dci);
        // The slot context as the controller has it, with room for the new endpoint.
        for i in 0..4 {
            set_dword(&d.input, context + 4 * i, dword(&d.output, 4 * i));
        }
        let entries = ((dword(&d.output, 0) >> 27) & 0x1F).max(dci as u32);
        let mut dw0 = (dword(&d.output, 0) & !(0x1F << 27)) | entries << 27;
        let mut dw1 = dword(&d.output, 4);
        let mut dw2 = dword(&d.output, 8);
        if let Some((ports, think_time)) = hub {
            dw0 |= 1 << 26;
            dw1 = (dw1 & !(0xFF << 24)) | (ports as u32) << 24;
            dw2 = (dw2 & !(3 << 16)) | think_time << 16;
        }
        set_dword(&d.input, context, dw0);
        set_dword(&d.input, context + 4, dw1);
        set_dword(&d.input, context + 8, dw2);
        set_dword(&d.input, context + 12, 0);
        let at = context * (dci as usize + 1);
        let start = ring.start();
        set_dword(&d.input, at, interval(speed, ep.interval) << 16);
        set_dword(&d.input, at + 4, 3 << 1 | xhci::EP_INTERRUPT_IN << 3 | burst << 8 | mps << 16);
        set_dword(&d.input, at + 8, start as u32);
        set_dword(&d.input, at + 12, (start >> 32) as u32);
        set_dword(&d.input, at + 16, mps | (mps * (burst + 1)) << 16);
        d.rings[dci as usize] = Some(ring);
        Some(d.input.phys)
    })
    .ok_or(Error::Bad("out of memory"))?;
    command(hc, |h| h.configure_endpoint(slot, input))?;
    Ok(dci)
}

/// Sets up a device's HID interfaces that are keyboards or mice, each with its interrupt endpoint. Returns what they
/// make it, for the log, and what to call it if it gives no name; None if none is a keyboard or a mouse (a game
/// controller, say, or a security key's own interface). An interface that can't be set up is passed over, and said
/// so in the log, unless none can be.
fn set_up_inputs(hc: usize, slot: u8, speed: Speed, interfaces: &[&Interface]) -> Result<Option<(String, &'static str)>, Error> {
    let mut inputs: Vec<Input> = Vec::new();
    let mut failed = None;
    for interface in interfaces {
        if inputs.len() == INPUTS_MAX {
            break;
        }
        let Some(ep) = interface.endpoints.iter().find(|e| e.interrupt_in()).copied() else { continue };
        let keyboard = interface.subclass == 1 && interface.protocol == 1;
        let kind = if keyboard {
            keyboard_protocol(hc, slot, interface.number);
            InputKind::Keyboard { last: [0; 8], lights: 0xFF }
        } else {
            let Some(layout) = mouse_layout(hc, slot, interface) else { continue };
            InputKind::Mouse(layout)
        };
        match add_interrupt_in(hc, slot, ep, speed, None) {
            Ok(dci) => {
                let (least, most) = if keyboard { (8, 64) } else { (1, INPUT_ROOM as u16) };
                let size = (ep.max_packet & 0x7FF).clamp(least, most);
                let at = (INPUTS_MAX - 1 - inputs.len()) * INPUT_ROOM;
                inputs.push(Input { interface: interface.number, dci, size, at, kind });
            }
            Err(e) => {
                log_port(hc, &format!("slot {slot}"), &format!("its interface {} couldn't be set up: {}", interface.number, e.say()));
                failed = Some(e);
            }
        }
    }
    if inputs.is_empty() {
        return failed.map_or(Ok(None), Err);
    }
    let keyboard = inputs.iter().any(|i| matches!(i.kind, InputKind::Keyboard { .. }));
    let mouse = inputs.iter().find_map(|i| match &i.kind {
        InputKind::Mouse(layout) => Some(describe_mouse(layout)),
        InputKind::Keyboard { .. } => None,
    });
    let described = match (keyboard, mouse) {
        (false, Some(m)) => (m, "USB mouse"),
        (true, Some(m)) => (format!("a keyboard and {m}"), "USB keyboard and mouse"),
        _ => (String::from("a keyboard"), "USB keyboard"),
    };
    with(hc, |c| {
        let db = c.host.doorbell();
        if let Some(Some(d)) = c.devices.get_mut(slot as usize) {
            let endpoints: Vec<u8> = inputs.iter().map(|i| i.dci).collect();
            d.role = Role::Input(inputs);
            for dci in endpoints {
                d.queue_interrupt(db, dci);
            }
        }
    });
    Ok(Some(described))
}

/// A keyboard interface: the boot protocol, the simple 8-byte reports every keyboard can send (PC firmware uses them
/// too), and a report only when something changes. A keyboard that refuses the boot protocol is used anyway (most
/// send the same reports regardless); some don't take the second, which is fine.
fn keyboard_protocol(hc: usize, slot: u8, number: u8) {
    let number = number as u16;
    if let Err(e) = control(hc, slot, Request::to_class(0x21, HID_SET_PROTOCOL, 0, number), &[]) {
        log_port(hc, &format!("slot {slot}"), &format!("the keyboard refused the boot protocol ({}); using it anyway", e.say()));
    }
    let _ = control(hc, slot, Request::to_class(0x21, HID_SET_IDLE, 0, number), &[]);
}

/// Where a HID interface's buttons, movement and wheel are, if it is a mouse: from its report descriptor, in the
/// report protocol. A boot mouse (one that can also send the boot protocol's fixed reports) whose descriptor can't be
/// read or used is read in the boot protocol instead.
fn mouse_layout(hc: usize, slot: u8, interface: &Interface) -> Option<hid::Pointer> {
    let number = interface.number as u16;
    // A report only when something changes (a mouse's usual way; one that refuses is fine).
    let _ = control(hc, slot, Request::to_class(0x21, HID_SET_IDLE, 0, number), &[]);
    let length = interface.report_length.unwrap_or(1024).clamp(1, CONTROL_MAX as u16);
    let get_report_descriptor = Request { kind: 0x81, request: 6, value: 0x22 << 8, index: number, length };
    // A test build can be told to pass the descriptor over, to try the boot protocol (check.py --keyboard).
    let layout = if option_env!("CHATCOMPUTER_TEST_USB_MOUSE") == Some("boot") {
        None
    } else {
        control(hc, slot, get_report_descriptor, &[]).ok().and_then(|d| hid::parse(&d))
    };
    // Only boot devices take a protocol. A device starts in the report protocol, but the firmware may have asked for
    // the boot protocol before Chat Computer took over.
    let boot = interface.subclass == 1;
    match layout {
        Some(layout) => {
            if boot {
                let _ = control(hc, slot, Request::to_class(0x21, HID_SET_PROTOCOL, 1, number), &[]);
            }
            Some(layout)
        }
        None if boot && interface.protocol == 2 => {
            control(hc, slot, Request::to_class(0x21, HID_SET_PROTOCOL, 0, number), &[]).ok()?;
            Some(hid::boot_mouse())
        }
        None => None,
    }
}

/// "a mouse (3 buttons, a wheel)", for the log.
fn describe_mouse(layout: &hid::Pointer) -> String {
    let buttons = match layout.button_count() {
        1 => String::from("1 button"),
        n => format!("{n} buttons"),
    };
    let wheel = if layout.wheel.is_some_and(|w| w.relative) { ", a wheel" } else { ", no wheel" };
    format!("a mouse ({buttons}{wheel})")
}

pub fn set_lights(hc: usize, slot: u8, interface: u8, lights: u8) {
    let r = Request { kind: 0x21, request: HID_SET_REPORT, value: 0x0200, index: interface as u16, length: 1 };
    let _ = control(hc, slot, r, &[lights]); // a keyboard without lights may refuse
}

fn set_up_hub(hc: usize, slot: u8, place: Place, speed: Speed, interface: &Interface) -> Result<(), Error> {
    if place.depth >= MAX_DEPTH {
        return Err(Error::Bad("too many hubs in a row"));
    }
    let ep = *interface.endpoints.iter().find(|e| e.interrupt_in()).ok_or(Error::Bad("the hub has no endpoint for changes"))?;
    let d = control(hc, slot, Request { kind: 0xA0, request: 6, value: 0x29 << 8, index: 0, length: 9 }, &[])?;
    if d.len() < 7 {
        return Err(Error::Bad("its hub descriptor was too short"));
    }
    let ports = d[2].min(15);
    let think_time = if speed == Speed::High { ((u16::from_le_bytes([d[3], d[4]]) >> 5) & 3) as u32 } else { 0 };
    let power_ms = d[5] as u64 * 2;
    let dci = add_interrupt_in(hc, slot, ep, speed, Some((d[2], think_time)))?;
    for port in 1..=ports {
        control(hc, slot, Request::to_class(0x23, HUB_SET_FEATURE, PORT_POWER, port as u16), &[])?;
    }
    sleep(hc, power_ms.max(100));
    with(hc, |c| {
        let db = c.host.doorbell();
        if let Some(Some(d)) = c.devices.get_mut(slot as usize) {
            let size = (ep.max_packet & 0x7FF).clamp(1, 4);
            // Every port is looked at once, then whenever the hub reports a change.
            let all = ((1u32 << (ports + 1)) - 1) & !1;
            d.role = Role::Hub(Hub { ports, dci, size, changes: all });
            d.queue_interrupt(db, dci);
        }
    });
    Ok(())
}

/// A hub reported changes (bit n: port n; bit 0: the hub itself).
pub fn hub_changes(hc: usize, hub: u8, changes: u32) {
    let ports = with(hc, |c| match c.devices.get(hub as usize) {
        Some(Some(Device { role: Role::Hub(h), .. })) => h.ports,
        _ => 0,
    });
    if changes & 1 != 0 {
        // The hub's own power or over-current changed. Clearing the change stops it being reported again.
        let status = control(hc, hub, Request { kind: 0xA0, request: HUB_GET_STATUS, value: 0, index: 0, length: 4 }, &[]);
        if let Ok(s) = status
            && s.len() >= 4
        {
            let change = u16::from_le_bytes([s[2], s[3]]);
            for feature in 0..2 {
                if change & (1 << feature) != 0 {
                    let _ = control(hc, hub, Request::to_class(0x20, HUB_CLEAR_FEATURE, feature, 0), &[]);
                }
            }
        }
    }
    for port in 1..=ports {
        if changes & (1 << port) != 0 {
            hub_port(hc, hub, port);
        }
    }
}

fn hub_port_status(hc: usize, hub: u8, port: u8) -> Result<(u16, u16), Error> {
    let d = control(hc, hub, Request { kind: 0xA3, request: HUB_GET_STATUS, value: 0, index: port as u16, length: 4 }, &[])?;
    if d.len() < 4 {
        return Err(Error::Bad("the hub's port status was too short"));
    }
    Ok((u16::from_le_bytes([d[0], d[1]]), u16::from_le_bytes([d[2], d[3]])))
}

fn hub_port(hc: usize, hub: u8, port: u8) {
    let Ok((status, change)) = hub_port_status(hc, hub, port) else { return };
    for bit in 0..5 {
        if change & (1 << bit) != 0 {
            let _ = control(hc, hub, Request::to_class(0x23, HUB_CLEAR_FEATURE, C_PORT_CONNECTION + bit, port as u16), &[]);
        }
    }
    let connected = status & 1 != 0;
    let existing = with(hc, |c| c.devices.iter().flatten().find(|d| d.parent == Some((hub, port))).map(|d| d.slot));
    if let Some(slot) = existing {
        if connected && change & 1 == 0 {
            return;
        }
        detach(hc, slot);
    }
    if !connected {
        return;
    }
    sleep(hc, 100); // let the plug settle
    if control(hc, hub, Request::to_class(0x23, HUB_SET_FEATURE, PORT_RESET, port as u16), &[]).is_err() {
        return;
    }
    let mut status = 0;
    let mut done = false;
    for _ in 0..50 {
        sleep(hc, 10);
        let Ok((s, c)) = hub_port_status(hc, hub, port) else { return };
        if c & (1 << 4) != 0 {
            status = s;
            done = true;
            break;
        }
    }
    let _ = control(hc, hub, Request::to_class(0x23, HUB_CLEAR_FEATURE, C_PORT_RESET, port as u16), &[]);
    let place = with(hc, |c| {
        let h = c.devices.get(hub as usize)?.as_ref()?;
        Some((h.root_port, h.route, h.depth, h.speed, h.tt))
    });
    let Some((root_port, route, depth, hub_speed, hub_tt)) = place else { return };
    let mut place = Place { root_port, route: route | (port.min(15) as u32) << (4 * depth), parent: Some((hub, port)), tt: (0, 0), depth: depth + 1 };
    if !done || status & 2 == 0 {
        log_port(hc, &place.describe(), "something is plugged in, but the hub's port didn't come on");
        return;
    }
    let speed = if status & (1 << 9) != 0 {
        Speed::Low
    } else if status & (1 << 10) != 0 {
        Speed::High
    } else {
        Speed::Full
    };
    if matches!(speed, Speed::Low | Speed::Full) {
        place.tt = if hub_speed == Speed::High { (hub, port) } else { hub_tt };
    }
    sleep(hc, 10);
    attach(hc, place, speed);
}

/// A device went away: so does everything behind it, then its slot.
fn detach(hc: usize, slot: u8) {
    let children: Vec<u8> =
        with(hc, |c| c.devices.iter().flatten().filter(|d| d.parent.map(|p| p.0) == Some(slot)).map(|d| d.slot).collect());
    for child in children {
        detach(hc, child);
    }
    let (name, used, drive) = with(hc, |c| match c.devices.get(slot as usize) {
        Some(Some(d)) => (d.name.clone(), matches!(d.role, Role::Input(_) | Role::Net(_)), matches!(d.role, Role::Storage(_))),
        _ => (String::new(), false, false),
    });
    let _ = command(hc, |h| h.disable_slot(slot));
    let place = with(hc, |c| {
        c.host.set_context(slot, None);
        let place = c.devices.get(slot as usize).and_then(|d| d.as_ref()).map(|d| Place {
            root_port: d.root_port,
            route: d.route,
            parent: d.parent,
            tt: d.tt,
            depth: d.depth,
        });
        if let Some(d) = c.devices.get_mut(slot as usize) {
            *d = None;
        }
        place
    });
    if used {
        keyboard::stop_repeat();
        changed();
    }
    if drive {
        storage::changed();
    }
    if let Some(place) = place {
        log_port(hc, &place.describe(), &format!("\"{name}\" was unplugged"));
    }
}
