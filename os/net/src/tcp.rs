//! TCP (RFC 9293): connections out (and, for tests and later use, in), data in order, what's lost sent again.
//!
//! What's sent waits until it's acknowledged; after a timeout everything not acknowledged is sent again (the
//! timeout measured from each connection's round trips, RFC 6298, doubling each time, up to a minute), and
//! three repeated acknowledgements send the first missing piece at once. Sending starts with ten segments in
//! flight and grows (slow start and congestion avoidance, RFC 5681). What arrives out of order is kept until
//! what's missing comes. No window scaling, so at most 64 KB is ever in flight each way: plenty for
//! conversations with Claude.

use crate::{be16, be32, put16, put32, transport_checksum, Ipv4, Stack, PROTO_TCP};
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    Closing,
    TimeWait,
    CloseWait,
    LastAck,
    Closed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// The other side said no (nothing listens there).
    Refused,
    /// The other side broke the connection off.
    Reset,
    /// The other side stopped answering.
    TimedOut,
    /// There's no way to reach it (no network address, or no gateway).
    NoRoute,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Error::Refused => "the connection was refused",
            Error::Reset => "the connection was broken off",
            Error::TimedOut => "the other side stopped answering",
            Error::NoRoute => "there's no way to reach it",
        })
    }
}

const FIN: u8 = 0x01;
const SYN: u8 = 0x02;
const RST: u8 = 0x04;
const PSH: u8 = 0x08;
const ACK: u8 = 0x10;

/// The biggest segment we take (Ethernet's 1500, less IP's and TCP's headers).
const OUR_MSS: u16 = 1460;
/// How much we take in before the program reads it.
const RECV_MAX: usize = 65535;
/// How much a program can hand over before it's sent.
const SEND_MAX: usize = 256 * 1024;
const OOO_MAX: usize = 32;
const RTO_FIRST: u64 = 1000;
const RTO_MIN: u64 = 200;
const RTO_MAX: u64 = 60_000;
const SYN_TRIES: u32 = 6;
const TRIES: u32 = 12;
const TIME_WAIT_MS: u64 = 2000;

fn lt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}
fn le(a: u32, b: u32) -> bool {
    !lt(b, a)
}
fn gt(a: u32, b: u32) -> bool {
    lt(b, a)
}
fn ge(a: u32, b: u32) -> bool {
    !lt(a, b)
}

/// A segment as it came in.
pub(crate) struct Seg<'a> {
    seq: u32,
    ack: u32,
    flags: u8,
    window: u16,
    mss: Option<u16>,
    data: &'a [u8],
}

/// A segment to send.
pub(crate) struct Out {
    seq: u32,
    ack: u32,
    flags: u8,
    window: u16,
    mss: bool,
    data: Vec<u8>,
}

pub(crate) struct Conn {
    pub(crate) state: State,
    pub(crate) error: Option<Error>,
    pub(crate) local: u16,
    pub(crate) remote: Ipv4,
    pub(crate) remote_port: u16,
    pub(crate) accepted: bool,
    iss: u32,
    snd_una: u32,
    snd_nxt: u32,
    snd_wnd: u32,
    snd_wl1: u32,
    snd_wl2: u32,
    /// The furthest anything has been sent (snd_nxt goes back to snd_una when everything is sent again).
    snd_max: u32,
    pub(crate) rcv_nxt: u32,
    mss: usize,
    /// From snd_una: the bytes sent and not yet acknowledged (`sent` of them), then those not sent yet.
    sendq: VecDeque<u8>,
    sent: usize,
    pub(crate) recvq: VecDeque<u8>,
    ooo: Vec<(u32, Vec<u8>)>,
    close_wanted: bool,
    /// Our FIN's place in the sequence, once it has been sent.
    fin_seq: Option<u32>,
    /// Whether it's out now (it isn't again after a timeout, until all the data before it has gone again).
    fin_sent: bool,
    rto: u64,
    srtt: Option<u64>,
    rttvar: u64,
    timer: Option<u64>,
    tries: u32,
    probe: bool,
    timing: Option<(u32, u64)>,
    cwnd: usize,
    ssthresh: usize,
    dup_acks: u32,
    ack_due: bool,
    advertised: usize,
    until: u64,
}

