//! The stack against itself (two computers on one wire, which can lose and reorder frames) and against
//! stand-ins for a DHCP server and a DNS server.

use super::*;
use crate::ipv6::checksum6;

fn mac(n: u8) -> Mac {
    Mac([0x52, 0x54, 0, 0x12, 0x34, n])
}

fn config(last: u8) -> Config {
    Config { address: Ipv4([10, 0, 0, last]), prefix: 24, gateway: Some(Ipv4([10, 0, 0, 1])), dns: vec![Ipv4([10, 0, 0, 3])] }
}

/// Two stacks on one wire. `lose(n)`: whether the n-th frame is lost; `late(n)`: whether it's held back one
/// step (so it arrives after the next one).
struct Wire {
    a: Stack,
    b: Stack,
    now: u64,
    count: u64,
    lose: fn(u64) -> bool,
    late: fn(u64) -> bool,
    held: Vec<(bool, Vec<u8>)>,
}

impl Wire {
    fn new() -> Wire {
        let mut a = Stack::new(mac(2), 1, false);
        let mut b = Stack::new(mac(5), 2, false);
        a.set_config(config(2));
        b.set_config(config(5));
        Wire { a, b, now: 1000, count: 0, lose: |_| false, late: |_| false, held: Vec::new() }
    }

    fn step(&mut self) {
        self.now += 1;
        self.a.poll(self.now);
        self.b.poll(self.now);
        let mut frames = Vec::new();
        while let Some(f) = self.a.transmit() {
            frames.push((true, f));
        }
        while let Some(f) = self.b.transmit() {
            frames.push((false, f));
        }
        let held = core::mem::take(&mut self.held);
        for (from_a, f) in frames {
            self.count += 1;
            if (self.lose)(self.count) {
                continue;
            }
            if (self.late)(self.count) {
                self.held.push((from_a, f));
                continue;
            }
            if from_a { self.b.receive(self.now, &f) } else { self.a.receive(self.now, &f) }
        }
        for (from_a, f) in held {
            if from_a { self.b.receive(self.now, &f) } else { self.a.receive(self.now, &f) }
        }
    }

    fn run(&mut self, ms: u64) {
        for _ in 0..ms {
            self.step();
        }
    }

    fn until(&mut self, ms: u64, mut done: impl FnMut(&mut Wire) -> bool) -> bool {
        for _ in 0..ms {
            if done(self) {
                return true;
            }
            self.step();
        }
        done(self)
    }
}

#[test]
fn checksums_are_the_internets() {
    // RFC 1071's example: 0001 f203 f4f5 f6f7 sums to ddf2, so the checksum is 220d.
    assert_eq!(checksum(&[&[0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7]]), 0x220d);
    // Split anywhere, even mid-word, it's the same.
    assert_eq!(checksum(&[&[0x00, 0x01, 0xf2], &[0x03, 0xf4, 0xf5, 0xf6, 0xf7]]), 0x220d);
    assert_eq!(checksum(&[&[0x00, 0x01, 0xf2, 0x03, 0xf4]]), checksum(&[&[0x00, 0x01, 0xf2, 0x03, 0xf4, 0x00]]));
}

#[test]
fn addresses_read_and_print() {
    assert_eq!(Ipv4::parse("10.0.2.15"), Some(Ipv4([10, 0, 2, 15])));
    for bad in ["10.0.2", "10.0.2.15.1", "10.0.2.256", "a.b.c.d", "", "10..2.1", "1.2.3.-4"] {
        assert_eq!(Ipv4::parse(bad), None, "{bad}");
    }
    assert_eq!(format!("{}", Ipv4([192, 168, 1, 20])), "192.168.1.20");
    assert_eq!(format!("{}", mac(5)), "52:54:00:12:34:05");
    let c = config(2);
    assert!(c.on_link(Ipv4([10, 0, 0, 200])) && !c.on_link(Ipv4([10, 0, 1, 1])));
    assert_eq!(c.broadcast(), Ipv4([10, 0, 0, 255]));
}

#[test]
fn neighbours_are_found_and_pings_answered() {
    let mut w = Wire::new();
    w.a.ping(w.now, Ipv4([10, 0, 0, 5]), 7, 1);
    w.run(10);
    assert_eq!(w.a.pongs(), vec![Pong { from: Ip::V4(Ipv4([10, 0, 0, 5])), id: 7, seq: 1 }]);
    // Both now know each other (b learned a from a's question).
    assert!(w.a.neighbours.iter().any(|n| n.mac == mac(5)) && w.b.neighbours.iter().any(|n| n.mac == mac(2)));
    // An address no one has: asked about once a second, and given up on after three.
    w.a.ping(w.now, Ipv4([10, 0, 0, 9]), 7, 2);
    w.run(5000);
    assert!(w.a.pongs().is_empty() && w.a.waiting.is_empty());
}

