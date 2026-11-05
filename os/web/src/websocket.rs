//! WebSockets (RFC 6455) for pages' scripts: the opening handshake (the request, and the server's answer checked:
//! its 101, its Upgrade and Connection, its Sec-WebSocket-Accept made from the key with SHA-1, the protocol it chose),
//! and frames: the client's, masked; the server's, read as they come, a message's fragments joined, its pings and
//! its close told apart.

use crate::url::Url;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The GUID the handshake's key is joined with (RFC 6455, section 1.3).
pub const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// The largest message this takes (bigger ones close the connection, 1009).
pub const MOST_MESSAGE: usize = 16 << 20;

pub const TEXT: u8 = 1;
pub const BINARY: u8 = 2;
pub const CLOSE: u8 = 8;
pub const PING: u8 = 9;
pub const PONG: u8 = 10;

/// SHA-1 (FIPS 180-4), for the handshake's Sec-WebSocket-Accept only.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// Base64 (with padding), for the handshake's key.
pub fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// What the server's Sec-WebSocket-Accept must be for a key.
pub fn accept_for(key: &str) -> String {
    let mut joined = String::from(key);
    joined.push_str(GUID);
    base64(&sha1(joined.as_bytes()))
}

/// The opening handshake's request: the socket's path, the Upgrade, its key (16 random bytes, in base64), the page's
/// origin, the protocols the script asked for, and the site's cookies.
pub fn handshake(url: &Url, key: &str, origin: &str, protocols: &[String], cookies: Option<&str>) -> Vec<u8> {
    let host = if url.port == 443 { url.host.clone() } else { format!("{}:{}", url.host, url.port) };
    let mut head = format!(
        "GET {} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nOrigin: {origin}\r\n",
        url.path,
        crate::USER_AGENT
    );
    if !protocols.is_empty() {
        head.push_str(&format!("Sec-WebSocket-Protocol: {}\r\n", protocols.join(", ")));
    }
    if let Some(c) = cookies {
        head.push_str(&format!("Cookie: {c}\r\n"));
    }
    head.push_str("\r\n");
    head.into_bytes()
}

/// The server's answer to the handshake, as far as it's come.
#[derive(Debug, PartialEq)]
pub enum Answer {
    /// Not all of it yet.
    More,
    /// Accepted: the protocol it chose (or ""), the cookies it set, and what came after its head (frames already).
    Accepted { protocol: String, set_cookies: Vec<String>, rest: Vec<u8> },
    /// Refused, and why.
    Refused(String),
}

/// Reads the server's answer to the handshake (`key`, the protocols asked for) from what's come.
pub fn answer(buf: &[u8], key: &str, protocols: &[String]) -> Answer {
    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        return if buf.len() > 64 << 10 { Answer::Refused(String::from("the server's answer was too long")) } else { Answer::More };
    };
    let head = String::from_utf8_lossy(&buf[..end]);
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or("");
    let code = status.split(' ').nth(1).unwrap_or("");
    if code != "101" {
        return Answer::Refused(format!("the server answered {} instead of switching to a WebSocket", status.trim()));
    }
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), String::from(v.trim())))
        .collect();
    let header = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
    if !header("upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket")) {
        return Answer::Refused(String::from("the server didn't upgrade the connection to a WebSocket"));
    }
    if !header("connection").is_some_and(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case("upgrade"))) {
        return Answer::Refused(String::from("the server's Connection header doesn't say Upgrade"));
    }
    if header("sec-websocket-accept") != Some(accept_for(key).as_str()) {
        return Answer::Refused(String::from("the server's Sec-WebSocket-Accept doesn't match the key"));
    }
    if header("sec-websocket-extensions").is_some_and(|v| !v.is_empty()) {
        return Answer::Refused(String::from("the server chose an extension that wasn't asked for"));
    }
    let protocol = String::from(header("sec-websocket-protocol").unwrap_or(""));
    if !protocol.is_empty() && !protocols.contains(&protocol) {
        return Answer::Refused(format!("the server chose a protocol that wasn't asked for ({protocol})"));
    }
    let set_cookies = headers.iter().filter(|(k, _)| k == "set-cookie").map(|(_, v)| v.clone()).collect();
    Answer::Accepted { protocol, set_cookies, rest: buf[end + 4..].to_vec() }
}

/// A frame from the client: whole (FIN), its payload masked with `mask` (as a client's must be).
pub fn frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(0x80 | opcode);
    let n = payload.len();
    if n < 126 {
        out.push(0x80 | n as u8);
    } else if n <= 0xFFFF {
        out.push(0x80 | 126);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(n as u64).to_be_bytes());
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

/// A close frame's payload: its code and reason.
pub fn close_payload(code: Option<u16>, reason: &str) -> Vec<u8> {
    let Some(code) = code else { return Vec::new() };
    let mut p = code.to_be_bytes().to_vec();
    p.extend_from_slice(reason.as_bytes());
    p
}

/// What came from the server: a message (text or binary), a ping (to answer with a pong), a pong, its close (its
/// code and reason), or a breach of the protocol (the code to close with, and why).
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong,
    Close(Option<u16>, String),
    Error(u16, String),
}

/// The server's frames, read as their bytes come; a message's fragments joined.
#[derive(Default)]
pub struct Reader {
    buf: Vec<u8>,
    /// A message coming in fragments: its kind (TEXT or BINARY) and what's come of it.
    partial: Option<(u8, Vec<u8>)>,
}

