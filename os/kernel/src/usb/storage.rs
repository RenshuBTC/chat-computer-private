//! USB drives: USB sticks, external SSDs and hard drives, and memory card readers (many laptops' SD card slot is one,
//! inside), as nearly all of them speak USB's mass storage class: its "bulk-only transport" (USB Mass Storage Class
//! Bulk-Only Transport, revision 1.0). Each command goes out in a 31-byte command block wrapper on the drive's bulk-out
//! endpoint, its data comes in on the bulk-in endpoint, then a 13-byte status wrapper says how it went. The commands
//! are SCSI's (the SCSI Primary and Block Commands): INQUIRY (what it is), TEST UNIT READY (whether a medium is in),
//! REQUEST SENSE (why not), READ CAPACITY (how big), READ. Nothing is ever written to a drive.
//!
//! A drive may have several logical units: a card reader has one a slot. Each is asked about once a second whether a
//! medium is in it (a card put in or taken out), so files.rs reads a card as soon as it's in, and forgets it when it's
//! taken out.
//!
//! Commands run in the kernel's main loop (files.rs reading a file), waiting for each transfer. The transfers' events
//! come in the controller's interrupt, where they're kept with the drive until its command takes them.

use super::device::{add_endpoints, control, restart_endpoint, Endpoint, Error, Interface, Request, Role, SET_INTERFACE};
use super::xhci::{self, Doorbell, Page, Ring, Trb};
use super::{wait, with, Speed};
use crate::time;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, AtomicU64, Ordering};

/// Pages of data a command can bring in: 64 KiB.
const DATA_PAGES: usize = 16;
/// The most bytes a read brings in at once.
pub const MOST: usize = DATA_PAGES * 4096;

const CBW_SIGNATURE: u32 = 0x4342_5355; // "USBC"
const CSW_SIGNATURE: u32 = 0x5342_5355; // "USBS"
/// Where the status wrapper lands in the command page (the command block wrapper is at its start).
const CSW_AT: usize = 2048;

// Requests on the control endpoint.
const MASS_STORAGE_RESET: u8 = 0xFF;
const GET_MAX_LUN: u8 = 0xFE;
const CLEAR_FEATURE: u8 = 1;
const ENDPOINT_HALT: u16 = 0;

// SCSI commands.
const TEST_UNIT_READY: u8 = 0x00;
const REQUEST_SENSE: u8 = 0x03;
const INQUIRY: u8 = 0x12;
const START_STOP_UNIT: u8 = 0x1B;
const READ_CAPACITY_10: u8 = 0x25;
const READ_10: u8 = 0x28;
const READ_16: u8 = 0x88;
const READ_CAPACITY_16: u8 = 0x9E;

// SCSI sense keys and codes.
const NOT_READY: u8 = 0x02;
const UNIT_ATTENTION: u8 = 0x06;
const MEDIUM_CHANGED: u8 = 0x28;
const MEDIUM_NOT_PRESENT: u8 = 0x3A;

/// How long a command may take (a hard drive in a USB case may have to spin up first), and a question about the
/// medium (asked every second, so a drive that doesn't answer isn't waited for long).
const COMMAND_MS: u64 = 10_000;
const QUESTION_MS: u64 = 2_000;
/// How often each logical unit is asked whether a medium is in it.
const POLL_MS: u64 = 1_000;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// Changes whenever a drive is plugged in or out, or a medium put in or taken out.
static CHANGES: AtomicU64 = AtomicU64::new(0);

pub(super) fn changed() {
    CHANGES.fetch_add(1, Ordering::Relaxed);
}

/// Changes whenever a USB drive, or a medium in one, comes or goes.
pub fn changes() -> u64 {
    CHANGES.load(Ordering::Relaxed)
}

