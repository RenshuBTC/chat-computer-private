//! IPv6 (RFC 8200), beside IPv4: an address of its own on the link (fe80::, from a random interface identifier made
//! each time Chat Computer starts, so it doesn't follow the person from network to network), and on the internet
//! from the routers' advertisements (SLAAC, RFC 4862: a prefix of 64 bits, and the same identifier); its routers,
//! the network's own prefixes and its DNS servers (RDNSS, RFC 8106) from the advertisements too, asked for when it
//! starts (Router Solicitations). Neighbours' card addresses by Neighbor Discovery (RFC 4861: solicitations to their
//! solicited-node groups, and advertisements back); our own answered. The groups it listens to (its solicited-node
//! group) reported with MLDv2 (RFC 3810), so switches that watch for them pass them on. ICMPv6's echo answered,
//! UDP (for DNS) and TCP carried. No extension headers but the hop-by-hop one (skipped), no fragments, no
//! duplicate address detection (a random identifier of 64 bits doesn't clash).

use crate::{be16, be32, checksum, put16, Mac, Stack, PROTO_TCP, PROTO_UDP};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

pub(crate) const ETHERTYPE_IPV6: u16 = 0x86DD;
pub(crate) const PROTO_ICMPV6: u8 = 58;
const HOP_BY_HOP: u8 = 0;

/// ICMPv6's messages.
const DESTINATION_UNREACHABLE: u8 = 1;
const ECHO_REQUEST: u8 = 128;
const ECHO_REPLY: u8 = 129;
const MLD_QUERY: u8 = 130;
const ROUTER_SOLICITATION: u8 = 133;
const ROUTER_ADVERTISEMENT: u8 = 134;
const NEIGHBOR_SOLICITATION: u8 = 135;
const NEIGHBOR_ADVERTISEMENT: u8 = 136;
const MLDV2_REPORT: u8 = 143;

/// Router Solicitations: at most 3, 4 seconds apart (RFC 4861's numbers).
const SOLICITATIONS: u32 = 3;
const SOLICIT_EVERY_MS: u64 = 4000;
/// Neighbours: kept 5 minutes; packets wait 3 seconds for one's address; asked again each second.
const KEEP_MS: u64 = 5 * 60 * 1000;
const WAIT_MS: u64 = 3000;
const ASK_EVERY_MS: u64 = 1000;
const NEIGHBOURS: usize = 64;
const WAITING_MAX: usize = 64;
/// The most addresses, routers and DNS servers kept from advertisements.
const MOST: usize = 8;

/// An IPv6 address.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Ipv6(pub [u8; 16]);

