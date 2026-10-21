//! Talking to Claude: Opus 5.5 at its highest effort, through Anthropic's Messages API at api.anthropic.com, over
//! Chat Computer's own network stack (net), its own TLS 1.3 (tls) and its own HTTP and JSON (claude), with the
//! person's own API key from the settings. One question at a time: find the address, connect, check who answers
//! (its certificate, against Mozilla's authorities), send the conversation, and pass the answer's words on as they
//! come. Nothing of the conversation goes in the log; a test build logs the answer, which a test checks.
//!
//! A test build (CHATCOMPUTER_TEST=claude, by check.py --claude) talks to a stand-in on the computer running the
//! virtual PC instead (10.0.2.2, as "localhost"), and trusts only that test's own certificate authority.

use crate::sync::SpinLock;
use crate::{clock, network, random, time};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use claude::api::{self, Role, Stream};
use net::tcp::Socket;
use net::{Ipv4, Stack, TcpState};
use tls::Cert;

/// What came of the question.
pub enum Event {
    /// More of the answer.
    Text(String),
    /// The answer is complete.
    Done,
    /// It failed: why, for the person.
    Failed(String),
}

/// Whether Claude can be asked now, and if not, why not.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ready {
    Yes,
    NoKey,
    BadKey(&'static str),
    NoClock,
    NoCard,
    NoCable,
    NoAddress,
}

/// Where the key comes from.
pub const GIVE_KEY: &str =
    "On Windows, double-click \u{201C}Chat Computer Own System\u{201D} and give it your Claude API key when it asks.";
pub const NO_KEY: &str = "To talk to Claude here, Chat Computer needs your Claude API key.";
pub const NO_CARD: &str = "Claude is on the internet, and this PC's network chip isn't one Chat Computer has a driver \
                           for yet.";
const NO_CLOCK: &str = "This PC's clock couldn't be read, so Chat Computer can't check it's really talking to Claude.";

/// Waiting for a network address; finding Claude's, and connecting; the secure connection being set up.
const NETWORK_MS: u64 = 60_000;
const CONNECT_MS: u64 = 30_000;
const HANDSHAKE_MS: u64 = 30_000;
/// Silence from Claude's API, once asked, before giving up. At its highest effort Claude may think for minutes,
/// but the stream carries its thinking, or a ping, all along.
const SILENCE_MS: u64 = 180_000;
/// How long api.anthropic.com's address is used before it's looked up again.
const ADDRESS_MS: u64 = 300_000;
/// A closing connection still not closed after this is broken off.
const CLOSING_MS: u64 = 30_000;

const SYSTEM: &str = "You are Claude, talking with a person on Chat Computer: a computer whose screen is a \
conversation with you. It is running Chat Computer's own operating system, which is new: here you can talk, but you \
can't yet use tools, the web or the computer's files. This screen shows plain text only, so write without Markdown: \
no asterisks, pound signs, backticks or tables. Separate paragraphs with a blank line, and start list items with a \
bullet (\u{2022}) or a number.";

fn test_build() -> bool {
    option_env!("CHATCOMPUTER_TEST") == Some("claude")
}

/// The stand-in's address and port, in a test build.
const TEST_SERVER: Ipv4 = Ipv4([10, 0, 2, 2]);
pub const TEST_PORT: u16 = 8472;
/// The test's certificate authority (empty in every other build: kernel/build.rs).
static TEST_CA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/test_ca.der"));

fn host() -> &'static str {
    if test_build() { "localhost" } else { api::HOST }
}

fn port() -> u16 {
    if test_build() { TEST_PORT } else { 443 }
}

fn roots() -> &'static [Cert<'static>] {
    let roots: Vec<Cert<'static>> = if test_build() {
        tls::x509::parse_root(TEST_CA).into_iter().collect()
    } else {
        tls::roots()
    };
    Box::leak(roots.into_boxed_slice())
}

struct Talk {
    socket: Socket,
    tls: Box<tls::Client>,
    stream: Stream,
    /// When the server last sent anything.
    heard: u64,
    /// For the server, not yet taken by TCP.
    out: Vec<u8>,
    /// Whether the request has been handed to TLS.
    sent: bool,
    chars: usize,
    /// The answer, kept only by a test build, for its log.
    text: String,
}