/// A USB drive, plugged in.
pub(super) struct Storage {
    /// Different for every drive plugged in (a slot number is reused).
    pub id: u64,
    interface: u8,
    in_dci: u8,
    out_dci: u8,
    in_address: u8,
    out_address: u8,
    /// The bulk-in endpoint's packet size, for each TRB's count of the packets still to come in its transfer.
    in_packet: usize,
    /// Old CD drives' and floppy drives' ways (ATAPI, SFF-8070i) take 12-byte commands only.
    twelve: bool,
    tag: u32,
    /// The command block wrapper (at 0) and the status wrapper (at `CSW_AT`).
    page: Page,
    data: Vec<Page>,
    /// Transfer events on its bulk endpoints, kept until its command takes them.
    events: Vec<Trb>,
    pub luns: Vec<Lun>,
    /// Its serial number, which Windows names a USB stick by.
    pub serial: Option<String>,
    /// When its logical units were last asked whether a medium is in them (None: not yet).
    polled: Option<u64>,
}

/// One of a drive's logical units: a disk, or a card reader's slot.
pub struct Lun {
    pub number: u8,
    /// SCSI's peripheral device type: 0 a disk (a stick, a card's slot); 5 a CD or DVD drive, and so on.
    pub kind: u8,
    /// Its medium can be taken out (sticks say so too).
    pub removable: bool,
    /// What it says it is (INQUIRY's vendor and product), as "SanDisk Ultra".
    pub name: String,
    pub medium: Option<Medium>,
    /// Counts media put in, so one taken out and another put in isn't taken for the first.
    generation: u64,
}

/// A medium in a logical unit: its blocks, their size, and which medium put in it this is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Medium {
    pub blocks: u64,
    pub block_size: u32,
    pub generation: u64,
}

impl Storage {
    /// A transfer on one of its bulk endpoints finished (from the controller's interrupt).
    pub fn transfer_event(&mut self, e: Trb) {
        if self.events.len() < 32 {
            self.events.push(e);
        }
    }
}

/// Why a command failed.
enum Fail {
    /// The drive was unplugged.
    Gone,
    /// The drive said the command failed: REQUEST SENSE says why.
    Failed,
    /// Something went wrong on the way (the drive has been reset): what.
    Broken(String),
}

/// The interface that speaks the bulk-only transport, if one does: its number, alternate setting and subclass, and
/// its bulk-in and bulk-out endpoints.
pub(super) fn interface(interfaces: &[Interface]) -> Option<(u8, u8, u8, Endpoint, Endpoint)> {
    // (Subclass 6: SCSI's commands, as nearly every drive has; 2 and 5, old CD and floppy drives' ATAPI and
    // SFF-8070i, are SCSI's commands 12 bytes long. Protocol 0x50: bulk-only.)
    interfaces.iter().filter(|i| i.class == 0x08 && matches!(i.subclass, 0x02 | 0x05 | 0x06) && i.protocol == 0x50).find_map(|i| {
        let input = i.endpoints.iter().find(|e| e.bulk_in())?;
        let output = i.endpoints.iter().find(|e| e.bulk_out())?;
        Some((i.number, i.alt, i.subclass, *input, *output))
    })
}

/// Runs `f` on drive `id` in `slot` of controller `hc`, with its endpoints' rings, if it's still plugged in.
fn with_drive<R>(hc: usize, slot: u8, id: u64, f: impl FnOnce(&mut Storage, &mut Vec<Option<Ring>>, Doorbell) -> R) -> Option<R> {
    with(hc, |c| {
        if !c.host.running {
            return None;
        }
        let db = c.host.doorbell();
        let d = c.devices.get_mut(slot as usize)?.as_mut()?;
        let (rings, role) = d.rings_and_role();
        match role {
            Role::Storage(s) if s.id == id => Some(f(s, rings, db)),
            _ => None,
        }
    })
}

/// Waits for the event of one of the transfers queued at `trbs`: its completion code, the bytes it didn't move, and
/// which it was. None if none came in `ms` milliseconds.
fn event(hc: usize, slot: u8, id: u64, trbs: &[u64], ms: u64) -> Result<Option<(u8, u32, usize)>, Fail> {
    let found = wait(hc, ms, |c| {
        let Some(Some(d)) = c.devices.get_mut(slot as usize) else { return Some(None) };
        let Role::Storage(s) = &mut d.role else { return Some(None) };
        if s.id != id {
            return Some(None);
        }
        let i = s.events.iter().position(|e| trbs.contains(&e.param))?;
        let e = s.events.remove(i);
        Some(Some((e.code(), e.residual(), trbs.iter().position(|&t| t == e.param).unwrap_or(0))))
    });
    match found {
        Some(Some(e)) => Ok(Some(e)),
        Some(None) => Err(Fail::Gone),
        None if with_drive(hc, slot, id, |_, _, _| ()).is_none() => Err(Fail::Gone),
        None => Ok(None),
    }
}

