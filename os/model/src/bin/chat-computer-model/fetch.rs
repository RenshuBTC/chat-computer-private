//! Getting files from the web by Chat Computer's own HTTPS: its own TLS 1.3 (os/tls, with its own cryptography and
//! Mozilla's authorities) over this computer's TCP, and HTTP/1.1 written and read here. It follows redirects (a
//! model's file on Hugging Face is sent from its storage), asks for the rest of a file from where it stopped
//! (Range), and goes through a proxy where one is set (HTTPS_PROXY), as other programs do. It also sends a request
//! with a body (`request`: a POST, its answer read as it comes), for the desktop app's sign-in with ChatGPT and its
//! questions to it (apps/desktop/src-tauri/src/chatgpt.rs, which builds this same file in). A proxy that reads what
//! passes through it (a company's, or a test machine's) is trusted with CHATCOMPUTER_EXTRA_CA: a PEM file of its
//! authority.
//!
//! It offers ChaCha20-Poly1305 alone first (the own AES is the constant-time kind without the processor's AES
//! instructions: slow for gigabytes), and all three ciphers only to a server that has none of it.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tls::{Cert, Client, Suite};

const REDIRECTS: usize = 8;
const HEAD_MAX: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub host: String,
    pub port: u16,
    /// With its query.
    pub path: String,
}

impl Url {
    pub fn parse(s: &str) -> Result<Url, String> {
        let rest = s.strip_prefix("https://").ok_or_else(|| format!("not an https address: {s}"))?;
        let (authority, path) = match rest.find(['/', '?']) {
            Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], rest[i..].to_string()),
            Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
            None => (rest, "/".to_string()),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => {
                (h, p.parse::<u16>().map_err(|_| format!("a bad port in {s}"))?)
            }
            _ => (authority, 443),
        };
        let ok = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '.';
        if host.is_empty() || !host.chars().all(ok) || path.contains(|c: char| c.is_whitespace() || c.is_control()) {
            return Err(format!("not an address this can fetch: {s}"));
        }
        Ok(Url { host: host.to_ascii_lowercase(), port, path })
    }

    /// Where a redirect sends it (the Location header): another https address, or another path on this one.
    pub fn join(&self, location: &str) -> Result<Url, String> {
        if location.starts_with("https://") {
            Url::parse(location)
        } else if let Some(rest) = location.strip_prefix("//") {
            Url::parse(&format!("https://{rest}"))
        } else if location.starts_with('/') && !location.contains(|c: char| c.is_whitespace() || c.is_control()) {
            Ok(Url { host: self.host.clone(), port: self.port, path: location.to_string() })
        } else {
            Err(format!("a redirect to somewhere other than an https address: {location}"))
        }
    }
}

/// The certificate authorities trusted: Mozilla's, and those in CHATCOMPUTER_EXTRA_CA.
pub fn roots() -> &'static [Cert<'static>] {
    static ROOTS: OnceLock<&'static [Cert<'static>]> = OnceLock::new();
    ROOTS.get_or_init(|| {
        let mut all = tls::roots();
        if let Some(path) = std::env::var_os("CHATCOMPUTER_EXTRA_CA")
            && let Ok(pem) = std::fs::read_to_string(&path)
        {
            for der in pem_certificates(&pem) {
                let der: &'static [u8] = Box::leak(der.into_boxed_slice());
                if let Ok(c) = tls::x509::parse_root(der) {
                    all.push(c);
                }
            }
        }
        Box::leak(all.into_boxed_slice())
    })
}

/// The certificates in a PEM file, as DER.
pub fn pem_certificates(pem: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut inside: Option<String> = None;
    for line in pem.lines().map(str::trim) {
        if line == "-----BEGIN CERTIFICATE-----" {
            inside = Some(String::new());
        } else if line == "-----END CERTIFICATE-----" {
            if let Some(b64) = inside.take()
                && let Some(der) = base64(&b64)
            {
                out.push(der);
            }
        } else if let Some(b) = inside.as_mut() {
            b.push_str(line);
        }
    }
    out
}

fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The proxy to go through for `host`, if one is set and `host` isn't one it's to be left out for.
fn proxy_for(host: &str) -> Option<(String, u16)> {
    let p = std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("https_proxy")).ok()?;
    let none = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")).unwrap_or_default();
    for n in none.split(',').map(|n| n.trim().trim_start_matches('*').to_ascii_lowercase()) {
        if n == "*" || (!n.is_empty() && (host == n.trim_start_matches('.') || host.ends_with(&format!(".{}", n.trim_start_matches('.'))))) {
            return None;
        }
    }
    let rest = p.trim().strip_prefix("http://").unwrap_or(p.trim());
    let rest = rest.rsplit_once('@').map(|(_, r)| r).unwrap_or(rest).trim_end_matches('/');
    let (h, port) = rest.rsplit_once(':')?;
    Some((h.trim_matches(['[', ']']).to_string(), port.parse().ok()?))
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// A TLS connection to a server, over TCP (through the proxy, where there is one).
struct Conn {
    sock: TcpStream,
    tls: Client,
    buf: Vec<u8>,
}

impl Conn {
    fn open(url: &Url, random: &mut dyn FnMut(&mut [u8]), suites: &[Suite]) -> Result<Conn, String> {
        let sock = match proxy_for(&url.host) {
            Some((ph, pp)) => {
                let mut s = TcpStream::connect((ph.as_str(), pp)).map_err(|e| format!("the proxy {ph}:{pp}: {e}"))?;
                let target = format!("{}:{}", url.host, url.port);
                s.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes()).map_err(|e| e.to_string())?;
                s.set_read_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if head.len() > HEAD_MAX || s.read(&mut b).map_err(|e| format!("the proxy: {e}"))? == 0 {
                        return Err("the proxy closed the connection".into());
                    }
                    head.push(b[0]);
                }
                let line = String::from_utf8_lossy(&head);
                if line.split_whitespace().nth(1) != Some("200") {
                    return Err(format!("the proxy said {}", line.lines().next().unwrap_or("?")));
                }
                s
            }
            None => TcpStream::connect((url.host.as_str(), url.port)).map_err(|e| format!("{}: {e}", url.host))?,
        };
        sock.set_read_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
        sock.set_write_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
        let _ = sock.set_nodelay(true);
        let tls = Client::offering(&url.host, now(), roots(), random, suites);
        let mut c = Conn { sock, tls, buf: vec![0u8; 64 * 1024] };
        while !c.tls.is_open() {
            c.send()?;
            if let Some(e) = c.tls.error() {
                return Err(format!("{}: {e}", url.host));
            }
            c.receive()?;
        }
        Ok(c)
    }

    /// What the TLS client has to send, sent.
    fn send(&mut self) -> Result<(), String> {
        let out = self.tls.outgoing();
        if !out.is_empty() {
            self.sock.write_all(&out).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// The next bytes from the server, to the TLS client; false when the connection closed.
    fn receive(&mut self) -> Result<bool, String> {
        match self.sock.read(&mut self.buf) {
            Ok(0) => Ok(false),
            Ok(n) => {
                self.tls.incoming(&self.buf[..n]);
                Ok(true)
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

/// What comes back, piece by piece: its head, then its body.
pub enum Part<'a> {
    /// The status (200 for the whole file, 206 for the rest from where it was asked), and its body's length.
    Head { status: u16, length: Option<u64> },
    Body(&'a [u8]),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Framing {
    Length(u64),
    Chunked,
    UntilClose,
}

/// A body in chunks (Transfer-Encoding: chunked), undone as it comes.
#[derive(Default)]
struct Chunks {
    line: Vec<u8>,
    left: u64,
    in_data: bool,
    crlf: u8,
    trailers: bool,
    done: bool,
}

impl Chunks {
    /// The body's bytes in `data`, given to `out`.
    fn feed(&mut self, mut data: &[u8], out: &mut dyn FnMut(&[u8]) -> Result<bool, String>) -> Result<bool, String> {
        while !data.is_empty() && !self.done {
            if self.in_data {
                let n = (self.left as usize).min(data.len());
                if !out(&data[..n])? {
                    return Ok(false);
                }
                self.left -= n as u64;
                data = &data[n..];
                if self.left == 0 {
                    self.in_data = false;
                    self.crlf = 2;
                }
            } else if self.crlf > 0 {
                self.crlf -= 1;
                data = &data[1..];
            } else {
                let b = data[0];
                data = &data[1..];
                if b != b'\n' {
                    if self.line.len() > 1024 {
                        return Err("a chunk's size line too long".into());
                    }
                    self.line.push(b);
                    continue;
                }
                let line = String::from_utf8_lossy(&self.line).trim().to_string();
                self.line.clear();
                if self.trailers {
                    if line.is_empty() {
                        self.done = true;
                    }
                    continue;
                }
                let size = u64::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16)
                    .map_err(|_| format!("a chunk's size isn't a number: {line:?}"))?;
                if size == 0 {
                    self.trailers = true;
                } else {
                    self.left = size;
                    self.in_data = true;
                }
            }
        }
        Ok(true)
    }
}

/// GETs `url`, from byte `from` on (0: all of it), following redirects; `on` is given its head, then its body in
/// pieces, and stops it by returning false. `random` fills buffers with hard-to-guess bytes (for TLS).
pub fn get(url: &str, from: u64, random: &mut dyn FnMut(&mut [u8]), on: &mut dyn FnMut(Part) -> Result<bool, String>)
    -> Result<(), String> {
    exchange("GET", url, &[], b"", from, random, on)
}

/// Sends `body` to `url` (`method`: POST, as a rule), with `headers` besides Host and Content-Length, and hands `on`
/// what comes back as `get` does, its body as it streams in. Not redirected: an answer that says go elsewhere is
/// given to `on` as it is.
#[allow(dead_code)]
pub fn request(method: &str, url: &str, headers: &[(&str, &str)], body: &[u8], random: &mut dyn FnMut(&mut [u8]),
               on: &mut dyn FnMut(Part) -> Result<bool, String>) -> Result<(), String> {
    exchange(method, url, headers, body, 0, random, on)
}

fn exchange(method: &str, url: &str, extra: &[(&str, &str)], body: &[u8], from: u64, random: &mut dyn FnMut(&mut [u8]),
            on: &mut dyn FnMut(Part) -> Result<bool, String>) -> Result<(), String> {
    let mut url = Url::parse(url)?;
    let token = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if !token(method) {
        return Err(format!("not a method: {method:?}"));
    }
    let mut more_headers = String::new();
    for (k, v) in extra {
        if !token(k) || v.contains(['\r', '\n']) {
            return Err(format!("not a header to send: {k:?}"));
        }
        more_headers.push_str(&format!("{k}: {v}\r\n"));
    }
    for _ in 0..=REDIRECTS {
        let mut conn = match Conn::open(&url, random, &[Suite::ChaCha20Poly1305]) {
            Ok(c) => c,
            // (A server without ChaCha20-Poly1305 says so with handshake_failure: then all three.)
            Err(e) if e.ends_with("(TLS alert 40)") => {
                Conn::open(&url, random, &[Suite::ChaCha20Poly1305, Suite::Aes128Gcm, Suite::Aes256Gcm])?
            }
            Err(e) => return Err(e),
        };
        let range = if from > 0 { format!("Range: bytes={from}-\r\n") } else { String::new() };
        let host = if url.port == 443 { url.host.clone() } else { format!("{}:{}", url.host, url.port) };
        let length = if method == "GET" && body.is_empty() { String::new() } else { format!("Content-Length: {}\r\n", body.len()) };
        let accept = if extra.iter().any(|(k, _)| k.eq_ignore_ascii_case("accept")) { "" } else { "Accept: */*\r\n" };
        let request = format!(
            "{method} {} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: ChatComputer/1.0\r\n{accept}Accept-Encoding: identity\r\n\
             {range}{more_headers}{length}Connection: close\r\n\r\n",
            url.path
        );
        conn.tls.write(request.as_bytes());
        if !body.is_empty() {
            conn.tls.write(body);
        }
        conn.send()?;
        // The head.
        let mut head = Vec::new();
        let rest;
        let mut app = vec![0u8; 64 * 1024];
        let at = loop {
            let n = conn.tls.read(&mut app);
            if n > 0 {
                head.extend_from_slice(&app[..n]);
                if let Some(i) = head.windows(4).position(|w| w == b"\r\n\r\n") {
                    rest = head.split_off(i + 4);
                    break i;
                }
                if head.len() > HEAD_MAX {
                    return Err(format!("{}: a response head too long", url.host));
                }
                continue;
            }
            conn.send()?;
            if let Some(e) = conn.tls.error() {
                return Err(format!("{}: {e}", url.host));
            }
            if conn.tls.finished() || !conn.receive()? {
                return Err(format!("{}: the connection closed before an answer", url.host));
            }
        };
        let text = String::from_utf8_lossy(&head[..at]).to_string();
        let mut lines = text.split("\r\n");
        let status: u16 = lines.next().and_then(|l| l.split_whitespace().nth(1)).and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("{}: not an HTTP answer", url.host))?;
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let header = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
        if method == "GET" && matches!(status, 301 | 302 | 303 | 307 | 308) {
            url = url.join(header("location").ok_or_else(|| format!("{}: a redirect without where to", url.host))?)?;
            continue;
        }
        let framing = if header("transfer-encoding").is_some_and(|t| t.to_ascii_lowercase().contains("chunked")) {
            Framing::Chunked
        } else if let Some(n) = header("content-length").and_then(|v| v.parse::<u64>().ok()) {
            Framing::Length(n)
        } else {
            Framing::UntilClose
        };
        let length = match framing {
            Framing::Length(n) => Some(n),
            _ => None,
        };
        if !on(Part::Head { status, length })? {
            return Ok(());
        }
        // The body.
        let mut chunks = Chunks::default();
        let mut got = 0u64;
        let mut deliver = |data: &[u8], got: &mut u64| -> Result<bool, String> {
            match framing {
                Framing::Length(n) => {
                    let take = ((n - *got) as usize).min(data.len());
                    *got += take as u64;
                    if take > 0 && !on(Part::Body(&data[..take]))? {
                        return Ok(false);
                    }
                    Ok(*got < n)
                }
                Framing::Chunked => {
                    let more = chunks.feed(data, &mut |d| on(Part::Body(d)))?;
                    Ok(more && !chunks.done)
                }
                Framing::UntilClose => {
                    if !data.is_empty() && !on(Part::Body(data))? {
                        return Ok(false);
                    }
                    Ok(true)
                }
            }
        };
        let mut more = framing != Framing::Length(0);
        if more && !rest.is_empty() {
            more = deliver(&rest, &mut got)?;
        }
        while more {
            let n = conn.tls.read(&mut app);
            if n > 0 {
                more = deliver(&app[..n], &mut got)?;
                continue;
            }
            conn.send()?;
            if let Some(e) = conn.tls.error() {
                return Err(format!("{}: {e}", url.host));
            }
            if conn.tls.finished() || !conn.receive()? {
                break;
            }
        }
        if let Framing::Length(n) = framing
            && got < n
        {
            return Err(format!("{}: the connection closed {} bytes short", url.host, n - got));
        }
        if framing == Framing::Chunked && !chunks.done {
            return Err(format!("{}: the connection closed before the last chunk", url.host));
        }
        return Ok(());
    }
    Err(format!("{}: too many redirects", url.host))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_redirects() {
        let u = Url::parse("https://huggingface.co/Qwen/Qwen3.5-0.8B/resolve/abc/config.json").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("huggingface.co", 443, "/Qwen/Qwen3.5-0.8B/resolve/abc/config.json"));
        let r = u.join("https://cas-bridge.xethub.hf.co/xet-bridge-us/a?X-Amz-Signature=1&b=2").unwrap();
        assert_eq!((r.host.as_str(), r.path.as_str()), ("cas-bridge.xethub.hf.co", "/xet-bridge-us/a?X-Amz-Signature=1&b=2"));
        assert_eq!(u.join("/api/x").unwrap().host, "huggingface.co");
        assert_eq!(Url::parse("https://Example.COM:8443?q=1").unwrap(), Url { host: "example.com".into(), port: 8443, path: "/?q=1".into() });
        for bad in ["http://example.com/", "https://", "https://a b/", "https://user@host/", "https://h/\r\nX: y"] {
            assert!(Url::parse(bad).is_err(), "{bad}");
        }
        assert!(u.join("ftp://x/").is_err() && u.join("relative").is_err());
    }

    #[test]
    fn chunks_are_undone() {
        let body = b"4\r\nWiki\r\n6;ext=1\r\npedia \r\nE\r\nin \r\n\r\nchunks.\r\n0\r\nX-Trailer: 1\r\n\r\n";
        // In every split, the same.
        for split in [1, 3, 7, body.len()] {
            let mut c = Chunks::default();
            let mut got = Vec::new();
            for piece in body.chunks(split) {
                c.feed(piece, &mut |d| {
                    got.extend_from_slice(d);
                    Ok(true)
                })
                .unwrap();
            }
            assert!(c.done);
            assert_eq!(String::from_utf8(got).unwrap(), "Wikipedia in \r\n\r\nchunks.");
        }
    }

    #[test]
    fn pem_reads() {
        let pem = "junk\n-----BEGIN CERTIFICATE-----\nTWFu\nTWE=\n-----END CERTIFICATE-----\n";
        assert_eq!(pem_certificates(pem), vec![b"ManMa".to_vec()]);
    }
}