impl Ipv6 {
    pub const ANY: Ipv6 = Ipv6([0; 16]);
    /// Every node on the link, and every router.
    pub const ALL_NODES: Ipv6 = Ipv6([0xFF, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    pub const ALL_ROUTERS: Ipv6 = Ipv6([0xFF, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);
    /// Where MLDv2's reports go.
    pub const MLDV2_ROUTERS: Ipv6 = Ipv6([0xFF, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x16]);

    pub fn is_multicast(&self) -> bool {
        self.0[0] == 0xFF
    }

    /// fe80::/10: on this link only.
    pub fn is_link_local(&self) -> bool {
        self.0[0] == 0xFE && self.0[1] & 0xC0 == 0x80
    }

    pub fn is_loopback(&self) -> bool {
        self.0 == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
    }

    /// The solicited-node group of an address (ff02::1:ff and its last 24 bits): where a neighbour asks for it.
    pub fn solicited_node(&self) -> Ipv6 {
        let mut a = [0u8; 16];
        a[0] = 0xFF;
        a[1] = 2;
        a[11] = 1;
        a[12] = 0xFF;
        a[13..16].copy_from_slice(&self.0[13..16]);
        Ipv6(a)
    }

    /// A multicast group's card address (33:33, then its last 32 bits).
    pub fn multicast_mac(&self) -> Mac {
        Mac([0x33, 0x33, self.0[12], self.0[13], self.0[14], self.0[15]])
    }

    /// "fe80::1", "2001:db8::5:1" as an address (RFC 4291's forms, but the one ending in an IPv4 address); None for
    /// anything else.
    pub fn parse(s: &str) -> Option<Ipv6> {
        if s.is_empty() || s.len() > 39 {
            return None;
        }
        let group = |g: &str| -> Option<u16> {
            if g.is_empty() || g.len() > 4 || !g.bytes().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            u16::from_str_radix(g, 16).ok()
        };
        let parts = |half: &str| -> Option<Vec<u16>> {
            if half.is_empty() {
                return Some(Vec::new());
            }
            half.split(':').map(group).collect()
        };
        let groups: Vec<u16> = match s.split_once("::") {
            Some((a, b)) => {
                let (a, b) = (parts(a)?, parts(b)?);
                if a.len() + b.len() > 7 || b.is_empty() && s.ends_with(":::") {
                    return None;
                }
                let mut g = a;
                g.resize(8 - b.len(), 0);
                g.extend(b);
                g
            }
            None => parts(s)?,
        };
        if groups.len() != 8 {
            return None;
        }
        let mut out = [0u8; 16];
        for (i, g) in groups.iter().enumerate() {
            out[2 * i..2 * i + 2].copy_from_slice(&g.to_be_bytes());
        }
        Some(Ipv6(out))
    }
}

impl fmt::Display for Ipv6 {
    /// As RFC 5952 writes it: groups in lower-case hex without their leading zeros, the longest run of two or more
    /// zero groups (the first, of two as long) as "::".
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let g: Vec<u16> = (0..8).map(|i| u16::from_be_bytes([self.0[2 * i], self.0[2 * i + 1]])).collect();
        let (mut best, mut best_len) = (8usize, 1usize);
        let mut i = 0;
        while i < 8 {
            if g[i] == 0 {
                let start = i;
                while i < 8 && g[i] == 0 {
                    i += 1;
                }
                if i - start > best_len {
                    best = start;
                    best_len = i - start;
                }
            } else {
                i += 1;
            }
        }
        let mut i = 0;
        while i < 8 {
            if i == best {
                f.write_str("::")?;
                i += best_len;
                continue;
            }
            if i > 0 && i != best + best_len {
                f.write_str(":")?;
            }
            write!(f, "{:x}", g[i])?;
            i += 1;
        }
        Ok(())
    }
}

impl fmt::Debug for Ipv6 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The checksum of an ICMPv6, UDP or TCP packet over IPv6, with its pseudo-header in front.
pub(crate) fn checksum6(src: Ipv6, dst: Ipv6, next: u8, packet: &[u8]) -> u16 {
    let mut pseudo = [0u8; 40];
    pseudo[0..16].copy_from_slice(&src.0);
    pseudo[16..32].copy_from_slice(&dst.0);
    pseudo[32..36].copy_from_slice(&(packet.len() as u32).to_be_bytes());
    pseudo[39] = next;
    checksum(&[&pseudo, packet])
}

/// Something learned from an advertisement, kept until a time (ms).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Until<T> {
    pub(crate) what: T,
    pub(crate) until: u64,
}

/// A packet waiting for its next hop's card address.
struct Waiting {
    hop: Ipv6,
    packet: Vec<u8>,
    since: u64,
}

/// IPv6's state: the interface identifier, addresses, routers, prefixes on the link, DNS servers, neighbours.
pub(crate) struct State {
    iid: [u8; 8],
    pub(crate) link_local: Ipv6,
    /// Addresses from routers' prefixes (SLAAC).
    pub(crate) addresses: Vec<Until<Ipv6>>,
    /// Prefixes on the link (their first 64 bits: every prefix used here is 64 bits long, or longer and kept as 64).
    on_link: Vec<Until<([u8; 16], u8)>>,
    /// Default routers (their link-local addresses).
    pub(crate) routers: Vec<Until<Ipv6>>,
    pub(crate) dns: Vec<Until<Ipv6>>,
    pub(crate) mtu: u16,
    /// Router Solicitations sent, and when the last was.
    solicited: (u32, u64),
    /// Whether our groups have been reported (MLDv2) since the link came up.
    reported: bool,
    neighbours: Vec<(Ipv6, Mac, u64)>,
    asked: Vec<(Ipv6, u64)>,
    waiting: Vec<Waiting>,
}

impl State {
    pub(crate) fn new(iid: [u8; 8]) -> State {
        let mut ll = [0u8; 16];
        ll[0] = 0xFE;
        ll[1] = 0x80;
        ll[8..16].copy_from_slice(&iid);
        State {
            iid,
            link_local: Ipv6(ll),
            addresses: Vec::new(),
            on_link: Vec::new(),
            routers: Vec::new(),
            dns: Vec::new(),
            mtu: 1500,
            solicited: (0, 0),
            reported: false,
            neighbours: Vec::new(),
            asked: Vec::new(),
            waiting: Vec::new(),
        }
    }