impl Conn {
    fn new(local: u16, remote: Ipv4, remote_port: u16, iss: u32, state: State) -> Conn {
        Conn {
            state,
            error: None,
            local,
            remote,
            remote_port,
            accepted: false,
            iss,
            snd_una: iss,
            snd_nxt: iss.wrapping_add(1),
            snd_wnd: 0,
            snd_wl1: 0,
            snd_wl2: 0,
            snd_max: iss.wrapping_add(1),
            rcv_nxt: 0,
            mss: 536,
            sendq: VecDeque::new(),
            sent: 0,
            recvq: VecDeque::new(),
            ooo: Vec::new(),
            close_wanted: false,
            fin_seq: None,
            fin_sent: false,
            rto: RTO_FIRST,
            srtt: None,
            rttvar: 0,
            timer: None,
            tries: 0,
            probe: false,
            timing: None,
            cwnd: 10 * 536,
            ssthresh: usize::MAX,
            dup_acks: 0,
            ack_due: false,
            advertised: 0,
            until: 0,
        }
    }

    fn window(&self) -> usize {
        RECV_MAX.saturating_sub(self.recvq.len())
    }

    fn close_now(&mut self, error: Option<Error>) {
        self.state = State::Closed;
        if self.error.is_none() {
            self.error = error;
        }
        self.timer = None;
        self.sendq.clear();
        self.sent = 0;
        self.ooo.clear();
    }

    fn sample(&mut self, rtt: u64) {
        match self.srtt {
            None => {
                self.srtt = Some(rtt);
                self.rttvar = rtt / 2;
            }
            Some(s) => {
                self.rttvar = (3 * self.rttvar + s.abs_diff(rtt)) / 4;
                self.srtt = Some((7 * s + rtt) / 8);
            }
        }
        self.rto = (self.srtt.unwrap_or(rtt) + (4 * self.rttvar).max(10)).clamp(RTO_MIN, RTO_MAX);
    }

    fn set_mss(&mut self, mss: Option<u16>) {
        self.mss = mss.map_or(536, |m| m as usize).clamp(64, OUR_MSS as usize);
        self.cwnd = 10 * self.mss;
    }

