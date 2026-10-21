//! DNS (RFC 1035): a name's IPv4 address, asked of the servers DHCP named, over UDP. Each question goes out from
//! its own random port with a random id, and an answer counts only if it comes from the server asked, to that
//! port, with that id and about that name. Not answered in 2 seconds, it's asked again (the next server, if
//! there's more than one), up to four times. Answers are kept for as long as they say (at least 30 seconds, at
//! most a day).

use crate::{be16, be32, put16, Ipv4, Random};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// There's no DNS server to ask (no network address yet).
    NoServer,
    /// The name doesn't exist, or has no IPv4 address.
    NotFound,
    /// The server couldn't answer.
    Failed,
    /// No answer came.
    TimedOut,
    /// Not a name that can be looked up.
    BadName,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Error::NoServer => "there's no DNS server to ask",
            Error::NotFound => "no such name",
            Error::Failed => "the DNS server couldn't answer",
            Error::TimedOut => "the DNS server didn't answer",
            Error::BadName => "not a name that can be looked up",
        })
    }
}

const WAIT_MS: u64 = 2000;
const TRIES: u32 = 4;
/// How long to wait for a DNS server to be known (DHCP may still be at work).
const NO_SERVER_MS: u64 = 15_000;

struct Query {
    id: u32,
    name: String,
    txid: u16,
    port: u16,
    created: Option<u64>,
    sent: Option<(u64, Ipv4)>,
    tries: u32,
    result: Option<Result<Ipv4, Error>>,
}

pub(crate) struct Resolver {
    queries: Vec<Query>,
    cache: Vec<(String, Ipv4, u64)>,
    next: u32,
}

fn valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'))
}

/// The name at `at`, following compression pointers; and where what follows it starts.
fn read_name(d: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut name = String::new();
    let mut after = None;
    for _ in 0..64 {
        let len = *d.get(at)? as usize;
        if len == 0 {
            return Some((name, after.unwrap_or(at + 1)));
        }
        if len & 0xC0 == 0xC0 {
            let to = ((len & 0x3F) << 8) | *d.get(at + 1)? as usize;
            if after.is_none() {
                after = Some(at + 2);
            }
            at = to;
            continue;
        }
        if len & 0xC0 != 0 {
            return None;
        }
        let label = d.get(at + 1..at + 1 + len)?;
        if !name.is_empty() {
            name.push('.');
        }
        for &c in label {
            name.push(c.to_ascii_lowercase() as char);
        }
        at += 1 + len;
    }
    None
}

impl Resolver {
    pub(crate) fn new() -> Resolver {
        Resolver { queries: Vec::new(), cache: Vec::new(), next: 1 }
    }

    pub(crate) fn start(&mut self, name: &str, random: &mut Random) -> u32 {
        let id = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        let name = String::from(name.trim().trim_end_matches('.')).to_ascii_lowercase();
        let result = if let Some(ip) = Ipv4::parse(&name) {
            Some(Ok(ip))
        } else if !valid(&name) {
            Some(Err(Error::BadName))
        } else {
            None
        };
        let r = random.next();
        self.queries.push(Query {
            id,
            name,
            txid: r as u16,
            port: 49152 + ((r >> 16) % 16384) as u16,
            created: None,
            sent: None,
            tries: 0,
            result,
        });
        id
    }

    pub(crate) fn take(&mut self, id: u32) -> Option<Result<Ipv4, Error>> {
        let i = self.queries.iter().position(|q| q.id == id && q.result.is_some())?;
        self.queries.swap_remove(i).result
    }

    fn query_packet(q: &Query) -> Vec<u8> {
        let mut p = vec![0u8; 12];
        put16(&mut p, 0, q.txid);
        put16(&mut p, 2, 0x0100); // recursion desired
        put16(&mut p, 4, 1);
        for label in q.name.split('.') {
            p.push(label.len() as u8);
            p.extend_from_slice(label.as_bytes());
        }
        p.extend_from_slice(&[0, 0, 1, 0, 1]); // the root; type A, class IN
        p
    }

    /// What to send now: (server, our port, the question). Also gives up on what waited too long.
    pub(crate) fn due(&mut self, now: u64, servers: &[Ipv4]) -> Vec<(Ipv4, u16, Vec<u8>)> {
        self.cache.retain(|c| c.2 > now);
        let mut out = Vec::new();
        for q in self.queries.iter_mut().filter(|q| q.result.is_none()) {
            if let Some(c) = self.cache.iter().find(|c| c.0 == q.name) {
                q.result = Some(Ok(c.1));
                continue;
            }
            let created = *q.created.get_or_insert(now);
            if servers.is_empty() {
                if now.saturating_sub(created) >= NO_SERVER_MS {
                    q.result = Some(Err(Error::NoServer));
                }
                continue;
            }
            if let Some((at, _)) = q.sent
                && now.saturating_sub(at) < WAIT_MS
            {
                continue;
            }
            if q.tries >= TRIES {
                q.result = Some(Err(Error::TimedOut));
                continue;
            }
            let server = servers[q.tries as usize % servers.len()];
            q.tries += 1;
            q.sent = Some((now, server));
            out.push((server, q.port, Self::query_packet(q)));
        }
        out
    }

    /// An answer that came to our `port` from `src`: whether it was one we were waiting for.
    pub(crate) fn answer(&mut self, now: u64, src: Ipv4, port: u16, d: &[u8]) -> bool {
        if d.len() < 12 {
            return false;
        }
        let txid = be16(d, 0);
        let Some(q) = self
            .queries
            .iter_mut()
            .find(|q| q.result.is_none() && q.port == port && q.txid == txid && q.sent.is_some_and(|(_, s)| s == src))
        else {
            return false;
        };
        let flags = be16(d, 2);
        if flags & 0x8000 == 0 || (flags >> 11) & 0xF != 0 {
            return false; // not an answer
        }
        // The question must be ours.
        if be16(d, 4) != 1 {
            return false;
        }
        let Some((asked, mut at)) = read_name(d, 12) else { return false };
        if asked != q.name || d.len() < at + 4 || be16(d, at) != 1 || be16(d, at + 2) != 1 {
            return false;
        }
        at += 4;
        let result = match flags & 0xF {
            0 if flags & 0x0200 != 0 => Err(Error::Failed), // cut short: it would need TCP
            0 => {
                let mut found: Option<(Ipv4, u32)> = None;
                for _ in 0..be16(d, 6) {
                    let Some((_, after)) = read_name(d, at) else { break };
                    if d.len() < after + 10 {
                        break;
                    }
                    let (kind, class, ttl, len) = (be16(d, after), be16(d, after + 2), be32(d, after + 4), be16(d, after + 8) as usize);
                    let data = after + 10;
                    if d.len() < data + len {
                        break;
                    }
                    if kind == 1 && class == 1 && len == 4 && found.is_none() {
                        found = Some((Ipv4([d[data], d[data + 1], d[data + 2], d[data + 3]]), ttl));
                    }
                    at = data + len;
                }
                match found {
                    Some((ip, ttl)) => {
                        let keep = (ttl as u64).clamp(30, 86_400) * 1000;
                        let name = q.name.clone();
                        q.result = Some(Ok(ip));
                        if self.cache.len() >= 64 {
                            self.cache.remove(0);
                        }
                        self.cache.retain(|c| c.0 != name);
                        self.cache.push((name, ip, now + keep));
                        return true;
                    }
                    None => Err(Error::NotFound),
                }
            }
            3 => Err(Error::NotFound),
            _ => Err(Error::Failed),
        };
        q.result = Some(result);
        true
    }
}
