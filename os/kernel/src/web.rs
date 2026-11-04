//! The web, for the model's tools (search_web, read_web_page): a page fetched over Chat Computer's own network stack
//! (net) and TLS 1.3 (tls): the site's address found with DNS, a TCP connection to it, its certificate checked
//! against Mozilla's authorities for the site's name, a GET (web::http), its response read; redirects followed (to
//! HTTPS addresses only, at most five), at most web::MOST_BYTES of a page kept, and given up on after 20 seconds with
//! nothing from the site. A fetch is moved on from the main loop (`Fetch::step`), never waiting, so the screen and the
//! keyboard go on meanwhile.
//!
//! The same way, a POST to an API (Meta's Model API, for the model: assistant.rs): its answer whatever its status, and
//! never sent on.
//!
//! A test build with a certificate authority of its own (check.py --web: CHATCOMPUTER_TEST_CA, kernel/build.rs)
//! trusts that one instead of Mozilla's, and reaches every site at a test server on the computer running the virtual
//! PC (10.0.2.2), still checking its certificate for the site's name.

use crate::{clock, network, random, time};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use net::tcp::{Socket, State as TcpState};
use net::{Ipv4, Stack};
use tls::Cert;
use web::http::Response;
use web::Url;

/// Waiting for a network address; finding the site and connecting to it; silence from it once connected.
const NETWORK_MS: u64 = 30_000;
const CONNECT_MS: u64 = 20_000;
const SILENCE_MS: u64 = 20_000;
/// The most redirects followed.
const MOST_REDIRECTS: u32 = 5;

/// The test's certificate authority (empty in every other build: kernel/build.rs), and its server.
static TEST_CA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/test_ca.der"));
const TEST_SERVER: Ipv4 = Ipv4([10, 0, 2, 2]);

fn test_build() -> bool {
    !TEST_CA.is_empty()
}

fn test_port() -> u16 {
    option_env!("CHATCOMPUTER_TEST_PORT").and_then(|p| p.parse().ok()).unwrap_or(8473)
}

/// The authorities trusted: Mozilla's (or the test's), read once.
fn roots() -> &'static [Cert<'static>] {
    static ROOTS: crate::sync::SpinLock<Option<&'static [Cert<'static>]>> = crate::sync::SpinLock::new(None);
    let mut roots = ROOTS.lock();
    if let Some(r) = *roots {
        return r;
    }
    let read: Vec<Cert<'static>> =
        if test_build() { tls::x509::parse_root(TEST_CA).into_iter().collect() } else { tls::roots() };
    let r: &'static [Cert<'static>] = Box::leak(read.into_boxed_slice());
    *roots = Some(r);
    r
}

/// A page, fetched: where it ended up (after redirects), its status (200, or an API's error's), its Content-Type, its
/// bytes (decompressed), and whether they were cut at web::MOST_BYTES.
pub struct Page {
    pub url: Url,
    pub status: u16,
    pub kind: String,
    pub bytes: Vec<u8>,
    pub cut: bool,
}

enum Phase {
    /// Waiting for a network address, since.
    Network(u64),
    Looking(net::Lookup, u64),
    Connecting(Socket, u64),
    Talking(Box<Talk>),
}

struct Talk {
    socket: Socket,
    tls: Box<tls::Client>,
    /// The request, sent once the secure connection is open.
    request: Vec<u8>,
    /// For the site, not yet taken by TCP.
    out: Vec<u8>,
    sent: bool,
    response: Response,
    /// When the site last sent anything.
    heard: u64,
}

pub struct Fetch {
    url: Url,
    phase: Phase,
    redirects: u32,
    /// A POST's headers and body (else it's a GET).
    post: Option<(Vec<(String, String)>, Vec<u8>)>,
}

impl Fetch {
    pub fn new(url: Url) -> Fetch {
        Fetch { url, phase: Phase::Network(time::uptime_ms()), redirects: 0, post: None }
    }

    /// A POST of `body` with `headers`: its answer, whatever its status, is the page.
    pub fn post(url: Url, headers: Vec<(String, String)>, body: Vec<u8>) -> Fetch {
        Fetch { url, phase: Phase::Network(time::uptime_ms()), redirects: 0, post: Some((headers, body)) }
    }

    /// The site being fetched from now (after any redirects).
    pub fn host(&self) -> &str {
        &self.url.host
    }