    /// Whether `a` is one of our addresses.
    fn ours(&self, a: Ipv6) -> bool {
        a == self.link_local || self.addresses.iter().any(|x| x.what == a)
    }

    /// Whether packets to `a` are for us: our addresses, every node's group, and our solicited-node group.
    fn for_us(&self, a: Ipv6) -> bool {
        self.ours(a) || a == Ipv6::ALL_NODES || a == self.link_local.solicited_node()
    }

    /// The address to send to `dst` from: the link-local one for the link, else one from a router's prefix.
    fn source_for(&self, dst: Ipv6) -> Option<Ipv6> {
        if dst.is_link_local() || (dst.is_multicast() && dst.0[1] & 0x0F <= 2) {
            Some(self.link_local)
        } else {
            self.addresses.first().map(|a| a.what)
        }
    }

    /// Whether `dst` is on the link (reached directly, not through a router).
    fn on_link(&self, dst: Ipv6) -> bool {
        dst.is_link_local()
            || self.on_link.iter().any(|p| {
                let (prefix, len) = p.what;
                let full = (len / 8) as usize;
                dst.0[..full] == prefix[..full] && (len % 8 == 0 || (dst.0[full] ^ prefix[full]) >> (8 - len % 8) == 0)
            })
    }

    /// Whether a global address can be reached: one of ours from a router's prefix, and a router (or the link).
    pub(crate) fn can_reach(&self, dst: Ipv6) -> bool {
        if dst.is_link_local() {
            return true;
        }
        !self.addresses.is_empty() && (!self.routers.is_empty() || self.on_link(dst))
    }

    /// Our card's groups (the multicast addresses whose frames are for us).
    pub(crate) fn wants_mac(&self, m: Mac) -> bool {
        m == Ipv6::ALL_NODES.multicast_mac() || m == self.link_local.solicited_node().multicast_mac()
    }
}

/// Keeps `what` (replacing what's there with the same `same`), until `until` (gone when that's now); at most MOST.
fn keep<T: Copy + PartialEq>(list: &mut Vec<Until<T>>, what: T, until: u64, now: u64) {
    list.retain(|x| x.what != what);
    if until > now {
        if list.len() >= MOST {
            list.remove(0);
        }
        list.push(Until { what, until });
    }
}

/// Seconds (from an advertisement: 0xFFFFFFFF for ever) after `now`, in ms.
fn after(now: u64, seconds: u32) -> u64 {
    if seconds == u32::MAX { u64::MAX } else { now + seconds as u64 * 1000 }
}

impl Stack {
    // ---------------------------------------------------------------- IPv6 in

    pub(crate) fn ip6_in(&mut self, now: u64, p: &[u8]) {
        if p.len() < 40 || p[0] >> 4 != 6 {
            self.stats.dropped += 1;
            return;
        }
        let len = be16(p, 4) as usize;
        if 40 + len > p.len() {
            self.stats.dropped += 1;
            return;
        }
        let mut next = p[6];
        let hops = p[7];
        let mut src = Ipv6::ANY;
        src.0.copy_from_slice(&p[8..24]);
        let mut dst = Ipv6::ANY;
        dst.0.copy_from_slice(&p[24..40]);
        if !self.v6.for_us(dst) || src.is_multicast() {
            self.stats.dropped += 1;
            return;
        }
        let mut body = &p[40..40 + len];
        // (A hop-by-hop header, as MLD's queries carry: skipped.)
        if next == HOP_BY_HOP {
            if body.len() < 8 {
                self.stats.dropped += 1;
                return;
            }
            let n = (body[1] as usize + 1) * 8;
            if n > body.len() {
                self.stats.dropped += 1;
                return;
            }
            next = body[0];
            body = &body[n..];
        }
        match next {
            PROTO_ICMPV6 => self.icmp6_in(now, src, dst, hops, body),
            PROTO_UDP => self.udp6_in(now, src, dst, body),
            PROTO_TCP => self.tcp_in(now, crate::Ip::V6(src), crate::Ip::V6(dst), body),
            _ => self.stats.dropped += 1,
        }
    }

