//! A site's address as a person types it in the message box ("google.com", "open en.wikipedia.org/wiki/Penang",
//! "go to news.ycombinator.com please"), read as the website's host.js reads one (its ADDRESS, TLD and BARE), and the
//! site's name for what's said about it.

use alloc::string::String;
use alloc::vec::Vec;
use web::Url;

/// The top-level domains an address typed alone may end in (as host.js's TLD: the common ones, and any country's two
/// letters but those more often a file's, as notes.md).
const TLDS: &[&str] = &[
    "com", "org", "net", "edu", "gov", "mil", "int", "info", "biz", "io", "ai", "app", "dev", "co", "me", "tv", "xyz", "site",
    "online", "store", "shop", "tech", "blog", "news", "cloud", "page", "live", "art", "design", "studio", "agency", "world",
    "today", "space", "website", "fun", "club", "life", "email", "link", "pro", "mobi", "name", "museum", "travel", "jobs",
    "asia", "wiki", "games", "game", "music", "video", "social", "network", "digital", "media", "global", "group", "one", "top",
    "health", "bank", "money", "finance", "law", "school", "academy", "university", "college", "press", "photo", "photos",
    "gallery",
];
const NOT_A_COUNTRY: &[&str] = &["md", "py", "sh", "rs", "cc", "so", "ps"];

/// Words alone that mean a site.
const BARE: &[(&str, &str)] = &[
    ("google", "www.google.com"),
    ("youtube", "www.youtube.com"),
    ("gmail", "mail.google.com"),
    ("facebook", "www.facebook.com"),
    ("instagram", "www.instagram.com"),
    ("netflix", "www.netflix.com"),
    ("wikipedia", "www.wikipedia.org"),
];

/// The site an address typed in the message box is for, when what's typed is just that (or "open ...", "go to ...",
/// "visit ...", with "please" and "thanks" round it), as the website's host.js reads one.
pub fn address(typed: &str) -> Option<Url> {
    let original = typed.trim();
    if original.is_empty() || original.len() > 2048 || original.contains('\n') {
        return None;
    }
    // (Lower case for the matching; the same bytes' places in what was typed, whose path keeps its case.)
    let lower = original.to_ascii_lowercase();
    let (mut from, mut to) = (0, lower.len());
    let skip = |words: &[&str], from: &mut usize| {
        for w in words {
            if lower[*from..].starts_with(w) {
                *from += w.len();
                while lower[*from..].starts_with(' ') {
                    *from += 1;
                }
                return;
            }
        }
    };
    skip(&["please "], &mut from);
    skip(&["open ", "go to ", "goto ", "visit ", "load ", "launch ", "browse to ", "take me to "], &mut from);
    let trim_end = |to: &mut usize| {
        while *to > from && matches!(lower.as_bytes()[*to - 1], b'.' | b'!' | b'?' | b',' | b' ') {
            *to -= 1;
        }
    };
    trim_end(&mut to);
    for tail in [" please", " pls", " thanks", " thank you"] {
        if lower[from..to].ends_with(tail) {
            to -= tail.len();
            trim_end(&mut to);
        }
    }
    let t = &lower[from..to];
    if let Some((_, host)) = BARE.iter().find(|(w, _)| *w == t) {
        return Url::parse(host).ok();
    }
    if t.is_empty() || t.contains(char::is_whitespace) {
        return None;
    }
    // [https://]name.name...tld[:port][/path]
    let rest = t.strip_prefix("https://").or_else(|| t.strip_prefix("http://")).unwrap_or(t);
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let host = match authority.rsplit_once(':') {
        Some((h, port)) if (2..=5).contains(&port.len()) && port.bytes().all(|b| b.is_ascii_digit()) => h,
        Some(_) => return None,
        None => authority,
    };
    let labels: Vec<&str> = host.split('.').collect();
    let good = |l: &&str| {
        !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && !l.starts_with('-') && !l.ends_with('-')
    };
    if labels.len() < 2 || !labels.iter().all(good) {
        return None;
    }
    let tld = labels[labels.len() - 1];
    let country = tld.len() == 2 && tld.bytes().all(|b| b.is_ascii_alphabetic()) && !NOT_A_COUNTRY.contains(&tld);
    if !TLDS.contains(&tld) && !country {
        return None;
    }
    Url::parse(&original[from..to]).ok()
}

/// A site's name, for its frame and what's said: "Google", "Wikipedia", or its address without "www.".
pub fn name_of(url: &Url) -> String {
    let host = url.host.trim_start_matches("www.");
    let known = [("google.com", "Google"), ("maps.google.com", "Google Maps"), ("youtube.com", "YouTube"), ("wikipedia.org", "Wikipedia")];
    if let Some((_, n)) = known.iter().find(|(h, _)| *h == host) {
        return String::from(*n);
    }
    if host.ends_with(".wikipedia.org") {
        return String::from("Wikipedia");
    }
    String::from(host)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn site(t: &str) -> Option<String> {
        address(t).map(|u| u.to_string())
    }

    #[test]
    fn an_address_alone_or_asked_for_opens_its_site() {
        assert_eq!(site("google.com").as_deref(), Some("https://google.com/"));
        assert_eq!(site("  Open en.wikipedia.org/wiki/Penang.").as_deref(), Some("https://en.wikipedia.org/wiki/Penang"));
        assert_eq!(site("go to news.ycombinator.com please").as_deref(), Some("https://news.ycombinator.com/"));
        assert_eq!(site("please visit https://example.org/a?b=C, thanks").as_deref(), Some("https://example.org/a?b=C"));
        assert_eq!(site("http://localhost.dev:8080/x").as_deref(), Some("https://localhost.dev:8080/x"));
        assert_eq!(site("bbc.co.uk").as_deref(), Some("https://bbc.co.uk/"));
        assert_eq!(site("youtube").as_deref(), Some("https://www.youtube.com/"));
        assert_eq!(site("Open Wikipedia!").as_deref(), Some("https://www.wikipedia.org/"));
    }

    #[test]
    fn words_and_files_are_not_addresses() {
        for t in ["what is google.com", "notes.md", "main.rs", "read me.txt", "hello", "e.g.", "3.14", "a..com", "-a.com",
                  "open the door", "google.com and bing.com", "x.y", "example.com:abc"] {
            assert_eq!(site(t), None, "{t}");
        }
    }

    #[test]
    fn a_site_s_name() {
        assert_eq!(name_of(&Url::parse("https://www.google.com/").unwrap()), "Google");
        assert_eq!(name_of(&Url::parse("https://en.wikipedia.org/wiki/X").unwrap()), "Wikipedia");
        assert_eq!(name_of(&Url::parse("https://www.example.com/").unwrap()), "example.com");
    }
}