enum Phase {
    /// Waiting for a network address, since.
    Network(u64),
    Looking(net::Lookup, u64),
    Connecting(Socket, u64),
    Talking(Box<Talk>),
}

struct Job {
    /// The request, key and all, until it's handed to TLS (then wiped).
    request: Vec<u8>,
    phase: Phase,
}

struct Assistant {
    key: Option<String>,
    key_problem: Option<&'static str>,
    roots: &'static [Cert<'static>],
    job: Option<Job>,
    /// Connections being closed, and since when.
    closing: Vec<(Socket, u64)>,
    /// api.anthropic.com's address, and when it was found.
    address: Option<(Ipv4, u64)>,
}

static ASSISTANT: SpinLock<Option<Assistant>> = SpinLock::new(None);

/// Starts, with the key from the settings (or why there's none that works).
pub fn init(key: Option<String>, key_problem: Option<&'static str>) {
    let roots = roots();
    crate::log!(
        "Chat Computer kernel: Claude: {} at {} with {}, {} certificate authorities{}",
        api::MODEL,
        host(),
        if key.is_some() { "the API key from the settings" } else { "no API key yet" },
        roots.len(),
        if test_build() { " (the test's own)" } else { " (Mozilla's)" }
    );
    *ASSISTANT.lock() = Some(Assistant { key, key_problem, roots, job: None, closing: Vec::new(), address: None });
}

pub fn ready() -> Ready {
    let (has_key, problem) = match ASSISTANT.lock().as_ref() {
        Some(a) => (a.key.is_some(), a.key_problem),
        None => (false, None),
    };
    let status = network::status();
    match (has_key, problem) {
        (false, Some(p)) => Ready::BadKey(p),
        (false, None) => Ready::NoKey,
        _ if clock::now().is_none() => Ready::NoClock,
        _ if !status.card => Ready::NoCard,
        _ if status.address.is_none() && !status.link => Ready::NoCable,
        _ if status.address.is_none() => Ready::NoAddress,
        _ => Ready::Yes,
    }
}

/// Whether an answer is on its way.
pub fn busy() -> bool {
    ASSISTANT.lock().as_ref().is_some_and(|a| a.job.is_some())
}

/// Asks Claude to answer `conversation` (oldest first, the person's newest message last). The answer comes from
/// `work`. Err: why it can't be asked, for the person.
pub fn ask(conversation: &[(Role, String)]) -> Result<(), String> {
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return Err(format!("{NO_KEY} {GIVE_KEY}")) };
    if a.job.is_some() {
        return Err(String::from("Claude is still answering."));
    }
    let key = match (&a.key, a.key_problem) {
        (Some(k), _) => k,
        (None, Some(p)) => return Err(format!("{}. {GIVE_KEY}", sentence(p))),
        (None, None) => return Err(format!("{NO_KEY} {GIVE_KEY}")),
    };
    if clock::now().is_none() {
        return Err(String::from(NO_CLOCK));
    }
    if !network::status().card {
        return Err(String::from(NO_CARD));
    }
    let mut system = String::from(SYSTEM);
    if let Some(now) = clock::now_text() {
        system.push_str(&format!(" The computer's clock says it is {now}."));
    }
    let request = api::request(host(), key, &system, conversation);
    a.job = Some(Job { request, phase: Phase::Network(time::uptime_ms()) });
    Ok(())
}

/// Stops the answer on its way (its connection broken off).
pub fn stop() {
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return };
    let Some(mut job) = a.job.take() else { return };
    wipe(&mut job.request);
    let socket = match job.phase {
        Phase::Connecting(s, _) => Some(s),
        Phase::Talking(t) => Some(t.socket),
        _ => None,
    };
    if let Some(s) = socket {
        network::with(|stack| stack.abort(s));
    }
    crate::log!("Chat Computer kernel: Claude: stopped");
}

/// From the main loop: moves the question on. Returns what came of it.
pub fn work() -> Vec<Event> {
    let now = time::uptime_ms();
    let mut events = Vec::new();
    let mut guard = ASSISTANT.lock();
    let Some(a) = guard.as_mut() else { return events };
    if a.job.is_none() && a.closing.is_empty() {
        return events;
    }
    let ran = network::with(|stack| {
        a.tidy(now, stack);
        if let Some(mut job) = a.job.take() {
            if !a.step(now, stack, &mut job, &mut events) {
                a.job = Some(job);
            } else {
                wipe(&mut job.request);
            }
        }
    });
    if ran.is_none() && a.job.take().is_some() {
        events.push(Event::Failed(String::from(NO_CARD)));
    }
    events
}