#[test]
fn a_connection_carries_data_both_ways_and_closes() {
    let mut w = Wire::new();
    w.b.listen(443);
    let s = w.a.connect(w.now, Ipv4([10, 0, 0, 5]), 443).unwrap();
    assert!(w.until(100, |w| w.a.state(s) == TcpState::Established));
    let mut t = None;
    assert!(w.until(100, |w| { t = w.b.accept(443); t.is_some() }), "b took the connection");
    let t = t.unwrap();
    assert_eq!(w.b.accept(443), None);
    assert_eq!(w.a.send(s, b"GET / HTTP/1.1\r\n\r\n"), 18);
    let mut buf = [0u8; 100];
    assert!(w.until(100, |w| w.b.tcp[t.0].as_ref().unwrap().recvq.len() == 18));
    assert_eq!(w.b.recv(t, &mut buf), 18);
    assert_eq!(&buf[..18], b"GET / HTTP/1.1\r\n\r\n");
    w.b.send(t, b"HTTP/1.1 200 OK\r\n\r\nhello");
    w.b.close(t);
    assert!(w.until(100, |w| w.a.finished(s) || w.a.tcp[s.0].as_ref().unwrap().recvq.len() == 24));
    assert_eq!(w.a.recv(s, &mut buf), 24);
    assert_eq!(&buf[..24], b"HTTP/1.1 200 OK\r\n\r\nhello");
    assert!(w.until(100, |w| w.a.finished(s)));
    assert_eq!(w.a.state(s), TcpState::CloseWait);
    w.a.close(s);
    assert!(w.until(100, |w| w.a.state(s) == TcpState::Closed && w.b.state(t) == TcpState::TimeWait));
    assert!(w.until(3000, |w| w.b.state(t) == TcpState::Closed));
    assert_eq!((w.a.error(s), w.b.error(t)), (None, None));
    w.a.release(s);
    assert!(w.a.tcp.iter().all(|c| c.is_none()));
}

