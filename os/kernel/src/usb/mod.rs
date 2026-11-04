//! USB: Chat Computer's own drivers for the PC's USB controllers and what is plugged into them. So far:
//! keyboards, mice, network adapters and phones sharing their connection (net.rs), drives (storage.rs: USB sticks,
//! external disks, card readers), sound cards (audio.rs: headsets, speakers), and the hubs they may be behind (a
//! monitor's, a dock's, or one inside the PC).
//!
//! The controllers' events are handled in their interrupt, or, on a controller that can't send
//! interrupt messages, whenever the kernel wakes. Work that has to wait for a device (finding out what
//! was just plugged in, setting a keyboard's lights) runs in the kernel's main loop, in `work`.

mod audio;
mod device;
mod net;
pub mod storage;
mod xhci;

use crate::sync::SpinLock;
use crate::{cpu, idt, keyboard, pci, time};
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use device::{Device, InputKind, Role};
use xhci::Host;

/// How fast a device talks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Speed {
    Low,
    Full,
    High,
    Super,
    SuperPlus,
}

impl Speed {
    /// From the controller's default speed numbers.
    fn from_xhci(id: u32) -> Option<Speed> {
        Some(match id {
            1 => Speed::Full,
            2 => Speed::Low,
            3 => Speed::High,
            4 => Speed::Super,
            5 => Speed::SuperPlus,
            _ => return None,
        })
    }

    fn xhci(self) -> u32 {
        match self {
            Speed::Full => 1,
            Speed::Low => 2,
            Speed::High => 3,
            Speed::Super => 4,
            Speed::SuperPlus => 5,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Speed::Low => "low speed",
            Speed::Full => "full speed",
            Speed::High => "high speed",
            Speed::Super => "SuperSpeed",
            Speed::SuperPlus => "SuperSpeed+",
        }
    }
}

/// A controller and the devices on it, by slot number.
pub struct Controller {
    host: Host,
    pci: pci::Address,
    devices: Vec<Option<Device>>,
}

static CONTROLLERS: SpinLock<Vec<Controller>> = SpinLock::new(Vec::new());
/// Counts changes to the lists of keyboards, mice and network adapters, so the screen knows when to say so.
static CHANGES: AtomicU64 = AtomicU64::new(0);

fn changed() {
    CHANGES.fetch_add(1, Ordering::Relaxed);
}

/// Before the firmware's memory is reused: takes every USB controller from the firmware and stops it.
/// Returns how many there are.
pub fn take_over() -> usize {
    let mut controllers = CONTROLLERS.lock();
    for f in pci::find(0x0C, 0x03, 0x30) {
        match Host::take_over(&f) {
            Ok(host) => {
                crate::log!(
                    "Chat Computer kernel: USB {}: xHCI controller {:04x}:{:04x}, {} ports, taken over from the firmware",
                    f.at,
                    f.vendor,
                    f.device,
                    host.ports
                );
                controllers.push(Controller { host, pci: f.at, devices: Vec::new() });
            }
            Err(why) => crate::log!("Chat Computer kernel: USB {}: couldn't take the controller over: {why}", f.at),
        }
    }
    controllers.len()
}

/// Once interrupts are on: starts the controllers, then finds what is plugged in.
pub fn start() {
    idt::set_device_handler(idt::USB_VECTOR, interrupt);
    {
        let mut controllers = CONTROLLERS.lock();
        for c in controllers.iter_mut() {
            let at = c.pci;
            match c.host.start(at, idt::USB_VECTOR) {
                Ok(()) => {
                    c.devices = (0..=c.host.max_slots as usize).map(|_| None).collect();
                    crate::log!(
                        "Chat Computer kernel: USB {}: running; interrupts {}",
                        c.host.name,
                        match c.host.signal {
                            pci::Signal::MsiX => "by MSI-X",
                            pci::Signal::Msi => "by MSI",
                            pci::Signal::None => "off (it is checked for work instead)",
                        }
                    );
                }
                Err(why) => crate::log!("Chat Computer kernel: USB {}: couldn't start it: {why}", c.host.name),
            }
        }
    }
    work();
    changed();
}

/// The controllers' interrupt.
fn interrupt() {
    // Interrupts are off while the lock is held, and only the first core uses USB (cores.rs: the others run the model's
    // arithmetic alone), so it is always free here; but an interrupt handler never waits for a lock.
    if let Some(mut controllers) = CONTROLLERS.try_lock() {
        for c in controllers.iter_mut() {
            handle_events(c);
        }
    }
}

