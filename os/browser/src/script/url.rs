//! Addresses as scripts see them (the WHATWG URL standard's parser, the parts pages use): any scheme, a user name
//! and password, the host, the port, the path, the query and the fragment; relative ones resolved against a base;
//! and application/x-www-form-urlencoded, for URLSearchParams and forms.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Url {
    /// Lower case, without its ":".
    pub scheme: String,
    pub username: String,
    pub password: String,
    /// None for an address with no host (mailto:, data:...).
    pub host: Option<String>,
    pub port: Option<u16>,
    /// The path (from its "/"), or an opaque one's text (mailto:x's "x").
    pub path: String,
    /// Whether the path is opaque (a scheme that isn't the web's and no "//").
    pub opaque: bool,
    pub query: Option<String>,
    pub fragment: Option<String>,
}

/// The schemes with hosts and their default ports.
fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        "ftp" => Some(21),
        _ => None,
    }
}

pub fn special(scheme: &str) -> bool {
    matches!(scheme, "http" | "https" | "ws" | "wss" | "ftp" | "file")
}

impl Url {
    /// An address, resolved against `base` if it's relative; None if it isn't one.
    pub fn parse(input: &str, base: Option<&Url>) -> Option<Url> {
        let s: String = input.trim_matches(|c: char| c <= ' ').chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
        let s = s.as_str();
        // A scheme?
        let scheme_end = s.find(':').filter(|&c| {
            c > 0 && s.as_bytes()[0].is_ascii_alphabetic() && s[..c].bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        });
        if let Some(c) = scheme_end {
            let scheme = s[..c].to_ascii_lowercase();
            let rest = &s[c + 1..];
            if special(&scheme) {
                let slashes = rest.starts_with("//") || rest.starts_with("\\\\") || rest.starts_with("/\\") || rest.starts_with("\\/");
                if !slashes
                    && let Some(b) = base
                    && b.scheme == scheme
                    && scheme != "file"
                {
                    // ("http:foo" against an http base: relative.)
                    return Url::relative(rest, b);
                }
                let rest = rest.trim_start_matches(['/', '\\']);
                return Url::authority_and_rest(&scheme, rest);
            }
            if let Some(after) = rest.strip_prefix("//") {
                return Url::authority_and_rest(&scheme, after);
            }
            // An opaque path.
            let (body, fragment) = split_fragment(rest);
            let (path, query) = split_query(body);
            return Some(Url {
                scheme,
                path: escape(path, OPAQUE),
                opaque: true,
                query: query.map(|q| escape(q, QUERY)),
                fragment: fragment.map(|f| escape(f, FRAGMENT)),
                ..Url::default()
            });
        }
        let base = base?;
        if base.opaque {
            let frag = s.strip_prefix('#')?;
            let mut u = base.clone();
            u.fragment = Some(escape(frag, FRAGMENT));
            return Some(u);
        }
        Url::relative(s, base)
    }

    /// A relative address, against a base that isn't opaque.
    fn relative(s: &str, base: &Url) -> Option<Url> {
        let sp = special(&base.scheme);
        let slash = |c: char| c == '/' || (sp && c == '\\');
        let mut chars = s.chars();
        let first = chars.next();
        let second = chars.next();
        if first.is_some_and(slash) && second.is_some_and(slash) {
            return Url::authority_and_rest(&base.scheme, &s[2..]);
        }
        let mut u = Url { scheme: base.scheme.clone(), username: base.username.clone(), password: base.password.clone(), host: base.host.clone(), port: base.port, ..Url::default() };
        let (body, fragment) = split_fragment(s);
        u.fragment = fragment.map(|f| escape(f, FRAGMENT));
        if body.is_empty() {
            u.path = base.path.clone();
            u.query = base.query.clone();
            return Some(u);
        }
        let (path, query) = split_query(body);
        u.query = query.map(|q| escape(q, if sp { SPECIAL_QUERY } else { QUERY }));
        if path.is_empty() {
            u.path = base.path.clone();
            if query.is_none() {
                u.query = base.query.clone();
            }
            return Some(u);
        }
        let joined = if path.starts_with(slash) {
            String::from(path)
        } else {
            let dir = &base.path[..base.path.rfind('/').map_or(0, |i| i + 1)];
            format!("{}{path}", if dir.is_empty() { "/" } else { dir })
        };
        u.path = tidy(&escape(&joined.replace('\\', if sp { "/" } else { "\\" }), PATH));
        Some(u)
    }