/// Sends `size` bytes from b to a and checks every byte arrives, in order.
fn transfer(w: &mut Wire, size: usize, limit_ms: u64) -> (Vec<u8>, u64) {
    w.b.listen(80);
    let s = w.a.connect(w.now, Ipv4([10, 0, 0, 5]), 80).unwrap();
    assert!(w.until(20_000, |w| w.a.state(s) == TcpState::Established), "never connected");
    let t = loop {
        if let Some(t) = w.b.accept(80) {
            break t;
        }
        w.step();
    };
    let data: Vec<u8> = (0..size).map(|i| (i * 7 + i / 251) as u8).collect();
    let (mut given, mut got) = (0, Vec::new());
    let start = w.now;
    let mut buf = vec![0u8; 4096];
    while got.len() < size && w.now - start < limit_ms {
        if given < size {
            given += w.b.send(t, &data[given..]);
        }
        w.step();
        loop {
            let n = w.a.recv(s, &mut buf);
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
    }
    assert_eq!(got.len(), size, "only {} of {} bytes in {} ms", got.len(), size, w.now - start);
    assert!(got == data, "the bytes came out different");
    eprintln!("transfer of {size} bytes took {} ms (simulated)", w.now - start);
    (got, w.now - start)
}

#[test]
fn a_megabyte_arrives_whole() {
    let mut w = Wire::new();
    let (_, took) = transfer(&mut w, 1 << 20, 60_000);
    assert!(took < 5_000, "took {took} ms");
}

#[test]
fn what_is_lost_is_sent_again() {
    let mut w = Wire::new();
    w.lose = |n| n % 7 == 3;
    transfer(&mut w, 300_000, 600_000);
}

#[test]
fn what_comes_out_of_order_is_put_back_in_order() {
    let mut w = Wire::new();
    w.late = |n| n % 5 == 2;
    transfer(&mut w, 300_000, 600_000);
    let mut w = Wire::new();
    w.late = |n| n % 3 == 0;
    w.lose = |n| n % 11 == 5;
    transfer(&mut w, 200_000, 600_000);
}

#[test]
fn refused_reset_and_unanswered_connections_say_so() {
    let mut w = Wire::new();
    let s = w.a.connect(w.now, Ipv4([10, 0, 0, 5]), 444).unwrap(); // b doesn't listen there
    assert!(w.until(100, |w| w.a.state(s) == TcpState::Closed));
    assert_eq!(w.a.error(s), Some(TcpError::Refused));
    // Broken off by the other side.
    w.b.listen(443);
    let s = w.a.connect(w.now, Ipv4([10, 0, 0, 5]), 443).unwrap();
    let mut t = None;
    assert!(w.until(100, |w| { t = w.b.accept(443); t.is_some() }));
    w.b.abort(t.unwrap());
    assert!(w.until(100, |w| w.a.state(s) == TcpState::Closed));
    assert_eq!(w.a.error(s), Some(TcpError::Reset));
    // No one there at all: given up after six tries (1+2+4+8+16+32+64 seconds).
    let s = w.a.connect(w.now, Ipv4([10, 0, 0, 77]), 443).unwrap();
    assert!(w.until(200_000, |w| w.a.state(s) == TcpState::Closed));
    assert_eq!(w.a.error(s), Some(TcpError::TimedOut));
    // No network address: no route.
    let mut lone = Stack::new(mac(9), 3, false);
    assert_eq!(lone.connect(0, Ipv4([1, 1, 1, 1]), 443).err(), Some(TcpError::NoRoute));
}

#[test]
fn a_forged_reset_or_data_out_of_the_window_does_nothing() {
    let mut w = Wire::new();
    w.b.listen(443);
    let s = w.a.connect(w.now, Ipv4([10, 0, 0, 5]), 443).unwrap();
    assert!(w.until(100, |w| w.a.state(s) == TcpState::Established));
    let c = w.a.tcp[s.0].as_ref().unwrap();
    let (local, rcv_nxt) = (c.local, c.rcv_nxt);
    // A reset with a guessed sequence number (not exactly the next): ignored, answered with an acknowledgement.
    let mut seg = vec![0u8; 20];
    put16(&mut seg, 0, 443);
    put16(&mut seg, 2, local);
    put32(&mut seg, 4, rcv_nxt.wrapping_add(1000));
    seg[12] = 5 << 4;
    seg[13] = 0x04;
    let sum = transport_checksum(Ipv4([10, 0, 0, 5]), Ipv4([10, 0, 0, 2]), PROTO_TCP, &seg);
    put16(&mut seg, 16, sum);
    w.a.tcp_in(w.now, Ip::V4(Ipv4([10, 0, 0, 5])), Ip::V4(Ipv4([10, 0, 0, 2])), &seg);
    assert_eq!(w.a.state(s), TcpState::Established);
    // Data far outside the window: not taken.
    let mut seg = vec![0u8; 25];
    put16(&mut seg, 0, 443);
    put16(&mut seg, 2, local);
    put32(&mut seg, 4, rcv_nxt.wrapping_add(1_000_000));
    seg[12] = 5 << 4;
    seg[13] = 0x18;
    seg[20..].copy_from_slice(b"evil!");
    let sum = transport_checksum(Ipv4([10, 0, 0, 5]), Ipv4([10, 0, 0, 2]), PROTO_TCP, &seg);
    put16(&mut seg, 16, sum);
    w.a.tcp_in(w.now, Ip::V4(Ipv4([10, 0, 0, 5])), Ip::V4(Ipv4([10, 0, 0, 2])), &seg);
    let mut buf = [0u8; 10];
    assert_eq!(w.a.recv(s, &mut buf), 0);
    // A damaged segment: dropped.
    let before = w.a.stats.dropped;
    seg[21] ^= 1;
    w.a.tcp_in(w.now, Ip::V4(Ipv4([10, 0, 0, 5])), Ip::V4(Ipv4([10, 0, 0, 2])), &seg);
    assert_eq!(w.a.stats.dropped, before + 1);
}

#[test]
fn malformed_frames_are_dropped_not_crashed_on() {
    let mut st = Stack::new(mac(2), 1, true);
    st.set_config(config(2));
    let mut seed = 12345u64;
    for len in 0..300 {
        for kind in [0x0800u16, 0x0806] {
            let mut f = vec![0u8; len.max(14)];
            f[0..6].copy_from_slice(&mac(2).0);
            put16(&mut f, 12, kind);
            for b in f[14..].iter_mut() {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                *b = (seed >> 33) as u8;
            }
            if f.len() > 34 && kind == 0x0800 {
                f[14] = 0x45; // looks like IPv4 to get further in
                f[23] = [1, 6, 17][len % 3];
            }
            st.receive(0, &f[..len.min(f.len())]);
        }
    }
    st.poll(10);
}

// ------------------------------------------------------------------- DHCP, with a stand-in server

/// The UDP payload of a frame the stack sent (if it's UDP to `port`).
fn udp_to(frame: &[u8], port: u16) -> Option<(Ipv4, Vec<u8>)> {
    if frame.len() < 42 || be16(frame, 12) != 0x0800 || frame[23] != 17 {
        return None;
    }
    let ihl = (frame[14] & 15) as usize * 4;
    let u = &frame[14 + ihl..];
    if be16(u, 2) != port {
        return None;
    }
    let len = be16(u, 4) as usize;
    Some((Ipv4([frame[30], frame[31], frame[32], frame[33]]), u[8..len].to_vec()))
}

/// A frame from `from` (MAC 02:..:01, address `src`) to the stack: UDP from `sport` to `dport`.
fn udp_frame(to: Mac, src: Ipv4, dst: Ipv4, sport: u16, dport: u16, data: &[u8]) -> Vec<u8> {
    let mut u = vec![0u8; 8 + data.len()];
    put16(&mut u, 0, sport);
    put16(&mut u, 2, dport);
    let len = u.len() as u16;
    put16(&mut u, 4, len);
    u[8..].copy_from_slice(data);
    let c = transport_checksum(src, dst, 17, &u);
    put16(&mut u, 6, c);
    let mut ip = vec![0u8; 20];
    ip[0] = 0x45;
    put16(&mut ip, 2, (20 + u.len()) as u16);
    ip[8] = 64;
    ip[9] = 17;
    ip[12..16].copy_from_slice(&src.0);
    ip[16..20].copy_from_slice(&dst.0);
    let c = checksum(&[&ip]);
    put16(&mut ip, 10, c);
    ip.extend_from_slice(&u);
    let mut f = Vec::new();
    f.extend_from_slice(&to.0);
    f.extend_from_slice(&[2, 0, 0, 0, 0, 1]);
    f.extend_from_slice(&[0x08, 0x00]);
    f.extend_from_slice(&ip);
    f
}

fn dhcp_reply(request: &[u8], kind: u8, yiaddr: Ipv4, lease: u32) -> Vec<u8> {
    let mut m = vec![0u8; 240];
    m[0] = 2;
    m[1] = 1;
    m[2] = 6;
    m[4..8].copy_from_slice(&request[4..8]);
    m[16..20].copy_from_slice(&yiaddr.0);
    m[28..34].copy_from_slice(&request[28..34]);
    m[236..240].copy_from_slice(&[99, 130, 83, 99]);
    m.extend_from_slice(&[53, 1, kind, 54, 4, 10, 0, 2, 2, 1, 4, 255, 255, 255, 0, 3, 4, 10, 0, 2, 2, 6, 8, 10, 0, 2, 3, 8, 8, 8, 8]);
    m.extend_from_slice(&[51, 4]);
    m.extend_from_slice(&lease.to_be_bytes());
    m.extend_from_slice(&[15, 4, b'l', b'a', b'n', 0, 255]);
    m
}

fn option(m: &[u8], code: u8) -> Option<Vec<u8>> {
    let mut at = 240;
    while at + 1 < m.len() && m[at] != 255 {
        if m[at] == 0 {
            at += 1;
            continue;
        }
        let len = m[at + 1] as usize;
        if m[at] == code {
            return Some(m[at + 2..at + 2 + len].to_vec());
        }
        at += 2 + len;
    }
    None
}

/// The next frame it sends that isn't IPv6's (which starts on its own: its group reported, a router asked for).
fn transmit4(st: &mut Stack) -> Option<Vec<u8>> {
    while let Some(f) = st.transmit() {
        if be16(&f, 12) != 0x86DD {
            return Some(f);
        }
    }
    None
}

#[test]
fn dhcp_gets_an_address_and_keeps_it() {
    let mut st = Stack::new(mac(2), 7, true);
    let server = Ipv4([10, 0, 2, 2]);
    st.poll(0);
    // DISCOVER, broadcast.
    let f = transmit4(&mut st).expect("a discover");
    let (dst, d) = udp_to(&f, 67).unwrap();
    assert_eq!((dst, option(&d, 53)), (Ipv4::BROADCAST, Some(vec![1])));
    assert_eq!(&d[28..34], &mac(2).0);
    assert!(st.dhcp_busy() && st.config().is_none());
    // Unanswered: asked again about 4 seconds later.
    st.poll(2900);
    assert!(transmit4(&mut st).is_none());
    st.poll(5100);
    let f = transmit4(&mut st).expect("asked again");
    let (_, d) = udp_to(&f, 67).unwrap();
    // OFFER, then REQUEST for it.
    let offer = dhcp_reply(&d, 2, Ipv4([10, 0, 2, 15]), 86400);
    st.receive(5200, &udp_frame(Mac::BROADCAST, server, Ipv4::BROADCAST, 67, 68, &offer));
    let f = transmit4(&mut st).expect("a request");
    let (_, r) = udp_to(&f, 67).unwrap();
    assert_eq!(option(&r, 53), Some(vec![3]));
    assert_eq!(option(&r, 50), Some(vec![10, 0, 2, 15]));
    assert_eq!(option(&r, 54), Some(vec![10, 0, 2, 2]));
    // An answer for someone else (another transaction): ignored.
    let mut other = dhcp_reply(&r, 5, Ipv4([10, 0, 2, 99]), 86400);
    other[4] ^= 1;
    st.receive(5300, &udp_frame(Mac::BROADCAST, server, Ipv4::BROADCAST, 67, 68, &other));
    assert!(st.config().is_none());
    // ACK: the address, the network, the gateway, the DNS servers.
    let ack = dhcp_reply(&r, 5, Ipv4([10, 0, 2, 15]), 86400);
    st.receive(5400, &udp_frame(Mac::BROADCAST, server, Ipv4::BROADCAST, 67, 68, &ack));
    let c = st.config().cloned().expect("an address");
    assert_eq!(c, Config { address: Ipv4([10, 0, 2, 15]), prefix: 24, gateway: Some(server), dns: vec![Ipv4([10, 0, 2, 3]), Ipv4([8, 8, 8, 8])] });
    assert_eq!(st.domain().map(|s| s.as_str()), Some("lan"));
    assert!(!st.dhcp_busy() && st.changes() == 1);
    st.learn(5400, server, Mac([2, 0, 0, 0, 0, 1])); // (the server's card address, as ARP would find it)
    // Halfway through the lease: renewed with that server, from the address.
    st.poll(5400 + 43_200_000 + 10);
    let f = transmit4(&mut st).expect("a renewal");
    let (dst, r) = udp_to(&f, 67).unwrap();
    assert_eq!((dst, option(&r, 53)), (server, Some(vec![3])));
    assert_eq!(&r[12..16], &[10, 0, 2, 15]);
    // Refused (NAK): the address is given up, and a new one looked for.
    let nak = dhcp_reply(&r, 6, Ipv4::ANY, 0);
    st.receive(5400 + 43_200_000 + 20, &udp_frame(mac(2), server, Ipv4([10, 0, 2, 15]), 67, 68, &nak));
    assert!(st.config().is_none() && st.dhcp_busy());
    let f = transmit4(&mut st).expect("looking again");
    assert_eq!(option(&udp_to(&f, 67).unwrap().1, 53), Some(vec![1]));
}

// ------------------------------------------------------------------- DNS, with a stand-in server

fn dns_reply(q: &[u8], rcode: u8, answers: &[[u8; 4]], id_off: u16) -> Vec<u8> {
    let mut r = q.to_vec();
    let id = be16(q, 0).wrapping_add(id_off);
    put16(&mut r, 0, id);
    put16(&mut r, 2, 0x8180 | rcode as u16);
    put16(&mut r, 6, answers.len() as u16 + 1);
    // A CNAME first (as for many big sites), then the addresses.
    r.extend_from_slice(&[0xC0, 12, 0, 5, 0, 1, 0, 0, 0, 60, 0, 6, 3, b'c', b'd', b'n', 0xC0, 12]);
    for a in answers {
        r.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1, 0, 0, 1, 0, 0, 4]);
        r.extend_from_slice(a);
    }
    r
}

