//! The network: the first card that started (or, without one, a USB network adapter plugged in, at any time), and
//! Chat Computer's own stack on it (the `net` crate: ARP, IPv4 and IPv6, DHCP, DNS, TCP), run from the kernel's main
//! loop: the frames that arrived go in, the time moves on, the frames to send go out.

use crate::netcard::{self, Card};
use crate::sync::SpinLock;
use crate::{random, time};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use net::{Ipv4, Ipv6, Mac, Stack};

struct Network {
    card: Box<dyn Card>,
    stack: Stack,
    shown: u64,
    /// The IPv6 addresses (from routers) last logged.
    shown6: Vec<Ipv6>,
    link: Option<bool>,
    test: Test,
}

static NETWORK: SpinLock<Option<Network>> = SpinLock::new(None);
/// USB's count of changes when its network adapters were last looked at.
static USB_SEEN: AtomicU64 = AtomicU64::new(u64::MAX);

/// What the screen says about the network.
pub struct Status {
    /// What it is.
    pub describe: Option<String>,
    pub link: bool,
    pub address: Option<Ipv4>,
    /// Its IPv6 address from a router, if it has one.
    pub address6: Option<Ipv6>,
}

/// Starts the cards there are drivers for, and the stack on the first. Returns how many cards started.
pub fn init() -> usize {
    let mut cards = netcard::find();
    for c in &cards {
        crate::log!("Chat Computer kernel: network: {}", c.describe());
    }
    if cards.is_empty() {
        return 0;
    }
    let count = cards.len();
    let card = cards.remove(0);
    let stack = Stack::new(Mac(card.mac()), random::u64(), true);
    crate::log!("Chat Computer kernel: network: random numbers from {}", random::source());
    *NETWORK.lock() = Some(Network { card, stack, shown: 0, shown6: Vec::new(), link: None, test: Test::Start });
    count
}

/// A USB network adapter plugged in or unplugged: without another card, the network moves to the first there is.
fn usb_adapters() {
    let changes = crate::usb::changes();
    let changed = USB_SEEN.swap(changes, Ordering::Relaxed) != changes;
    let mut guard = NETWORK.lock();
    let gone = guard.as_ref().is_some_and(|n| !n.card.present());
    if gone {
        crate::log!("Chat Computer kernel: network: the USB network adapter was unplugged");
        *guard = None;
    }
    if guard.is_some() || !(changed || gone) {
        return;
    }
    if let Some(a) = crate::usb::net_adapters().first() {
        let card: Box<dyn Card> = Box::new(netcard::UsbNet::new(a));
        crate::log!("Chat Computer kernel: network: {}", card.describe());
        let stack = Stack::new(Mac(card.mac()), random::u64(), true);
        *guard = Some(Network { card, stack, shown: 0, shown6: Vec::new(), link: None, test: Test::Start });
    }
}

/// From the main loop: takes what arrived, does what's due, sends what's ready.
pub fn work() {
    usb_adapters();
    let now = time::uptime_ms();
    let mut guard = NETWORK.lock();
    let Some(n) = guard.as_mut() else { return };
    {
        let Network { card, stack, .. } = n;
        card.receive(&mut |f| stack.receive(now, f));
        stack.poll(now);
        if option_env!("CHATCOMPUTER_TEST") == Some("network") {
            n.test.step(now, &mut n.stack);
        }
        while let Some(f) = n.stack.transmit() {
            n.card.send(&f);
        }
    }
    let link = n.card.link_up();
    if n.link != Some(link) {
        n.link = Some(link);
        crate::log!("Chat Computer kernel: network: {}", if link { "cable connected" } else { "no cable connected" });
    }
    if n.stack.changes() != n.shown {
        n.shown = n.stack.changes();
        match n.stack.config() {
            Some(c) => {
                let dns: Vec<String> = c.dns.iter().map(|d| alloc::format!("{d}")).collect();
                crate::log!(
                    "Chat Computer kernel: network: address {}/{}, gateway {}, DNS {}",
                    c.address,
                    c.prefix,
                    c.gateway.map_or(String::from("none"), |g| alloc::format!("{g}")),
                    if dns.is_empty() { String::from("none") } else { dns.join(", ") }
                );
            }
            None => crate::log!("Chat Computer kernel: network: no address ({})", n.stack.dhcp_state()),
        }
        // IPv6's, from a router's advertisement.
        let v6: Vec<Ipv6> = n.stack.ipv6_addresses().into_iter().skip(1).collect();
        if v6 != n.shown6 {
            n.shown6 = v6.clone();
            if !v6.is_empty() {
                let list = |a: &[Ipv6]| a.iter().map(|x| alloc::format!("{x}")).collect::<Vec<String>>().join(", ");
                let dns = n.stack.ipv6_dns();
                crate::log!(
                    "Chat Computer kernel: network: IPv6 address {} (on the link {}), DNS {}",
                    list(&v6),
                    n.stack.ipv6_addresses()[0],
                    if dns.is_empty() { String::from("none") } else { list(&dns) }
                );
            }
        }
    }
}