    /// What follows "scheme://": the authority, then the path, query and fragment.
    fn authority_and_rest(scheme: &str, s: &str) -> Option<Url> {
        let sp = special(scheme);
        let end = s.find(|c: char| c == '/' || c == '?' || c == '#' || (sp && c == '\\')).unwrap_or(s.len());
        let (authority, tail) = s.split_at(end);
        let mut u = Url { scheme: String::from(scheme), ..Url::default() };
        let hostport = match authority.rfind('@') {
            Some(at) => {
                let info = &authority[..at];
                let (user, pass) = info.split_once(':').unwrap_or((info, ""));
                u.username = escape(user, USERINFO);
                u.password = escape(pass, USERINFO);
                &authority[at + 1..]
            }
            None => authority,
        };
        let (host, port) = match hostport.rfind(':') {
            Some(c) if !hostport[c..].contains(']') => (&hostport[..c], Some(&hostport[c + 1..])),
            _ => (hostport, None),
        };
        if let Some(p) = port
            && !p.is_empty()
        {
            let n: u32 = p.parse().ok()?;
            if n > 65535 {
                return None;
            }
            if default_port(scheme) != Some(n as u16) {
                u.port = Some(n as u16);
            }
        }
        if host.is_empty() && sp && scheme != "file" {
            return None;
        }
        let host = if sp { host.to_ascii_lowercase() } else { String::from(host) };
        if host.chars().any(|c| matches!(c, ' ' | '<' | '>' | '^' | '|' | '%' | '\\')) && sp {
            return None;
        }
        u.host = Some(host);
        let (body, fragment) = split_fragment(tail);
        let (path, query) = split_query(body);
        u.fragment = fragment.map(|f| escape(f, FRAGMENT));
        u.query = query.map(|q| escape(q, if sp { SPECIAL_QUERY } else { QUERY }));
        let path = if sp { path.replace('\\', "/") } else { String::from(path) };
        u.path = if path.is_empty() {
            if sp { String::from("/") } else { String::new() }
        } else {
            tidy(&escape(&path, PATH))
        };
        Some(u)
    }

    /// The whole address (href).
    pub fn href(&self) -> String {
        let mut out = format!("{}:", self.scheme);
        if let Some(h) = &self.host {
            out.push_str("//");
            if !self.username.is_empty() || !self.password.is_empty() {
                out.push_str(&self.username);
                if !self.password.is_empty() {
                    out.push(':');
                    out.push_str(&self.password);
                }
                out.push('@');
            }
            out.push_str(h);
            if let Some(p) = self.port {
                out.push_str(&format!(":{p}"));
            }
        } else if !self.opaque && self.path.starts_with("//") {
            out.push_str("/.");
        }
        out.push_str(&self.path);
        if let Some(q) = &self.query {
            out.push('?');
            out.push_str(q);
        }
        if let Some(f) = &self.fragment {
            out.push('#');
            out.push_str(f);
        }
        out
    }

    /// "https://host:port" (or "null" for addresses with no origin of that kind).
    pub fn origin(&self) -> String {
        match (&self.host, default_port(&self.scheme)) {
            (Some(_), Some(_)) => format!("{}://{}", self.scheme, self.host_port()),
            _ => String::from("null"),
        }
    }

