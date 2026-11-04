//! Networking over USB: the standard ways a USB device carries Ethernet frames, so one driver covers many USB
//! Ethernet adapters and phones sharing their connection ("USB tethering"):
//! * CDC ECM (the USB standard's Ethernet Control Model): one frame per transfer. Many adapters offer it (often as
//!   their second configuration), and QEMU's usb-net.
//! * CDC NCM (Network Control Model): frames packed into "transfer blocks". Newer adapters and phones.
//! * RNDIS (Microsoft's Remote NDIS): each frame behind a 44-byte header, and settings asked for and made through
//!   messages on the control endpoint. Most Android phones' USB tethering, and QEMU's usb-net again.
//!
//! (An iPhone shares its connection over USB in a way of Apple's own, which isn't one of these.)
//!
//! Frames that arrive come in on the bulk-in endpoint's transfers, which are handled where the controller's events
//! are (its interrupt, mostly): they're unpacked into a queue, which the network stack takes from in the main loop
//! (`take`). Frames to send go straight onto the bulk-out endpoint's ring.

use super::device::{self, add_endpoints, control, string, Endpoint, Error, Interface, Request, SET_INTERFACE};
use super::xhci::{self, Doorbell, Page, Ring, Trb};
use super::{sleep, with, Speed};
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Ecm,
    Ncm,
    Rndis,
}

/// For the log: what kind of adapter.
pub fn describe(kind: Kind) -> &'static str {
    match kind {
        Kind::Ecm => "a network adapter (USB CDC ECM)",
        Kind::Ncm => "a network adapter (USB CDC NCM)",
        Kind::Rndis => "a network adapter (RNDIS)",
    }
}

/// Which kind to use when a device offers several: the USB standard's over Microsoft's. (A test build can be told
/// to prefer RNDIS, to test it with QEMU's usb-net, which offers both.)
pub fn preference(kind: Kind) -> u8 {
    let rndis_first = option_env!("CHATCOMPUTER_TEST_USB_NET") == Some("rndis");
    match kind {
        Kind::Ncm => 3,
        Kind::Ecm => 2,
        Kind::Rndis if rndis_first => 4,
        Kind::Rndis => 1,
    }
}

/// The interfaces' network function, if one of them is the control interface of one, with a data interface that
/// has the two bulk endpoints frames go over.
pub fn kind(interfaces: &[Interface]) -> Option<Kind> {
    let (control, kind) = control_interface(interfaces)?;
    data_interface(interfaces, control).map(|_| kind)
}

fn control_interface(interfaces: &[Interface]) -> Option<(&Interface, Kind)> {
    interfaces.iter().filter(|i| i.alt == 0).find_map(|i| {
        let kind = match (i.class, i.subclass, i.protocol) {
            (0x02, 0x0D, _) => Kind::Ncm,
            (0x02, 0x06, _) => Kind::Ecm,
            (0x02, 0x02, 0xFF) | (0xE0, 0x01, 0x03) | (0xEF, 0x04, 0x01) => Kind::Rndis,
            _ => return None,
        };
        Some((i, kind))
    })
}

/// The control interface's class descriptor of `subtype` (CDC functional descriptors: 0x06 union, 0x0F
/// Ethernet, 0x1A NCM).
fn functional(control: &Interface, subtype: u8) -> Option<&[u8]> {
    control.extra.iter().find(|d| d.len() >= 3 && d[2] == subtype).map(|d| d.as_slice())
}

/// The data interface's setting with a bulk-in and a bulk-out endpoint: (interface, alternate setting, in, out).
fn data_interface(interfaces: &[Interface], control: &Interface) -> Option<(u8, u8, Endpoint, Endpoint)> {
    let number = functional(control, 0x06).and_then(|u| u.get(4).copied()).unwrap_or(control.number + 1);
    interfaces.iter().filter(|i| i.number == number).find_map(|i| {
        let input = i.endpoints.iter().find(|e| e.bulk_in())?;
        let output = i.endpoints.iter().find(|e| e.bulk_out())?;
        Some((i.number, i.alt, *input, *output))
    })
}