    /// A segment for this connection.
    fn input(&mut self, now: u64, seg: &Seg) -> Option<Out> {
        let has_ack = seg.flags & ACK != 0;
        if self.state == State::SynSent {
            if has_ack && !(gt(seg.ack, self.iss) && le(seg.ack, self.snd_nxt)) {
                if seg.flags & RST != 0 {
                    return None;
                }
                return Some(Out { seq: seg.ack, ack: 0, flags: RST, window: 0, mss: false, data: Vec::new() });
            }
            if seg.flags & RST != 0 {
                if has_ack {
                    self.close_now(Some(Error::Refused));
                }
                return None;
            }
            if seg.flags & SYN != 0 && has_ack {
                self.rcv_nxt = seg.seq.wrapping_add(1);
                self.snd_una = seg.ack;
                self.snd_wnd = seg.window as u32;
                self.snd_wl1 = seg.seq;
                self.snd_wl2 = seg.ack;
                self.set_mss(seg.mss);
                if self.tries == 0
                    && let Some((_, at)) = self.timing {
                        self.sample(now.saturating_sub(at));
                    }
                self.timing = None;
                self.timer = None;
                self.tries = 0;
                self.state = State::Established;
                self.ack_due = true;
            }
            return None;
        }
        if self.state == State::Closed {
            return None;
        }

        // Is it in the window we offered?
        let len = seg.data.len() as u32 + (seg.flags & SYN != 0) as u32 + (seg.flags & FIN != 0) as u32;
        let wnd = self.window() as u32;
        let in_window = |s: u32| ge(s, self.rcv_nxt) && lt(s, self.rcv_nxt.wrapping_add(wnd));
        let acceptable = if len == 0 {
            if wnd == 0 { seg.seq == self.rcv_nxt } else { seg.seq == self.rcv_nxt || in_window(seg.seq) }
        } else {
            wnd > 0 && (in_window(seg.seq) || in_window(seg.seq.wrapping_add(len - 1)) || lt(seg.seq, self.rcv_nxt))
        };
        if !acceptable {
            if seg.flags & RST == 0 {
                self.ack_due = true;
            }
            return None;
        }
        if seg.flags & RST != 0 {
            if seg.seq == self.rcv_nxt {
                let error = match self.state {
                    State::SynReceived => Some(Error::Refused),
                    State::Closing | State::TimeWait | State::LastAck => None,
                    _ => Some(Error::Reset),
                };
                self.close_now(error);
            } else {
                self.ack_due = true; // (RFC 5961: a reset that isn't exactly in place gets an acknowledgement)
            }
            return None;
        }
        if seg.flags & SYN != 0 {
            if self.state == State::SynReceived && seg.seq.wrapping_add(1) == self.rcv_nxt {
                self.timer = Some(now); // their SYN again: our SYN-ACK goes again
                return None;
            }
            self.ack_due = true;
            return None;
        }
        if !has_ack {
            return None;
        }
        if self.state == State::SynReceived {
            if seg.ack != self.snd_nxt {
                return Some(Out { seq: seg.ack, ack: 0, flags: RST, window: 0, mss: false, data: Vec::new() });
            }
            self.snd_una = seg.ack;
            self.snd_wnd = seg.window as u32;
            self.snd_wl1 = seg.seq;
            self.snd_wl2 = seg.ack;
            self.timer = None;
            self.tries = 0;
            self.state = State::Established;
        }

        // What they've received (perhaps more than we've sent again since a timeout: what they already have
        // isn't sent a third time).
        if gt(seg.ack, self.snd_max) {
            self.ack_due = true;
            return None;
        }
        let in_flight = self.snd_max.wrapping_sub(self.snd_una) as usize;
        if gt(seg.ack, self.snd_una) {
            let fin_acked = self.fin_seq.is_some_and(|f| gt(seg.ack, f));
            let data_end = match self.fin_seq {
                Some(f) if fin_acked => f,
                _ => seg.ack,
            };
            let bytes = (data_end.wrapping_sub(self.snd_una) as usize).min(self.sendq.len());
            self.sendq.drain(..bytes);
            self.sent = self.sent.saturating_sub(bytes);
            self.snd_una = seg.ack;
            if lt(self.snd_nxt, self.snd_una) {
                self.snd_nxt = self.snd_una;
                self.sent = 0;
            }
            if fin_acked {
                self.fin_sent = true;
                self.snd_nxt = self.snd_una;
            }
            if let Some((s, at)) = self.timing
                && ge(seg.ack, s)
            {
                self.sample(now.saturating_sub(at));
                self.timing = None;
            }
            self.tries = 0;
            self.dup_acks = 0;
            self.probe = false;
            self.timer = if self.snd_una != self.snd_max { Some(now + self.rto) } else { None };
            if self.cwnd < self.ssthresh {
                self.cwnd += bytes.min(self.mss);
            } else {
                self.cwnd += (self.mss * self.mss / self.cwnd.max(1)).max(1);
            }
            if fin_acked {
                match self.state {
                    State::FinWait1 => self.state = State::FinWait2,
                    State::Closing => {
                        self.state = State::TimeWait;
                        self.until = now + TIME_WAIT_MS;
                    }
                    State::LastAck => {
                        self.close_now(None);
                        return None;
                    }
                    _ => {}
                }
            }
        } else if seg.ack == self.snd_una && seg.data.is_empty() && seg.flags & FIN == 0 && in_flight > 0 {
            self.dup_acks += 1;
            if self.dup_acks == 3 && !self.sendq.is_empty() {
                // Three of the same: the first piece not acknowledged was lost. Sent again at once.
                self.ssthresh = (in_flight / 2).max(2 * self.mss);
                self.cwnd = self.ssthresh;
                self.timing = None;
                let n = self.sendq.len().min(self.mss);
                let data: Vec<u8> = self.sendq.range(..n).copied().collect();
                return Some(Out { seq: self.snd_una, ack: self.rcv_nxt, flags: ACK, window: self.window().min(65535) as u16, mss: false, data });
            }
        }
        if lt(self.snd_wl1, seg.seq) || (self.snd_wl1 == seg.seq && le(self.snd_wl2, seg.ack)) {
            self.snd_wnd = seg.window as u32;
            self.snd_wl1 = seg.seq;
            self.snd_wl2 = seg.ack;
        }

        // Their data, in order.
        let mut fin_at = if seg.flags & FIN != 0 { Some(seg.seq.wrapping_add(seg.data.len() as u32)) } else { None };
        if matches!(self.state, State::Established | State::FinWait1 | State::FinWait2) && !seg.data.is_empty() {
            self.ack_due = true;
            let mut seq = seg.seq;
            let mut data = seg.data;
            if lt(seq, self.rcv_nxt) {
                let skip = self.rcv_nxt.wrapping_sub(seq) as usize;
                data = &data[skip.min(data.len())..];
                seq = self.rcv_nxt;
            }
            if seq == self.rcv_nxt {
                let take = data.len().min(self.window());
                self.recvq.extend(&data[..take]);
                self.rcv_nxt = self.rcv_nxt.wrapping_add(take as u32);
                if take < data.len() {
                    fin_at = None; // (what didn't fit comes again, and its FIN with it)
                }
                self.pull_ooo();
            } else if !data.is_empty() && self.ooo.len() < OOO_MAX && !self.ooo.iter().any(|(s, _)| *s == seq) {
                self.ooo.push((seq, data.to_vec()));
                fin_at = None;
            }
        }
        if let Some(at) = fin_at
            && at == self.rcv_nxt
        {
            self.rcv_nxt = self.rcv_nxt.wrapping_add(1);
            self.ack_due = true;
            match self.state {
                State::Established => self.state = State::CloseWait,
                State::FinWait1 => {
                    self.state = State::Closing;
                }
                State::FinWait2 => {
                    self.state = State::TimeWait;
                    self.until = now + TIME_WAIT_MS;
                }
                State::TimeWait => self.until = now + TIME_WAIT_MS,
                _ => {}
            }
        }
        None
    }