fn dns_stack() -> Stack {
    let mut st = Stack::new(mac(2), 9, false);
    st.set_config(config(2));
    // The DNS server is on the network; its card address is known.
    st.learn(0, Ipv4([10, 0, 0, 3]), Mac([2, 0, 0, 0, 0, 1]));
    st
}

#[test]
fn dns_finds_addresses_and_trusts_only_real_answers() {
    let mut st = dns_stack();
    let l = st.resolve(0, "API.Anthropic.com.");
    let f = st.transmit().expect("a question");
    let (dst, q) = udp_to(&f, 53).unwrap();
    assert_eq!(dst, Ipv4([10, 0, 0, 3]));
    assert_eq!(&q[12..], b"\x03api\x09anthropic\x03com\x00\x00\x01\x00\x01");
    let port = be16(&f, 34);
    assert!(port >= 49152);
    // A forged answer (wrong id), and one from elsewhere: ignored.
    st.receive(10, &udp_frame(mac(2), Ipv4([10, 0, 0, 3]), Ipv4([10, 0, 0, 2]), 53, port, &dns_reply(&q, 0, &[[6, 6, 6, 6]], 1)));
    st.receive(10, &udp_frame(mac(2), Ipv4([10, 0, 0, 66]), Ipv4([10, 0, 0, 2]), 53, port, &dns_reply(&q, 0, &[[6, 6, 6, 6]], 0)));
    assert_eq!(st.resolved(l), None);
    // The real one.
    st.receive(20, &udp_frame(mac(2), Ipv4([10, 0, 0, 3]), Ipv4([10, 0, 0, 2]), 53, port, &dns_reply(&q, 0, &[[160, 79, 104, 10], [1, 2, 3, 4]], 0)));
    assert_eq!(st.resolved(l), Some(Ok(Ip::V4(Ipv4([160, 79, 104, 10])))));
    assert_eq!(st.resolved(l), None);
    // Asked again: from what was kept, nothing sent.
    let l = st.resolve(30, "api.anthropic.com");
    assert!(st.transmit().is_none());
    assert_eq!(st.resolved(l), Some(Ok(Ip::V4(Ipv4([160, 79, 104, 10])))));
    // An address is its own answer; nonsense isn't asked.
    let l = st.resolve(40, "10.0.2.2");
    assert_eq!(st.resolved(l), Some(Ok(Ip::V4(Ipv4([10, 0, 2, 2])))));
    let l = st.resolve(40, "not a name!");
    assert_eq!(st.resolved(l), Some(Err(DnsError::BadName)));
    // No such name.
    let l = st.resolve(50, "nowhere.invalid");
    let f = st.transmit().unwrap();
    let (_, q) = udp_to(&f, 53).unwrap();
    st.receive(60, &udp_frame(mac(2), Ipv4([10, 0, 0, 3]), Ipv4([10, 0, 0, 2]), 53, be16(&f, 34), &dns_reply(&q, 3, &[], 0)));
    assert_eq!(st.resolved(l), Some(Err(DnsError::NotFound)));
}

