//! DHCP (RFC 2131): asking the network for an address. Discover, take the first offer, request it, keep it:
//! renewed with the server that gave it halfway through the lease, with any server after seven-eighths, and
//! given up when it ends. Lost answers are asked for again after 4, 8, 16, 32 and then every 64 seconds.

use crate::{be32, put16, put32, Config, Ipv4, Mac, Random};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub(crate) const SERVER_PORT: u16 = 67;
pub(crate) const CLIENT_PORT: u16 = 68;

const DISCOVER: u8 = 1;
const OFFER: u8 = 2;
const REQUEST: u8 = 3;
const ACK: u8 = 5;
const NAK: u8 = 6;

const MAGIC: [u8; 4] = [99, 130, 83, 99];
const NAME: &[u8] = b"chatcomputer";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Off,
    Selecting,
    Requesting,
    Bound,
    Renewing,
    Rebinding,
}

struct Lease {
    config: Config,
    server: Ipv4,
    t1: u64,
    t2: u64,
    end: u64,
}

pub(crate) struct Client {
    phase: Phase,
    xid: u32,
    began: u64,
    next: u64,
    tries: u32,
    offer: Option<(Ipv4, Ipv4)>,
    lease: Option<Lease>,
    pub(crate) domain: Option<String>,
}

pub(crate) struct Message {
    /// The server, for a renewal; else everyone.
    pub to: Option<Ipv4>,
    pub data: Vec<u8>,
}

pub(crate) enum Outcome {
    Nothing,
    /// Something to send straight away.
    Send,
    Bound(Config),
    Lost,
}

/// What was in an answer.
#[derive(Default)]
struct Answer {
    kind: u8,
    yiaddr: Ipv4,
    mask: Option<Ipv4>,
    router: Option<Ipv4>,
    dns: Vec<Ipv4>,
    domain: Option<String>,
    lease: Option<u32>,
    server: Option<Ipv4>,
    t1: Option<u32>,
    t2: Option<u32>,
}