    /// Moves what arrived early into place once what was missing has come.
    fn pull_ooo(&mut self) {
        while let Some(i) = self.ooo.iter().position(|(s, d)| le(*s, self.rcv_nxt) && gt(s.wrapping_add(d.len() as u32), self.rcv_nxt)) {
            let (s, d) = self.ooo.swap_remove(i);
            let skip = self.rcv_nxt.wrapping_sub(s) as usize;
            let take = (d.len() - skip).min(self.window());
            self.recvq.extend(&d[skip..skip + take]);
            self.rcv_nxt = self.rcv_nxt.wrapping_add(take as u32);
            if take < d.len() - skip {
                break;
            }
        }
        let rcv_nxt = self.rcv_nxt;
        self.ooo.retain(|(s, d)| gt(s.wrapping_add(d.len() as u32), rcv_nxt));
    }

    /// What to send now: a SYN again, data, a FIN, an acknowledgement.
    fn output(&mut self, now: u64) -> Vec<Out> {
        let mut out = Vec::new();
        if self.state == State::Closed {
            return out;
        }
        if self.state == State::TimeWait
            && now >= self.until {
                self.state = State::Closed;
                return out;
            }
        let window = self.window().min(65535) as u16;
        // Timeouts.
        if let Some(t) = self.timer
            && now >= t
        {
            self.tries += 1;
            let limit = if matches!(self.state, State::SynSent | State::SynReceived) { SYN_TRIES } else { TRIES };
            if self.tries > limit {
                self.close_now(Some(Error::TimedOut));
                return out;
            }
            self.rto = (self.rto * 2).min(RTO_MAX);
            self.timer = Some(now + self.rto);
            self.timing = None;
            match self.state {
                State::SynSent => {
                    out.push(Out { seq: self.iss, ack: 0, flags: SYN, window, mss: true, data: Vec::new() });
                    return out;
                }
                State::SynReceived => {
                    out.push(Out { seq: self.iss, ack: self.rcv_nxt, flags: SYN | ACK, window, mss: true, data: Vec::new() });
                    return out;
                }
                _ => {
                    if self.snd_wnd == 0 && self.snd_una == self.snd_max && !self.sendq.is_empty() {
                        self.probe = true; // their window is shut: one byte to ask whether it's open again
                    } else {
                        // Everything not acknowledged goes again, starting small.
                        let in_flight = self.snd_max.wrapping_sub(self.snd_una) as usize;
                        self.ssthresh = (in_flight / 2).max(2 * self.mss);
                        self.cwnd = self.mss;
                        self.snd_nxt = self.snd_una;
                        self.sent = 0;
                        if self.fin_seq.is_some_and(|f| ge(f, self.snd_una)) {
                            self.fin_sent = false;
                        }
                    }
                }
            }
        }
        if matches!(self.state, State::SynSent | State::SynReceived) {
            return out;
        }
        // Data.
        loop {
            let unsent = self.sendq.len() - self.sent;
            let room = (self.snd_wnd as usize).min(self.cwnd);
            let room = if self.probe && self.sent == 0 { room.max(1) } else { room };
            if unsent == 0 || self.sent >= room || self.fin_sent {
                break;
            }
            let n = unsent.min(self.mss).min(room - self.sent);
            let data: Vec<u8> = self.sendq.range(self.sent..self.sent + n).copied().collect();
            let last = self.sent + n == self.sendq.len();
            if self.timing.is_none() {
                self.timing = Some((self.snd_nxt, now));
            }
            out.push(Out { seq: self.snd_nxt, ack: self.rcv_nxt, flags: ACK | if last { PSH } else { 0 }, window, mss: false, data });
            self.snd_nxt = self.snd_nxt.wrapping_add(n as u32);
            if gt(self.snd_nxt, self.snd_max) {
                self.snd_max = self.snd_nxt;
            }
            self.sent += n;
            self.ack_due = false;
            self.probe = false;
            if self.timer.is_none() {
                self.timer = Some(now + self.rto);
            }
        }
        if self.snd_wnd == 0 && self.sent < self.sendq.len() && self.timer.is_none() {
            self.timer = Some(now + self.rto); // (to probe a shut window)
        }
        // The end.
        if self.close_wanted && !self.fin_sent && self.sent == self.sendq.len()
            && matches!(self.state, State::Established | State::CloseWait | State::FinWait1 | State::Closing | State::LastAck)
        {
            out.push(Out { seq: self.snd_nxt, ack: self.rcv_nxt, flags: FIN | ACK, window, mss: false, data: Vec::new() });
            self.fin_seq = Some(self.snd_nxt);
            self.snd_nxt = self.snd_nxt.wrapping_add(1);
            if gt(self.snd_nxt, self.snd_max) {
                self.snd_max = self.snd_nxt;
            }
            self.fin_sent = true;
            self.ack_due = false;
            self.state = match self.state {
                State::Established => State::FinWait1,
                State::CloseWait => State::LastAck,
                s => s,
            };
            if self.timer.is_none() {
                self.timer = Some(now + self.rto);
            }
        }
        if self.ack_due {
            out.push(Out { seq: self.snd_nxt, ack: self.rcv_nxt, flags: ACK, window, mss: false, data: Vec::new() });
            self.ack_due = false;
        }
        if !out.is_empty() {
            self.advertised = window as usize;
        }
        out
    }
}