    fn udp6_in(&mut self, now: u64, src: Ipv6, dst: Ipv6, u: &[u8]) {
        if u.len() < 8 {
            self.stats.dropped += 1;
            return;
        }
        let len = be16(u, 4) as usize;
        // (Over IPv6 the checksum isn't optional.)
        if len < 8 || len > u.len() || checksum6(src, dst, PROTO_UDP, &u[..len]) != 0 {
            self.stats.dropped += 1;
            return;
        }
        let (sport, dport) = (be16(u, 0), be16(u, 2));
        if sport == 53 {
            self.dns_in(now, crate::Ip::V6(src), dport, &u[8..len]);
        } else {
            self.stats.dropped += 1;
        }
    }

    pub(crate) fn send_udp6(&mut self, now: u64, sport: u16, dst: Ipv6, dport: u16, data: &[u8]) {
        let Some(src) = self.v6.source_for(dst) else {
            self.stats.dropped += 1;
            return;
        };
        let mut u = vec![0u8; 8 + data.len()];
        put16(&mut u, 0, sport);
        put16(&mut u, 2, dport);
        put16(&mut u, 4, (8 + data.len()) as u16);
        u[8..].copy_from_slice(data);
        let c = match checksum6(src, dst, PROTO_UDP, &u) {
            0 => 0xFFFF,
            c => c,
        };
        put16(&mut u, 6, c);
        self.send_ip6(now, src, dst, PROTO_UDP, 64, &u);
    }

    // ---------------------------------------------------------------- IPv6 out

    /// Our address for sending to `dst`, if there's one.
    pub(crate) fn source6(&self, dst: Ipv6) -> Option<Ipv6> {
        self.v6.source_for(dst)
    }

    /// Sends `payload` (its checksum made with `src`) to `dst`: to a group's card address, or its next hop's (asked
    /// for, the packet waiting, when it isn't known yet).
    pub(crate) fn send_ip6(&mut self, now: u64, src: Ipv6, dst: Ipv6, next: u8, hops: u8, payload: &[u8]) {
        let mut p = vec![0u8; 40];
        p[0] = 0x60;
        put16(&mut p, 4, payload.len() as u16);
        p[6] = next;
        p[7] = hops;
        p[8..24].copy_from_slice(&src.0);
        p[24..40].copy_from_slice(&dst.0);
        p.extend_from_slice(payload);
        self.send_packet6(now, dst, p);
    }

    fn send_packet6(&mut self, now: u64, dst: Ipv6, p: Vec<u8>) {
        if dst.is_multicast() {
            self.frame(dst.multicast_mac(), ETHERTYPE_IPV6, &p);
            return;
        }
        let hop = if self.v6.on_link(dst) {
            dst
        } else {
            match self.v6.routers.first() {
                Some(r) => r.what,
                None => {
                    self.stats.dropped += 1; // no way out
                    return;
                }
            }
        };
        if let Some(mac) = self.v6.neighbours.iter().find(|n| n.0 == hop).map(|n| n.1) {
            self.frame(mac, ETHERTYPE_IPV6, &p);
            return;
        }
        if self.v6.waiting.len() >= WAITING_MAX {
            self.v6.waiting.remove(0);
            self.stats.dropped += 1;
        }
        self.v6.waiting.push(Waiting { hop, packet: p, since: now });
        self.solicit(now, hop);
    }

    /// Asks who has `ip` (a Neighbor Solicitation to its solicited-node group), at most once a second.
    fn solicit(&mut self, now: u64, ip: Ipv6) {
        if let Some((_, at)) = self.v6.asked.iter().find(|(a, _)| *a == ip)
            && now.saturating_sub(*at) < ASK_EVERY_MS
        {
            return;
        }
        self.v6.asked.retain(|(a, _)| *a != ip);
        self.v6.asked.push((ip, now));
        let src = if ip.is_link_local() { Some(self.v6.link_local) } else { self.v6.addresses.first().map(|a| a.what) };
        let src = src.unwrap_or(self.v6.link_local);
        let mut m = vec![0u8; 24 + 8];
        m[0] = NEIGHBOR_SOLICITATION;
        m[8..24].copy_from_slice(&ip.0);
        m[24] = 1; // our card's address
        m[25] = 1;
        m[26..32].copy_from_slice(&self.mac.0);
        self.send_icmp6(now, src, ip.solicited_node(), 255, m);
    }