fn handle_events(c: &mut Controller) {
    if !c.host.running {
        return;
    }
    let db = c.host.doorbell();
    while let Some(e) = c.host.next_event() {
        match e.kind() {
            xhci::EVENT_COMMAND => c.host.command_done = Some((e.param, e.code(), e.slot())),
            xhci::EVENT_PORT => {
                let port = (e.param >> 24) as u8;
                if (1..=c.host.ports).contains(&port) {
                    c.host.mark_port(port);
                }
            }
            xhci::EVENT_TRANSFER => {
                if let Some(Some(d)) = c.devices.get_mut(e.slot() as usize) {
                    d.transfer_event(db, e);
                }
            }
            xhci::EVENT_HOST => {
                crate::log!("Chat Computer kernel: USB {}: the controller reported a problem: {}", c.host.name, xhci::code_name(e.code()))
            }
            _ => {}
        }
    }
    // Always, even with no new events: until this is written, the controller sends no more interrupts.
    c.host.events_handled();
    if c.host.failed() {
        crate::log!("Chat Computer kernel: USB {}: the controller stopped itself after an error", c.host.name);
        c.host.running = false;
        changed();
    }
}

/// In the kernel's main loop: devices plugged in or out, hubs' changes, keyboards' lights, media put in drives or
/// taken out.
pub fn work() {
    let count = CONTROLLERS.lock().len();
    for hc in 0..count {
        // A hub found on one pass has its ports looked at on the next, so a few passes find everything.
        for _ in 0..8 {
            if !pass(hc) {
                break;
            }
        }
        storage::poll(hc);
    }
}

/// One look at controller `hc`. Returns whether anything needed doing.
fn pass(hc: usize) -> bool {
    let (ports, hubs, broken) = with(hc, |c| {
        handle_events(c);
        if !c.host.running {
            return (Vec::new(), Vec::new(), Vec::new());
        }
        let hubs: Vec<(u8, u32)> = c
            .devices
            .iter_mut()
            .flatten()
            .filter_map(|d| match &mut d.role {
                Role::Hub(h) if h.changes != 0 => Some((d.slot, core::mem::take(&mut h.changes))),
                _ => None,
            })
            .collect();
        let mut broken: Vec<(u8, u8)> = Vec::new();
        for d in c.devices.iter_mut().flatten() {
            let stopped = core::mem::take(&mut d.stopped);
            broken.extend((1..32u8).filter(|dci| stopped & 1 << dci != 0).map(|dci| (d.slot, dci)));
        }
        (c.host.take_port_changes(), hubs, broken)
    });
    let busy = !ports.is_empty() || !hubs.is_empty() || !broken.is_empty();
    for port in ports {
        device::root_port(hc, port);
    }
    for (slot, changes) in hubs {
        device::hub_changes(hc, slot, changes);
    }
    for (slot, dci) in broken {
        device::restart_endpoint(hc, slot, dci);
    }
    let lights = keyboard::lights();
    let keyboards: Vec<(u8, u8)> = with(hc, |c| {
        let mut out = Vec::new();
        for d in c.devices.iter_mut().flatten() {
            let Role::Input(inputs) = &mut d.role else { continue };
            for i in inputs.iter_mut() {
                if let InputKind::Keyboard { lights: shown, .. } = &mut i.kind
                    && *shown != lights
                {
                    *shown = lights;
                    out.push((d.slot, i.interface));
                }
            }
        }
        out
    });
    for (slot, interface) in keyboards {
        device::set_lights(hc, slot, interface, lights);
    }
    busy
}

/// Runs `f` on controller `hc`, with the controllers locked (and interrupts off).
fn with<R>(hc: usize, f: impl FnOnce(&mut Controller) -> R) -> R {
    let mut controllers = CONTROLLERS.lock();
    f(&mut controllers[hc])
}

/// Waits up to `ms` milliseconds for `done` to find what it waits for, handling the controller's events
/// meanwhile. The processor sleeps between looks, after the first few milliseconds.
fn wait<R>(hc: usize, ms: u64, mut done: impl FnMut(&mut Controller) -> Option<R>) -> Option<R> {
    let start = time::now_us();
    loop {
        {
            let mut controllers = CONTROLLERS.lock();
            let c = &mut controllers[hc];
            handle_events(c);
            if let Some(r) = done(c) {
                return Some(r);
            }
            if !c.host.running {
                return None;
            }
        }
        let waited = time::now_us().saturating_sub(start);
        if waited >= ms * 1000 {
            return None;
        }
        if waited < 3000 || !cpu::interrupts_enabled() {
            time::delay_us(50);
        } else {
            cpu::wait_for_interrupt();
        }
    }
}

/// Waits `ms` milliseconds, handling the controller's events meanwhile.
fn sleep(hc: usize, ms: u64) {
    let _ = wait(hc, ms, |_| None::<()>);
}

/// What the screen says about USB.
pub struct Status {
    pub controllers: usize,
    pub running: usize,
    pub keyboards: Vec<String>,
    pub mice: Vec<String>,
}