/// After the drive stalled an endpoint: the controller's side reset (what was queued on it skipped), then the
/// drive's (CLEAR_FEATURE ENDPOINT_HALT).
fn clear_halt(hc: usize, slot: u8, dci: u8, address: u8) {
    restart_endpoint(hc, slot, dci);
    let _ = control(hc, slot, Request { kind: 0x02, request: CLEAR_FEATURE, value: ENDPOINT_HALT, index: address as u16, length: 0 }, &[]);
}

/// When a command went wrong: the bulk-only transport's reset recovery (the drive reset, both endpoints' stalls
/// cleared), so the next command starts afresh.
fn reset(hc: usize, slot: u8, id: u64) {
    let Some((interface, in_dci, out_dci, in_address, out_address)) =
        with_drive(hc, slot, id, |s, _, _| (s.interface, s.in_dci, s.out_dci, s.in_address, s.out_address))
    else {
        return;
    };
    let _ = control(hc, slot, Request::to_class(0x21, MASS_STORAGE_RESET, 0, interface as u16), &[]);
    clear_halt(hc, slot, in_dci, in_address);
    clear_halt(hc, slot, out_dci, out_address);
}

/// One command for logical unit `lun`: its command block, then `len` bytes in (none: no data), then its status.
/// Returns how many bytes came in (in the data pages).
fn command(hc: usize, slot: u8, id: u64, lun: u8, cb: &[u8], len: usize, ms: u64) -> Result<usize, Fail> {
    let len = len.min(MOST);
    let sent = with_drive(hc, slot, id, |s, rings, db| {
        s.tag = s.tag.wrapping_add(1);
        s.events.clear();
        let mut w = [0u8; 31];
        w[0..4].copy_from_slice(&CBW_SIGNATURE.to_le_bytes());
        w[4..8].copy_from_slice(&s.tag.to_le_bytes());
        w[8..12].copy_from_slice(&(len as u32).to_le_bytes());
        w[12] = if len > 0 { 0x80 } else { 0 };
        w[13] = lun;
        let cb_len = if s.twelve { 12 } else { cb.len().min(16) };
        w[14] = cb_len as u8;
        w[15..15 + cb.len().min(16)].copy_from_slice(&cb[..cb.len().min(16)]);
        for (i, b) in w.iter().enumerate() {
            unsafe { write_volatile(s.page.at::<u8>(i), *b) };
        }
        let ring = rings.get_mut(s.out_dci as usize)?.as_mut()?;
        let at = ring.push(Trb::new(xhci::TRB_NORMAL, s.page.phys, 31, xhci::ON_COMPLETION));
        db.ring(slot, s.out_dci);
        Some((at, s.tag))
    });
    let (cbw, tag) = sent.flatten().ok_or(Fail::Gone)?;
    match event(hc, slot, id, &[cbw], ms)? {
        Some((code, _, _)) if xhci::ok(code) => {}
        other => {
            reset(hc, slot, id);
            return Err(Fail::Broken(match other {
                Some((code, _, _)) => format!("it didn't take a command (\"{}\")", xhci::code_name(code)),
                None => String::from("it didn't take a command"),
            }));
        }
    }
    let mut got = 0;
    if len > 0 {
        // A page a TRB, chained into one transfer.
        let version = with(hc, |c| c.host.version);
        let queued = with_drive(hc, slot, id, |s, rings, db| {
            let ring = rings.get_mut(s.in_dci as usize)?.as_mut()?;
            let pages = len.div_ceil(4096);
            let mut trbs = Vec::with_capacity(pages);
            for k in 0..pages {
                let n = (len - 4096 * k).min(4096);
                let last = k + 1 == pages;
                let flags = xhci::SHORT_OK | if last { xhci::ON_COMPLETION } else { xhci::CHAIN };
                // Its "TD size": the packets still to come in the transfer after this TRB's (xHCI 1.0 on), or the
                // KiB still to come from this TRB on (before).
                let to_come = if last {
                    0
                } else if version >= 0x100 {
                    len.div_ceil(s.in_packet) - (4096 * k + n) / s.in_packet
                } else {
                    (len - 4096 * k) >> 10
                };
                let status = n as u32 | (to_come.min(31) as u32) << 17;
                trbs.push(ring.push(Trb::new(xhci::TRB_NORMAL, s.data[k].phys, status, flags)));
            }
            db.ring(slot, s.in_dci);
            Some((trbs, s.in_dci, s.in_address))
        });
        let (trbs, in_dci, in_address) = queued.flatten().ok_or(Fail::Gone)?;
        match event(hc, slot, id, &trbs, ms)? {
            // All of it, or less (a short packet ends the transfer early: the bytes before its TRB, and those in it).
            Some((xhci::SUCCESS, residual, k)) => got = (4096 * k + (len - 4096 * k).min(4096)).saturating_sub(residual as usize),
            Some((xhci::SHORT_PACKET, residual, k)) => got = (4096 * k + (len - 4096 * k).min(4096)).saturating_sub(residual as usize),
            // Refused: the stall cleared, then the status read, as the transport says.
            Some((STALLED, _, _)) => clear_halt(hc, slot, in_dci, in_address),
            other => {
                restart_endpoint(hc, slot, in_dci);
                reset(hc, slot, id);
                return Err(Fail::Broken(match other {
                    Some((code, _, _)) => format!("its data didn't come (\"{}\")", xhci::code_name(code)),
                    None => String::from("its data didn't come"),
                }));
            }
        }
    }
    // The status wrapper (asked for again once, if the drive stalls the first time).
    for attempt in 0..2 {
        let queued = with_drive(hc, slot, id, |s, rings, db| {
            let ring = rings.get_mut(s.in_dci as usize)?.as_mut()?;
            let at = ring.push(Trb::new(xhci::TRB_NORMAL, s.page.phys + CSW_AT as u64, 13, xhci::ON_COMPLETION | xhci::SHORT_OK));
            db.ring(slot, s.in_dci);
            Some((at, s.in_dci, s.in_address))
        });
        let (at, in_dci, in_address) = queued.flatten().ok_or(Fail::Gone)?;
        match event(hc, slot, id, &[at], ms)? {
            Some((code, residual, _)) if xhci::ok(code) => {
                let csw = with_drive(hc, slot, id, |s, _, _| (0..13).map(|i| unsafe { read_volatile(s.page.at::<u8>(CSW_AT + i)) }).collect::<Vec<u8>>())
                    .ok_or(Fail::Gone)?;
                let fine = residual == 0
                    && u32::from_le_bytes(csw[0..4].try_into().unwrap()) == CSW_SIGNATURE
                    && u32::from_le_bytes(csw[4..8].try_into().unwrap()) == tag;
                return match (fine, csw[12]) {
                    (true, 0) => Ok(got),
                    (true, 1) => Err(Fail::Failed),
                    (true, _) => {
                        reset(hc, slot, id);
                        Err(Fail::Broken(String::from("it lost track of a command (a phase error)")))
                    }
                    (false, _) => {
                        reset(hc, slot, id);
                        Err(Fail::Broken(String::from("its answer to a command didn't make sense")))
                    }
                };
            }
            Some((STALLED, _, _)) if attempt == 0 => clear_halt(hc, slot, in_dci, in_address),
            other => {
                restart_endpoint(hc, slot, in_dci);
                reset(hc, slot, id);
                return Err(Fail::Broken(match other {
                    Some((code, _, _)) => format!("it didn't say how a command went (\"{}\")", xhci::code_name(code)),
                    None => String::from("it didn't say how a command went"),
                }));
            }
        }
    }
    Err(Fail::Broken(String::from("it didn't say how a command went")))
}