    /// An ICMPv6 message, its checksum filled in.
    fn send_icmp6(&mut self, now: u64, src: Ipv6, dst: Ipv6, hops: u8, mut m: Vec<u8>) {
        put16(&mut m, 2, 0);
        let c = checksum6(src, dst, PROTO_ICMPV6, &m);
        put16(&mut m, 2, c);
        self.send_ip6(now, src, dst, PROTO_ICMPV6, hops, &m);
    }

    /// Learns a neighbour's card address, and sends what was waiting for it.
    pub(crate) fn learn6(&mut self, now: u64, ip: Ipv6, mac: Mac) {
        if mac.0[0] & 1 != 0 || ip.is_multicast() || ip == Ipv6::ANY {
            return;
        }
        let n = &mut self.v6.neighbours;
        match n.iter_mut().find(|x| x.0 == ip) {
            Some(x) => {
                x.1 = mac;
                x.2 = now + KEEP_MS;
            }
            None => {
                if n.len() >= NEIGHBOURS {
                    let oldest = (0..n.len()).min_by_key(|&i| n[i].2).unwrap_or(0);
                    n.swap_remove(oldest);
                }
                n.push((ip, mac, now + KEEP_MS));
            }
        }
        self.v6.asked.retain(|(a, _)| *a != ip);
        let (go, keep): (Vec<Waiting>, Vec<Waiting>) = core::mem::take(&mut self.v6.waiting).into_iter().partition(|w| w.hop == ip);
        self.v6.waiting = keep;
        for w in go {
            self.frame(mac, ETHERTYPE_IPV6, &w.packet);
        }
    }

    // ---------------------------------------------------------------- ICMPv6

    fn icmp6_in(&mut self, now: u64, src: Ipv6, dst: Ipv6, hops: u8, m: &[u8]) {
        if m.len() < 8 || checksum6(src, dst, PROTO_ICMPV6, m) != 0 {
            self.stats.dropped += 1;
            return;
        }
        // Neighbor Discovery's messages come only from the link: their hop limit is still 255 (RFC 4861 6.1).
        let ndp = matches!(m[0], ROUTER_SOLICITATION..=NEIGHBOR_ADVERTISEMENT);
        if ndp && (hops != 255 || m[1] != 0) {
            self.stats.dropped += 1;
            return;
        }
        match m[0] {
            ECHO_REQUEST if self.v6.ours(dst) || dst == Ipv6::ALL_NODES => {
                let mut r = m.to_vec();
                r[0] = ECHO_REPLY;
                let from = if self.v6.ours(dst) { dst } else { self.v6.source_for(src).unwrap_or(self.v6.link_local) };
                self.send_icmp6(now, from, src, 64, r);
            }
            ECHO_REPLY => {
                if self.pongs.len() < 64 {
                    self.pongs.push(crate::Pong { from: crate::Ip::V6(src), id: be16(m, 4), seq: be16(m, 6) });
                }
            }
            ROUTER_ADVERTISEMENT => self.advertised(now, src, m),
            NEIGHBOR_SOLICITATION => self.solicited(now, src, m),
            NEIGHBOR_ADVERTISEMENT => {
                if m.len() < 24 {
                    return;
                }
                let mut target = Ipv6::ANY;
                target.0.copy_from_slice(&m[8..24]);
                if let Some(mac) = link_layer_option(&m[24..], 2) {
                    self.learn6(now, target, mac);
                }
            }
            MLD_QUERY => self.report(now),
            // Destination unreachable, about one of our TCP segments (the start of it is in the message): a
            // connection still opening there is given up (port unreachable: refused).
            DESTINATION_UNREACHABLE if m.len() >= 8 + 40 + 4 && m[8 + 6] == PROTO_TCP => {
                let (mut from, mut to) = (Ipv6::ANY, Ipv6::ANY);
                from.0.copy_from_slice(&m[16..32]);
                to.0.copy_from_slice(&m[32..48]);
                if self.v6.ours(from) {
                    self.tcp_unreachable(crate::Ip::V6(to), be16(m, 48), be16(m, 50), m[1] == 4);
                }
            }
            _ => {}
        }
    }

