//! Chat Computer's own network stack, from Ethernet frames up to TCP connections: ARP, IPv4, ICMP (answering
//! pings), UDP, DHCP (an address from the network), DNS (names to addresses) and TCP (connections, with what
//! was lost sent again). No code from other projects.
//!
//! It knows nothing of network cards or time: the kernel's main loop hands it the frames that arrived
//! (`receive`), tells it the time now and then (`poll`), and takes the frames it wants sent (`transmit`) to the
//! card's driver. Nothing in it waits. Everything is bounds-checked: a malformed frame is dropped, never a
//! crash. IPv4 only for now; no IP fragments (Chat Computer never sends them and drops those it gets).
#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod dhcp;
mod dns;
pub mod tcp;
#[cfg(test)]
mod tests;

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

pub use dns::Error as DnsError;
pub use tcp::{Error as TcpError, State as TcpState};

/// An IPv4 address.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Ipv4(pub [u8; 4]);

impl Ipv4 {
    pub const ANY: Ipv4 = Ipv4([0; 4]);
    pub const BROADCAST: Ipv4 = Ipv4([255; 4]);

    pub fn u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    pub fn from_u32(v: u32) -> Ipv4 {
        Ipv4(v.to_be_bytes())
    }

    /// "10.0.2.15" as an address; None for anything else.
    pub fn parse(s: &str) -> Option<Ipv4> {
        let mut out = [0u8; 4];
        let mut parts = s.split('.');
        for b in out.iter_mut() {
            let p = parts.next()?;
            if p.is_empty() || p.len() > 3 || !p.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            *b = p.parse().ok()?;
        }
        parts.next().is_none().then_some(Ipv4(out))
    }
}

impl fmt::Display for Ipv4 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

impl fmt::Debug for Ipv4 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A network card's address.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Mac(pub [u8; 6]);

impl Mac {
    pub const BROADCAST: Mac = Mac([0xFF; 6]);

    fn is_group(self) -> bool {
        self.0[0] & 1 != 0
    }
}

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let m = self.0;
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5])
    }
}

impl fmt::Debug for Mac {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Where this computer is on the network: its address, how big the local network is, the way out to the rest
/// of the internet, and who answers DNS. From DHCP, or set by hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub address: Ipv4,
    /// The length of the network's mask: 24 for 255.255.255.0.
    pub prefix: u8,
    pub gateway: Option<Ipv4>,
    pub dns: Vec<Ipv4>,
}

impl Config {
    fn mask(&self) -> u32 {
        if self.prefix == 0 { 0 } else { !0u32 << (32 - self.prefix.min(32) as u32) }
    }

    /// Whether `ip` is on the local network (reached directly, not through the gateway).
    pub fn on_link(&self, ip: Ipv4) -> bool {
        (ip.u32() ^ self.address.u32()) & self.mask() == 0
    }

    pub fn broadcast(&self) -> Ipv4 {
        Ipv4::from_u32(self.address.u32() | !self.mask())
    }
}

/// Counts, for the log.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub received: u64,
    pub sent: u64,
    /// Frames thrown away: damaged, not for us, or for nothing we have open.
    pub dropped: u64,
}

pub(crate) fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

pub(crate) fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub(crate) fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_be_bytes());
}

pub(crate) fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_be_bytes());
}

