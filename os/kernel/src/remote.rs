//! A question to Claude, through Anthropic's Messages API, with the person's own key (the spark beside + chooses who
//! answers: Chat Computer's own model, or Claude): the request goes over Chat Computer's own TCP and TLS 1.3, as
//! web.rs fetches pages (the address found with DNS, the certificate checked against Mozilla's authorities for
//! api.anthropic.com), handed to TLS once the secure connection is open and then wiped (it holds the key), and the
//! answer read as it streams (server-sent events: claude::api::Stream), its words given out as they come. Moved on
//! from the main loop (`Call::step`), never waiting, so the screen and the keyboard go on meanwhile.
//!
//! A test build with a certificate authority of its own (check.py --claude) trusts that one instead of Mozilla's,
//! and asks a stand-in on the computer running the virtual PC (10.0.2.2), still checking its certificate for
//! api.anthropic.com.

use crate::web::{network_ready, no_address, no_network, roots, test_build, test_port, test_server};
use crate::{clock, network, random, time};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use claude::api::{self, Stream, ToolUse};
use claude::json::Json;
use net::tcp::{Socket, State as TcpState};
use net::Stack;

/// Waiting for a network address; finding the API and connecting to it; setting up the secure connection; silence
/// once asked (Claude at its highest effort can think for minutes, but the stream says it's there every few seconds).
const NETWORK_MS: u64 = 30_000;
const CONNECT_MS: u64 = 20_000;
const HANDSHAKE_MS: u64 = 30_000;
const SILENCE_MS: u64 = 180_000;

/// What came of the question so far.
pub enum Got {
    /// More of the answer's words.
    Words(String),
    /// Claude began thinking.
    Thinking,
    /// The answer is whole, or it failed (why, for the person).
    Over(Result<Answered, String>),
}

/// A whole answer: its content blocks as they came (to go back to the API in the conversation: its thinking with
/// them), why it stopped, and the tools it called.
pub struct Answered {
    pub content: Vec<Json>,
    pub stop: Option<String>,
    pub calls: Vec<ToolUse>,
}

enum Phase {
    /// Waiting for a network address, since.
    Network(u64),
    Looking(net::Lookup, u64),
    Connecting(Socket, u64),
    Talking(Box<Talk>),
    Over,
}

struct Talk {
    socket: Socket,
    tls: Box<tls::Client>,
    /// For the API, not yet taken by TCP.
    out: Vec<u8>,
    sent: bool,
    /// When the API last sent anything.
    heard: u64,
}

pub struct Call {
    /// The request, key and all, until it's handed to TLS (then wiped).
    request: Vec<u8>,
    phase: Phase,
    stream: Stream,
    /// How many characters of words came (for the log).
    chars: usize,
}

/// Where the API is (its name, for its certificate).
fn host() -> &'static str {
    api::HOST
}

impl Call {
    pub fn new(request: Vec<u8>) -> Call {
        Call { request, phase: Phase::Network(time::uptime_ms()), stream: Stream::new(), chars: 0 }
    }

    /// Moves the question on as far as it goes now; true once it's over (the last of `out` says how).
    pub fn step(&mut self, out: &mut Vec<Got>) -> bool {
        if matches!(self.phase, Phase::Over) {
            return true;
        }
        let now = time::uptime_ms();
        let over = network::with(|stack| self.step_with(now, stack, out));
        let over = match over {
            Some(o) => o,
            None => {
                out.push(Got::Over(Err(no_network())));
                true
            }
        };
        if over {
            wipe(&mut self.request);
            self.phase = Phase::Over;
        }
        over
    }

    /// Stops it (the connection broken off).
    pub fn stop(&mut self) {
        wipe(&mut self.request);
        let phase = core::mem::replace(&mut self.phase, Phase::Over);
        let socket = match phase {
            Phase::Connecting(s, _) => Some(s),
            Phase::Talking(t) => Some(t.socket),
            _ => None,
        };
        if let Some(s) = socket {
            network::with(|stack| stack.abort(s));
        }
    }