#[test]
fn dns_asks_again_then_gives_up() {
    let mut st = dns_stack();
    let l = st.resolve(0, "slow.example");
    let mut asked = 0;
    for t in 0..20_000 {
        st.poll(t);
        while let Some(f) = st.transmit() {
            if udp_to(&f, 53).is_some() {
                asked += 1;
            }
        }
        if let Some(r) = st.resolved(l) {
            assert_eq!(r, Err(DnsError::TimedOut));
            assert_eq!(asked, 4);
            return;
        }
    }
    panic!("never gave up");
}

// ---------------------------------------------------------------- IPv6

fn v6(s: &str) -> Ipv6 {
    Ipv6::parse(s).unwrap()
}

/// An IPv6 packet in a frame, from `smac` to `dmac`, its ICMPv6 checksum made (`next` 58) or UDP's (17).
fn ip6_frame(smac: Mac, dmac: Mac, src: Ipv6, dst: Ipv6, next: u8, hops: u8, payload: &[u8]) -> Vec<u8> {
    let mut body = payload.to_vec();
    let at = if next == 58 { 2 } else { 6 };
    body[at] = 0;
    body[at + 1] = 0;
    let c = ipv6::checksum6(src, dst, next, &body);
    body[at..at + 2].copy_from_slice(&c.to_be_bytes());
    let mut f = Vec::new();
    f.extend_from_slice(&dmac.0);
    f.extend_from_slice(&smac.0);
    f.extend_from_slice(&[0x86, 0xDD, 0x60, 0, 0, 0]);
    f.extend_from_slice(&(body.len() as u16).to_be_bytes());
    f.extend_from_slice(&[next, hops]);
    f.extend_from_slice(&src.0);
    f.extend_from_slice(&dst.0);
    f.extend_from_slice(&body);
    f
}