/// The completion code of a stall.
const STALLED: u8 = 6;

/// The bytes a command brought in, from the data pages.
fn data(hc: usize, slot: u8, id: u64, out: &mut [u8]) -> Result<(), Fail> {
    with_drive(hc, slot, id, |s, _, _| {
        fence(Ordering::Acquire);
        for (k, chunk) in out.chunks_mut(4096).enumerate() {
            unsafe { core::ptr::copy_nonoverlapping(s.data[k].at::<u8>(0), chunk.as_mut_ptr(), chunk.len()) };
        }
    })
    .ok_or(Fail::Gone)
}

/// A command that brings in `len` bytes, and those bytes.
fn ask(hc: usize, slot: u8, id: u64, lun: u8, cb: &[u8], len: usize, ms: u64) -> Result<Vec<u8>, Fail> {
    let got = command(hc, slot, id, lun, cb, len, ms)?;
    let mut out = vec![0u8; got];
    data(hc, slot, id, &mut out)?;
    Ok(out)
}

/// Why the last command failed, as the drive says: (sense key, additional sense code, its qualifier).
fn sense(hc: usize, slot: u8, id: u64, lun: u8) -> Option<(u8, u8, u8)> {
    let d = ask(hc, slot, id, lun, &[REQUEST_SENSE, 0, 0, 0, 18, 0], 18, QUESTION_MS).ok()?;
    (d.len() >= 14).then(|| (d[2] & 0x0F, d[12], d[13]))
}