    /// A router's advertisement: its prefixes (an address from each that's for SLAAC; those on the link), whether
    /// it's a default router (and for how long), the link's MTU, DNS servers.
    fn advertised(&mut self, now: u64, src: Ipv6, m: &[u8]) {
        if m.len() < 16 || !src.is_link_local() {
            self.stats.dropped += 1;
            return;
        }
        let lifetime = be16(m, 6) as u32;
        let before: Vec<Ipv6> = self.v6.addresses.iter().map(|a| a.what).collect();
        keep(&mut self.v6.routers, src, now + lifetime as u64 * 1000, now);
        let mut at = 16;
        while at + 8 <= m.len() {
            let (kind, len) = (m[at], m[at + 1] as usize * 8);
            if len == 0 || at + len > m.len() {
                break;
            }
            let o = &m[at..at + len];
            match kind {
                1 if len == 8 => {
                    let mut mac = [0u8; 6];
                    mac.copy_from_slice(&o[2..8]);
                    self.learn6(now, src, Mac(mac));
                }
                // Prefix Information: its length, whether it's on the link (L) and for SLAAC (A), how long it's
                // valid, then the prefix.
                3 if len == 32 => {
                    let (plen, flags, valid) = (o[2], o[3], be32(o, 4));
                    let mut prefix = [0u8; 16];
                    prefix.copy_from_slice(&o[16..32]);
                    if prefix[0] == 0xFE && prefix[1] & 0xC0 == 0x80 || prefix[0] == 0xFF || plen > 128 {
                        at += len;
                        continue;
                    }
                    if flags & 0x80 != 0 {
                        keep(&mut self.v6.on_link, (prefix, plen), after(now, valid), now);
                    }
                    if flags & 0x40 != 0 && plen == 64 {
                        let mut a = [0u8; 16];
                        a[..8].copy_from_slice(&prefix[..8]);
                        a[8..].copy_from_slice(&self.v6.iid);
                        keep(&mut self.v6.addresses, Ipv6(a), after(now, valid), now);
                    }
                }
                5 if len == 8 => {
                    let mtu = be32(o, 4);
                    if (1280..=1500).contains(&mtu) {
                        self.v6.mtu = mtu as u16;
                    }
                }
                // RDNSS: how long, then the servers.
                25 if len >= 24 => {
                    let life = be32(o, 4);
                    for s in o[8..].chunks_exact(16) {
                        let mut a = Ipv6::ANY;
                        a.0.copy_from_slice(s);
                        keep(&mut self.v6.dns, a, after(now, life), now);
                    }
                }
                _ => {}
            }
            at += len;
        }
        let now_addresses: Vec<Ipv6> = self.v6.addresses.iter().map(|a| a.what).collect();
        if now_addresses != before {
            self.changes += 1;
        }
    }

    /// A neighbour asks for one of our addresses: answered (Neighbor Advertisement, our card's address in it),
    /// and its own card address learned.
    fn solicited(&mut self, now: u64, src: Ipv6, m: &[u8]) {
        if m.len() < 24 {
            self.stats.dropped += 1;
            return;
        }
        let mut target = Ipv6::ANY;
        target.0.copy_from_slice(&m[8..24]);
        if !self.v6.ours(target) {
            return;
        }
        let unspecified = src == Ipv6::ANY;
        if !unspecified && let Some(mac) = link_layer_option(&m[24..], 1) {
            self.learn6(now, src, mac);
        }
        let mut r = vec![0u8; 24 + 8];
        r[0] = NEIGHBOR_ADVERTISEMENT;
        // Solicited (unless it was a duplicate address check, from ::), and overriding what it had.
        r[4] = if unspecified { 0x20 } else { 0x60 };
        r[8..24].copy_from_slice(&target.0);
        r[24] = 2;
        r[25] = 1;
        r[26..32].copy_from_slice(&self.mac.0);
        let to = if unspecified { Ipv6::ALL_NODES } else { src };
        self.send_icmp6(now, target, to, 255, r);
    }

    /// Our groups reported to the switches and routers that watch for them (MLDv2): the solicited-node group our
    /// addresses share (they all end in the same identifier).
    fn report(&mut self, now: u64) {
        let group = self.v6.link_local.solicited_node();
        let mut m = vec![0u8; 8 + 20];
        m[0] = MLDV2_REPORT;
        put16(&mut m, 6, 1);
        m[8] = 4; // "changed to exclude none": joined
        m[12..28].copy_from_slice(&group.0);
        put16(&mut m, 2, 0);
        let src = self.v6.link_local;
        let c = checksum6(src, Ipv6::MLDV2_ROUTERS, PROTO_ICMPV6, &m);
        put16(&mut m, 2, c);
        // A hop-by-hop header first, with Router Alert (for MLD) and padding.
        let mut p = vec![PROTO_ICMPV6, 0, 5, 2, 0, 0, 1, 0];
        p.extend_from_slice(&m);
        self.send_ip6(now, src, Ipv6::MLDV2_ROUTERS, HOP_BY_HOP, 1, &p);
    }