/// A connection, as the program using it knows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Socket(pub(crate) usize);

impl Stack {
    fn conn(&self, s: Socket) -> Option<&Conn> {
        self.tcp.get(s.0).and_then(|c| c.as_ref())
    }

    fn conn_mut(&mut self, s: Socket) -> Option<&mut Conn> {
        self.tcp.get_mut(s.0).and_then(|c| c.as_mut())
    }

    fn slot(&mut self, c: Conn) -> Socket {
        match self.tcp.iter().position(|c| c.is_none()) {
            Some(i) => {
                self.tcp[i] = Some(c);
                Socket(i)
            }
            None => {
                self.tcp.push(Some(c));
                Socket(self.tcp.len() - 1)
            }
        }
    }

    fn free_port(&mut self) -> u16 {
        loop {
            let p = 49152 + (self.random.next() % 16384) as u16;
            if !self.tcp.iter().flatten().any(|c| c.local == p) && !self.listening.contains(&p) {
                return p;
            }
        }
    }

    /// Opens a connection to `to`, port `port`. It's ready once `state` says Established.
    pub fn connect(&mut self, now: u64, to: Ipv4, port: u16) -> Result<Socket, Error> {
        let reachable = match &self.config {
            Some(c) => c.on_link(to) || c.gateway.is_some(),
            None => false,
        };
        if !reachable {
            return Err(Error::NoRoute);
        }
        let local = self.free_port();
        let iss = self.random.next() as u32;
        let mut c = Conn::new(local, to, port, iss, State::SynSent);
        c.timer = Some(now + c.rto);
        c.timing = Some((iss, now));
        let window = c.window().min(65535) as u16;
        let s = self.slot(c);
        self.send_segment(now, to, local, port, Out { seq: iss, ack: 0, flags: SYN, window, mss: true, data: Vec::new() });
        Ok(s)
    }