fn ip_at(d: &[u8], at: usize) -> Ipv4 {
    Ipv4([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

fn parse(d: &[u8]) -> Option<Answer> {
    if d.len() < 240 || d[0] != 2 || d[236..240] != MAGIC {
        return None;
    }
    let mut a = Answer { yiaddr: ip_at(d, 16), ..Answer::default() };
    let mut at = 240;
    while at < d.len() {
        let code = d[at];
        if code == 0 {
            at += 1;
            continue;
        }
        if code == 255 || at + 1 >= d.len() {
            break;
        }
        let len = d[at + 1] as usize;
        let v = d.get(at + 2..at + 2 + len)?;
        match (code, len) {
            (53, 1) => a.kind = v[0],
            (1, 4) => a.mask = Some(ip_at(v, 0)),
            (3, l) if l >= 4 => a.router = Some(ip_at(v, 0)),
            (6, l) if l >= 4 => a.dns = v.chunks_exact(4).map(|c| ip_at(c, 0)).filter(|ip| *ip != Ipv4::ANY).collect(),
            (15, l) if l > 0 => {
                a.domain = core::str::from_utf8(v).ok().map(|s| String::from(s.trim_end_matches('\0')));
            }
            (51, 4) => a.lease = Some(be32(v, 0)),
            (54, 4) => a.server = Some(ip_at(v, 0)),
            (58, 4) => a.t1 = Some(be32(v, 0)),
            (59, 4) => a.t2 = Some(be32(v, 0)),
            _ => {}
        }
        at += 2 + len;
    }
    (a.kind != 0).then_some(a)
}

/// The mask's length (255.255.255.0: 24); a mask that isn't one run of ones: none.
fn prefix(mask: Ipv4) -> Option<u8> {
    let m = mask.u32();
    let ones = m.leading_ones();
    (m.checked_shl(ones).unwrap_or(0) == 0).then_some(ones as u8)
}

impl Client {
    pub(crate) fn new(on: bool, random: &mut Random) -> Client {
        let mut c = Client {
            phase: Phase::Off,
            xid: 0,
            began: 0,
            next: 0,
            tries: 0,
            offer: None,
            lease: None,
            domain: None,
        };
        if on {
            c.restart(0, random);
        }
        c
    }

    fn restart(&mut self, now: u64, random: &mut Random) {
        self.phase = Phase::Selecting;
        self.xid = random.next() as u32;
        self.began = now;
        self.next = now;
        self.tries = 0;
        self.offer = None;
        self.lease = None;
    }

    pub(crate) fn stop(&mut self) {
        self.phase = Phase::Off;
        self.lease = None;
    }

    pub(crate) fn busy(&self) -> bool {
        matches!(self.phase, Phase::Selecting | Phase::Requesting)
    }

    pub(crate) fn describe(&self) -> &'static str {
        match self.phase {
            Phase::Off => "off",
            Phase::Selecting => "looking for a network address",
            Phase::Requesting => "asking for the address offered",
            Phase::Bound => "has an address",
            Phase::Renewing => "renewing its address",
            Phase::Rebinding => "renewing its address with any server",
        }
    }

    /// Waits 4, 8, 16, 32, then 64 seconds, each give or take one.
    fn backoff(&mut self, now: u64, random: &mut Random) {
        let base = 4000u64 << self.tries.min(4);
        self.next = now + base - 1000 + random.next() % 2000;
        self.tries += 1;
    }

    fn message(&self, kind: u8, mac: Mac, now: u64, ciaddr: Ipv4, requested: Option<Ipv4>, server: Option<Ipv4>) -> Vec<u8> {
        let mut m = vec![0u8; 240];
        m[0] = 1; // a request
        m[1] = 1; // Ethernet
        m[2] = 6;
        put32(&mut m, 4, self.xid);
        put16(&mut m, 8, (now.saturating_sub(self.began) / 1000).min(0xFFFF) as u16);
        // Until there's an address, answers can only reach this computer broadcast.
        put16(&mut m, 10, if ciaddr == Ipv4::ANY { 0x8000 } else { 0 });
        m[12..16].copy_from_slice(&ciaddr.0);
        m[28..34].copy_from_slice(&mac.0);
        m[236..240].copy_from_slice(&MAGIC);
        m.extend_from_slice(&[53, 1, kind]);
        m.extend_from_slice(&[61, 7, 1]);
        m.extend_from_slice(&mac.0);
        if let Some(ip) = requested {
            m.extend_from_slice(&[50, 4]);
            m.extend_from_slice(&ip.0);
        }
        if let Some(ip) = server {
            m.extend_from_slice(&[54, 4]);
            m.extend_from_slice(&ip.0);
        }
        m.extend_from_slice(&[12, NAME.len() as u8]);
        m.extend_from_slice(NAME);
        m.extend_from_slice(&[57, 2, 0x05, 0xDC]); // answers up to 1500 bytes
        m.extend_from_slice(&[55, 8, 1, 3, 6, 15, 51, 54, 58, 59]);
        m.push(255);
        if m.len() < 300 {
            m.resize(300, 0); // (some servers want at least BOOTP's size)
        }
        m
    }

    /// What's due now. Also says whether the address was lost (its lease ended).
    pub(crate) fn poll(&mut self, now: u64, mac: Mac, config: Option<&Config>, random: &mut Random) -> (Vec<Message>, bool) {
        let mut out = Vec::new();
        let mut lost = false;
        if let Some(end) = self.lease.as_ref().map(|l| l.end)
            && now >= end
        {
            lost = true;
            self.restart(now, random);
        }
        let ours = config.map_or(Ipv4::ANY, |c| c.address);
        match self.phase {
            Phase::Off => {}
            Phase::Selecting => {
                if now >= self.next {
                    out.push(Message { to: None, data: self.message(DISCOVER, mac, now, Ipv4::ANY, None, None) });
                    self.backoff(now, random);
                }
            }
            Phase::Requesting => {
                if now >= self.next {
                    if self.tries >= 4 {
                        self.restart(now, random);
                        out.push(Message { to: None, data: self.message(DISCOVER, mac, now, Ipv4::ANY, None, None) });
                        self.backoff(now, random);
                    } else if let Some((address, server)) = self.offer {
                        out.push(Message {
                            to: None,
                            data: self.message(REQUEST, mac, now, Ipv4::ANY, Some(address), Some(server)),
                        });
                        self.backoff(now, random);
                    }
                }
            }
            Phase::Bound | Phase::Renewing | Phase::Rebinding => {
                let Some(l) = self.lease.as_ref() else { return (out, lost) };
                let (t1, t2, end, server) = (l.t1, l.t2, l.end, l.server);
                if self.phase == Phase::Bound && now >= t1 {
                    self.phase = Phase::Renewing;
                    self.xid = random.next() as u32;
                    self.began = now;
                    self.next = now;
                }
                if self.phase == Phase::Renewing && now >= t2 {
                    self.phase = Phase::Rebinding;
                    self.next = now;
                }
                if self.phase != Phase::Bound && now >= self.next && ours != Ipv4::ANY {
                    let to = if self.phase == Phase::Renewing { Some(server) } else { None };
                    out.push(Message { to, data: self.message(REQUEST, mac, now, ours, None, None) });
                    let until = if self.phase == Phase::Renewing { t2 } else { end };
                    self.next = now + (until.saturating_sub(now) / 2).max(60_000);
                }
            }
        }
        (out, lost)
    }

    /// An answer from a server.
    pub(crate) fn answer(&mut self, now: u64, mac: Mac, d: &[u8]) -> Outcome {
        let Some(a) = parse(d) else { return Outcome::Nothing };
        if be32(d, 4) != self.xid || d[28..34] != mac.0 {
            return Outcome::Nothing;
        }
        match (self.phase, a.kind) {
            (Phase::Selecting, OFFER) => {
                let (Some(server), true) = (a.server, a.yiaddr != Ipv4::ANY) else { return Outcome::Nothing };
                self.offer = Some((a.yiaddr, server));
                self.phase = Phase::Requesting;
                self.tries = 0;
                self.next = now;
                Outcome::Send
            }
            (Phase::Requesting | Phase::Renewing | Phase::Rebinding, ACK) => {
                let address = if a.yiaddr != Ipv4::ANY {
                    a.yiaddr
                } else if let Some(l) = &self.lease {
                    l.config.address
                } else {
                    return Outcome::Nothing;
                };
                let prefix = a.mask.and_then(prefix).unwrap_or(24);
                let config = Config { address, prefix, gateway: a.router, dns: a.dns };
                let secs = a.lease.unwrap_or(3600) as u64;
                let secs = if secs == 0xFFFF_FFFF { 100 * 365 * 86400 } else { secs.max(60) };
                let t1 = a.t1.map_or(secs / 2, |t| t as u64).min(secs);
                let t2 = a.t2.map_or(secs * 7 / 8, |t| t as u64).clamp(t1, secs);
                let server = a.server.or(self.offer.map(|o| o.1)).or(self.lease.as_ref().map(|l| l.server)).unwrap_or(Ipv4::BROADCAST);
                self.lease = Some(Lease { config: config.clone(), server, t1: now + t1 * 1000, t2: now + t2 * 1000, end: now + secs * 1000 });
                self.phase = Phase::Bound;
                self.tries = 0;
                if a.domain.is_some() {
                    self.domain = a.domain;
                }
                Outcome::Bound(config)
            }
            (Phase::Requesting | Phase::Renewing | Phase::Rebinding, NAK) => {
                let xid = self.xid;
                self.phase = Phase::Selecting;
                self.xid = xid.wrapping_add(0x9E37_79B9);
                self.began = now;
                self.next = now;
                self.tries = 0;
                self.offer = None;
                self.lease = None;
                Outcome::Lost
            }
            _ => Outcome::Nothing,
        }
    }
}
