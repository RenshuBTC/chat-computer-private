//! Cookies (RFC 6265, the parts sites lean on): what a site sets (Set-Cookie: its name and value, the domain and
//! path it's for, when it ends) kept while Chat Computer runs, and sent back to that site (Cookie:) with each request
//! to it, so a site's choices (a consent asked once, a language) and sign-ins last from page to page. Nothing is kept
//! on the disk, and a cookie for one site never goes to another.

use crate::url::Url;
use alloc::string::String;
use alloc::vec::Vec;

/// The most cookies kept, and the most for one site.
const MOST: usize = 600;
const MOST_A_SITE: usize = 60;
/// The longest cookie (name and value) kept.
const LONGEST: usize = 4096;

#[derive(Clone, Debug, PartialEq)]
struct Cookie {
    name: String,
    value: String,
    /// The host it was set by, or (with a Domain attribute) the domain it's for, in lower case.
    domain: String,
    /// Whether it's for the domain's subdomains too (it had a Domain attribute).
    subdomains: bool,
    path: String,
    /// When it ends (seconds since 1970), if it said.
    expires: Option<i64>,
}

#[derive(Default)]
pub struct Jar {
    cookies: Vec<Cookie>,
}

impl Jar {
    pub fn new() -> Jar {
        Jar::default()
    }

    /// What a response from `url` set (its Set-Cookie headers' values), `now` seconds since 1970 (if known).
    pub fn set(&mut self, url: &Url, set_cookie: &str, now: Option<i64>) {
        let mut parts = set_cookie.split(';');
        let Some((name, value)) = parts.next().and_then(|p| p.split_once('=')) else { return };
        let (name, value) = (name.trim(), value.trim().trim_matches('"'));
        if name.is_empty() || name.len() + value.len() > LONGEST {
            return;
        }
        let host = url.host.to_ascii_lowercase();
        let mut c = Cookie {
            name: String::from(name),
            value: String::from(value),
            domain: host.clone(),
            subdomains: false,
            path: default_path(&url.path),
            expires: None,
        };
        let mut gone = false;
        for attr in parts {
            let (k, v) = attr.split_once('=').unwrap_or((attr, ""));
            let (k, v) = (k.trim(), v.trim());
            if k.eq_ignore_ascii_case("domain") {
                let d = v.trim_start_matches('.').to_ascii_lowercase();
                // (Only the host itself or a domain it's in; never a whole top-level domain.)
                if d.is_empty() || !d.contains('.') || !(host == d || host.ends_with(&alloc::format!(".{d}"))) {
                    return;
                }
                c.domain = d;
                c.subdomains = true;
            } else if k.eq_ignore_ascii_case("path") {
                if v.starts_with('/') {
                    c.path = String::from(v);
                }
            } else if k.eq_ignore_ascii_case("max-age") {
                match v.parse::<i64>() {
                    Ok(s) if s <= 0 => gone = true,
                    Ok(s) => c.expires = now.map(|n| n.saturating_add(s)),
                    Err(_) => {}
                }
            } else if k.eq_ignore_ascii_case("expires") && c.expires.is_none() {
                if let (Some(t), Some(n)) = (http_date(v), now)
                    && t <= n
                {
                    gone = true;
                }
                c.expires = http_date(v);
            }
        }
        self.cookies.retain(|o| !(o.name == c.name && o.domain == c.domain && o.path == c.path));
        if gone {
            return;
        }
        // (The oldest let go first: for this site, then of all.)
        if self.cookies.iter().filter(|o| o.domain == c.domain).count() >= MOST_A_SITE
            && let Some(i) = self.cookies.iter().position(|o| o.domain == c.domain)
        {
            self.cookies.remove(i);
        }
        if self.cookies.len() >= MOST {
            self.cookies.remove(0);
        }
        self.cookies.push(c);
    }

    /// The Cookie header's value for a request to `url`, if it has any.
    pub fn header(&self, url: &Url, now: Option<i64>) -> Option<String> {
        let host = url.host.to_ascii_lowercase();
        let path = url.path.split('?').next().unwrap_or("/");
        let mut out = String::new();
        // (Longer paths first, as browsers send them.)
        let mut found: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|c| {
                let domain_ok = host == c.domain || (c.subdomains && host.ends_with(&alloc::format!(".{}", c.domain)));
                let path_ok = path == c.path || (path.starts_with(&c.path) && (c.path.ends_with('/') || path[c.path.len()..].starts_with('/')));
                let alive = match (c.expires, now) {
                    (Some(e), Some(n)) => e > n,
                    _ => true,
                };
                domain_ok && path_ok && alive
            })
            .collect();
        found.sort_by(|a, b| b.path.len().cmp(&a.path.len()));
        for c in found {
            if !out.is_empty() {
                out.push_str("; ");
            }
            out.push_str(&c.name);
            out.push('=');
            out.push_str(&c.value);
        }
        (!out.is_empty()).then_some(out)
    }

    /// How many cookies are kept.
    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }
}