    /// Takes connections to `port` from now on (`accept` hands them over).
    pub fn listen(&mut self, port: u16) {
        if !self.listening.contains(&port) {
            self.listening.push(port);
        }
    }

    /// A connection that came in to `port`, once.
    pub fn accept(&mut self, port: u16) -> Option<Socket> {
        let i = self.tcp.iter().position(|c| {
            c.as_ref().is_some_and(|c| c.local == port && !c.accepted && c.state != State::SynReceived && c.state != State::Closed)
        })?;
        self.tcp[i].as_mut()?.accepted = true;
        Some(Socket(i))
    }

    pub fn state(&self, s: Socket) -> State {
        self.conn(s).map_or(State::Closed, |c| c.state)
    }

    /// Why it closed, if it didn't close normally.
    pub fn error(&self, s: Socket) -> Option<Error> {
        self.conn(s).and_then(|c| c.error)
    }

    /// Hands over data to send: how much was taken (all of it unless a lot is still waiting to go).
    pub fn send(&mut self, s: Socket, data: &[u8]) -> usize {
        let now = self.now;
        let Some(c) = self.conn_mut(s) else { return 0 };
        if c.close_wanted || !matches!(c.state, State::SynSent | State::SynReceived | State::Established | State::CloseWait) {
            return 0;
        }
        let n = data.len().min(SEND_MAX.saturating_sub(c.sendq.len()));
        c.sendq.extend(&data[..n]);
        self.tcp_output(now, s.0);
        n
    }

    /// How much more `send` would take now.
    pub fn send_room(&self, s: Socket) -> usize {
        self.conn(s).map_or(0, |c| SEND_MAX.saturating_sub(c.sendq.len()))
    }

    /// Whether everything handed to `send` has been received by the other side.
    pub fn all_sent(&self, s: Socket) -> bool {
        self.conn(s).is_none_or(|c| c.sendq.is_empty())
    }

    /// Takes what has arrived, in order: how many bytes.
    pub fn recv(&mut self, s: Socket, buf: &mut [u8]) -> usize {
        let now = self.now;
        let Some(c) = self.conn_mut(s) else { return 0 };
        let n = buf.len().min(c.recvq.len());
        for (b, v) in buf.iter_mut().zip(c.recvq.drain(..n)) {
            *b = v;
        }
        // The window opened a good deal: they're told, so they can send more.
        if n > 0 && matches!(c.state, State::Established | State::FinWait1 | State::FinWait2) && c.window() >= c.advertised + 2 * c.mss {
            c.ack_due = true;
            self.tcp_output(now, s.0);
        }
        n
    }

    /// Whether the other side has finished sending (and everything it sent has been read).
    pub fn finished(&self, s: Socket) -> bool {
        self.conn(s).is_none_or(|c| {
            c.recvq.is_empty() && matches!(c.state, State::CloseWait | State::LastAck | State::Closing | State::TimeWait | State::Closed)
        })
    }

    /// Closes it once everything handed over is sent.
    pub fn close(&mut self, s: Socket) {
        let now = self.now;
        let Some(c) = self.conn_mut(s) else { return };
        match c.state {
            State::SynSent => c.close_now(None),
            _ => c.close_wanted = true,
        }
        self.tcp_output(now, s.0);
    }

    /// Breaks it off at once, and forgets it.
    pub fn abort(&mut self, s: Socket) {
        let now = self.now;
        if let Some(c) = self.conn(s)
            && !matches!(c.state, State::Closed | State::SynSent | State::TimeWait)
        {
            let (to, local, port, seq) = (c.remote, c.local, c.remote_port, c.snd_nxt);
            self.send_segment(now, to, local, port, Out { seq, ack: 0, flags: RST, window: 0, mss: false, data: Vec::new() });
        }
        if let Some(slot) = self.tcp.get_mut(s.0) {
            *slot = None;
        }
    }

    /// Forgets a connection that's done with (one still open is broken off).
    pub fn release(&mut self, s: Socket) {
        match self.conn(s).map(|c| c.state) {
            Some(State::Closed) | Some(State::TimeWait) | None => {
                if let Some(slot) = self.tcp.get_mut(s.0) {
                    *slot = None;
                }
            }
            Some(_) => self.abort(s),
        }
    }