    fn step_with(&mut self, now: u64, stack: &mut Stack, out: &mut Vec<Got>) -> bool {
        loop {
            match &mut self.phase {
                Phase::Over => return true,
                Phase::Network(since) => {
                    if !network_ready(stack, now.saturating_sub(*since)) {
                        if now.saturating_sub(*since) > NETWORK_MS {
                            return fail(out, no_address());
                        }
                        return false;
                    }
                    if test_build() {
                        let to = test_server(stack);
                        match stack.connect(now, to, test_port()) {
                            Ok(s) => self.phase = Phase::Connecting(s, now),
                            Err(e) => return fail(out, format!("Couldn't connect to Claude's API: {e}.")),
                        }
                    } else {
                        self.phase = Phase::Looking(stack.resolve(now, host()), now);
                    }
                }
                Phase::Looking(lookup, since) => match stack.resolved(*lookup) {
                    None if now.saturating_sub(*since) > CONNECT_MS => {
                        return fail(out, format!("Couldn't find {} on the network: no answer.", host()));
                    }
                    None => return false,
                    Some(Ok(ip)) => match stack.connect(now, ip, 443) {
                        Ok(s) => self.phase = Phase::Connecting(s, now),
                        Err(e) => return fail(out, format!("Couldn't connect to Claude's API: {e}.")),
                    },
                    Some(Err(e)) => return fail(out, format!("Couldn't find {} on the network: {e}.", host())),
                },
                Phase::Connecting(s, since) => {
                    let s = *s;
                    match stack.state(s) {
                        TcpState::Established => {
                            let Some(when) = clock::now() else {
                                stack.release(s);
                                return fail(
                                    out,
                                    String::from("This PC's clock couldn't be read, so Chat Computer can't check it's really talking to Claude."),
                                );
                            };
                            let mut tls = Box::new(tls::Client::new(host(), when, roots(), &mut random::fill));
                            tls.allow_clock_error(clock::error());
                            let first = tls.outgoing();
                            self.phase = Phase::Talking(Box::new(Talk { socket: s, tls, out: first, sent: false, heard: now }));
                        }
                        TcpState::Closed => {
                            let why = stack.error(s).map_or(String::from("the connection closed"), |e| format!("{e}"));
                            stack.release(s);
                            return fail(out, format!("Couldn't connect to Claude's API: {why}."));
                        }
                        _ if now.saturating_sub(*since) > CONNECT_MS => {
                            stack.abort(s);
                            return fail(out, String::from("Couldn't connect to Claude's API: no answer."));
                        }
                        _ => return false,
                    }
                }
                Phase::Talking(_) => return self.talk(now, stack, out),
            }
        }
    }