/// A cookie's path when it doesn't say: the request's folder.
fn default_path(path: &str) -> String {
    let p = path.split('?').next().unwrap_or("/");
    match p.rfind('/') {
        Some(0) | None => String::from("/"),
        Some(i) => String::from(&p[..i]),
    }
}

/// An HTTP date ("Wed, 21 Oct 2026 07:28:00 GMT", and the older forms with dashes) as seconds since 1970.
fn http_date(s: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let mut day = None;
    let mut month = None;
    let mut year = None;
    let mut time = None;
    for part in s.split([' ', ',', '-']).filter(|p| !p.is_empty()) {
        if part.contains(':') {
            let t: Vec<i64> = part.split(':').filter_map(|n| n.parse().ok()).collect();
            if t.len() == 3 {
                time = Some(t[0] * 3600 + t[1] * 60 + t[2]);
            }
        } else if let Some(m) = MONTHS.iter().position(|m| part.len() >= 3 && part[..3].eq_ignore_ascii_case(m)) {
            month = Some(m as i64 + 1);
        } else if let Ok(n) = part.parse::<i64>() {
            if part.len() <= 2 && day.is_none() {
                day = Some(n);
            } else {
                year = Some(if n < 70 { n + 2000 } else if n < 100 { n + 1900 } else { n });
            }
        }
    }
    let (y, m, d) = (year?, month?, day?);
    // Days from 1970 (days_from_civil).
    let yy = if m <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some(days * 86_400 + time.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn a_cookie_goes_back_to_its_site_only() {
        let mut j = Jar::new();
        j.set(&url("https://www.example.com/a/b"), "SID=31d4d96e407aad42; Path=/; Secure; HttpOnly", None);
        assert_eq!(j.header(&url("https://www.example.com/x"), None).as_deref(), Some("SID=31d4d96e407aad42"));
        assert_eq!(j.header(&url("https://example.com/"), None), None);
        assert_eq!(j.header(&url("https://other.org/"), None), None);
    }

    #[test]
    fn a_domain_cookie_goes_to_its_subdomains_too() {
        let mut j = Jar::new();
        j.set(&url("https://www.example.com/"), "lang=en-GB; Domain=.example.com; Path=/", None);
        assert_eq!(j.header(&url("https://mail.example.com/inbox"), None).as_deref(), Some("lang=en-GB"));
        assert_eq!(j.header(&url("https://badexample.com/"), None), None);
        // A site can't set one for another, nor for a whole top-level domain.
        j.set(&url("https://www.example.com/"), "x=1; Domain=other.org", None);
        j.set(&url("https://www.example.com/"), "y=1; Domain=com", None);
        assert_eq!(j.len(), 1);
    }

    #[test]
    fn a_cookie_s_path_and_default_path() {
        let mut j = Jar::new();
        j.set(&url("https://example.com/docs/page"), "a=1", None);
        j.set(&url("https://example.com/"), "b=2; Path=/docs/deep", None);
        assert_eq!(j.header(&url("https://example.com/docs/other"), None).as_deref(), Some("a=1"));
        assert_eq!(j.header(&url("https://example.com/docs/deep/x"), None).as_deref(), Some("b=2; a=1"));
        assert_eq!(j.header(&url("https://example.com/docsx"), None), None);
        assert_eq!(j.header(&url("https://example.com/"), None), None);
    }

    #[test]
    fn a_cookie_set_again_replaces_the_old_one_and_one_that_has_ended_goes() {
        let mut j = Jar::new();
        let now = Some(1_800_000_000);
        j.set(&url("https://example.com/"), "a=1; Path=/", now);
        j.set(&url("https://example.com/"), "a=2; Path=/", now);
        assert_eq!(j.header(&url("https://example.com/"), now).as_deref(), Some("a=2"));
        j.set(&url("https://example.com/"), "a=; Path=/; Max-Age=0", now);
        assert_eq!(j.header(&url("https://example.com/"), now), None);
        j.set(&url("https://example.com/"), "b=1; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT", now);
        assert_eq!(j.header(&url("https://example.com/"), now), None);
        j.set(&url("https://example.com/"), "c=1; Path=/; Max-Age=60", now);
        assert_eq!(j.header(&url("https://example.com/"), now).as_deref(), Some("c=1"));
        assert_eq!(j.header(&url("https://example.com/"), Some(1_800_000_100)), None);
    }

    #[test]
    fn dates_as_sites_write_them() {
        assert_eq!(http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(http_date("Wed, 21 Oct 2026 07:28:00 GMT"), Some(1_792_567_680));
        assert_eq!(http_date("Wednesday, 21-Oct-26 07:28:00 GMT"), Some(1_792_567_680));
    }
}