impl Reader {
    pub fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next thing that's come whole, if one has.
    pub fn next(&mut self) -> Option<Incoming> {
        loop {
            let b = &self.buf;
            if b.len() < 2 {
                return None;
            }
            let (fin, opcode, masked) = (b[0] & 0x80 != 0, b[0] & 0x0F, b[1] & 0x80 != 0);
            if b[0] & 0x70 != 0 {
                return Some(self.fail(1002, "a frame with reserved bits set"));
            }
            if masked {
                return Some(self.fail(1002, "a masked frame from the server"));
            }
            let (len, at) = match b[1] & 0x7F {
                126 if b.len() >= 4 => (u16::from_be_bytes([b[2], b[3]]) as u64, 4),
                127 if b.len() >= 10 => (u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]), 10),
                126 | 127 => return None,
                n => (n as u64, 2),
            };
            if len > MOST_MESSAGE as u64 {
                return Some(self.fail(1009, "a message too big"));
            }
            let len = len as usize;
            if b.len() < at + len {
                return None;
            }
            let payload = b[at..at + len].to_vec();
            self.buf.drain(..at + len);
            match opcode {
                CLOSE | PING | PONG => {
                    if !fin || len > 125 {
                        return Some(self.fail(1002, "a control frame in fragments, or too long"));
                    }
                    return Some(match opcode {
                        PING => Incoming::Ping(payload),
                        PONG => Incoming::Pong,
                        _ => {
                            if payload.len() == 1 {
                                return Some(self.fail(1002, "a close frame of one byte"));
                            }
                            let code = (payload.len() >= 2).then(|| u16::from_be_bytes([payload[0], payload[1]]));
                            let reason = payload.get(2..).map(|r| String::from_utf8_lossy(r).into_owned()).unwrap_or_default();
                            Incoming::Close(code, reason)
                        }
                    });
                }
                TEXT | BINARY => {
                    if self.partial.is_some() {
                        return Some(self.fail(1002, "a new message before the last one's end"));
                    }
                    if fin {
                        return Some(self.message(opcode, payload));
                    }
                    self.partial = Some((opcode, payload));
                }
                0 => {
                    let Some((kind, mut got)) = self.partial.take() else {
                        return Some(self.fail(1002, "a continuation with no message to go on"));
                    };
                    got.extend_from_slice(&payload);
                    if got.len() > MOST_MESSAGE {
                        return Some(self.fail(1009, "a message too big"));
                    }
                    if fin {
                        return Some(self.message(kind, got));
                    }
                    self.partial = Some((kind, got));
                }
                _ => return Some(self.fail(1002, "a frame of an unknown kind")),
            }
        }
    }

    fn message(&mut self, kind: u8, payload: Vec<u8>) -> Incoming {
        if kind == BINARY {
            return Incoming::Binary(payload);
        }
        match String::from_utf8(payload) {
            Ok(s) => Incoming::Text(s),
            Err(_) => self.fail(1007, "text that isn't UTF-8"),
        }
    }

    fn fail(&mut self, code: u16, why: &str) -> Incoming {
        self.buf.clear();
        self.partial = None;
        Incoming::Error(code, format!("the server sent {why}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handshake_key_and_its_answer() {
        // RFC 6455's own example (section 1.3).
        assert_eq!(accept_for("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
        assert_eq!(base64(&sha1(b"abc")), "qZk+NkcGgWq6PiVxeFDCbJzQ2J0=");
        let ok = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\nSec-WebSocket-Protocol: chat\r\n\r\n\x81\x02hi";
        let protocols = [String::from("chat")];
        assert_eq!(answer(&ok[..20], "dGhlIHNhbXBsZSBub25jZQ==", &protocols), Answer::More);
        assert_eq!(
            answer(ok, "dGhlIHNhbXBsZSBub25jZQ==", &protocols),
            Answer::Accepted { protocol: String::from("chat"), set_cookies: Vec::new(), rest: b"\x81\x02hi".to_vec() }
        );
        assert!(matches!(answer(ok, "another key", &protocols), Answer::Refused(_)));
        assert!(matches!(answer(b"HTTP/1.1 403 Forbidden\r\n\r\n", "k", &[]), Answer::Refused(_)));
    }

    #[test]
    fn frames_written_masked_and_read_whole() {
        let f = frame(TEXT, b"Hello", [0x37, 0xfa, 0x21, 0x3d]);
        // RFC 6455, section 5.7: a masked "Hello".
        assert_eq!(f, [0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58]);
        assert_eq!(frame(BINARY, &[0u8; 300], [0; 4])[1..4], [0xFE, 0x01, 0x2C]);
        let mut r = Reader::default();
        // A fragmented "Hel" + "lo", a ping between, a close.
        r.feed(&[0x01, 0x03, b'H', b'e', b'l', 0x89, 0x00]);
        assert_eq!(r.next(), Some(Incoming::Ping(Vec::new())));
        assert_eq!(r.next(), None);
        r.feed(&[0x80, 0x02, b'l', b'o', 0x88, 0x05, 0x03, 0xE8, b'b', b'y', b'e']);
        assert_eq!(r.next(), Some(Incoming::Text(String::from("Hello"))));
        assert_eq!(r.next(), Some(Incoming::Close(Some(1000), String::from("bye"))));
        let mut r = Reader::default();
        r.feed(&[0x82, 0x7E, 0x01, 0x00]);
        assert_eq!(r.next(), None);
        r.feed(&[7u8; 256]);
        assert_eq!(r.next(), Some(Incoming::Binary(alloc::vec![7u8; 256])));
        let mut r = Reader::default();
        r.feed(&[0x81, 0x82, 1, 2, 3, 4, 0, 0]);
        assert!(matches!(r.next(), Some(Incoming::Error(1002, _))));
        let mut r = Reader::default();
        r.feed(&[0x81, 0x02, 0xC3, 0x28]);
        assert!(matches!(r.next(), Some(Incoming::Error(1007, _))));
    }
}
