//! A search's results, from DuckDuckGo's plain results page (html.duckduckgo.com/html/), as the main Chat Computer
//! reads it (chatcomputer/brain/builtin.py `_search_results`): each result's title, address and snippet, its
//! advertisements left out.

use crate::page::{attribute, entities, from_html};
use crate::url::{query_escape, unescape};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The most results given.
pub const MOST: usize = 8;

/// The search's address for `words`.
pub fn address(words: &str) -> String {
    format!("https://html.duckduckgo.com/html/?q={}", query_escape(words))
}

pub struct Found {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// The results on DuckDuckGo's plain results page, in order, its advertisements left out.
pub fn results(page: &str) -> Vec<Found> {
    let lower = page.to_ascii_lowercase();
    let mut out = Vec::new();
    // Each result's link: an <a> whose class has "result__a".
    let mut starts: Vec<usize> = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find("<a") {
        let at = from + at;
        from = at + 2;
        let end = lower[at..].find('>').map_or(lower.len(), |n| at + n + 1);
        if attribute(&page[at..end], "class").is_some_and(|c| c.split_whitespace().any(|w| w == "result__a")) {
            starts.push(at);
        }
    }
    for (k, &at) in starts.iter().enumerate() {
        let next = starts.get(k + 1).copied().unwrap_or(page.len());
        let tag_end = lower[at..].find('>').map_or(page.len(), |n| at + n + 1);
        let href = attribute(&page[at..tag_end], "href").unwrap_or_default();
        let close = lower[tag_end..next].find("</a").map_or(next, |n| tag_end + n);
        let title = words(&page[tag_end..close]);
        let target = target(&href);
        if target.contains("duckduckgo.com/y.js") || target.is_empty() {
            continue;
        }
        // Its snippet: the element after it whose class has "result__snippet".
        let rest = &page[close..next];
        let rest_l = &lower[close..next];
        let snippet = rest_l.find("result__snippet").and_then(|n| {
            let open = rest_l[n..].find('>')? + n + 1;
            let end = rest_l[open..].find("</a").or_else(|| rest_l[open..].find("</div")).map_or(rest.len(), |m| open + m);
            Some(words(&rest[open..end]))
        });
        out.push(Found { title, url: target, snippet: snippet.unwrap_or_default() });
        if out.len() == MOST {
            break;
        }
    }
    out
}

/// Where a result's link leads: DuckDuckGo's own redirect's `uddg` (the real address), else the link as it is.
fn target(href: &str) -> String {
    let href = entities(href);
    if let Some(q) = href.split_once('?').map(|(_, q)| q) {
        for pair in q.split('&') {
            if let Some(v) = pair.strip_prefix("uddg=") {
                return unescape(v);
            }
        }
    }
    if href.starts_with("//") { format!("https:{href}") } else { href }
}

/// An element's words, on one line.
fn words(html: &str) -> String {
    from_html(html).text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The results, for the model: a numbered list of each one's title, address and snippet.
pub fn listed(found: &[Found]) -> String {
    let mut out = String::new();
    for (i, f) in found.iter().enumerate() {
        out.push_str(&format!("{}. {}\n   {}\n", i + 1, f.title, f.url));
        if !f.snippet.is_empty() {
            out.push_str(&format!("   {}\n", f.snippet));
        }
    }
    out.trim_end().into()
}