pub fn status() -> Status {
    let controllers = CONTROLLERS.lock();
    let (mut keyboards, mut mice) = (Vec::new(), Vec::new());
    for c in controllers.iter().filter(|c| c.host.running) {
        for d in c.devices.iter().flatten() {
            let Role::Input(inputs) = &d.role else { continue };
            // A keyboard if its first interface is one: a wireless receiver's is (its mouse comes second), while a
            // gaming mouse's keys (for its macros) come after its mouse.
            if inputs.first().is_some_and(|i| matches!(i.kind, InputKind::Keyboard { .. })) {
                keyboards.push(d.name.clone());
            }
            if inputs.iter().any(|i| matches!(i.kind, InputKind::Mouse(_))) {
                mice.push(d.name.clone());
            }
        }
    }
    let running = controllers.iter().filter(|c| c.host.running).count();
    Status { controllers: controllers.len(), running, keyboards, mice }
}

/// Changes whenever the lists of keyboards, mice or network adapters may have changed.
pub fn changes() -> u64 {
    CHANGES.load(Ordering::Relaxed)
}

/// A USB network adapter, as the network side knows it.
pub struct NetAdapter {
    pub hc: usize,
    pub slot: u8,
    pub id: u64,
    pub mac: [u8; 6],
    pub name: String,
    pub kind: &'static str,
    pub speed: &'static str,
}

/// The network adapters plugged in.
pub fn net_adapters() -> Vec<NetAdapter> {
    let controllers = CONTROLLERS.lock();
    let mut found = Vec::new();
    for (hc, c) in controllers.iter().enumerate().filter(|(_, c)| c.host.running) {
        for d in c.devices.iter().flatten() {
            if let Role::Net(n) = &d.role {
                let kind = match n.kind {
                    net::Kind::Ecm => "USB CDC ECM",
                    net::Kind::Ncm => "USB CDC NCM",
                    net::Kind::Rndis => "RNDIS",
                };
                found.push(NetAdapter { hc, slot: d.slot, id: n.id, mac: n.mac, name: d.name.clone(), kind, speed: d.speed_name() });
            }
        }
    }
    found
}

/// Runs `f` on adapter `id` in `slot` of controller `hc`, if it's still plugged in.
fn with_net<R>(hc: usize, slot: u8, id: u64, f: impl FnOnce(&mut net::Net, &mut Vec<Option<xhci::Ring>>, xhci::Doorbell) -> R) -> Option<R> {
    let mut controllers = CONTROLLERS.lock();
    let c = controllers.get_mut(hc)?;
    if !c.host.running {
        return None;
    }
    let db = c.host.doorbell();
    let d = c.devices.get_mut(slot as usize)?.as_mut()?;
    let (rings, role) = d.rings_and_role();
    match role {
        Role::Net(n) if n.id == id => Some(f(n, rings, db)),
        _ => None,
    }
}

/// The frames that have arrived at an adapter, and whether its link is up. None: it was unplugged.
pub fn net_take(hc: usize, slot: u8, id: u64) -> Option<(Vec<Vec<u8>>, bool)> {
    with_net(hc, slot, id, |n, _, _| (n.frames.drain(..).collect(), n.link))
}

/// Sends a frame. None: the adapter was unplugged; false: no room just now.
pub fn net_send(hc: usize, slot: u8, id: u64, frame: &[u8]) -> Option<bool> {
    with_net(hc, slot, id, |n, rings, db| n.send(rings, db, slot, frame))
}

/// The USB sound card to play on, if one is plugged in (the first found): its controller and slot, its name and its
/// rate.
pub fn sound_card() -> Option<(usize, u8, String, u32)> {
    let controllers = CONTROLLERS.lock();
    for (hc, c) in controllers.iter().enumerate() {
        if !c.host.running {
            continue;
        }
        for d in c.devices.iter().flatten() {
            if let Role::Audio(a) = &d.role {
                return Some((hc, d.slot, d.name.clone(), a.rate));
            }
        }
    }
    None
}

/// Plays `samples` (one channel, 16 bits, at its rate) on the sound card in `slot` of controller `hc`. False if
/// it's gone, or memory ran out.
pub fn sound_card_play(hc: usize, slot: u8, samples: &[i16]) -> bool {
    let mut controllers = CONTROLLERS.lock();
    let Some(c) = controllers.get_mut(hc) else { return false };
    if !c.host.running {
        return false;
    }
    let (db, low) = (c.host.doorbell(), c.host.low);
    let Some(Some(d)) = c.devices.get_mut(slot as usize) else { return false };
    let (rings, role) = d.rings_and_role();
    match role {
        Role::Audio(a) => a.play(rings, db, slot, samples, low),
        _ => false,
    }
}

/// Stops what the sound card in `slot` of controller `hc` plays.
pub fn sound_card_stop(hc: usize, slot: u8) {
    let mut controllers = CONTROLLERS.lock();
    if let Some(Some(d)) = controllers.get_mut(hc).and_then(|c| c.devices.get_mut(slot as usize))
        && let Role::Audio(a) = &mut d.role
    {
        a.stop();
    }
}