/// A router's advertisement (from fe80::1, at card `router`): default router for `life` seconds, `prefix`/64 on
/// the link and for SLAAC, the link's MTU, and a DNS server.
fn router_advertisement(router: Mac, prefix: &str, life: u16, dns: &str) -> Vec<u8> {
    let mut m = vec![134, 0, 0, 0, 64, 0];
    m.extend_from_slice(&life.to_be_bytes());
    m.extend_from_slice(&[0; 8]);
    m.extend_from_slice(&[1, 1]);
    m.extend_from_slice(&router.0);
    m.extend_from_slice(&[3, 4, 64, 0xC0]);
    m.extend_from_slice(&86400u32.to_be_bytes());
    m.extend_from_slice(&14400u32.to_be_bytes());
    m.extend_from_slice(&[0; 4]);
    m.extend_from_slice(&v6(prefix).0);
    m.extend_from_slice(&[5, 1, 0, 0]);
    m.extend_from_slice(&1480u32.to_be_bytes());
    m.extend_from_slice(&[25, 3, 0, 0]);
    m.extend_from_slice(&3600u32.to_be_bytes());
    m.extend_from_slice(&v6(dns).0);
    ip6_frame(router, Ipv6::ALL_NODES.multicast_mac(), v6("fe80::1"), Ipv6::ALL_NODES, 58, 255, &m)
}

/// An IPv6 frame's (source, destination, next header, hop limit, payload).
fn ip6_of(f: &[u8]) -> Option<(Ipv6, Ipv6, u8, u8, Vec<u8>)> {
    if f.len() < 54 || be16(f, 12) != 0x86DD {
        return None;
    }
    let p = &f[14..];
    let len = be16(p, 4) as usize;
    let (mut src, mut dst) = (Ipv6::ANY, Ipv6::ANY);
    src.0.copy_from_slice(&p[8..24]);
    dst.0.copy_from_slice(&p[24..40]);
    Some((src, dst, p[6], p[7], p[40..40 + len].to_vec()))
}

#[test]
fn ipv6_addresses_read_and_print() {
    for (s, shown) in [
        ("fe80::1", "fe80::1"),
        ("2001:DB8::5:1", "2001:db8::5:1"),
        ("::", "::"),
        ("::1", "::1"),
        ("2001:db8:0:0:1:0:0:1", "2001:db8::1:0:0:1"),
        ("2001:db8:0:1:1:1:1:1", "2001:db8:0:1:1:1:1:1"),
        ("2001:0db8:0000:0000:0000:ff00:0042:8329", "2001:db8::ff00:42:8329"),
        ("1:0:0:2:0:0:0:3", "1:0:0:2::3"),
        ("fe80::", "fe80::"),
    ] {
        assert_eq!(format!("{}", v6(s)), shown, "{s}");
        assert_eq!(v6(shown), v6(s));
    }
    for bad in [":::", "1::2::3", "12345::", "g::", "1:2:3:4:5:6:7", "1:2:3:4:5:6:7:8:9", "", "1:2:3:4:5:6:7::8", "::1:2:3:4:5:6:7:8"] {
        assert_eq!(Ipv6::parse(bad), None, "{bad}");
    }
    assert_eq!(Ip::parse("[fe80::1]"), Some(Ip::V6(v6("fe80::1"))));
    assert_eq!(Ip::parse("10.0.2.2"), Some(Ip::V4(Ipv4([10, 0, 2, 2]))));
    assert_eq!(v6("2001:db8::1:2:3").solicited_node(), v6("ff02::1:ff02:3"));
}

