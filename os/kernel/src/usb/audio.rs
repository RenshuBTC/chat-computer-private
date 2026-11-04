//! USB audio: a headset, speakers or a sound card (a DAC) on USB, of USB's Audio Device Class 1, which most of them
//! are: what Chat Computer plays its chimes on while one is plugged in, as a computer switches to one.
//!
//! The device says what it takes in its descriptors: an audio streaming interface whose alternate settings are each a
//! format (PCM's: how many channels, how many bytes a sample) with its sample rates, and an isochronous OUT endpoint,
//! which takes a packet of sound in each of its service intervals (a millisecond at full speed). Chat Computer picks
//! the setting (16 bits, two channels or one, 48 kHz if it has it, else 44.1), sets its rate and unmutes it, and
//! sends a sound's packets on the endpoint's own ring, a few hundred milliseconds ahead, one more each time one has
//! gone out. A sound's packets are packed into pages so none crosses one (a packet must be in one piece of memory).

use super::device::{control, Endpoint, Error, Interface, Request, SET_INTERFACE};
use super::xhci::{self, Doorbell, Page, Ring, Trb};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use bootinfo::PAGE_SIZE;
use core::ptr::write_volatile;

/// The audio class, and its subclasses: control and streaming.
const AUDIO: u8 = 1;
const STREAMING: u8 = 2;
const CONTROL: u8 = 1;
/// Class 2's interfaces say so in their protocol (its descriptors differ: not used yet).
const CLASS_2: u8 = 0x20;
// Class-specific interface descriptors' subtypes: a streaming interface's general one and its format; a control
// interface's feature units (mute and volume).
const AS_GENERAL: u8 = 1;
const FORMAT_TYPE: u8 = 2;
const FEATURE_UNIT: u8 = 6;
const PCM: u16 = 1;
const SET_CUR: u8 = 0x01;
const MUTE: u16 = 0x01;
const SAMPLING_FREQUENCY: u16 = 0x01;
/// Packets queued ahead of the one going out.
const AHEAD: usize = 150;
// Completion codes for an isochronous ring that ran dry or overfull: no packet of it finished.
const RING_UNDERRUN: u8 = 14;
const RING_OVERRUN: u8 = 15;

/// How a USB audio output is used: its streaming interface's setting, its endpoint, its format, and what's playing.
pub struct Audio {
    interface: u8,
    alt: u8,
    endpoint: Endpoint,
    pub channels: u8,
    pub rate: u32,
    /// Microseconds between packets.
    period_us: u32,
    playing: Option<Playing>,
    /// Sounds stopped while their packets were still going out, oldest first, each with how many still are: kept
    /// until their last has (the controller reads them).
    stopping: Vec<(Playing, usize)>,
}

/// A sound being sent: its packets (where each is, how long), the next to queue, and how many are queued.
struct Playing {
    #[allow(dead_code)]
    pages: Vec<Page>,
    packets: Vec<(u64, u32)>,
    next: usize,
    in_flight: usize,
}

/// A streaming interface's setting Chat Computer can play on: its number and alternate setting, its endpoint, its
/// channels, and the rate to use.
pub(super) struct Plan {
    interface: u8,
    alt: u8,
    endpoint: Endpoint,
    channels: u8,
    rate: u32,
    control: Option<(u8, Vec<u8>)>,
}

/// The rate to use of a format's: 48 kHz if it has it, else 44.1, else its first (a range: 48 kHz if it's in it).
fn rate_of(format: &[u8]) -> Option<u32> {
    let rate = |at: usize| format.get(at..at + 3).map(|b| u32::from_le_bytes([b[0], b[1], b[2], 0]));
    let kinds = *format.get(7)? as usize;
    if kinds == 0 {
        let (low, high) = (rate(8)?, rate(11)?);
        return [48_000, 44_100].into_iter().find(|r| (low..=high).contains(r));
    }
    let rates: Vec<u32> = (0..kinds).filter_map(|k| rate(8 + 3 * k)).collect();
    [48_000, 44_100].into_iter().find(|r| rates.contains(r)).or_else(|| rates.first().copied())
}

