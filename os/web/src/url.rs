//! Web addresses: `https://host[:port]/path?query`, as Chat Computer reaches them. Only HTTPS: an `http://` address is
//! tried as `https://` (nearly every site has it), one with no scheme is taken as `https://`, and anything else
//! (`file:`, `ftp:`, `javascript:`, a name and password before the host) is refused. Hosts are names in English
//! letters (or an IPv4 address); the path keeps what's escaped already, and escapes spaces, quotes and anything not
//! ASCII.

use alloc::format;
use alloc::string::String;
use core::fmt;

/// The longest address followed.
const MOST: usize = 4096;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Url {
    /// In lower case.
    pub host: String,
    pub port: u16,
    /// From its first "/": the path and the query (never empty).
    pub path: String,
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.port == 443 {
            write!(f, "https://{}{}", self.host, self.path)
        } else {
            write!(f, "https://{}:{}{}", self.host, self.port, self.path)
        }
    }
}

/// Why an address can't be followed, for the model.
pub type Refused = &'static str;

impl Url {
    /// An address as a person or a page writes it.
    pub fn parse(s: &str) -> Result<Url, Refused> {
        let s = s.trim();
        if s.is_empty() {
            return Err("there's no address");
        }
        if s.len() > MOST {
            return Err("the address is too long");
        }
        let rest = match s.find("://") {
            Some(at) => {
                let scheme = &s[..at];
                if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
                    return Err("only web addresses (https://) can be read");
                }
                &s[at + 3..]
            }
            None if s.starts_with("//") => &s[2..],
            None => {
                // ("mailto:", "javascript:" and the like: a scheme with no slashes.)
                if let Some(colon) = s.find(':') {
                    let before = &s[..colon];
                    if !before.is_empty() && !before.contains(['.', '/']) && !s[colon + 1..].starts_with(|c: char| c.is_ascii_digit()) {
                        return Err("only web addresses (https://) can be read");
                    }
                }
                s
            }
        };
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        if authority.contains('@') {
            return Err("addresses with a name and password in them aren't followed");
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !h.contains(']') => {
                let port: u16 = p.parse().map_err(|_| "the address's port isn't a number")?;
                if port == 0 {
                    return Err("the address's port isn't a number");
                }
                (h, port)
            }
            _ => (authority, 443),
        };
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if host.is_empty() {
            return Err("the address has no site in it");
        }
        if host.starts_with('[') {
            return Err("IPv6 addresses can't be reached yet");
        }
        if !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
            || host.split('.').any(|label| label.is_empty() || label.len() > 63)
        {
            return Err("the site's name has letters Chat Computer can't look up yet (only English letters, digits and -)");
        }
        let tail = tail.split('#').next().unwrap_or("");
        let path = if tail.starts_with('/') { escape_path(tail) } else { escape_path(&format!("/{tail}")) };
        Ok(Url { host, port, path })
    }

    /// Where a link (or a redirect's Location) on this page leads: absolute, from the site's root, or from this page's
    /// folder.
    pub fn join(&self, link: &str) -> Result<Url, Refused> {
        let link = link.trim();
        // (A link with a scheme of its own, "mailto:" and "javascript:" too, is absolute: refused unless it's the web's.)
        let scheme = link.find(':').is_some_and(|c| {
            let s = &link[..c];
            s.starts_with(|ch: char| ch.is_ascii_alphabetic())
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        });
        if scheme || link.starts_with("//") {
            return Url::parse(link);
        }
        if link.is_empty() {
            return Ok(self.clone());
        }
        let path = if link.starts_with('/') {
            String::from(link)
        } else if link.starts_with('?') {
            format!("{}{link}", self.path.split('?').next().unwrap_or("/"))
        } else {
            let here = self.path.split('?').next().unwrap_or("/");
            let folder = &here[..here.rfind('/').map_or(0, |i| i + 1)];
            format!("{}{link}", if folder.is_empty() { "/" } else { folder })
        };
        let path = escape_path(&tidy(&path));
        let mut url = Url { host: self.host.clone(), port: self.port, path };
        if url.path.len() > MOST {
            url.path.truncate(MOST);
        }
        Ok(url)
    }
}

/// "." and ".." taken out of a path (its query left as it is).
fn tidy(path: &str) -> String {
    let (p, query) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    let mut parts: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    for seg in p.split('/').skip(1) {
        match seg {
            "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let mut out = String::new();
    for s in &parts {
        out.push('/');
        out.push_str(s);
    }
    if out.is_empty() || p.ends_with("/.") || p.ends_with("/..") {
        out.push('/');
    }
    if let Some(q) = query {
        out.push('?');
        out.push_str(q);
    }
    out
}

/// The path with spaces, quotes, angle brackets, backslashes, control characters and anything not ASCII escaped
/// (%XX, UTF-8's bytes); what's escaped already stays.
fn escape_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_graphic() && !matches!(b, b'"' | b'<' | b'>' | b'\\' | b'^' | b'`' | b'{' | b'|' | b'}') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Words for a query, as a form sends them (application/x-www-form-urlencoded): spaces as "+", letters, digits and
/// "-_.~" as they are, anything else %XX (UTF-8's bytes).
pub fn query_escape(words: &str) -> String {
    let mut out = String::with_capacity(words.len() * 3 / 2);
    for b in words.bytes() {
        match b {
            b' ' => out.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// %XX (and "+" as a space) taken back to bytes, read as UTF-8 (anything that isn't, as U+FFFD).
pub fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = alloc::vec::Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = |c: u8| (c as char).to_digit(16);
                match (hex(b[i + 1]), hex(b[i + 2])) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                        continue;
                    }
                    _ => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