#[test]
fn ipv6_gets_its_addresses_from_a_router() {
    let mut st = Stack::new(mac(2), 9, false);
    let ll = st.ipv6_addresses()[0];
    assert!(ll.is_link_local() && !st.ipv6_ready());
    // As it starts: its group reported (MLDv2, through a hop-by-hop header with Router Alert), a router asked for.
    st.poll(0);
    let frames: Vec<Vec<u8>> = core::iter::from_fn(|| st.transmit()).collect();
    let report = frames.iter().find_map(|f| ip6_of(f).filter(|p| p.1 == Ipv6::MLDV2_ROUTERS)).expect("an MLDv2 report");
    assert_eq!((report.0, report.2, report.3), (ll, 0, 1));
    assert_eq!(&report.4[..8], &[58, 0, 5, 2, 0, 0, 1, 0]);
    assert_eq!(checksum6(ll, Ipv6::MLDV2_ROUTERS, 58, &report.4[8..]), 0);
    assert_eq!(&report.4[8 + 12..8 + 28], &ll.solicited_node().0);
    let rs = frames.iter().find_map(|f| ip6_of(f).filter(|p| p.1 == Ipv6::ALL_ROUTERS)).expect("a router solicitation");
    assert_eq!((rs.0, rs.2, rs.3, rs.4[0]), (ll, 58, 255, 133));
    assert_eq!(&rs.4[8..16], &[1, 1, 0x52, 0x54, 0, 0x12, 0x34, 2]);
    // Asked again 4 seconds later, then at most 3 times in all.
    st.poll(3000);
    assert!(st.transmit().is_none());
    let mut asked = 1;
    for t in (4000..20_000).step_by(500) {
        st.poll(t);
        asked += core::iter::from_fn(|| st.transmit()).filter(|f| ip6_of(f).is_some_and(|p| p.1 == Ipv6::ALL_ROUTERS)).count();
    }
    assert_eq!(asked, 3);
    // An advertisement whose hop limit isn't 255 came from off the link: ignored.
    let changes = st.changes();
    let mut forged = router_advertisement(mac(1), "2001:db8:1::", 1800, "2001:db8:1::53");
    forged[14 + 7] = 64;
    st.receive(20_000, &forged);
    assert!(!st.ipv6_ready() && st.changes() == changes);
    // The router's: an address from its prefix (the same identifier as on the link), it as the way out, its DNS.
    st.receive(20_000, &router_advertisement(mac(1), "2001:db8:1::", 1800, "2001:db8:1::53"));
    let addresses = st.ipv6_addresses();
    assert_eq!(addresses.len(), 2);
    assert_eq!(&addresses[1].0[..8], &v6("2001:db8:1::").0[..8]);
    assert_eq!(&addresses[1].0[8..], &ll.0[8..]);
    assert!(st.ipv6_ready() && st.changes() > changes);
    assert_eq!(st.ipv6_dns(), vec![v6("2001:db8:1::53")]);
    assert_eq!(st.v6.mtu, 1480);
    // The router stops being one (a lifetime of 0): no way out, though the address stays.
    st.receive(21_000, &router_advertisement(mac(1), "2001:db8:1::", 0, "2001:db8:1::53"));
    assert!(!st.ipv6_ready() && st.ipv6_addresses().len() == 2);
}

#[test]
fn ipv6_neighbours_are_found_and_pings_answered() {
    let mut w = Wire::new();
    let b = w.b.ipv6_addresses()[0];
    w.run(5);
    w.a.ping6(w.now, b, 3, 1);
    w.run(10);
    assert_eq!(w.a.pongs(), vec![Pong { from: Ip::V6(b), id: 3, seq: 1 }]);
    // Each now knows the other's card (b from a's solicitation).
    assert!(w.a.v6_knows(b) && w.b.v6_knows(w.a.ipv6_addresses()[0]));
    // An address no one has: asked about once a second, given up on after three.
    w.a.ping6(w.now, v6("fe80::99"), 3, 2);
    w.run(5000);
    assert!(w.a.pongs().is_empty());
}

#[test]
fn a_connection_over_ipv6() {
    let mut w = Wire::new();
    for t in [0, 1] {
        let ra = router_advertisement(mac(1), "2001:db8:7::", 1800, "2001:db8:7::53");
        if t == 0 { w.a.receive(w.now, &ra) } else { w.b.receive(w.now, &ra) }
    }
    let b = w.b.ipv6_addresses()[1];
    w.b.listen(443);
    let s = w.a.connect(w.now, b, 443).unwrap();
    assert!(w.until(200, |w| w.a.state(s) == TcpState::Established));
    let mut t = None;
    assert!(w.until(100, |w| { t = w.b.accept(443); t.is_some() }));
    let t = t.unwrap();
    // Its segments carry IPv6's smaller maximum (1,440 bytes).
    let big = vec![7u8; 5000];
    assert_eq!(w.a.send(s, &big), 5000);
    let mut got = Vec::new();
    let mut buf = [0u8; 2000];
    assert!(w.until(500, |w| {
        let n = w.b.recv(t, &mut buf);
        got.extend_from_slice(&buf[..n]);
        got.len() == 5000
    }));
    assert_eq!(got, big);
    assert_eq!(w.b.tcp[t.0].as_ref().unwrap().remote, Ip::V6(w.a.ipv6_addresses()[1]));
    w.a.close(s);
    w.b.close(t);
    assert!(w.until(3000, |w| w.a.state(s) == TcpState::Closed || w.a.state(s) == TcpState::TimeWait));
}