    /// IPv6's timers: Router Solicitations until a router answers, our groups reported once, what was learned let
    /// go when it ends, what waits for a neighbour asked for again (or dropped).
    pub(crate) fn ip6_poll(&mut self, now: u64) {
        if !self.v6.reported {
            self.v6.reported = true;
            self.report(now);
        }
        let (sent, last) = self.v6.solicited;
        if self.v6.routers.is_empty() && self.v6.addresses.is_empty() && sent < SOLICITATIONS && (sent == 0 || now.saturating_sub(last) >= SOLICIT_EVERY_MS) {
            self.v6.solicited = (sent + 1, now);
            let mut m = vec![0u8; 8 + 8];
            m[0] = ROUTER_SOLICITATION;
            m[8] = 1;
            m[9] = 1;
            m[10..16].copy_from_slice(&self.mac.0);
            let src = self.v6.link_local;
            self.send_icmp6(now, src, Ipv6::ALL_ROUTERS, 255, m);
        }
        let before = self.v6.addresses.len();
        self.v6.addresses.retain(|a| a.until > now);
        self.v6.routers.retain(|a| a.until > now);
        self.v6.on_link.retain(|a| a.until > now);
        self.v6.dns.retain(|a| a.until > now);
        self.v6.neighbours.retain(|n| n.2 > now);
        if self.v6.addresses.len() != before {
            self.changes += 1;
        }
        let waited = self.v6.waiting.len();
        self.v6.waiting.retain(|w| now.saturating_sub(w.since) < WAIT_MS);
        self.stats.dropped += (waited - self.v6.waiting.len()) as u64;
        let hops: Vec<Ipv6> = self.v6.waiting.iter().map(|w| w.hop).collect();
        for hop in hops {
            self.solicit(now, hop);
        }
    }

    /// Our IPv6 addresses: the one on the link, then those from routers' prefixes.
    pub fn ipv6_addresses(&self) -> Vec<Ipv6> {
        let mut out = vec![self.v6.link_local];
        out.extend(self.v6.addresses.iter().map(|a| a.what));
        out
    }

    /// Whether a neighbour's card address is known (for the tests).
    #[cfg(test)]
    pub(crate) fn v6_knows(&self, ip: Ipv6) -> bool {
        self.v6.neighbours.iter().any(|n| n.0 == ip)
    }

    /// The IPv6 DNS servers routers named.
    pub fn ipv6_dns(&self) -> Vec<Ipv6> {
        self.v6.dns.iter().map(|d| d.what).collect()
    }

    /// Whether this computer can reach the internet over IPv6 (an address from a router's prefix, and a router).
    pub fn ipv6_ready(&self) -> bool {
        !self.v6.addresses.is_empty() && !self.v6.routers.is_empty()
    }

    /// Sends a ping to `to` over IPv6.
    pub fn ping6(&mut self, now: u64, to: Ipv6, id: u16, seq: u16) {
        let Some(src) = self.v6.source_for(to) else { return };
        let mut m = vec![0u8; 8 + 32];
        m[0] = ECHO_REQUEST;
        put16(&mut m, 4, id);
        put16(&mut m, 6, seq);
        for (i, b) in m[8..].iter_mut().enumerate() {
            *b = b'a' + (i % 26) as u8;
        }
        self.send_icmp6(now, src, to, 64, m);
    }
}

/// A Neighbor Discovery option's card address (`kind` 1: the sender's, 2: the target's), if the options have one.
fn link_layer_option(mut o: &[u8], kind: u8) -> Option<Mac> {
    while o.len() >= 8 {
        let len = o[1] as usize * 8;
        if len == 0 || len > o.len() {
            return None;
        }
        if o[0] == kind && len == 8 {
            let mut mac = [0u8; 6];
            mac.copy_from_slice(&o[2..8]);
            return Some(Mac(mac));
        }
        o = &o[len..];
    }
    None
}