const RX_BUFFERS: usize = 16;
const TX_BUFFERS: usize = 16;
/// Each transfer's buffer: a page. Devices are told so (RNDIS, NCM); ECM's frames are smaller anyway.
const BUF: usize = 4096;
/// Frames kept for the network stack, at most.
const FRAMES_MAX: usize = 256;

// CDC class requests.
const SET_ETHERNET_PACKET_FILTER: u8 = 0x43;
const GET_NTB_PARAMETERS: u8 = 0x80;
const SET_NTB_INPUT_SIZE: u8 = 0x86;
const SEND_ENCAPSULATED_COMMAND: u8 = 0x00;
const GET_ENCAPSULATED_RESPONSE: u8 = 0x01;
/// Frames to us, broadcasts, and all multicasts (for IPv6 later).
const PACKET_FILTER: u16 = 0x04 | 0x08 | 0x02;

// RNDIS messages and the settings (OIDs) used.
const RNDIS_PACKET: u32 = 1;
const RNDIS_INITIALIZE: u32 = 2;
const RNDIS_QUERY: u32 = 4;
const RNDIS_SET: u32 = 5;
const OID_PERMANENT_ADDRESS: u32 = 0x0101_0101;
const OID_CURRENT_ADDRESS: u32 = 0x0101_0102;
const OID_PACKET_FILTER: u32 = 0x0001_010E;
const RNDIS_HEADER: usize = 44;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// NCM's rules for the blocks sent (from GET_NTB_PARAMETERS).
#[derive(Clone, Copy)]
struct Ncm {
    out_max: usize,
    divisor: usize,
    remainder: usize,
    alignment: usize,
    sequence: u16,
}

pub struct Net {
    /// Different for every adapter plugged in (a slot number is reused).
    pub id: u64,
    pub kind: Kind,
    in_dci: u8,
    out_dci: u8,
    notify_dci: Option<u8>,
    notify_size: u16,
    /// The bulk-out endpoint's packet size: a transfer of a whole number of packets needs an end mark.
    out_packet: usize,
    pub mac: [u8; 6],
    pub link: bool,
    rx: Vec<Page>,
    /// Receive buffers queued on the bulk-in endpoint, in order.
    rx_queued: VecDeque<usize>,
    tx: Vec<Page>,
    tx_free: Vec<usize>,
    tx_queued: VecDeque<usize>,
    notify: Option<Page>,
    /// Frames that arrived, for the network stack.
    pub frames: VecDeque<Vec<u8>>,
    pub dropped: u64,
    ncm: Ncm,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

/// "0123456789AB": six bytes, as ECM's address string writes them.
fn hex_mac(s: &str) -> Option<[u8; 6]> {
    let digits: Vec<u8> = s.chars().filter_map(|c| c.to_digit(16).map(|d| d as u8)).collect();
    if digits.len() != 12 {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, b) in mac.iter_mut().enumerate() {
        *b = digits[2 * i] << 4 | digits[2 * i + 1];
    }
    Some(mac)
}

impl Net {
    fn queue_rx(&mut self, rings: &mut [Option<Ring>], db: Doorbell, slot: u8, i: usize) {
        if let Some(Some(ring)) = rings.get_mut(self.in_dci as usize) {
            ring.push(Trb::new(xhci::TRB_NORMAL, self.rx[i].phys, BUF as u32, xhci::ON_COMPLETION | xhci::SHORT_OK));
            self.rx_queued.push_back(i);
            db.ring(slot, self.in_dci);
        }
    }

    fn queue_notify(&mut self, rings: &mut [Option<Ring>], db: Doorbell, slot: u8) {
        let (Some(dci), Some(page)) = (self.notify_dci, self.notify.as_ref()) else { return };
        if let Some(Some(ring)) = rings.get_mut(dci as usize) {
            ring.push(Trb::new(xhci::TRB_NORMAL, page.phys, self.notify_size as u32, xhci::ON_COMPLETION | xhci::SHORT_OK));
            db.ring(slot, dci);
        }
    }

