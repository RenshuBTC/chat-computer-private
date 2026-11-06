//! HTTP/1.1 (RFC 9112), the client's side: a GET written out, and a response read as it arrives: its status, its
//! headers, and its body, whether its length is given, it comes in chunks, or it lasts until the connection closes;
//! then its body as the page's bytes, gzip's or DEFLATE's taken off. One request a connection (it asks the server to
//! close after it), and at most `MOST_BYTES` of a body kept (or as many as the response is made for).

use crate::url::Url;
use crate::{MOST_BYTES, USER_AGENT};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// What a page reads: pages and text (the model's tools, and a site's pages).
pub const ACCEPT_PAGE: &str = "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.5";
/// A style sheet, and a picture (in the forms Chat Computer reads).
pub const ACCEPT_CSS: &str = "text/css,*/*;q=0.1";
pub const ACCEPT_PICTURE: &str = "image/webp,image/png,image/jpeg,image/svg+xml,image/gif;q=0.9,*/*;q=0.1";
/// A page's script, and what a page's scripts fetch (anything).
pub const ACCEPT_SCRIPT: &str = "*/*";

/// The request for `url`'s page: GET, with who's asking, what it reads (pages, text) and how it may be compressed.
pub fn get(url: &Url) -> Vec<u8> {
    request(url, ACCEPT_PAGE, None, None, None)
}

/// A request for `url`: GET, or POST with `form`'s bytes (a form's fields, application/x-www-form-urlencoded); what
/// it accepts, the cookies the site set (`cookies`: the Cookie header's value), and the page it's asked from
/// (`referer`, its site only, as browsers send it to another site).
pub fn request(url: &Url, accept: &str, cookies: Option<&str>, form: Option<&[u8]>, referer: Option<&Url>) -> Vec<u8> {
    request_with(url, if form.is_some() { "POST" } else { "GET" }, accept, cookies, &[], form, referer)
}

/// A request a page's script makes: its method and headers its own (but for those only a browser sets), and its
/// body, of the type its Content-Type header says (a form's, if it says none).
pub fn request_with(url: &Url, method: &str, accept: &str, cookies: Option<&str>, headers: &[(String, String)], form: Option<&[u8]>, referer: Option<&Url>) -> Vec<u8> {
    let host = if url.port == 443 { url.host.clone() } else { format!("{}:{}", url.host, url.port) };
    let accept = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("accept")).map_or(accept, |(_, v)| v.as_str());
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {USER_AGENT}\r\nAccept: {accept}\r\nAccept-Language: en\r\n\
         Accept-Encoding: gzip, deflate\r\nConnection: close\r\n",
        method,
        url.path
    );
    // (What a script may set: not those the browser itself sets, nor ones that would break the request.)
    const OURS: [&str; 12] = ["host", "user-agent", "accept", "accept-encoding", "connection", "content-length", "cookie", "referer", "transfer-encoding", "te", "upgrade", "keep-alive"];
    for (k, v) in headers {
        if OURS.iter().any(|o| k.eq_ignore_ascii_case(o)) || k.contains(['\r', '\n', ':']) || v.contains(['\r', '\n']) {
            continue;
        }
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(c) = cookies {
        head.push_str(&format!("Cookie: {c}\r\n"));
    }
    if let Some(r) = referer {
        let from = if r.host == url.host { format!("{r}") } else { format!("https://{}/", r.host) };
        head.push_str(&format!("Referer: {from}\r\n"));
    }
    if let Some(body) = form {
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
            head.push_str("Content-Type: application/x-www-form-urlencoded\r\n");
        }
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    let mut out = head.into_bytes();
    if let Some(body) = form {
        out.extend_from_slice(body);
    }
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
    /// The body so far.
    ready: Vec<u8>,
    done: bool,
    broken: bool,
    /// More came than `most`: what's kept is cut there.
    cut: bool,
    /// The most of its body kept (0: MOST_BYTES).
    most: usize,
}

impl Response {
    pub fn new() -> Response {
        Response::default()
    }

    /// A response that keeps at most `most` bytes of its body (as it came: compressed, if it was), and gives at most
    /// that many (or four times MOST_BYTES, if that's more) once decompressed.
    pub fn with_most(most: usize) -> Response {
        Response { most, ..Response::default() }
    }

    fn most(&self) -> usize {
        if self.most == 0 { MOST_BYTES } else { self.most }
    }

