//! The stack against itself (two computers on one wire, which can lose and reorder frames) and against
//! stand-ins for a DHCP server and a DNS server.

use super::*;

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
    assert_eq!(w.a.pongs(), vec![Pong { from: Ipv4([10, 0, 0, 5]), id: 7, seq: 1 }]);
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
    w.a.tcp_in(w.now, Ipv4([10, 0, 0, 5]), Ipv4([10, 0, 0, 2]), &seg);
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
    w.a.tcp_in(w.now, Ipv4([10, 0, 0, 5]), Ipv4([10, 0, 0, 2]), &seg);
    let mut buf = [0u8; 10];
    assert_eq!(w.a.recv(s, &mut buf), 0);
    // A damaged segment: dropped.
    let before = w.a.stats.dropped;
    seg[21] ^= 1;
    w.a.tcp_in(w.now, Ipv4([10, 0, 0, 5]), Ipv4([10, 0, 0, 2]), &seg);
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

#[test]
fn dhcp_gets_an_address_and_keeps_it() {
    let mut st = Stack::new(mac(2), 7, true);
    let server = Ipv4([10, 0, 2, 2]);
    st.poll(0);
    // DISCOVER, broadcast.
    let f = st.transmit().expect("a discover");
    let (dst, d) = udp_to(&f, 67).unwrap();
    assert_eq!((dst, option(&d, 53)), (Ipv4::BROADCAST, Some(vec![1])));
    assert_eq!(&d[28..34], &mac(2).0);
    assert!(st.dhcp_busy() && st.config().is_none());
    // Unanswered: asked again about 4 seconds later.
    st.poll(2900);
    assert!(st.transmit().is_none());
    st.poll(5100);
    let f = st.transmit().expect("asked again");
    let (_, d) = udp_to(&f, 67).unwrap();
    // OFFER, then REQUEST for it.
    let offer = dhcp_reply(&d, 2, Ipv4([10, 0, 2, 15]), 86400);
    st.receive(5200, &udp_frame(Mac::BROADCAST, server, Ipv4::BROADCAST, 67, 68, &offer));
    let f = st.transmit().expect("a request");
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
    let f = st.transmit().expect("a renewal");
    let (dst, r) = udp_to(&f, 67).unwrap();
    assert_eq!((dst, option(&r, 53)), (server, Some(vec![3])));
    assert_eq!(&r[12..16], &[10, 0, 2, 15]);
    // Refused (NAK): the address is given up, and a new one looked for.
    let nak = dhcp_reply(&r, 6, Ipv4::ANY, 0);
    st.receive(5400 + 43_200_000 + 20, &udp_frame(mac(2), server, Ipv4([10, 0, 2, 15]), 67, 68, &nak));
    assert!(st.config().is_none() && st.dhcp_busy());
    let f = st.transmit().expect("looking again");
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
    assert_eq!(st.resolved(l), Some(Ok(Ipv4([160, 79, 104, 10]))));
    assert_eq!(st.resolved(l), None);
    // Asked again: from what was kept, nothing sent.
    let l = st.resolve(30, "api.anthropic.com");
    assert!(st.transmit().is_none());
    assert_eq!(st.resolved(l), Some(Ok(Ipv4([160, 79, 104, 10]))));
    // An address is its own answer; nonsense isn't asked.
    let l = st.resolve(40, "10.0.2.2");
    assert_eq!(st.resolved(l), Some(Ok(Ipv4([10, 0, 2, 2]))));
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