    /// Endpoint `dci` was restarted after an error (everything queued on it skipped): queues again what it needs.
    pub fn restarted(&mut self, rings: &mut [Option<Ring>], db: Doorbell, slot: u8, dci: u8) {
        if dci == self.in_dci {
            self.rx_queued.clear();
            for i in 0..self.rx.len() {
                self.queue_rx(rings, db, slot, i);
            }
        } else if dci == self.out_dci {
            while let Some(i) = self.tx_queued.pop_front() {
                self.tx_free.push(i);
            }
        } else if Some(dci) == self.notify_dci {
            self.queue_notify(rings, db, slot);
        }
    }

    /// A transfer on one of its endpoints finished. False if the endpoint stopped on an error.
    pub fn transfer_event(&mut self, rings: &mut [Option<Ring>], db: Doorbell, slot: u8, e: Trb) -> bool {
        let dci = e.endpoint();
        let ok = xhci::ok(e.code());
        if dci == self.in_dci {
            let Some(i) = self.rx_queued.pop_front() else { return true };
            if !ok {
                self.rx_queued.clear();
                return false;
            }
            let len = BUF.saturating_sub(e.residual() as usize);
            let data: Vec<u8> = (0..len).map(|n| unsafe { read_volatile(self.rx[i].at::<u8>(n)) }).collect();
            self.unpack(&data);
            self.queue_rx(rings, db, slot, i);
        } else if dci == self.out_dci {
            if let Some(i) = self.tx_queued.pop_front() {
                self.tx_free.push(i);
            }
            if !ok {
                return false;
            }
        } else if Some(dci) == self.notify_dci {
            if !ok {
                return false;
            }
            if let Some(page) = self.notify.as_ref() {
                let head: Vec<u8> = (0..8).map(|n| unsafe { read_volatile(page.at::<u8>(n)) }).collect();
                // CDC's NETWORK_CONNECTION notification: connected or not.
                if head[0] == 0xA1 && head[1] == 0x00 {
                    self.link = head[2] != 0;
                }
            }
            self.queue_notify(rings, db, slot);
        }
        true
    }

    fn keep(&mut self, frame: &[u8]) {
        if frame.len() < 14 || frame.len() > 1522 {
            return;
        }
        if self.frames.len() >= FRAMES_MAX {
            self.dropped += 1;
            return;
        }
        self.frames.push_back(frame.to_vec());
    }

    /// The frames in one transfer that arrived.
    fn unpack(&mut self, data: &[u8]) {
        match self.kind {
            Kind::Ecm => self.keep(data),
            Kind::Rndis => {
                let mut at = 0;
                while at + RNDIS_HEADER <= data.len() {
                    let (Some(kind), Some(len)) = (u32_at(data, at), u32_at(data, at + 4)) else { break };
                    let len = len as usize;
                    if len < 8 || at + len > data.len() {
                        break;
                    }
                    if kind == RNDIS_PACKET {
                        let offset = u32_at(data, at + 8).unwrap_or(0) as usize;
                        let size = u32_at(data, at + 12).unwrap_or(0) as usize;
                        let start = at + 8 + offset;
                        if let Some(frame) = data.get(start..start + size) {
                            let frame = frame.to_vec();
                            self.keep(&frame);
                        }
                    }
                    at += len;
                }
            }
            Kind::Ncm => {
                // A transfer block: its header ("NCMH"), then tables of where the frames are ("NCM0"), chained.
                if u32_at(data, 0) != Some(0x484D_434E) {
                    return;
                }
                let mut ndp = u16_at(data, 10).unwrap_or(0) as usize;
                for _ in 0..16 {
                    if ndp == 0 || ndp + 8 > data.len() || !matches!(u32_at(data, ndp), Some(0x304D_434E | 0x314D_434E)) {
                        break;
                    }
                    let length = u16_at(data, ndp + 4).unwrap_or(0) as usize;
                    let next = u16_at(data, ndp + 6).unwrap_or(0) as usize;
                    let mut entry = ndp + 8;
                    while entry + 4 <= (ndp + length).min(data.len()) {
                        let (index, size) = (u16_at(data, entry).unwrap_or(0) as usize, u16_at(data, entry + 2).unwrap_or(0) as usize);
                        if index == 0 || size == 0 {
                            break;
                        }
                        if let Some(frame) = data.get(index..index + size) {
                            let frame = frame.to_vec();
                            self.keep(&frame);
                        }
                        entry += 4;
                    }
                    ndp = next;
                }
            }
        }
    }