    /// The host, with its port if it has one.
    pub fn host_port(&self) -> String {
        match (&self.host, self.port) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.clone(),
            _ => String::new(),
        }
    }

    /// "?query" or "" (location.search's).
    pub fn search(&self) -> String {
        match &self.query {
            Some(q) if !q.is_empty() => format!("?{q}"),
            _ => String::new(),
        }
    }

    pub fn hash(&self) -> String {
        match &self.fragment {
            Some(f) if !f.is_empty() => format!("#{f}"),
            _ => String::new(),
        }
    }

    /// Sets one part of the address (a URL object's or location's setter): false if the value doesn't fit.
    pub fn set(&mut self, part: &str, v: &str) -> bool {
        match part {
            "href" => match Url::parse(v, None) {
                Some(u) => *self = u,
                None => return false,
            },
            "protocol" => {
                let s = v.trim_end_matches(':').to_ascii_lowercase();
                if s.is_empty() || special(&s) != special(&self.scheme) || !s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.')) {
                    return false;
                }
                self.scheme = s;
                if self.port.is_some() && self.port == default_port(&self.scheme) {
                    self.port = None;
                }
            }
            "username" => self.username = escape(v, USERINFO),
            "password" => self.password = escape(v, USERINFO),
            "host" | "hostname" => {
                if self.opaque {
                    return false;
                }
                let (h, p) = match v.rsplit_once(':') {
                    Some((h, p)) if part == "host" && p.bytes().all(|b| b.is_ascii_digit()) => (h, Some(p)),
                    _ => (v, None),
                };
                if h.is_empty() {
                    return false;
                }
                self.host = Some(if special(&self.scheme) { h.to_ascii_lowercase() } else { String::from(h) });
                if let Some(p) = p.and_then(|p| p.parse::<u16>().ok()) {
                    self.port = if default_port(&self.scheme) == Some(p) { None } else { Some(p) };
                }
            }
            "port" => {
                if v.is_empty() {
                    self.port = None;
                } else {
                    let digits: String = v.chars().take_while(|c| c.is_ascii_digit()).collect();
                    match digits.parse::<u32>() {
                        Ok(p) if p <= 65535 => self.port = if default_port(&self.scheme) == Some(p as u16) { None } else { Some(p as u16) },
                        _ => return false,
                    }
                }
            }
            "pathname" => {
                if self.opaque {
                    return false;
                }
                let p = if v.starts_with('/') { String::from(v) } else { format!("/{v}") };
                self.path = tidy(&escape(&p, PATH));
            }
            "search" => {
                let q = v.strip_prefix('?').unwrap_or(v);
                self.query = if v.is_empty() { None } else { Some(escape(q, if special(&self.scheme) { SPECIAL_QUERY } else { QUERY })) };
            }
            "hash" => {
                let f = v.strip_prefix('#').unwrap_or(v);
                self.fragment = if v.is_empty() { None } else { Some(escape(f, FRAGMENT)) };
            }
            _ => return false,
        }
        true
    }

    /// The address as the network layer takes it (https only), if it's one.
    pub fn to_web(&self) -> Option<web::Url> {
        if !matches!(self.scheme.as_str(), "http" | "https") {
            return None;
        }
        let mut u = self.clone();
        u.fragment = None;
        web::Url::parse(&u.href()).ok()
    }
}

impl From<&web::Url> for Url {
    fn from(w: &web::Url) -> Url {
        Url::parse(&w.to_string(), None).unwrap_or_default()
    }
}

fn split_fragment(s: &str) -> (&str, Option<&str>) {
    match s.find('#') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    }
}

fn split_query(s: &str) -> (&str, Option<&str>) {
    match s.find('?') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    }
}

/// "." and ".." taken out of a path.
fn tidy(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let segs: Vec<&str> = path.split('/').skip(1).collect();
    let n = segs.len();
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == n;
        match seg.to_ascii_lowercase().as_str() {
            "." | "%2e" => {
                if last {
                    parts.push("");
                }
            }
            ".." | ".%2e" | "%2e." | "%2e%2e" => {
                parts.pop();
                if last {
                    parts.push("");
                }
            }
            _ => parts.push(seg),
        }
    }
    let mut out = String::new();
    for s in &parts {
        out.push('/');
        out.push_str(s);
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

// Which characters each part escapes (beyond controls, spaces and what isn't ASCII).
const FRAGMENT: &[u8] = b" \"<>`";
const QUERY: &[u8] = b" \"#<>";
const SPECIAL_QUERY: &[u8] = b" \"#<>'";
const PATH: &[u8] = b" \"#<>?`{}";
const USERINFO: &[u8] = b" \"#<>?`{}/:;=@[\\]^|";
const OPAQUE: &[u8] = b"";

/// Percent-escapes what a part can't hold as it is (UTF-8's bytes); what's escaped already stays.
fn escape(s: &str, set: &[u8]) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b < 0x20 || b >= 0x7f || set.contains(&b) {
            out.push_str(&format!("%{b:02X}"));
        } else {
            out.push(b as char);
        }
    }
    out
}

/// %XX escapes read (as UTF-8; what isn't valid stays as it was).
pub fn unescape(s: &str, plus_is_space: bool) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(if plus_is_space && b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// application/x-www-form-urlencoded read: (name, value) pairs.
pub fn form_decode(s: &str) -> Vec<(String, String)> {
    let s = s.strip_prefix('?').unwrap_or(s);
    s.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (unescape(k, true), unescape(v, true))
        })
        .collect()
}

/// application/x-www-form-urlencoded written.
pub fn form_encode(pairs: &[(String, String)]) -> String {
    let mut out = String::new();
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push('&');
        }
        form_escape(&mut out, k);
        out.push('=');
        form_escape(&mut out, v);
    }
    out
}

fn form_escape(out: &mut String, s: &str) {
    for b in s.bytes() {
        match b {
            b' ' => out.push('+'),
            b'*' | b'-' | b'.' | b'_' => out.push(b as char),
            b if b.is_ascii_alphanumeric() => out.push(b as char),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
}

/// encodeURIComponent's kind of escape, for what a script hands the network.
pub fn component_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl core::fmt::Display for Url {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.write_str(&self.href())
    }
}