/// The setting to play on, of a device's interfaces (all their alternate settings): a class 1 streaming interface
/// whose setting takes PCM in 16 bits, two channels (else one), by an isochronous OUT endpoint; with the feature
/// units of its control interface, to unmute.
pub(super) fn plan(interfaces: &[Interface]) -> Option<Plan> {
    let mut best: Option<Plan> = None;
    for i in interfaces.iter().filter(|i| i.class == AUDIO && i.subclass == STREAMING && i.protocol != CLASS_2) {
        let Some(&endpoint) = i.endpoints.iter().find(|e| e.attributes & 3 == 1 && e.address & 0x80 == 0) else { continue };
        let general = i.extra.iter().find(|d| d.len() >= 7 && d[2] == AS_GENERAL);
        if general.is_some_and(|d| u16::from_le_bytes([d[5], d[6]]) != PCM) {
            continue;
        }
        let Some(format) = i.extra.iter().find(|d| d.len() >= 8 && d[2] == FORMAT_TYPE && d[3] == 1) else { continue };
        let (channels, bytes) = (format[4], format[5]);
        if bytes != 2 || !(1..=2).contains(&channels) {
            continue;
        }
        let Some(rate) = rate_of(format) else { continue };
        if best.as_ref().is_some_and(|b| b.channels >= channels) {
            continue;
        }
        best = Some(Plan { interface: i.number, alt: i.alt, endpoint, channels, rate, control: None });
    }
    let mut plan = best?;
    // The control interface's feature units (their descriptors whole), to unmute.
    plan.control = interfaces
        .iter()
        .find(|i| i.class == AUDIO && i.subclass == CONTROL && i.alt == 0)
        .map(|i| (i.number, i.extra.iter().filter(|d| d.len() >= 7 && d[2] == FEATURE_UNIT).flatten().copied().collect()));
    Some(plan)
}

/// Sets the device up to play as `plan` says: its setting chosen, its rate set, unmuted, its endpoint added. What it
/// is, for the log, and the audio output to keep.
pub(super) fn set_up(hc: usize, slot: u8, speed: super::Speed, plan: Plan) -> Result<(String, Audio), Error> {
    control(hc, slot, Request::to_class(0x01, SET_INTERFACE, plan.alt as u16, plan.interface as u16), &[])?;
    // Its rate (an endpoint that has only the one may not take this: that's no matter).
    let rate = plan.rate.to_le_bytes();
    let set_rate = Request { kind: 0x22, request: SET_CUR, value: SAMPLING_FREQUENCY << 8, index: plan.endpoint.address as u16, length: 3 };
    let _ = control(hc, slot, set_rate, &rate[..3]);
    // Every feature unit's mute off, for the master channel and the first two (where it has a mute control).
    if let Some((number, units)) = &plan.control {
        let mut at = 0;
        while at + 7 <= units.len() {
            let len = units[at] as usize;
            if len < 7 || at + len > units.len() {
                break;
            }
            let d = &units[at..at + len];
            let (unit, size) = (d[3], d[5] as usize);
            for channel in 0..3usize {
                let controls = d.get(6 + channel * size).copied().unwrap_or(0);
                if size > 0 && controls & 1 != 0 {
                    let r = Request { kind: 0x21, request: SET_CUR, value: MUTE << 8 | channel as u16, index: (unit as u16) << 8 | *number as u16, length: 1 };
                    let _ = control(hc, slot, r, &[0]);
                }
            }
            at += len;
        }
    }
    super::device::add_endpoints(hc, slot, speed, &[plan.endpoint])?;
    // Its service interval: 2^(bInterval - 1) frames (milliseconds) at full speed, microframes (125 µs) faster.
    let unit = if matches!(speed, super::Speed::Low | super::Speed::Full) { 1000 } else { 125 };
    let period_us = unit << (plan.endpoint.interval.clamp(1, 16) - 1);
    let what = format!(
        "a USB sound card ({} channel{}, 16 bits, {} kHz)",
        plan.channels,
        if plan.channels == 1 { "" } else { "s" },
        plan.rate as f64 / 1000.0
    );
    Ok((what, Audio {
        interface: plan.interface,
        alt: plan.alt,
        endpoint: plan.endpoint,
        channels: plan.channels,
        rate: plan.rate,
        period_us,
        playing: None,
        stopping: Vec::new(),
    }))
}

impl Audio {
    /// The endpoint's device context index.
    pub fn dci(&self) -> u8 {
        self.endpoint.dci()
    }

    /// For the log: its interface and setting.
    pub fn describe(&self) -> String {
        format!("interface {} setting {}, endpoint {:#x}, a packet every {} µs", self.interface, self.alt, self.endpoint.address, self.period_us)
    }