/// `s` with a capital first letter.
pub fn sentence(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Overwrites what held the key.
fn wipe(v: &mut Vec<u8>) {
    for b in v.iter_mut() {
        unsafe { core::ptr::write_volatile(b, 0) };
    }
    v.clear();
}

fn fail(events: &mut Vec<Event>, why: String) -> bool {
    crate::log!("Chat Computer kernel: Claude: failed: {why}");
    events.push(Event::Failed(why));
    true
}

fn tcp_text(e: Option<net::TcpError>) -> String {
    match e {
        Some(e) => format!("{e}"),
        None => String::from("the connection closed"),
    }
}

/// A failed secure connection, for the person.
fn tls_text(e: tls::Error) -> String {
    match e {
        tls::Error::Certificate(tls::Problem::Expired) if clock::behind() || clock::now().is_none() => format!(
            "Chat Computer couldn't check who answered for {}: its certificate isn't valid at the time this PC's clock \
             says ({}), and that's earlier than it can be. Set the clock in the PC's setup screen, or in Windows, and \
             start again.",
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
        Some(401) => format!("Claude's API didn't accept the API key ({message}). {GIVE_KEY}"),
        Some(403) => format!("Claude's API says this key may not do that ({message})."),
        Some(429) => format!("Claude's API says there have been too many requests ({message}). Try again in a minute."),
        Some(529) | Some(503) => format!("Claude is very busy right now ({message}). Try again in a moment."),
        _ => format!("Claude's API: {message}"),
    }
}

fn suite_name(s: Option<tls::Suite>) -> &'static str {
    match s {
        Some(tls::Suite::Aes128Gcm) => "AES-128-GCM",
        Some(tls::Suite::Aes256Gcm) => "AES-256-GCM",
        Some(tls::Suite::ChaCha20Poly1305) => "ChaCha20-Poly1305",
        None => "?",
    }
}

impl Assistant {
    /// Lets go of closed connections; breaks off ones that won't close.
    fn tidy(&mut self, now: u64, stack: &mut Stack) {
        self.closing.retain(|&(s, since)| match stack.state(s) {
            TcpState::Closed | TcpState::TimeWait => {
                stack.release(s);
                false
            }
            _ if now.saturating_sub(since) > CLOSING_MS => {
                stack.abort(s);
                false
            }
            _ => true,
        });
    }

    fn connect(&mut self, now: u64, stack: &mut Stack, job: &mut Job, ip: Ipv4, events: &mut Vec<Event>) -> bool {
        match stack.connect(now, ip, port()) {
            Ok(s) => {
                job.phase = Phase::Connecting(s, now);
                false
            }
            Err(e) => fail(events, format!("Couldn't connect to Claude's API: {e}.")),
        }
    }

    /// Moves `job` on as far as it goes now. True when it's over (its last event given).
    fn step(&mut self, now: u64, stack: &mut Stack, job: &mut Job, events: &mut Vec<Event>) -> bool {
        loop {
            match &mut job.phase {
                Phase::Network(since) => {
                    if stack.config().is_none() {
                        if now.saturating_sub(*since) > NETWORK_MS {
                            return fail(
                                events,
                                String::from("Chat Computer has no network address yet: is the network cable plugged in?"),
                            );
                        }
                        return false;
                    }
                    if test_build() {
                        if self.connect(now, stack, job, TEST_SERVER, events) {
                            return true;
                        }
                    } else if let Some((ip, when)) = self.address
                        && now.saturating_sub(when) < ADDRESS_MS
                    {
                        if self.connect(now, stack, job, ip, events) {
                            return true;
                        }
                    } else {
                        job.phase = Phase::Looking(stack.resolve(now, host()), now);
                    }
                }
                Phase::Looking(lookup, since) => match stack.resolved(*lookup) {
                    None if now.saturating_sub(*since) > CONNECT_MS => {
                        return fail(events, format!("Couldn't find {} on the network: no answer.", host()));
                    }
                    None => return false,
                    Some(Ok(ip)) => {
                        crate::log!("Chat Computer kernel: Claude: {} is at {ip}", host());
                        self.address = Some((ip, now));
                        if self.connect(now, stack, job, ip, events) {
                            return true;
                        }
                    }
                    Some(Err(e)) => return fail(events, format!("Couldn't find {} on the network: {e}.", host())),
                },
                Phase::Connecting(s, since) => {
                    let s = *s;
                    match stack.state(s) {
                        TcpState::Established => {
                            let Some(now_s) = clock::now() else { return fail(events, String::from(NO_CLOCK)) };
                            let mut tls = Box::new(tls::Client::new(host(), now_s, self.roots, &mut random::fill));
                            tls.allow_clock_error(clock::error());
                            let out = tls.outgoing();
                            job.phase = Phase::Talking(Box::new(Talk {
                                socket: s,
                                tls,
                                stream: Stream::new(),
                                heard: now,
                                out,
                                sent: false,
                                chars: 0,
                                text: String::new(),
                            }));
                        }
                        TcpState::Closed => {
                            let why = tcp_text(stack.error(s));
                            stack.release(s);
                            return fail(events, format!("Couldn't connect to Claude's API: {why}."));
                        }
                        _ if now.saturating_sub(*since) > CONNECT_MS => {
                            stack.abort(s);
                            return fail(events, String::from("Couldn't connect to Claude's API: no answer."));
                        }
                        _ => return false,
                    }
                }
                Phase::Talking(t) => return self.talk(now, stack, t, &mut job.request, events),
            }
        }
    }

    /// The connection is open: the secure connection's set-up, the request, and the answer.
    fn talk(&mut self, now: u64, stack: &mut Stack, t: &mut Talk, request: &mut Vec<u8>, events: &mut Vec<Event>) -> bool {
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
            self.close(now, stack, t.socket);
            return fail(events, tls_text(e));
        }
        if t.tls.is_open() && !t.sent {
            crate::log!("Chat Computer kernel: Claude: connected (TLS 1.3, {}), asking", suite_name(t.tls.suite()));
            t.tls.write(request);
            wipe(request);
            t.sent = true;
        }
        let mut over = None;
        'reading: loop {
            let n = t.tls.read(&mut buf);
            if n == 0 {
                break;
            }
            for e in t.stream.feed(&buf[..n]) {
                if let api::Event::Text(s) = &e {
                    t.chars += s.chars().count();
                    if test_build() {
                        t.text.push_str(s);
                    }
                }
                match e {
                    api::Event::Text(s) => events.push(Event::Text(s)),
                    e => {
                        over = Some(e);
                        break 'reading;
                    }
                }
            }
        }
        if over.is_none() && (stack.finished(t.socket) || t.tls.finished()) {
            for e in t.stream.closed() {
                match e {
                    api::Event::Text(s) => events.push(Event::Text(s)),
                    e => over = Some(e),
                }
            }
        }
        match over {
            Some(api::Event::Done) => {
                crate::log!("Chat Computer kernel: Claude: answered ({} characters)", t.chars);
                if test_build() {
                    crate::log!("Chat Computer kernel: test: Claude said {:?}", t.text);
                }
                t.tls.close();
                t.out.extend(t.tls.outgoing());
                stack.send(t.socket, &t.out);
                self.close(now, stack, t.socket);
                events.push(Event::Done);
                return true;
            }
            Some(api::Event::Failed(m)) => {
                let why = api_text(t.stream.status(), &m);
                t.tls.close();
                t.out.extend(t.tls.outgoing());
                stack.send(t.socket, &t.out);
                self.close(now, stack, t.socket);
                return fail(events, why);
            }
            _ => {}
        }
        t.out.extend(t.tls.outgoing());
        if !t.out.is_empty() {
            let n = stack.send(t.socket, &t.out);
            t.out.drain(..n);
        }
        if stack.state(t.socket) == TcpState::Closed {
            let why = tcp_text(stack.error(t.socket));
            stack.release(t.socket);
            return fail(events, format!("The connection to Claude's API broke: {why}."));
        }
        let limit = if t.sent { SILENCE_MS } else { HANDSHAKE_MS };
        if now.saturating_sub(t.heard) > limit {
            stack.abort(t.socket);
            return fail(events, String::from("Claude's API stopped answering."));
        }
        false
    }

    fn close(&mut self, now: u64, stack: &mut Stack, s: Socket) {
        stack.close(s);
        self.closing.push((s, now));
    }
}