/// What a sense key and code mean, for the log and for the model.
fn said(key: u8, asc: u8, ascq: u8) -> String {
    match (key, asc) {
        (NOT_READY, MEDIUM_NOT_PRESENT) => String::from("no medium is in it"),
        (NOT_READY, 0x04) => String::from("it isn't ready yet"),
        (UNIT_ATTENTION, MEDIUM_CHANGED) => String::from("its medium was changed"),
        (0x03, _) => format!("it couldn't read its medium there (medium error {asc:02X}h/{ascq:02X}h)"),
        (0x05, _) => format!("it refused the command (illegal request {asc:02X}h/{ascq:02X}h)"),
        _ => format!("sense key {key:X}h, {asc:02X}h/{ascq:02X}h"),
    }
}

/// Text from INQUIRY's answer: printable ASCII, the spaces it's padded with taken off.
fn text(b: &[u8]) -> String {
    let s: String = b.iter().map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { ' ' }).collect();
    String::from(s.trim())
}

/// Sets up a USB drive (its configuration is set already): its endpoints, then what each logical unit is. Returns
/// what it is, for the log. Whether a medium is in each is asked in `poll`, from the main loop.
pub(super) fn set_up(hc: usize, slot: u8, speed: Speed, interfaces: &[Interface], serial: Option<String>) -> Result<String, Error> {
    let (number, alt, subclass, bulk_in, bulk_out) = interface(interfaces).ok_or(Error::Bad("no mass storage interface"))?;
    add_endpoints(hc, slot, speed, &[bulk_in, bulk_out])?;
    if alt != 0 {
        control(hc, slot, Request::to_class(0x01, SET_INTERFACE, alt as u16, number as u16), &[])?;
    }
    // (A drive with one logical unit may refuse the question.)
    let get_max_lun = Request { kind: 0xA1, request: GET_MAX_LUN, value: 0, index: number as u16, length: 1 };
    let max_lun = control(hc, slot, get_max_lun, &[]).ok().and_then(|d| d.first().copied()).unwrap_or(0).min(15);
    let low = with(hc, |c| c.devices.get(slot as usize).and_then(|d| d.as_ref()).map(|d| d.low)).ok_or(Error::Gone)?;
    let page = Page::new(low).ok_or(Error::Bad("out of memory"))?;
    let data: Option<Vec<Page>> = (0..DATA_PAGES).map(|_| Page::new(low)).collect();
    let data = data.ok_or(Error::Bad("out of memory"))?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let drive = Storage {
        id,
        interface: number,
        in_dci: bulk_in.dci(),
        out_dci: bulk_out.dci(),
        in_address: bulk_in.address,
        out_address: bulk_out.address,
        in_packet: (bulk_in.max_packet & 0x7FF).max(8) as usize,
        twelve: subclass != 0x06,
        tag: 0,
        page,
        data,
        events: Vec::new(),
        luns: Vec::new(),
        serial,
        polled: None,
    };
    with(hc, |c| {
        if let Some(Some(d)) = c.devices.get_mut(slot as usize) {
            d.role = Role::Storage(alloc::boxed::Box::new(drive));
        }
    });
    // What each logical unit is.
    let mut luns = Vec::new();
    let mut said_of: Vec<String> = Vec::new();
    for number in 0..=max_lun {
        match ask(hc, slot, id, number, &[INQUIRY, 0, 0, 0, 36, 0], 36, QUESTION_MS) {
            Ok(d) if d.len() >= 32 && d[0] >> 5 == 0 => {
                let (kind, removable) = (d[0] & 0x1F, d[1] & 0x80 != 0);
                let name = [text(&d[8..16]), text(&d[16..32])].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<String>>().join(" ");
                let what = match kind {
                    0 | 0x0E => if removable { "removable" } else { "fixed" },
                    5 => "a CD or DVD drive, not read",
                    _ => "not a disk, not read",
                };
                said_of.push(format!("LUN {number} \"{name}\" ({what})"));
                luns.push(Lun { number, kind, removable, name, medium: None, generation: 0 });
            }
            Ok(_) => said_of.push(format!("LUN {number}: nothing there")),
            Err(Fail::Gone) => return Err(Error::Gone),
            Err(Fail::Failed) | Err(Fail::Broken(_)) => said_of.push(format!("LUN {number}: didn't say what it is")),
        }
    }
    with_drive(hc, slot, id, |s, _, _| s.luns = luns).ok_or(Error::Gone)?;
    changed();
    Ok(format!("a USB drive ({})", said_of.join("; ")))
}