    /// Puts a frame on the bulk-out endpoint. False if there's no room just now (or it's too big).
    pub fn send(&mut self, rings: &mut [Option<Ring>], db: Doorbell, slot: u8, frame: &[u8]) -> bool {
        if frame.len() > 1522 {
            return false;
        }
        let Some(i) = self.tx_free.pop() else { return false };
        let page = &self.tx[i];
        let put = |at: usize, bytes: &[u8]| {
            for (n, b) in bytes.iter().enumerate() {
                unsafe { write_volatile(page.at::<u8>(at + n), *b) };
            }
        };
        let mut len = match self.kind {
            Kind::Ecm => {
                put(0, frame);
                frame.len()
            }
            Kind::Rndis => {
                let mut head = [0u8; RNDIS_HEADER];
                head[0..4].copy_from_slice(&RNDIS_PACKET.to_le_bytes());
                head[4..8].copy_from_slice(&((RNDIS_HEADER + frame.len()) as u32).to_le_bytes());
                head[8..12].copy_from_slice(&((RNDIS_HEADER - 8) as u32).to_le_bytes()); // from offset 8
                head[12..16].copy_from_slice(&(frame.len() as u32).to_le_bytes());
                put(0, &head);
                put(RNDIS_HEADER, frame);
                RNDIS_HEADER + frame.len()
            }
            Kind::Ncm => {
                let n = &mut self.ncm;
                let ndp = 12usize.next_multiple_of(n.alignment.max(4));
                let mut data_at = ndp + 16;
                while data_at % n.divisor.max(1) != n.remainder % n.divisor.max(1) {
                    data_at += 1;
                }
                let mut total = data_at + frame.len();
                if total.is_multiple_of(self.out_packet) {
                    total += 1; // (a byte of padding at the end of the block: see below)
                }
                if total > n.out_max.min(BUF) {
                    self.tx_free.push(i);
                    return false;
                }
                let mut head = alloc::vec![0u8; data_at];
                head[0..4].copy_from_slice(b"NCMH");
                head[4..6].copy_from_slice(&12u16.to_le_bytes());
                head[6..8].copy_from_slice(&n.sequence.to_le_bytes());
                head[8..10].copy_from_slice(&(total as u16).to_le_bytes());
                head[10..12].copy_from_slice(&(ndp as u16).to_le_bytes());
                head[ndp..ndp + 4].copy_from_slice(b"NCM0");
                head[ndp + 4..ndp + 6].copy_from_slice(&16u16.to_le_bytes());
                head[ndp + 8..ndp + 10].copy_from_slice(&(data_at as u16).to_le_bytes());
                head[ndp + 10..ndp + 12].copy_from_slice(&(frame.len() as u16).to_le_bytes());
                n.sequence = n.sequence.wrapping_add(1);
                put(0, &head);
                put(data_at, frame);
                if total > data_at + frame.len() {
                    put(data_at + frame.len(), &[0]);
                }
                total
            }
        };
        // A transfer that's a whole number of packets would need an empty packet after it to say it ended; a byte
        // of padding does the same (Ethernet ignores bytes after a frame; RNDIS's header and NCM's block say how long
        // what's in them is).
        if self.kind != Kind::Ncm && len.is_multiple_of(self.out_packet) {
            put(len, &[0]);
            len += 1;
        }
        let Some(Some(ring)) = rings.get_mut(self.out_dci as usize) else {
            self.tx_free.push(i);
            return false;
        };
        ring.push(Trb::new(xhci::TRB_NORMAL, self.tx[i].phys, len as u32, xhci::ON_COMPLETION));
        self.tx_queued.push_back(i);
        db.ring(slot, self.out_dci);
        true
    }
}

/// An RNDIS message on the control endpoint, and its answer (the one with the same request id; status messages
/// that come first are passed over).
fn rndis(hc: usize, slot: u8, interface: u8, message: &[u8]) -> Result<Vec<u8>, Error> {
    let id = u32_at(message, 8).unwrap_or(0);
    let send = Request { kind: 0x21, request: SEND_ENCAPSULATED_COMMAND, value: 0, index: interface as u16, length: message.len() as u16 };
    control(hc, slot, send, message)?;
    for _ in 0..50 {
        let get = Request { kind: 0xA1, request: GET_ENCAPSULATED_RESPONSE, value: 0, index: interface as u16, length: 1025 };
        if let Ok(r) = control(hc, slot, get, &[])
            && r.len() >= 16
            && u32_at(&r, 0).is_some_and(|k| k & 0x8000_0000 != 0)
            && u32_at(&r, 8) == Some(id)
        {
            return match u32_at(&r, 12) {
                Some(0) => Ok(r),
                _ => Err(Error::Bad("the adapter refused a setting")),
            };
        }
        sleep(hc, 20);
    }
    Err(Error::NoAnswer)
}

fn rndis_message(kind: u32, id: u32, fields: &[u32], data: &[u8]) -> Vec<u8> {
    let mut m = Vec::new();
    m.extend_from_slice(&kind.to_le_bytes());
    m.extend_from_slice(&((12 + 4 * fields.len() + data.len()) as u32).to_le_bytes());
    m.extend_from_slice(&id.to_le_bytes());
    for f in fields {
        m.extend_from_slice(&f.to_le_bytes());
    }
    m.extend_from_slice(data);
    m
}

/// An RNDIS setting's value. (The query carries an empty buffer of the size of the answer: some adapters refuse a
/// query without one.)
fn rndis_query(hc: usize, slot: u8, interface: u8, id: u32, oid: u32) -> Result<Vec<u8>, Error> {
    let r = rndis(hc, slot, interface, &rndis_message(RNDIS_QUERY, id, &[oid, 48, 20, 0], &[0; 48]))?;
    let (len, offset) = (u32_at(&r, 16).unwrap_or(0) as usize, u32_at(&r, 20).unwrap_or(0) as usize);
    r.get(8 + offset..8 + offset + len).map(|v| v.to_vec()).ok_or(Error::Bad("the adapter's answer didn't make sense"))
}

/// Sets up a network adapter: its configuration is set already. Starts it, and gives the device its role.
pub fn set_up(hc: usize, slot: u8, speed: Speed, kind: Kind, interfaces: &[Interface]) -> Result<(), Error> {
    let (control_if, _) = control_interface(interfaces).ok_or(Error::Bad("no network interface"))?;
    let control_number = control_if.number;
    let (data_number, data_alt, bulk_in, bulk_out) = data_interface(interfaces, control_if).ok_or(Error::Bad("no data interface"))?;
    let notify_ep = control_if.endpoints.iter().find(|e| e.interrupt_in()).copied();
    let ethernet = functional(control_if, 0x0F).map(|d| d.to_vec());
    let ncm_caps = functional(control_if, 0x1A).and_then(|d| d.get(5).copied()).unwrap_or(0);

    let mut ncm = Ncm { out_max: 2048, divisor: 4, remainder: 0, alignment: 4, sequence: 0 };
    if kind == Kind::Ncm {
        let r = Request { kind: 0xA1, request: GET_NTB_PARAMETERS, value: 0, index: control_number as u16, length: 28 };
        if let Ok(p) = control(hc, slot, r, &[])
            && p.len() >= 28
        {
            ncm.out_max = u32_at(&p, 16).unwrap_or(2048) as usize;
            ncm.divisor = u16_at(&p, 20).unwrap_or(4).max(1) as usize;
            ncm.remainder = u16_at(&p, 22).unwrap_or(0) as usize;
            ncm.alignment = u16_at(&p, 24).unwrap_or(4).max(1) as usize;
        }
        // Blocks from the adapter no bigger than a buffer (8 bytes long if it says it takes that form).
        let size = (BUF as u32).to_le_bytes();
        let mut data = size.to_vec();
        if ncm_caps & (1 << 5) != 0 {
            data.extend_from_slice(&[0; 4]);
        }
        let r = Request { kind: 0x21, request: SET_NTB_INPUT_SIZE, value: 0, index: control_number as u16, length: data.len() as u16 };
        let _ = control(hc, slot, r, &data);
    }

    // The endpoints, then (ECM, NCM) the data interface's setting that has them, which starts it.
    let mut eps = alloc::vec![bulk_in, bulk_out];
    eps.extend(notify_ep);
    add_endpoints(hc, slot, speed, &eps)?;
    if data_alt != 0 {
        control(hc, slot, Request::to_class(0x01, SET_INTERFACE, data_alt as u16, data_number as u16), &[])?;
    }

    let mut mac = None;
    match kind {
        Kind::Ecm | Kind::Ncm => {
            let _ = control(hc, slot, Request::to_class(0x21, SET_ETHERNET_PACKET_FILTER, PACKET_FILTER, control_number as u16), &[]);
            if let Some(e) = ethernet.as_ref()
                && let Some(&index) = e.get(3)
            {
                mac = string(hc, slot, index, 0x0409).and_then(|s| hex_mac(&s));
            }
        }
        Kind::Rndis => {
            rndis(hc, slot, control_number, &rndis_message(RNDIS_INITIALIZE, 1, &[1, 0, BUF as u32], &[]))?;
            match rndis_query(hc, slot, control_number, 2, OID_PERMANENT_ADDRESS)
                .or_else(|_| rndis_query(hc, slot, control_number, 3, OID_CURRENT_ADDRESS))
            {
                Ok(m) if m.len() >= 6 => mac = Some([m[0], m[1], m[2], m[3], m[4], m[5]]),
                Ok(m) => crate::log!("Chat Computer kernel: USB: the RNDIS adapter's address was {} bytes long", m.len()),
                Err(e) => crate::log!("Chat Computer kernel: USB: couldn't read the RNDIS adapter's address: {}", e.say()),
            }
            let filter = (0x1u32 | 0x2 | 0x4 | 0x8).to_le_bytes(); // to us, multicasts, all multicasts, broadcasts
            rndis(hc, slot, control_number, &rndis_message(RNDIS_SET, 4, &[OID_PACKET_FILTER, 4, 20, 0], &filter))?;
        }
    }
    let mac = mac.unwrap_or_else(|| {
        let r = crate::random::u64().to_le_bytes();
        [0x02, r[1], r[2], r[3], r[4], r[5]]
    });

    let low = with(hc, |c| c.devices.get(slot as usize).and_then(|d| d.as_ref()).map(|d| d.low)).ok_or(Error::Gone)?;
    let pages = |n: usize| -> Option<Vec<Page>> { (0..n).map(|_| Page::new(low)).collect() };
    let (Some(rx), Some(tx)) = (pages(RX_BUFFERS), pages(TX_BUFFERS)) else { return Err(Error::Bad("out of memory")) };
    let notify = match notify_ep {
        Some(_) => Some(Page::new(low).ok_or(Error::Bad("out of memory"))?),
        None => None,
    };
    let mut net = Net {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        kind,
        in_dci: bulk_in.dci(),
        out_dci: bulk_out.dci(),
        notify_dci: notify_ep.map(|e| e.dci()),
        notify_size: notify_ep.map_or(8, |e| (e.max_packet & 0x7FF).clamp(8, 64)),
        out_packet: (bulk_out.max_packet & 0x7FF).max(8) as usize,
        mac,
        link: true,
        rx,
        rx_queued: VecDeque::new(),
        tx,
        tx_free: (0..TX_BUFFERS).rev().collect(),
        tx_queued: VecDeque::new(),
        notify,
        frames: VecDeque::new(),
        dropped: 0,
        ncm,
    };
    with(hc, |c| {
        let db = c.host.doorbell();
        let Some(Some(d)) = c.devices.get_mut(slot as usize) else { return };
        let (rings, role) = d.rings_and_role();
        for i in 0..RX_BUFFERS {
            net.queue_rx(rings, db, slot, i);
        }
        net.queue_notify(rings, db, slot);
        *role = device::Role::Net(alloc::boxed::Box::new(net));
    });
    Ok(())
}