#[test]
fn dns_finds_ipv6_addresses() {
    // Only IPv6 (no address from DHCP): AAAA asked of the router's DNS server, over IPv6.
    let mut st = Stack::new(mac(2), 4, false);
    st.receive(0, &router_advertisement(mac(1), "2001:db8:1::", 1800, "2001:db8:1::53"));
    st.learn6(0, v6("2001:db8:1::53"), mac(1));
    while st.transmit().is_some() {}
    let l = st.resolve(1, "example.org");
    let f = st.transmit().expect("a question");
    let (src, dst, next, _, u) = ip6_of(&f).expect("over IPv6");
    assert_eq!((dst, next, be16(&u, 2)), (v6("2001:db8:1::53"), 17, 53));
    assert_eq!(src, st.ipv6_addresses()[1]);
    let q = u[8..].to_vec();
    assert_eq!(&q[q.len() - 4..], &[0, 28, 0, 1]);
    let mut answer = dns_reply(&q, 0, &[], 0);
    // (dns_reply writes a CNAME and A records: an AAAA one after the CNAME, by hand.)
    answer[7] = 2;
    answer.extend_from_slice(&[0xC0, 12, 0, 28, 0, 1, 0, 0, 1, 0, 0, 16]);
    answer.extend_from_slice(&v6("2606:2800:21f:cb07:6820:80da:af6b:8b2c").0);
    let mut reply = vec![0u8; 8];
    reply[0..2].copy_from_slice(&53u16.to_be_bytes());
    reply[2..4].copy_from_slice(&u[0..2]);
    reply[4..6].copy_from_slice(&((8 + answer.len()) as u16).to_be_bytes());
    reply.extend_from_slice(&answer);
    st.receive(2, &ip6_frame(mac(1), mac(2), dst, src, 17, 64, &reply));
    assert_eq!(st.resolved(l), Some(Ok(Ip::V6(v6("2606:2800:21f:cb07:6820:80da:af6b:8b2c")))));
    // With IPv4 too: A first; a name with none (an answer without records), then AAAA.
    let mut st = dns_stack();
    st.receive(0, &router_advertisement(mac(1), "2001:db8:1::", 1800, "2001:db8:1::53"));
    while st.transmit().is_some() {}
    let l = st.resolve(1, "v6only.example");
    let f = st.transmit().expect("a question");
    let (_, q) = udp_to(&f, 53).expect("over IPv4");
    assert_eq!(&q[q.len() - 4..], &[0, 1, 0, 1]);
    st.receive(2, &udp_frame(mac(2), Ipv4([10, 0, 0, 3]), Ipv4([10, 0, 0, 2]), 53, be16(&f, 34), &dns_reply(&q, 0, &[], 0)));
    assert_eq!(st.resolved(l), None);
    st.poll(3);
    let f = core::iter::from_fn(|| st.transmit()).find(|f| udp_to(f, 53).is_some()).expect("asked again, for AAAA");
    let (_, q) = udp_to(&f, 53).unwrap();
    assert_eq!(&q[q.len() - 4..], &[0, 28, 0, 1]);
}

#[test]
fn unreachable_gives_up_a_connection_opening() {
    // A router says there's nothing at the far end (ICMPv6 "destination unreachable", port unreachable, with the
    // start of our SYN in it): the connection is given up at once, refused.
    let mut st = Stack::new(mac(2), 4, false);
    st.receive(0, &router_advertisement(mac(1), "2001:db8:1::", 1800, "2001:db8:1::53"));
    st.learn6(0, v6("fe80::1"), mac(1));
    while st.transmit().is_some() {}
    let far = v6("2001:db8:99::7");
    let s = st.connect(1, far, 443).unwrap();
    let syn = core::iter::from_fn(|| st.transmit()).find_map(|f| ip6_of(&f).filter(|p| p.2 == 6)).expect("a SYN");
    let mut m = vec![1, 4, 0, 0, 0, 0, 0, 0];
    m.extend_from_slice(&[0x60, 0, 0, 0]);
    m.extend_from_slice(&(syn.4.len() as u16).to_be_bytes());
    m.extend_from_slice(&[6, 64]);
    m.extend_from_slice(&syn.0.0);
    m.extend_from_slice(&syn.1.0);
    m.extend_from_slice(&syn.4[..8]);
    st.receive(2, &ip6_frame(mac(1), mac(2), v6("fe80::1"), syn.0, 58, 64, &m));
    assert_eq!((st.state(s), st.error(s)), (TcpState::Closed, Some(TcpError::Refused)));
}