    /// Moves the fetch on as far as it goes now: Some when it's over (the page, or why there's none).
    pub fn step(&mut self) -> Option<Result<Page, String>> {
        let now = time::uptime_ms();
        let Some(over) = network::with(|stack| self.step_with(now, stack)) else {
            return Some(Err(no_network()));
        };
        let result = over?;
        match result {
            // A redirect: the same again, from where it says.
            Ok(Got::Redirect(to)) => {
                self.redirects += 1;
                if self.redirects > MOST_REDIRECTS {
                    return Some(Err(String::from("the site sent it on too many times (more than five redirects)")));
                }
                crate::log!("Chat Computer kernel: web: sent on to {}", to.host);
                self.url = to;
                self.phase = Phase::Network(now);
                None
            }
            Ok(Got::Page(p)) => Some(Ok(p)),
            Err(why) => Some(Err(why)),
        }
    }

    fn step_with(&mut self, now: u64, stack: &mut Stack) -> Option<Result<Got, String>> {
        loop {
            match &mut self.phase {
                Phase::Network(since) => {
                    if stack.config().is_none() {
                        if now.saturating_sub(*since) > NETWORK_MS {
                            return Some(Err(no_address()));
                        }
                        return None;
                    }
                    if test_build() {
                        match connect(now, stack, TEST_SERVER, test_port(), &self.url.host) {
                            Ok(p) => self.phase = p,
                            Err(why) => return Some(Err(why)),
                        }
                    } else if let Some(ip) = Ipv4::parse(&self.url.host) {
                        match connect(now, stack, ip, self.url.port, &self.url.host) {
                            Ok(p) => self.phase = p,
                            Err(why) => return Some(Err(why)),
                        }
                    } else {
                        self.phase = Phase::Looking(stack.resolve(now, &self.url.host), now);
                    }
                }
                Phase::Looking(lookup, since) => match stack.resolved(*lookup) {
                    None if now.saturating_sub(*since) > CONNECT_MS => {
                        return Some(Err(format!("{} couldn't be found on the network (no answer)", self.url.host)));
                    }
                    None => return None,
                    Some(Ok(ip)) => match connect(now, stack, ip, self.url.port, &self.url.host) {
                        Ok(p) => self.phase = p,
                        Err(why) => return Some(Err(why)),
                    },
                    Some(Err(e)) => return Some(Err(format!("{} couldn't be found on the network: {e}", self.url.host))),
                },
                Phase::Connecting(s, since) => {
                    let s = *s;
                    match stack.state(s) {
                        TcpState::Established => {
                            let Some(when) = clock::now() else {
                                stack.release(s);
                                return Some(Err(String::from(
                                    "this PC's clock couldn't be read, so Chat Computer can't check who answers",
                                )));
                            };
                            let mut tls = Box::new(tls::Client::new(&self.url.host, when, roots(), &mut random::fill));
                            tls.allow_clock_error(clock::error());
                            let out = tls.outgoing();
                            let request = match &self.post {
                                Some((headers, body)) => web::http::post(&self.url, headers, body),
                                None => web::http::get(&self.url),
                            };
                            self.phase = Phase::Talking(Box::new(Talk {
                                socket: s,
                                tls,
                                request,
                                out,
                                sent: false,
                                response: Response::new(),
                                heard: now,
                            }));
                        }
                        TcpState::Closed => {
                            let why = stack.error(s).map_or(String::from("the connection closed"), |e| format!("{e}"));
                            stack.release(s);
                            return Some(Err(format!("couldn't connect to {}: {why}", self.url.host)));
                        }
                        _ if now.saturating_sub(*since) > CONNECT_MS => {
                            stack.abort(s);
                            return Some(Err(format!("couldn't connect to {}: no answer", self.url.host)));
                        }
                        _ => return None,
                    }
                }
                Phase::Talking(t) => return talk(now, stack, t, &self.url, self.post.is_some()),
            }
        }
    }
}

enum Got {
    Page(Page),
    Redirect(Url),
}

/// Opens a connection to `host` at `ip` (the next phase), or says why it can't.
fn connect(now: u64, stack: &mut Stack, ip: Ipv4, port: u16, host: &str) -> Result<Phase, String> {
    stack.connect(now, ip, port).map(|s| Phase::Connecting(s, now)).map_err(|e| format!("couldn't connect to {host}: {e}"))
}