/// From the main loop: each USB drive's logical units asked, about once a second, whether a medium is in them, and
/// a medium found put to use (its size read) or forgotten.
pub(super) fn poll(hc: usize) {
    let now = time::uptime_ms();
    let due: Vec<(u8, u64, Vec<(usize, bool)>)> = with(hc, |c| {
        let mut due = Vec::new();
        for d in c.devices.iter_mut().flatten() {
            let slot = d.slot;
            if let Role::Storage(s) = &mut d.role
                && s.polled.is_none_or(|t| now.saturating_sub(t) >= POLL_MS)
            {
                s.polled = Some(now);
                // Disks only; a fixed one whose medium is known isn't asked again.
                let luns = s.luns.iter().enumerate().filter(|(_, l)| matches!(l.kind, 0 | 0x0E) && (l.removable || l.medium.is_none()));
                due.push((slot, s.id, luns.map(|(i, l)| (i, l.medium.is_some())).collect()));
            }
        }
        due
    });
    for (slot, id, luns) in due {
        for (i, had) in luns {
            check(hc, slot, id, i, had);
        }
    }
}

/// Asks logical unit `i` whether a medium is in it; if one came, reads its size.
fn check(hc: usize, slot: u8, id: u64, i: usize, mut had: bool) {
    let Some((lun, name)) = with_drive(hc, slot, id, |s, _, _| s.luns.get(i).map(|l| (l.number, l.name.clone()))).flatten() else { return };
    // Not ready: no medium, or one not ready yet; or a "unit attention", which the drive says once after it's reset or
    // its medium changed (a card taken out and another put in between two looks), and then it's asked again. The
    // drive's answer to REQUEST SENSE ends the condition. One that waits to be told to start (a hard drive in a USB
    // case, some) is told (START STOP UNIT, start).
    let mut ready = false;
    let mut changed_medium = false;
    for attempt in 0..2 {
        match command(hc, slot, id, lun, &[TEST_UNIT_READY, 0, 0, 0, 0, 0], 0, QUESTION_MS) {
            Ok(_) => {
                ready = true;
                break;
            }
            Err(Fail::Failed) => match sense(hc, slot, id, lun) {
                Some((UNIT_ATTENTION, asc, _)) if attempt == 0 => changed_medium |= asc == MEDIUM_CHANGED,
                Some((NOT_READY, 0x04, 0x02)) => {
                    let _ = command(hc, slot, id, lun, &[START_STOP_UNIT, 0, 0, 0, 1, 0], 0, COMMAND_MS);
                    break;
                }
                _ => break,
            },
            Err(Fail::Gone) => return,
            Err(Fail::Broken(why)) => {
                crate::log!("Chat Computer kernel: USB drive \"{name}\": couldn't ask whether a medium is in it: {why}");
                return;
            }
        }
    }
    if had && (!ready || changed_medium) {
        set_medium(hc, slot, id, i, None);
        crate::log!("Chat Computer kernel: USB drive \"{name}\": its medium was taken out");
        had = false;
    }
    if !ready || had {
        return;
    }
    match capacity(hc, slot, id, lun) {
        Ok((blocks, block_size)) => {
            set_medium(hc, slot, id, i, Some((blocks, block_size)));
            crate::log!(
                "Chat Computer kernel: USB drive \"{name}\": a medium is in it, {blocks} blocks of {block_size} bytes ({} MB)",
                blocks * block_size as u64 / 1_000_000
            );
        }
        Err(why) => crate::log!("Chat Computer kernel: USB drive \"{name}\": a medium is in it, but its size couldn't be read: {why}"),
    }
}