    pub(crate) fn tcp_in(&mut self, now: u64, src: Ipv4, dst: Ipv4, p: &[u8]) {
        if p.len() < 20 || transport_checksum(src, dst, PROTO_TCP, p) != 0 {
            self.stats.dropped += 1;
            return;
        }
        let off = (p[12] >> 4) as usize * 4;
        if off < 20 || off > p.len() {
            self.stats.dropped += 1;
            return;
        }
        let (sport, dport) = (be16(p, 0), be16(p, 2));
        let mut mss = None;
        let mut at = 20;
        while at < off {
            match p[at] {
                0 => break,
                1 => at += 1,
                kind => {
                    let Some(&len) = p.get(at + 1) else { break };
                    if len < 2 || at + len as usize > off {
                        break;
                    }
                    if kind == 2 && len == 4 {
                        mss = Some(be16(p, at + 2));
                    }
                    at += len as usize;
                }
            }
        }
        let seg = Seg { seq: be32(p, 4), ack: be32(p, 8), flags: p[13] & 0x3F, window: be16(p, 14), mss, data: &p[off..] };
        let found = self.tcp.iter().position(|c| {
            c.as_ref().is_some_and(|c| c.local == dport && c.remote == src && c.remote_port == sport && c.state != State::Closed)
        });
        let i = match found {
            Some(i) => i,
            None if self.listening.contains(&dport) && seg.flags & (SYN | ACK | RST) == SYN => {
                let iss = self.random.next() as u32;
                let mut c = Conn::new(dport, src, sport, iss, State::SynReceived);
                c.rcv_nxt = seg.seq.wrapping_add(1);
                c.set_mss(seg.mss);
                c.snd_wnd = seg.window as u32;
                c.timer = Some(now + c.rto);
                let window = c.window().min(65535) as u16;
                let ack = c.rcv_nxt;
                self.slot(c);
                self.send_segment(now, src, dport, sport, Out { seq: iss, ack, flags: SYN | ACK, window, mss: true, data: Vec::new() });
                return;
            }
            None => {
                // Nothing open there: it's told so (unless it was itself a reset).
                self.stats.dropped += 1;
                if seg.flags & RST == 0 {
                    let out = if seg.flags & ACK != 0 {
                        Out { seq: seg.ack, ack: 0, flags: RST, window: 0, mss: false, data: Vec::new() }
                    } else {
                        let len = seg.data.len() as u32 + (seg.flags & SYN != 0) as u32 + (seg.flags & FIN != 0) as u32;
                        Out { seq: 0, ack: seg.seq.wrapping_add(len), flags: RST | ACK, window: 0, mss: false, data: Vec::new() }
                    };
                    self.send_segment(now, src, dport, sport, out);
                }
                return;
            }
        };
        let Some(c) = self.tcp[i].as_mut() else { return };
        if let Some(reply) = c.input(now, &seg) {
            let (to, local, port) = (c.remote, c.local, c.remote_port);
            self.send_segment(now, to, local, port, reply);
        }
        self.tcp_output(now, i);
    }

    pub(crate) fn tcp_poll(&mut self, now: u64) {
        for i in 0..self.tcp.len() {
            self.tcp_output(now, i);
        }
    }

    fn tcp_output(&mut self, now: u64, i: usize) {
        let Some(c) = self.tcp.get_mut(i).and_then(|c| c.as_mut()) else { return };
        let (to, local, port) = (c.remote, c.local, c.remote_port);
        for out in c.output(now) {
            self.send_segment(now, to, local, port, out);
        }
    }

    fn send_segment(&mut self, now: u64, to: Ipv4, local: u16, port: u16, out: Out) {
        let src = self.config.as_ref().map_or(Ipv4::ANY, |c| c.address);
        let head = if out.mss { 24 } else { 20 };
        let mut p = vec![0u8; head + out.data.len()];
        put16(&mut p, 0, local);
        put16(&mut p, 2, port);
        put32(&mut p, 4, out.seq);
        put32(&mut p, 8, out.ack);
        p[12] = ((head / 4) as u8) << 4;
        p[13] = out.flags;
        put16(&mut p, 14, out.window);
        if out.mss {
            p[20] = 2;
            p[21] = 4;
            put16(&mut p, 22, OUR_MSS);
        }
        p[head..].copy_from_slice(&out.data);
        let c = transport_checksum(src, to, PROTO_TCP, &p);
        put16(&mut p, 16, c);
        self.send_ip(now, to, PROTO_TCP, &p);
    }
}
