//! HTTP/1.1 (RFC 9112), the client's side: a request written out, and a response read as it arrives: its status,
//! its headers, and its body, whether its length is given, it comes in chunks (as a stream of events does),
//! or it lasts until the connection closes.

use alloc::string::String;
use alloc::vec::Vec;

/// A request: `method path`, the Host, the headers given, and the body (with its length).
pub fn request(method: &str, host: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut r = String::new();
    r.push_str(method);
    r.push(' ');
    r.push_str(path);
    r.push_str(" HTTP/1.1\r\nHost: ");
    r.push_str(host);
    r.push_str("\r\n");
    for (k, v) in headers {
        r.push_str(k);
        r.push_str(": ");
        r.push_str(v);
        r.push_str("\r\n");
    }
    if !body.is_empty() || method == "POST" {
        r.push_str("Content-Length: ");
        r.push_str(&alloc::format!("{}", body.len()));
        r.push_str("\r\n");
    }
    r.push_str("\r\n");
    let mut out = r.into_bytes();
    out.reserve_exact(body.len());
    out.extend_from_slice(body);
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Body {
    Length(usize),
    Chunked,
    UntilClose,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Chunk {
    Size,
    Data(usize),
    DataEnd,
    Trailers,
}

const HEAD_MAX: usize = 64 * 1024;

#[derive(Default)]
pub struct Response {
    buf: Vec<u8>,
    status: Option<u16>,
    headers: Vec<(String, String)>,
    body: Option<Body>,
    chunk: Option<Chunk>,
    ready: Vec<u8>,
    done: bool,
    broken: bool,
}

impl Response {
    pub fn new() -> Response {
        Response::default()
    }

    pub fn status(&self) -> Option<u16> {
        self.status
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Whether the whole response has arrived.
    pub fn done(&self) -> bool {
        self.done
    }

    /// Whether what arrived wasn't HTTP.
    pub fn broken(&self) -> bool {
        self.broken
    }

    /// The body that has arrived since last asked.
    pub fn take_body(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.ready)
    }

    /// The connection closed.
    pub fn closed(&mut self) {
        if self.body == Some(Body::UntilClose) {
            self.done = true;
        }
    }

    pub fn feed(&mut self, data: &[u8]) {
        if self.done || self.broken {
            return;
        }
        self.buf.extend_from_slice(data);
        if self.body.is_none() {
            let Some(end) = self.buf.windows(4).position(|w| w == b"\r\n\r\n") else {
                if self.buf.len() > HEAD_MAX {
                    self.broken = true;
                }
                return;
            };
            let head = String::from_utf8_lossy(&self.buf[..end]).into_owned();
            self.buf.drain(..end + 4);
            let mut lines = head.split("\r\n");
            let first = lines.next().unwrap_or("");
            let mut parts = first.splitn(3, ' ');
            let (version, code) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            match (version.starts_with("HTTP/1."), code.parse::<u16>()) {
                (true, Ok(c)) if (100..600).contains(&c) => self.status = Some(c),
                _ => {
                    self.broken = true;
                    return;
                }
            }
            for line in lines {
                if let Some((k, v)) = line.split_once(':') {
                    self.headers.push((String::from(k.trim()), String::from(v.trim())));
                }
            }
            let chunked = self.header("transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked"));
            self.body = Some(if chunked {
                self.chunk = Some(Chunk::Size);
                Body::Chunked
            } else if let Some(n) = self.header("content-length").and_then(|n| n.parse().ok()) {
                Body::Length(n)
            } else {
                Body::UntilClose
            });
            if self.status == Some(204) || self.status == Some(304) {
                self.body = Some(Body::Length(0));
            }
        }
        match self.body {
            Some(Body::Length(left)) => {
                let n = left.min(self.buf.len());
                self.ready.extend(self.buf.drain(..n));
                self.body = Some(Body::Length(left - n));
                if left - n == 0 {
                    self.done = true;
                }
            }
            Some(Body::UntilClose) => self.ready.append(&mut self.buf),
            Some(Body::Chunked) => self.chunks(),
            None => {}
        }
    }

    fn chunks(&mut self) {
        loop {
            match self.chunk {
                Some(Chunk::Size) => {
                    let Some(end) = self.buf.windows(2).position(|w| w == b"\r\n") else { return };
                    let line = String::from_utf8_lossy(&self.buf[..end]).into_owned();
                    self.buf.drain(..end + 2);
                    let hex = line.split(';').next().unwrap_or("").trim();
                    match usize::from_str_radix(hex, 16) {
                        Ok(0) => self.chunk = Some(Chunk::Trailers),
                        Ok(n) if n < 1 << 28 => self.chunk = Some(Chunk::Data(n)),
                        _ => {
                            self.broken = true;
                            return;
                        }
                    }
                }
                Some(Chunk::Data(left)) => {
                    if self.buf.is_empty() {
                        return;
                    }
                    let n = left.min(self.buf.len());
                    self.ready.extend(self.buf.drain(..n));
                    self.chunk = Some(if left == n { Chunk::DataEnd } else { Chunk::Data(left - n) });
                }
                Some(Chunk::DataEnd) => {
                    if self.buf.len() < 2 {
                        return;
                    }
                    if &self.buf[..2] != b"\r\n" {
                        self.broken = true;
                        return;
                    }
                    self.buf.drain(..2);
                    self.chunk = Some(Chunk::Size);
                }
                Some(Chunk::Trailers) => {
                    let Some(end) = self.buf.windows(2).position(|w| w == b"\r\n") else { return };
                    self.buf.drain(..end + 2);
                    if end == 0 {
                        self.done = true;
                        return;
                    }
                }
                None => return,
            }
        }
    }
}