fn set_medium(hc: usize, slot: u8, id: u64, i: usize, size: Option<(u64, u32)>) {
    with_drive(hc, slot, id, |s, _, _| {
        if let Some(l) = s.luns.get_mut(i) {
            if size.is_some() {
                l.generation += 1;
            }
            l.medium = size.map(|(blocks, block_size)| Medium { blocks, block_size, generation: l.generation });
        }
    });
    changed();
}

/// A medium's blocks and their size (READ CAPACITY: its 16-byte form for a drive of more than 2^32 blocks).
fn capacity(hc: usize, slot: u8, id: u64, lun: u8) -> Result<(u64, u32), String> {
    let why = |f: Fail| match f {
        Fail::Gone => String::from("it was unplugged"),
        Fail::Failed => sense(hc, slot, id, lun).map_or_else(|| String::from("it refused"), |(k, a, q)| said(k, a, q)),
        Fail::Broken(why) => why,
    };
    let d = ask(hc, slot, id, lun, &[READ_CAPACITY_10, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, QUESTION_MS).map_err(why)?;
    if d.len() < 8 {
        return Err(String::from("its answer was too short"));
    }
    let (mut last, mut size) = (u32::from_be_bytes(d[0..4].try_into().unwrap()) as u64, u32::from_be_bytes(d[4..8].try_into().unwrap()));
    if last == 0xFFFF_FFFF {
        let mut cb = [0u8; 16];
        cb[0] = READ_CAPACITY_16;
        cb[1] = 0x10;
        cb[10..14].copy_from_slice(&32u32.to_be_bytes());
        let d = ask(hc, slot, id, lun, &cb, 32, QUESTION_MS).map_err(why)?;
        if d.len() < 12 {
            return Err(String::from("its answer was too short"));
        }
        last = u64::from_be_bytes(d[0..8].try_into().unwrap());
        size = u32::from_be_bytes(d[8..12].try_into().unwrap());
    }
    if !matches!(size, 512 | 1024 | 2048 | 4096) || last == u64::MAX {
        return Err(format!("its blocks are {size} bytes, which Chat Computer's own system doesn't read"));
    }
    Ok((last + 1, size))
}

/// A USB drive's logical unit with a medium in it, as storage.rs knows it.
#[derive(Clone)]
pub struct Unit {
    pub hc: usize,
    pub slot: u8,
    pub id: u64,
    pub lun: u8,
    pub medium: Medium,
    pub name: String,
    pub serial: Option<String>,
}

/// Every USB drive's logical units with a medium in them.
pub fn units() -> Vec<Unit> {
    let controllers = super::CONTROLLERS.lock();
    let mut out = Vec::new();
    for (hc, c) in controllers.iter().enumerate().filter(|(_, c)| c.host.running) {
        for d in c.devices.iter().flatten() {
            let Role::Storage(s) = &d.role else { continue };
            for l in &s.luns {
                if let Some(medium) = l.medium {
                    let name = if l.name.is_empty() { d.name.clone() } else { l.name.clone() };
                    out.push(Unit { hc, slot: d.slot, id: s.id, lun: l.number, medium, name, serial: s.serial.clone() });
                }
            }
        }
    }
    out
}

/// Reads `out.len()` bytes (whole blocks, `MOST` at most) from block `lba` of a unit's medium, if it's still that
/// medium.
pub fn read(u: &Unit, lba: u64, out: &mut [u8]) -> Result<(), String> {
    let bs = u.medium.block_size as usize;
    if out.is_empty() || out.len() % bs != 0 || out.len() > MOST || lba.checked_add((out.len() / bs) as u64).is_none_or(|end| end > u.medium.blocks) {
        return Err(format!("a read of {} bytes from block {lba} isn't one the drive can do", out.len()));
    }
    let blocks = (out.len() / bs) as u64;
    let cb: Vec<u8> = if lba + blocks <= 0xFFFF_FFFF {
        let mut cb = vec![READ_10, 0];
        cb.extend_from_slice(&(lba as u32).to_be_bytes());
        cb.extend_from_slice(&[0]);
        cb.extend_from_slice(&(blocks as u16).to_be_bytes());
        cb.push(0);
        cb
    } else {
        let mut cb = vec![READ_16, 0];
        cb.extend_from_slice(&lba.to_be_bytes());
        cb.extend_from_slice(&(blocks as u32).to_be_bytes());
        cb.extend_from_slice(&[0, 0]);
        cb
    };
    for attempt in 0..2 {
        let same = with_drive(u.hc, u.slot, u.id, |s, _, _| s.luns.iter().any(|l| l.number == u.lun && l.medium == Some(u.medium)));
        match same {
            None => return Err(String::from("the USB drive was unplugged")),
            Some(false) => return Err(String::from("the USB drive's medium was taken out")),
            Some(true) => {}
        }
        match command(u.hc, u.slot, u.id, u.lun, &cb, out.len(), COMMAND_MS) {
            Ok(got) if got == out.len() => return data(u.hc, u.slot, u.id, out).map_err(|_| String::from("the USB drive was unplugged")),
            Ok(got) => return Err(format!("the USB drive gave {got} of {} bytes", out.len())),
            Err(Fail::Gone) => return Err(String::from("the USB drive was unplugged")),
            Err(Fail::Failed) => match sense(u.hc, u.slot, u.id, u.lun) {
                // (Reset: asked again. Its medium changed, or gone: not this medium any more.)
                Some((UNIT_ATTENTION, asc, _)) if asc != MEDIUM_CHANGED && attempt == 0 => continue,
                Some((UNIT_ATTENTION, MEDIUM_CHANGED, _) | (NOT_READY, MEDIUM_NOT_PRESENT, _)) => {
                    if let Some(i) = with_drive(u.hc, u.slot, u.id, |s, _, _| s.luns.iter().position(|l| l.number == u.lun)).flatten() {
                        set_medium(u.hc, u.slot, u.id, i, None);
                    }
                    return Err(String::from("the USB drive's medium was taken out"));
                }
                Some((k, a, q)) => return Err(format!("the USB drive couldn't read block {lba}: {}", said(k, a, q))),
                None => return Err(format!("the USB drive couldn't read block {lba}")),
            },
            Err(Fail::Broken(why)) if attempt == 0 => crate::log!("Chat Computer kernel: USB drive \"{}\": {why}; trying again", u.name),
            Err(Fail::Broken(why)) => return Err(format!("the USB drive didn't read: {why}")),
        }
    }
    Err(String::from("the USB drive didn't read"))
}