/// Runs `f` with the stack (what talks over the network does so through this: the web, web.rs), and sends what it
/// queued.
pub fn with<R>(f: impl FnOnce(&mut Stack) -> R) -> Option<R> {
    let mut guard = NETWORK.lock();
    let n = guard.as_mut()?;
    let r = f(&mut n.stack);
    while let Some(frame) = n.stack.transmit() {
        n.card.send(&frame);
    }
    Some(r)
}

pub fn status() -> Status {
    match NETWORK.lock().as_ref() {
        Some(n) => Status {
            describe: Some(n.card.describe()),
            link: n.link.unwrap_or(false),
            address: n.stack.config().map(|c| c.address),
            address6: n.stack.ipv6_addresses().get(1).copied(),
        },
        None => Status { describe: None, link: false, address: None, address6: None },
    }
}

/// A test build's check of the whole path (CHATCOMPUTER_TEST=network, by `check.py --network`): an address
/// from DHCP, a name looked up, and a conversation with a server on the computer running the virtual PC
/// (10.0.2.2 is that computer, to QEMU's virtual network); then an IPv6 address from QEMU's router, and the same
/// conversation over IPv6 (with fec0::2, the same computer).
enum Test {
    Start,
    Looking(net::Lookup),
    /// The connection, since when, and whether it's over IPv6.
    Connecting(net::tcp::Socket, u64, bool),
    Reading(net::tcp::Socket, Vec<u8>, u64, bool),
    /// Waiting for an IPv6 address, since.
    Start6(u64),
    Done,
}

const TEST_SERVER: Ipv4 = Ipv4([10, 0, 2, 2]);

fn test_port() -> u16 {
    option_env!("CHATCOMPUTER_TEST_PORT").and_then(|p| p.parse().ok()).unwrap_or(8471)
}

impl Test {
    fn step(&mut self, now: u64, stack: &mut Stack) {
        let next = match core::mem::replace(self, Test::Done) {
            Test::Start => {
                if stack.config().is_none() {
                    Test::Start
                } else {
                    Test::Looking(stack.resolve(now, "api.anthropic.com"))
                }
            }
            Test::Looking(l) => match stack.resolved(l) {
                None => Test::Looking(l),
                Some(r) => {
                    match r {
                        Ok(ip) => crate::log!("Chat Computer kernel: test: api.anthropic.com is at {ip}"),
                        Err(e) => crate::log!("Chat Computer kernel: test: looking up api.anthropic.com: {e}"),
                    }
                    match stack.connect(now, TEST_SERVER, test_port()) {
                        Ok(s) => Test::Connecting(s, now, false),
                        Err(e) => {
                            crate::log!("Chat Computer kernel: test: couldn't connect: {e}");
                            Test::Done
                        }
                    }
                }
            },
            Test::Start6(since) => {
                if stack.ipv6_ready() {
                    match stack.connect(now, crate::web::TEST_SERVER6, test_port()) {
                        Ok(s) => Test::Connecting(s, now, true),
                        Err(e) => {
                            crate::log!("Chat Computer kernel: test: couldn't connect over IPv6: {e}");
                            Test::Done
                        }
                    }
                } else if now - since > 30_000 {
                    crate::log!("Chat Computer kernel: test: no IPv6 address from a router");
                    Test::Done
                } else {
                    Test::Start6(since)
                }
            }
            Test::Connecting(s, since, v6) => match stack.state(s) {
                net::TcpState::Established => {
                    stack.send(s, if v6 { b"hello from Chat Computer over IPv6\n" as &[u8] } else { b"hello from Chat Computer\n" });
                    Test::Reading(s, Vec::new(), now, v6)
                }
                net::TcpState::Closed => {
                    crate::log!("Chat Computer kernel: test: couldn't connect{}: {:?}", if v6 { " over IPv6" } else { "" }, stack.error(s));
                    Test::Done
                }
                _ if now - since > 20_000 => {
                    crate::log!("Chat Computer kernel: test: the connection never opened{}", if v6 { " over IPv6" } else { "" });
                    Test::Done
                }
                _ => Test::Connecting(s, since, v6),
            },
            Test::Reading(s, mut got, since, v6) => {
                let mut buf = [0u8; 512];
                let n = stack.recv(s, &mut buf);
                got.extend_from_slice(&buf[..n]);
                if let Some(end) = got.iter().position(|&b| b == b'\n') {
                    let over = if v6 { "over IPv6, " } else { "" };
                    crate::log!("Chat Computer kernel: test: {over}the server said {:?}", String::from_utf8_lossy(&got[..end]));
                    stack.close(s);
                    if v6 { Test::Done } else { Test::Start6(now) }
                } else if stack.state(s) == net::TcpState::Closed || now - since > 20_000 {
                    crate::log!("Chat Computer kernel: test: no answer from the server{} ({:?})", if v6 { " over IPv6" } else { "" }, stack.error(s));
                    Test::Done
                } else {
                    Test::Reading(s, got, since, v6)
                }
            }
            Test::Done => Test::Done,
        };
        *self = next;
    }
}