    pub fn status(&self) -> Option<u16> {
        self.status
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// All its headers, as they came.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// Every value of a header that can come more than once (Set-Cookie).
    pub fn headers_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.headers.iter().filter(move |(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Whether the whole response has arrived (or as much of it as is kept).
    pub fn done(&self) -> bool {
        self.done
    }

    /// Whether what arrived wasn't HTTP.
    pub fn broken(&self) -> bool {
        self.broken
    }

    /// Whether the body was cut at the most it keeps.
    pub fn cut(&self) -> bool {
        self.cut
    }

    /// The body as it arrived (still compressed, if it was).
    pub fn body(&self) -> &[u8] {
        &self.ready
    }

    /// The connection closed: a body that lasts until it does is whole; any other isn't, if it hasn't all come.
    pub fn closed(&mut self) {
        if self.body == Some(Body::UntilClose) {
            self.done = true;
        } else if !self.done {
            self.broken = true;
        }
    }

    pub fn feed(&mut self, data: &[u8]) {
        if self.done || self.broken {
            return;
        }
        self.buf.extend_from_slice(data);
        // (An interim response, 1xx, is followed by the real one.)
        while self.body.is_none() {
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
                (true, Ok(c)) if (100..200).contains(&c) => continue,
                (true, Ok(c)) if (200..600).contains(&c) => self.status = Some(c),
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
            if matches!(self.status, Some(204 | 304)) {
                self.body = Some(Body::Length(0));
            }
        }
        match self.body {
            Some(Body::Length(left)) => {
                let n = left.min(self.buf.len());
                self.keep_from_buf(n);
                self.body = Some(Body::Length(left - n));
                if left - n == 0 {
                    self.done = true;
                }
            }
            Some(Body::UntilClose) => {
                let n = self.buf.len();
                self.keep_from_buf(n);
            }
            Some(Body::Chunked) => self.chunks(),
            None => {}
        }
    }

    /// `n` bytes of the body from `buf`, kept up to the most it keeps (past it, the response counts as done, cut).
    fn keep_from_buf(&mut self, n: usize) {
        let room = self.most().saturating_sub(self.ready.len());
        let kept = n.min(room);
        self.ready.extend(self.buf.drain(..kept));
        if kept < n {
            self.buf.clear();
            self.cut = true;
            self.done = true;
        }
    }

    fn chunks(&mut self) {
        loop {
            if self.done {
                return;
            }
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
                    self.keep_from_buf(n);
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

    /// The page's bytes: the body, gzip's or DEFLATE's compression taken off (as the response says it has), at most
    /// four times `MOST_BYTES` of them (or the most the response keeps, if that's more). Err: why they couldn't be.
    pub fn page(&self) -> Result<Vec<u8>, &'static str> {
        let encoding = self.header("content-encoding").map(|e| e.trim().to_ascii_lowercase()).unwrap_or_default();
        let most = (4 * MOST_BYTES).max(self.most());
        let tidy = |e: zip::Error| match e {
            zip::Error::TooBig => "the page is too big to read",
            _ => "the page came compressed in a way that didn't add up",
        };
        match encoding.as_str() {
            "" | "identity" => Ok(self.ready.clone()),
            "gzip" | "x-gzip" => gunzip(&self.ready, most).map_err(tidy),
            "deflate" => zip::zlib(&self.ready, most).or_else(|_| zip::inflate(&self.ready, most)).map_err(tidy),
            _ => Err("the page came compressed in a way Chat Computer doesn't read (not gzip or DEFLATE)"),
        }
    }
}

/// gzip (RFC 1952): its header (and any name, comment, extra field and header checksum it has), then DEFLATE. (Its
/// own checksum isn't checked: TLS has made sure the bytes are what the server sent.)
fn gunzip(data: &[u8], most: usize) -> Result<Vec<u8>, zip::Error> {
    if data.len() < 18 || data[0] != 0x1F || data[1] != 0x8B || data[2] != 8 {
        return Err(zip::Error::NotZip);
    }
    let flags = data[3];
    let mut at = 10;
    if flags & 4 != 0 {
        let len = u16::from_le_bytes([*data.get(at).ok_or(zip::Error::Damaged)?, *data.get(at + 1).ok_or(zip::Error::Damaged)?]);
        at += 2 + len as usize;
    }
    for bit in [8u8, 16] {
        if flags & bit != 0 {
            let end = data.get(at..).and_then(|d| d.iter().position(|&b| b == 0)).ok_or(zip::Error::Damaged)?;
            at += end + 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    zip::inflate(data.get(at..).ok_or(zip::Error::Damaged)?, most)
}