    /// The connection is open: the secure connection's set-up, the request, and the answer.
    fn talk(&mut self, now: u64, stack: &mut Stack, out: &mut Vec<Got>) -> bool {
        let Phase::Talking(t) = &mut self.phase else { return true };
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let n = stack.recv(t.socket, &mut buf);
            if n == 0 {
                break;
            }
            t.heard = now;
            t.tls.incoming(&buf[..n]);
        }
        if let Some(e) = t.tls.error() {
            let alert = t.tls.outgoing();
            stack.send(t.socket, &alert);
            stack.release(t.socket);
            return fail(out, tls_text(e));
        }
        if t.tls.is_open() && !t.sent {
            crate::log!("Chat Computer kernel: Claude: connected (TLS 1.3), asking");
            t.tls.write(&self.request);
            wipe(&mut self.request);
            t.sent = true;
        }
        let mut over: Option<api::Event> = None;
        'reading: loop {
            let n = t.tls.read(&mut buf);
            if n == 0 {
                break;
            }
            for e in self.stream.feed(&buf[..n]) {
                match e {
                    api::Event::Text(s) => {
                        self.chars += s.chars().count();
                        out.push(Got::Words(s));
                    }
                    api::Event::Thinking => out.push(Got::Thinking),
                    api::Event::Tool(_) => {}
                    e => {
                        over = Some(e);
                        break 'reading;
                    }
                }
            }
        }
        if over.is_none() && (stack.finished(t.socket) || t.tls.finished()) {
            for e in self.stream.closed() {
                match e {
                    api::Event::Text(s) => out.push(Got::Words(s)),
                    api::Event::Thinking | api::Event::Tool(_) => {}
                    e => over = Some(e),
                }
            }
        }
        match over {
            Some(api::Event::Done) => {
                crate::log!(
                    "Chat Computer kernel: Claude: answered ({} characters, {})",
                    self.chars,
                    self.stream.stop_reason().unwrap_or("no reason given")
                );
                t.tls.close();
                let bye = t.tls.outgoing();
                stack.send(t.socket, &bye);
                stack.close(t.socket);
                stack.release(t.socket);
                out.push(Got::Over(Ok(Answered {
                    content: self.stream.content(),
                    stop: self.stream.stop_reason().map(String::from),
                    calls: self.stream.tool_uses(),
                })));
                return true;
            }
            Some(api::Event::Failed(m)) => {
                let status = self.stream.status();
                stack.release(t.socket);
                return fail(out, api_text(status, &m));
            }
            _ => {}
        }
        t.out.extend(t.tls.outgoing());
        if !t.out.is_empty() {
            let n = stack.send(t.socket, &t.out);
            t.out.drain(..n);
        }
        if stack.state(t.socket) == TcpState::Closed {
            let why = stack.error(t.socket).map_or(String::from("it closed"), |e| format!("{e}"));
            stack.release(t.socket);
            return fail(out, format!("The connection to Claude's API broke: {why}."));
        }
        let limit = if t.sent { SILENCE_MS } else { HANDSHAKE_MS };
        if now.saturating_sub(t.heard) > limit {
            stack.abort(t.socket);
            return fail(out, String::from("Claude's API stopped answering."));
        }
        false
    }
}

fn fail(out: &mut Vec<Got>, why: String) -> bool {
    crate::log!("Chat Computer kernel: Claude: failed: {why}");
    out.push(Got::Over(Err(why)));
    true
}

/// Overwrites what held the key.
pub fn wipe(v: &mut Vec<u8>) {
    for b in v.iter_mut() {
        unsafe { core::ptr::write_volatile(b, 0) };
    }
    v.clear();
}

/// A failed secure connection, for the person.
fn tls_text(e: tls::Error) -> String {
    match e {
        tls::Error::Certificate(tls::Problem::Expired) => format!(
            "Chat Computer couldn't check who answered for {}: its certificate isn't valid at the time this PC's clock \
             says ({}). If that's wrong, set the clock in Windows or the PC's setup screen.",
            host(),
            clock::now_text().unwrap_or_default()
        ),
        tls::Error::Certificate(p) => format!("Chat Computer didn't trust the server that answered for {}: {p}.", host()),
        e => format!("The secure connection to Claude's API failed: {e}."),
    }
}

/// A failure Claude's API reported, for the person.
fn api_text(status: Option<u16>, message: &str) -> String {
    match status {
        Some(401) => format!(
            "Claude's API didn't accept your key ({message}). Choose Claude again with the spark beside + to give it \
             another (keys are made at platform.claude.com/settings/keys)."
        ),
        Some(403) => format!("Claude's API says your key may not do that ({message})."),
        Some(404) => format!("Claude's API doesn't know that model ({message}): choose another with the spark beside +."),
        Some(429) => format!("Claude's API says there have been too many requests ({message}). Try again in a minute."),
        Some(529) | Some(503) => format!("Claude is very busy right now ({message}). Try again in a moment."),
        _ => format!("Claude's API: {message}"),
    }
}