    /// Plays `samples` (one channel, 16 bits, at its rate) from their start: what was playing stops (its packets
    /// already queued go out first). False when memory runs out.
    pub fn play(&mut self, rings: &mut [Option<Ring>], db: Doorbell, slot: u8, samples: &[i16], low: bool) -> bool {
        let Some(sound) = packets(samples, self.channels, self.rate, self.period_us, self.endpoint.max_packet & 0x7FF, low) else {
            return false;
        };
        self.stop();
        self.playing = Some(sound);
        self.queue(rings, db, slot);
        true
    }

    /// Stops playing: what's queued goes out, nothing more.
    pub fn stop(&mut self) {
        if let Some(mut p) = self.playing.take()
            && p.in_flight > 0
        {
            p.next = p.packets.len();
            let left = p.in_flight;
            self.stopping.push((p, left));
        }
    }

    /// Queues packets until AHEAD are going out, or the sound has none left.
    fn queue(&mut self, rings: &mut [Option<Ring>], db: Doorbell, slot: u8) {
        let dci = self.dci();
        let Some(Some(ring)) = rings.get_mut(dci as usize) else { return };
        let Some(p) = &mut self.playing else { return };
        let queued: usize = self.stopping.iter().map(|s| s.1).sum();
        let mut any = false;
        while p.next < p.packets.len() && p.in_flight + queued < AHEAD {
            let (at, len) = p.packets[p.next];
            // (Each a packet of its own, to go out as soon as it can after the one before.)
            ring.push(Trb::new(xhci::TRB_ISOCH, at, len, xhci::ON_COMPLETION | xhci::ISOCH_ASAP));
            p.next += 1;
            p.in_flight += 1;
            any = true;
        }
        if any {
            db.ring(slot, dci);
        }
    }

    /// One of its packets went out (or didn't, in time): the next queued. False if the endpoint stopped on an error.
    pub fn transfer_event(&mut self, rings: &mut [Option<Ring>], db: Doorbell, slot: u8, e: Trb) -> bool {
        let code = e.code();
        if code != RING_UNDERRUN && code != RING_OVERRUN {
            // Packets go out in order: stopped sounds' first.
            if let Some(first) = self.stopping.first_mut() {
                first.1 -= 1;
                if first.1 == 0 {
                    self.stopping.remove(0);
                }
            } else if let Some(p) = &mut self.playing {
                p.in_flight = p.in_flight.saturating_sub(1);
            }
        }
        if self.playing.as_ref().is_some_and(|p| p.next == p.packets.len() && p.in_flight == 0) {
            self.playing = None;
        }
        self.queue(rings, db, slot);
        // (Missing a service interval, or the ring running dry, isn't the endpoint stopping.)
        xhci::ok(code) || matches!(code, RING_UNDERRUN | RING_OVERRUN | 18)
    }
}

/// A sound's packets: each service interval's samples (the rate's share of a period, a frame more now and then where
/// it isn't a whole number), each sample in every channel, packed into pages (none across one).
fn packets(samples: &[i16], channels: u8, rate: u32, period_us: u32, max_packet: u16, low: bool) -> Option<Playing> {
    let frame = 2 * channels as usize;
    let mut sizes: Vec<usize> = Vec::new();
    let (mut done, mut owed) = (0usize, 0u64);
    while done < samples.len() {
        owed += rate as u64 * period_us as u64;
        let n = (owed / 1_000_000) as usize;
        owed %= 1_000_000;
        let n = n.min(samples.len() - done).min(max_packet as usize / frame).max(1);
        sizes.push(n);
        done += n;
    }
    let page = PAGE_SIZE as usize;
    let mut pages: Vec<Page> = Vec::new();
    let mut packets: Vec<(u64, u32)> = Vec::with_capacity(sizes.len());
    let (mut at, mut sample) = (page, 0usize);
    for n in sizes {
        let bytes = n * frame;
        if at + bytes > page {
            pages.push(Page::new(low)?);
            at = 0;
        }
        let p = pages.last()?;
        for k in 0..n {
            let v = samples[sample + k] as u16;
            for c in 0..channels as usize {
                unsafe { write_volatile(p.at::<u16>(at + k * frame + 2 * c), v) };
            }
        }
        packets.push((p.phys + at as u64, bytes as u32));
        at += bytes;
        sample += n;
    }
    // (A controller set not to snoop the caches reads memory itself.)
    for p in &pages {
        for line in (0..page).step_by(64) {
            crate::cpu::flush_line(p.at::<u8>(line) as u64);
        }
    }
    crate::cpu::memory_fence();
    Some(Playing { pages, packets, next: 0, in_flight: 0 })
}