/// The connection is open: TLS's set-up, the request, and the response. Some when it's over.
fn talk(now: u64, stack: &mut Stack, t: &mut Talk, url: &Url, post: bool) -> Option<Result<Got, String>> {
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
        return Some(Err(tls_text(e, &url.host)));
    }
    if t.tls.is_open() && !t.sent {
        let request = core::mem::take(&mut t.request);
        t.tls.write(&request);
        t.sent = true;
    }
    loop {
        let n = t.tls.read(&mut buf);
        if n == 0 {
            break;
        }
        t.response.feed(&buf[..n]);
    }
    let ended = stack.finished(t.socket) || t.tls.finished() || stack.state(t.socket) == TcpState::Closed;
    if ended && !t.response.done() {
        t.response.closed();
    }
    if t.response.broken() {
        stack.release(t.socket);
        return Some(Err(format!("{} didn't answer as a web site does", url.host)));
    }
    if t.response.done() {
        stack.release(t.socket);
        return Some(answered(&t.response, url, post));
    }
    t.out.extend(t.tls.outgoing());
    if !t.out.is_empty() {
        let n = stack.send(t.socket, &t.out);
        t.out.drain(..n);
    }
    if stack.state(t.socket) == TcpState::Closed {
        let why = stack.error(t.socket).map_or(String::from("it closed"), |e| format!("{e}"));
        stack.release(t.socket);
        return Some(Err(format!("the connection to {} broke: {why}", url.host)));
    }
    if now.saturating_sub(t.heard) > SILENCE_MS {
        stack.abort(t.socket);
        return Some(Err(format!("{} stopped answering", url.host)));
    }
    None
}

/// What a whole response comes to: a redirect, the page, or why there's none.
fn answered(r: &Response, url: &Url, post: bool) -> Result<Got, String> {
    let status = r.status().unwrap_or(0);
    if post {
        let bytes = r.page().map_err(String::from)?;
        let kind = String::from(r.header("content-type").unwrap_or(""));
        return Ok(Got::Page(Page { url: url.clone(), status, kind, bytes, cut: r.cut() }));
    }
    if matches!(status, 301 | 302 | 303 | 307 | 308) {
        let Some(to) = r.header("location") else {
            return Err(format!("{} sent it on without saying where", url.host));
        };
        return url.join(to).map(Got::Redirect).map_err(|why| format!("{} sent it on to an address that can't be read: {why}", url.host));
    }
    if status >= 400 {
        return Err(match status {
            401 | 403 => format!("{} refused to show it ({status}: it's private, or the site turns away programs)", url.host),
            404 | 410 => format!("{} says there's no such page ({status})", url.host),
            429 => format!("{} says it's had too many requests ({status}): try again later", url.host),
            s if s >= 500 => format!("{} had a problem ({s}): try again later", url.host),
            s => format!("{} refused it ({s})", url.host),
        });
    }
    let bytes = r.page().map_err(String::from)?;
    let kind = String::from(r.header("content-type").unwrap_or(""));
    Ok(Got::Page(Page { url: url.clone(), status, kind, bytes, cut: r.cut() }))
}

/// A failed secure connection, in words.
fn tls_text(e: tls::Error, host: &str) -> String {
    match e {
        tls::Error::Certificate(tls::Problem::Expired) => format!(
            "{host}'s certificate isn't valid at the time this PC's clock says ({}): if that's wrong, set the clock in \
             Windows or the PC's setup screen",
            clock::now_text().unwrap_or_default()
        ),
        tls::Error::Certificate(p) => format!("Chat Computer didn't trust the site that answered for {host}: {p}"),
        e => format!("the secure connection to {host} failed: {e}"),
    }
}

/// No network card Chat Computer drives.
fn no_network() -> String {
    String::from(
        "this PC has no network connection Chat Computer can use: Wi-Fi isn't supported yet, and its wired network \
         chip isn't one Chat Computer has a driver for. A network cable to a supported chip, a USB network adapter, \
         or an Android phone sharing its connection over USB (USB tethering) works",
    )
}

/// A card, but no address.
fn no_address() -> String {
    let status = network::status();
    if !status.link {
        String::from(
            "this PC isn't connected to a network: plug in a network cable (Wi-Fi isn't supported yet), or connect an \
             Android phone by USB and turn on USB tethering",
        )
    } else {
        String::from("this PC is connected to a network, but got no address on it (no DHCP server answered)")
    }
}