/// The internet checksum (RFC 1071) over several pieces, as if they were one: 0 when checking something that
/// carries its own correct checksum.
pub(crate) fn checksum(pieces: &[&[u8]]) -> u16 {
    let mut sum: u32 = 0;
    let mut odd: Option<u8> = None;
    for piece in pieces {
        for &b in piece.iter() {
            match odd.take() {
                Some(hi) => sum += u16::from_be_bytes([hi, b]) as u32,
                None => odd = Some(b),
            }
            if sum > 0xFFFF_0000 {
                sum = (sum & 0xFFFF) + (sum >> 16);
            }
        }
    }
    if let Some(hi) = odd {
        sum += (hi as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// The checksum of a UDP or TCP packet with the pseudo-header in front of it.
pub(crate) fn transport_checksum(src: Ipv4, dst: Ipv4, protocol: u8, packet: &[u8]) -> u16 {
    let mut pseudo = [0u8; 12];
    pseudo[0..4].copy_from_slice(&src.0);
    pseudo[4..8].copy_from_slice(&dst.0);
    pseudo[9] = protocol;
    put16(&mut pseudo, 10, packet.len() as u16);
    checksum(&[&pseudo, packet])
}

/// Numbers that are hard to guess (ports, TCP's first sequence numbers, DNS and DHCP ids), from a seed the
/// kernel gives (xoshiro256**, reseeded by `Stack::mix`).
pub(crate) struct Random([u64; 4]);

impl Random {
    fn new(seed: u64) -> Random {
        let mut r = Random([0x9E37_79B9_7F4A_7C15, 0xBF58_476D_1CE4_E5B9, 0x94D0_49BB_1331_11EB, 0x2545_F491_4F6C_DD1D]);
        r.mix(seed);
        r
    }

    fn mix(&mut self, seed: u64) {
        let mut x = seed;
        for s in self.0.iter_mut() {
            // splitmix64
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            *s ^= z ^ (z >> 31);
        }
        if self.0 == [0; 4] {
            self.0[0] = 1;
        }
    }

    pub(crate) fn next(&mut self) -> u64 {
        let s = &mut self.0;
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }
}

const ETHERTYPE_IPV4: u16 = 0x0800;
const ETHERTYPE_ARP: u16 = 0x0806;
const PROTO_ICMP: u8 = 1;
const PROTO_TCP: u8 = 6;
const PROTO_UDP: u8 = 17;

/// How long a learned neighbour's address is kept, and how long packets wait for one.
const ARP_KEEP_MS: u64 = 5 * 60 * 1000;
const ARP_WAIT_MS: u64 = 3000;
const ARP_ASK_EVERY_MS: u64 = 1000;
const ARP_TABLE: usize = 64;
const WAITING_MAX: usize = 64;

struct Neighbour {
    ip: Ipv4,
    mac: Mac,
    until: u64,
}

/// An IP packet waiting for its next hop's network card address.
struct Waiting {
    hop: Ipv4,
    packet: Vec<u8>,
    since: u64,
}

/// A ping's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pong {
    pub from: Ipv4,
    pub id: u16,
    pub seq: u16,
}

pub struct Stack {
    mac: Mac,
    config: Option<Config>,
    dhcp: dhcp::Client,
    neighbours: Vec<Neighbour>,
    asked: Vec<(Ipv4, u64)>,
    waiting: Vec<Waiting>,
    out: VecDeque<Vec<u8>>,
    pub(crate) tcp: Vec<Option<tcp::Conn>>,
    listening: Vec<u16>,
    dns: dns::Resolver,
    ident: u16,
    pub(crate) random: Random,
    now: u64,
    pongs: Vec<Pong>,
    pub stats: Stats,
    /// Changes when the address does (DHCP's lease, or lost), for the screen and the log.
    changes: u64,
}

impl Stack {
    /// A stack for the card at `mac`. With `dhcp`, it asks the network for an address straight away; else it
    /// waits for `set_config`. `seed`: hard-to-guess bits from the kernel.
    pub fn new(mac: Mac, seed: u64, dhcp: bool) -> Stack {
        let mut random = Random::new(seed ^ u64::from_le_bytes([mac.0[0], mac.0[1], mac.0[2], mac.0[3], mac.0[4], mac.0[5], 0, 0]));
        let ident = random.next() as u16;
        Stack {
            mac,
            config: None,
            dhcp: dhcp::Client::new(dhcp, &mut random),
            neighbours: Vec::new(),
            asked: Vec::new(),
            waiting: Vec::new(),
            out: VecDeque::new(),
            tcp: Vec::new(),
            listening: Vec::new(),
            dns: dns::Resolver::new(),
            ident,
            random,
            now: 0,
            pongs: Vec::new(),
            stats: Stats::default(),
            changes: 0,
        }
    }

    pub fn mac(&self) -> Mac {
        self.mac
    }

    /// Adds hard-to-guess bits (the kernel's random numbers) to those the stack makes its own from.
    pub fn mix(&mut self, seed: u64) {
        self.random.mix(seed);
    }

    pub fn config(&self) -> Option<&Config> {
        self.config.as_ref()
    }

    /// Sets the address by hand (DHCP, if it was on, stops).
    pub fn set_config(&mut self, config: Config) {
        self.dhcp.stop();
        self.config = Some(config);
        self.changes += 1;
    }

    /// Changes whenever the address does.
    pub fn changes(&self) -> u64 {
        self.changes
    }

    /// The next frame to send, if any.
    pub fn transmit(&mut self) -> Option<Vec<u8>> {
        let f = self.out.pop_front();
        if f.is_some() {
            self.stats.sent += 1;
        }
        f
    }

    /// The time (in milliseconds, from any start) has moved on: sends again what wasn't answered, and lets go of
    /// what has waited too long.
    pub fn poll(&mut self, now: u64) {
        self.now = now;
        self.dhcp_poll(now);
        self.neighbours.retain(|n| n.until > now);
        let before = self.waiting.len();
        self.waiting.retain(|w| now.saturating_sub(w.since) < ARP_WAIT_MS);
        self.stats.dropped += (before - self.waiting.len()) as u64;
        let hops: Vec<Ipv4> = self.waiting.iter().map(|w| w.hop).collect();
        for hop in hops {
            self.ask(now, hop);
        }
        self.dns_poll(now);
        self.tcp_poll(now);
    }

    /// A frame the card received.
    pub fn receive(&mut self, now: u64, frame: &[u8]) {
        self.now = now;
        self.stats.received += 1;
        if frame.len() < 14 {
            self.stats.dropped += 1;
            return;
        }
        let mut dst = [0u8; 6];
        dst.copy_from_slice(&frame[0..6]);
        let dst = Mac(dst);
        if dst != self.mac && dst != Mac::BROADCAST {
            self.stats.dropped += 1; // (multicast isn't joined: nothing here needs it yet)
            return;
        }
        match be16(frame, 12) {
            ETHERTYPE_ARP => self.arp_in(now, &frame[14..]),
            ETHERTYPE_IPV4 => self.ip_in(now, &frame[14..]),
            _ => self.stats.dropped += 1,
        }
    }

    fn frame(&mut self, to: Mac, ethertype: u16, payload: &[u8]) {
        let mut f = Vec::with_capacity(14 + payload.len().max(46));
        f.extend_from_slice(&to.0);
        f.extend_from_slice(&self.mac.0);
        f.extend_from_slice(&ethertype.to_be_bytes());
        f.extend_from_slice(payload);
        if f.len() < 60 {
            f.resize(60, 0); // Ethernet's shortest frame (cards add the checksum)
        }
        self.out.push_back(f);
    }

    // ---------------------------------------------------------------- ARP: neighbours' card addresses

    fn arp_in(&mut self, now: u64, p: &[u8]) {
        if p.len() < 28 || be16(p, 0) != 1 || be16(p, 2) != ETHERTYPE_IPV4 || p[4] != 6 || p[5] != 4 {
            self.stats.dropped += 1;
            return;
        }
        let op = be16(p, 6);
        let mut sha = [0u8; 6];
        sha.copy_from_slice(&p[8..14]);
        let sha = Mac(sha);
        let spa = Ipv4([p[14], p[15], p[16], p[17]]);
        let tpa = Ipv4([p[24], p[25], p[26], p[27]]);
        let ours = self.config.as_ref().map(|c| c.address);
        // Learned when it's for us, or from a neighbour already known (not every ARP on the network).
        if Some(tpa) == ours || self.neighbours.iter().any(|n| n.ip == spa) {
            self.learn(now, spa, sha);
        }
        if op == 1 && ours.is_some() && Some(tpa) == ours {
            let mut r = [0u8; 28];
            r[0..8].copy_from_slice(&[0, 1, 8, 0, 6, 4, 0, 2]);
            r[8..14].copy_from_slice(&self.mac.0);
            r[14..18].copy_from_slice(&tpa.0);
            r[18..24].copy_from_slice(&sha.0);
            r[24..28].copy_from_slice(&spa.0);
            self.frame(sha, ETHERTYPE_ARP, &r);
        }
    }

    fn learn(&mut self, now: u64, ip: Ipv4, mac: Mac) {
        if ip == Ipv4::ANY || mac.is_group() {
            return;
        }
        match self.neighbours.iter_mut().find(|n| n.ip == ip) {
            Some(n) => {
                n.mac = mac;
                n.until = now + ARP_KEEP_MS;
            }
            None => {
                if self.neighbours.len() >= ARP_TABLE {
                    let oldest = (0..self.neighbours.len()).min_by_key(|&i| self.neighbours[i].until).unwrap_or(0);
                    self.neighbours.swap_remove(oldest);
                }
                self.neighbours.push(Neighbour { ip, mac, until: now + ARP_KEEP_MS });
            }
        }
        self.asked.retain(|(a, _)| *a != ip);
        let (go, keep): (Vec<Waiting>, Vec<Waiting>) = core::mem::take(&mut self.waiting).into_iter().partition(|w| w.hop == ip);
        self.waiting = keep;
        for w in go {
            self.frame(mac, ETHERTYPE_IPV4, &w.packet);
        }
    }

    /// Asks who has `ip` (at most once a second).
    fn ask(&mut self, now: u64, ip: Ipv4) {
        if let Some((_, at)) = self.asked.iter().find(|(a, _)| *a == ip)
            && now.saturating_sub(*at) < ARP_ASK_EVERY_MS
        {
            return;
        }
        self.asked.retain(|(a, _)| *a != ip);
        self.asked.push((ip, now));
        let from = self.config.as_ref().map_or(Ipv4::ANY, |c| c.address);
        let mut r = [0u8; 28];
        r[0..8].copy_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]);
        r[8..14].copy_from_slice(&self.mac.0);
        r[14..18].copy_from_slice(&from.0);
        r[24..28].copy_from_slice(&ip.0);
        self.frame(Mac::BROADCAST, ETHERTYPE_ARP, &r);
    }

    // ---------------------------------------------------------------- IPv4

    fn ip_in(&mut self, now: u64, p: &[u8]) {
        if p.len() < 20 || p[0] >> 4 != 4 {
            self.stats.dropped += 1;
            return;
        }
        let ihl = (p[0] & 0x0F) as usize * 4;
        let total = be16(p, 2) as usize;
        if ihl < 20 || total < ihl || total > p.len() || checksum(&[&p[..ihl]]) != 0 || be16(p, 6) & 0x3FFF != 0 {
            self.stats.dropped += 1; // (damaged, or a fragment)
            return;
        }
        let src = Ipv4([p[12], p[13], p[14], p[15]]);
        let dst = Ipv4([p[16], p[17], p[18], p[19]]);
        let for_us = match &self.config {
            Some(c) => dst == c.address || dst == Ipv4::BROADCAST || dst == c.broadcast(),
            None => true, // (DHCP's answers, before there's an address)
        };
        if !for_us {
            self.stats.dropped += 1;
            return;
        }
        let body = &p[ihl..total];
        match p[9] {
            PROTO_ICMP => self.icmp_in(now, src, dst, body),
            PROTO_UDP => self.udp_in(now, src, dst, body),
            PROTO_TCP => self.tcp_in(now, src, dst, body),
            _ => self.stats.dropped += 1,
        }
    }

    pub(crate) fn send_ip(&mut self, now: u64, dst: Ipv4, protocol: u8, payload: &[u8]) {
        let src = self.config.as_ref().map_or(Ipv4::ANY, |c| c.address);
        let mut p = vec![0u8; 20];
        p[0] = 0x45;
        put16(&mut p, 2, (20 + payload.len()) as u16);
        self.ident = self.ident.wrapping_add(1);
        put16(&mut p, 4, self.ident);
        put16(&mut p, 6, 0x4000); // don't fragment
        p[8] = 64;
        p[9] = protocol;
        p[12..16].copy_from_slice(&src.0);
        p[16..20].copy_from_slice(&dst.0);
        let c = checksum(&[&p[..20]]);
        put16(&mut p, 10, c);
        p.extend_from_slice(payload);
        let hop = match &self.config {
            _ if dst == Ipv4::BROADCAST => None,
            Some(c) if dst == c.broadcast() => None,
            Some(c) if c.on_link(dst) => Some(dst),
            Some(c) => match c.gateway {
                Some(g) => Some(g),
                None => {
                    self.stats.dropped += 1; // no way out
                    return;
                }
            },
            None => {
                self.stats.dropped += 1;
                return;
            }
        };
        let Some(hop) = hop else {
            self.frame(Mac::BROADCAST, ETHERTYPE_IPV4, &p);
            return;
        };
        if let Some(mac) = self.neighbours.iter().find(|n| n.ip == hop).map(|n| n.mac) {
            self.frame(mac, ETHERTYPE_IPV4, &p);
            return;
        }
        if self.waiting.len() >= WAITING_MAX {
            self.waiting.remove(0);
            self.stats.dropped += 1;
        }
        self.waiting.push(Waiting { hop, packet: p, since: now });
        self.ask(now, hop);
    }

    // ---------------------------------------------------------------- ICMP: pings

    fn icmp_in(&mut self, now: u64, src: Ipv4, dst: Ipv4, m: &[u8]) {
        if m.len() < 8 || checksum(&[m]) != 0 {
            self.stats.dropped += 1;
            return;
        }
        let ours = self.config.as_ref().map(|c| c.address);
        match m[0] {
            8 if Some(dst) == ours => {
                let mut r = m.to_vec();
                r[0] = 0;
                put16(&mut r, 2, 0);
                let c = checksum(&[&r]);
                put16(&mut r, 2, c);
                self.send_ip(now, src, PROTO_ICMP, &r);
            }
            0
                if self.pongs.len() < 64 => {
                    self.pongs.push(Pong { from: src, id: be16(m, 4), seq: be16(m, 6) });
                }
            _ => {}
        }
    }

    /// Sends a ping to `to`.
    pub fn ping(&mut self, now: u64, to: Ipv4, id: u16, seq: u16) {
        let mut m = vec![0u8; 8 + 32];
        m[0] = 8;
        put16(&mut m, 4, id);
        put16(&mut m, 6, seq);
        for (i, b) in m[8..].iter_mut().enumerate() {
            *b = b'a' + (i % 26) as u8;
        }
        let c = checksum(&[&m]);
        put16(&mut m, 2, c);
        self.send_ip(now, to, PROTO_ICMP, &m);
    }

    /// The pings answered since last asked.
    pub fn pongs(&mut self) -> Vec<Pong> {
        core::mem::take(&mut self.pongs)
    }

    // ---------------------------------------------------------------- UDP

    fn udp_in(&mut self, now: u64, src: Ipv4, dst: Ipv4, u: &[u8]) {
        if u.len() < 8 {
            self.stats.dropped += 1;
            return;
        }
        let len = be16(u, 4) as usize;
        if len < 8 || len > u.len() || (be16(u, 6) != 0 && transport_checksum(src, dst, PROTO_UDP, &u[..len]) != 0) {
            self.stats.dropped += 1;
            return;
        }
        let (sport, dport) = (be16(u, 0), be16(u, 2));
        let data = &u[8..len];
        if sport == dhcp::SERVER_PORT && dport == dhcp::CLIENT_PORT {
            self.dhcp_in(now, data);
        } else if sport == 53 {
            self.dns_in(now, src, dport, data);
        } else {
            self.stats.dropped += 1;
        }
    }

    pub(crate) fn send_udp(&mut self, now: u64, sport: u16, dst: Ipv4, dport: u16, data: &[u8]) {
        let src = self.config.as_ref().map_or(Ipv4::ANY, |c| c.address);
        let mut u = vec![0u8; 8 + data.len()];
        put16(&mut u, 0, sport);
        put16(&mut u, 2, dport);
        put16(&mut u, 4, (8 + data.len()) as u16);
        u[8..].copy_from_slice(data);
        let c = match transport_checksum(src, dst, PROTO_UDP, &u) {
            0 => 0xFFFF,
            c => c,
        };
        put16(&mut u, 6, c);
        self.send_ip(now, dst, PROTO_UDP, &u);
    }

    // ---------------------------------------------------------------- DNS (dns.rs does the work)

    /// Starts finding `name`'s address. An address written as one ("10.0.2.2") is its own answer.
    pub fn resolve(&mut self, now: u64, name: &str) -> Lookup {
        let id = self.dns.start(name, &mut self.random);
        self.dns_poll(now);
        Lookup(id)
    }

    /// The answer to `lookup`, once there is one (asked once: then it's forgotten).
    pub fn resolved(&mut self, lookup: Lookup) -> Option<Result<Ipv4, DnsError>> {
        self.dns.take(lookup.0)
    }

    fn dns_poll(&mut self, now: u64) {
        let servers = self.config.as_ref().map(|c| c.dns.clone()).unwrap_or_default();
        for (server, port, query) in self.dns.due(now, &servers) {
            self.send_udp(now, port, server, 53, &query);
        }
    }

    fn dns_in(&mut self, now: u64, src: Ipv4, port: u16, data: &[u8]) {
        if !self.dns.answer(now, src, port, data) {
            self.stats.dropped += 1;
        }
    }

    // ---------------------------------------------------------------- DHCP (dhcp.rs does the work)

    /// Whether DHCP is still looking for an address.
    pub fn dhcp_busy(&self) -> bool {
        self.dhcp.busy()
    }

    fn dhcp_poll(&mut self, now: u64) {
        let (send, lost) = self.dhcp.poll(now, self.mac, self.config.as_ref(), &mut self.random);
        if lost && self.config.take().is_some() {
            self.changes += 1;
        }
        for m in send {
            match m.to {
                Some(server) => self.send_udp(now, dhcp::CLIENT_PORT, server, dhcp::SERVER_PORT, &m.data),
                None => self.send_udp(now, dhcp::CLIENT_PORT, Ipv4::BROADCAST, dhcp::SERVER_PORT, &m.data),
            }
        }
    }

    fn dhcp_in(&mut self, now: u64, data: &[u8]) {
        match self.dhcp.answer(now, self.mac, data) {
            dhcp::Outcome::Nothing => {}
            dhcp::Outcome::Send => self.dhcp_poll(now),
            dhcp::Outcome::Bound(config) => {
                if self.config.as_ref() != Some(&config) {
                    self.config = Some(config);
                    self.changes += 1;
                }
            }
            dhcp::Outcome::Lost => {
                if self.config.take().is_some() {
                    self.changes += 1;
                }
                self.dhcp_poll(now);
            }
        }
    }

    /// What DHCP is doing, for the log and the screen.
    pub fn dhcp_state(&self) -> &'static str {
        self.dhcp.describe()
    }

    /// The name DHCP gave the network, if any (for the log).
    pub fn domain(&self) -> Option<&String> {
        self.dhcp.domain.as_ref()
    }
}

/// A name being looked up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Lookup(u32);
