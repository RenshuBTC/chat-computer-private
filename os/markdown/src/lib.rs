//! The Markdown Claude writes, read into blocks and styled spans for Chat Computer's own screen to lay out. The rules
//! are those of the main screen's renderer (chatcomputer/ui/assets/md.js), so an answer looks the same on both:
//! paragraphs (a line break inside one is kept), headings (# to ####), fenced code blocks, rules (---), quotes (>),
//! tables (a header row, then a row of dashes), lists (-, *, + or numbers, with [ ] and [x] tasks), and inside them
//! **bold**, *italic*, `code`, ~~struck out~~, [links](...) (their words: nothing can be opened from here) and
//! images' descriptions. Nothing in it is ever run or fetched.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

#[cfg(test)]
mod tests;

/// Styles a span can have, together.
pub const BOLD: u8 = 1;
pub const ITALIC: u8 = 2;
pub const CODE: u8 = 4;
pub const STRUCK: u8 = 8;
pub const LINK: u8 = 16;

/// Words in one style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: u8,
}

/// A list item: its words, and for a task, whether it's done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub spans: Vec<Span>,
    pub task: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// A paragraph ('\n' in its spans: a line break).
    Para(Vec<Span>),
    /// A heading, level 1 to 4.
    Heading(u8, Vec<Span>),
    /// A block of code, its lines as written.
    Code(String),
    Rule,
    Quote(Vec<Block>),
    /// A list, numbered from `start` if `ordered`.
    List { ordered: bool, start: u32, items: Vec<Item> },
    /// A table: its header's cells, then its rows' cells.
    Table { head: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>> },
}

fn blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// A word character, as JavaScript's \w (ASCII letters, digits, _).
fn word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// "```" with an optional language, or (`close`) alone.
fn fence(line: &str, close: bool) -> bool {
    let t = line.trim();
    let Some(rest) = t.strip_prefix("```") else { return false };
    if close { rest.is_empty() } else { rest.bytes().all(word) }
}

/// "# Heading" to "#### Heading": its level and words.
fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=4).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    let words = rest.trim_start();
    (words.len() < rest.len()).then_some((hashes as u8, words))
}

/// A rule: three -, * or _ (the same one), with spaces between them allowed, and more of them after.
fn rule(line: &str) -> bool {
    let mut chars = line.trim().bytes().filter(|b| !b.is_ascii_whitespace());
    let Some(first) = chars.next() else { return false };
    if !matches!(first, b'-' | b'*' | b'_') {
        return false;
    }
    // (md.js: the first three must be the same; any of the three may follow.)
    let rest: Vec<u8> = chars.collect();
    rest.len() >= 2 && rest[0] == first && rest[1] == first && rest[2..].iter().all(|b| matches!(b, b'-' | b'*' | b'_'))
}

/// A table's second row: dashes between bars (colons allowed at their ends).
fn table_rule(line: &str) -> bool {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    !t.trim().is_empty()
        && t.split('|').all(|cell| {
            let c = cell.trim();
            let c = c.strip_prefix(':').unwrap_or(c);
            let c = c.strip_suffix(':').unwrap_or(c);
            c.len() >= 2 && c.bytes().all(|b| b == b'-')
        })
}

/// A table row's cells.
fn cells(line: &str) -> Vec<Vec<Span>> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| inline(c.trim())).collect()
}

/// A list item's line: whether it's numbered, its number, and its words.
fn list_item(line: &str) -> Option<(bool, u32, &str)> {
    let t = line.trim_start();
    let (ordered, number, after) = match t.bytes().next()? {
        b'-' | b'*' | b'+' => (false, 1, &t[1..]),
        b'0'..=b'9' => {
            let digits = t.bytes().take_while(|b| b.is_ascii_digit()).count();
            if !matches!(t.as_bytes().get(digits), Some(b'.' | b')')) {
                return None;
            }
            (true, t[..digits].parse().unwrap_or(1), &t[digits + 1..])
        }
        _ => return None,
    };
    let words = after.trim_start();
    (words.len() < after.len()).then_some((ordered, number, words))
}

/// A list item's words go on, on a line indented two spaces or more.
fn continues(line: &str) -> bool {
    let words = line.trim_start();
    !words.is_empty() && line.len() - words.len() >= 2 && line[..line.len() - words.len()].bytes().all(|b| b.is_ascii_whitespace())
}

/// The blocks of a Markdown text.
pub fn parse(md: &str) -> Vec<Block> {
    let text = md.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(inline(&para.join("\n"))));
            para.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if fence(line, false) {
            flush(&mut para, &mut out);
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !fence(lines[i], true) {
                code.push(lines[i]);
                i += 1;
            }
            i += 1;
            out.push(Block::Code(code.join("\n")));
            continue;
        }
        if blank(line) {
            flush(&mut para, &mut out);
            i += 1;
            continue;
        }
        if let Some((level, words)) = heading(line) {
            flush(&mut para, &mut out);
            out.push(Block::Heading(level, inline(words)));
            i += 1;
            continue;
        }
        if rule(line) {
            flush(&mut para, &mut out);
            out.push(Block::Rule);
            i += 1;
            continue;
        }
        if line.trim_start().starts_with('>') {
            flush(&mut para, &mut out);
            let mut quoted = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let rest = &lines[i].trim_start()[1..];
                quoted.push(rest.strip_prefix(' ').unwrap_or(rest));
                i += 1;
            }
            out.push(Block::Quote(parse(&quoted.join("\n"))));
            continue;
        }
        if line.contains('|') && i + 1 < lines.len() && table_rule(lines[i + 1]) {
            flush(&mut para, &mut out);
            let head = cells(line);
            i += 2;
            let mut rows = Vec::new();
            while i < lines.len() && lines[i].contains('|') && !blank(lines[i]) {
                rows.push(cells(lines[i]));
                i += 1;
            }
            out.push(Block::Table { head, rows });
            continue;
        }
        if let Some((ordered, start, _)) = list_item(line) {
            flush(&mut para, &mut out);
            let mut items: Vec<String> = Vec::new();
            while i < lines.len() {
                match list_item(lines[i]) {
                    Some((o, _, words)) if o == ordered => items.push(String::from(words)),
                    _ if continues(lines[i]) && !items.is_empty() => {
                        let last = items.last_mut().unwrap();
                        last.push('\n');
                        last.push_str(lines[i].trim());
                    }
                    _ => break,
                }
                i += 1;
            }
            let items = items
                .iter()
                .map(|t| {
                    let task = t.strip_prefix('[').and_then(|r| {
                        let (mark, rest) = (r.as_bytes().first()?, &r[1..]);
                        let rest = rest.strip_prefix(']')?;
                        let words = rest.trim_start();
                        (matches!(mark, b' ' | b'x' | b'X') && words.len() < rest.len()).then_some((*mark != b' ', words))
                    });
                    match task {
                        Some((done, words)) => Item { spans: inline(words), task: Some(done) },
                        None => Item { spans: inline(t), task: None },
                    }
                })
                .collect();
            out.push(Block::List { ordered, start: if ordered { start } else { 1 }, items });
            continue;
        }
        para.push(line);
        i += 1;
    }
    flush(&mut para, &mut out);
    out
}

/// Where `close` next comes in `s` from `from` (an ASCII marker, so byte positions are character boundaries).
fn find(s: &str, from: usize, close: &str) -> Option<usize> {
    s.get(from..)?.find(close).map(|at| from + at)
}

/// A link (or, after its "!", an image) at `at`: "[words](address)", the address without spaces or ")". Its words,
/// and where the text goes on after it.
fn link(s: &str, at: usize, empty_words: bool) -> Option<(&str, usize)> {
    let end = find(s, at + 1, "]")?;
    let words = &s[at + 1..end];
    if (words.is_empty() && !empty_words) || !s[end + 1..].starts_with('(') {
        return None;
    }
    let close = find(s, end + 2, ")")?;
    let address = &s[end + 2..close];
    if address.is_empty() || address.bytes().any(|b| b.is_ascii_whitespace()) {
        return None;
    }
    Some((words, close + 1))
}

/// An emphasis marker (* or _) at `at` that opens: not after a word character or the same marker, followed by
/// something other than a space or the marker, and closed by the next one, which no word character follows. Its
/// words, and where the text goes on. (md.js takes out code and bold first, so neither ends it here either.)
fn emphasis(s: &str, at: usize, marker: u8) -> Option<(&str, usize)> {
    let b = s.as_bytes();
    if at > 0 && (word(b[at - 1]) || b[at - 1] == marker) {
        return None;
    }
    let first = *b.get(at + 1)?;
    if first == marker || first.is_ascii_whitespace() {
        return None;
    }
    let double = if marker == b'*' { "**" } else { "__" };
    let mut k = at + 1;
    let end = loop {
        match *b.get(k)? {
            b'`' => match find(s, k + 1, "`") {
                Some(close) => k = close + 1,
                None => k += 1,
            },
            c if c == marker && b.get(k + 1) == Some(&marker) => k = pair(s, k, double)?.1,
            c if c == marker => break k,
            _ => k += 1,
        }
    };
    if b.get(end + 1).is_some_and(|&c| word(c)) {
        return None;
    }
    Some((&s[at + 1..end], end + 1))
}

/// A pair of markers ("**", "__", "~~") around words without the marker's character in them.
fn pair<'s>(s: &'s str, at: usize, marker: &str) -> Option<(&'s str, usize)> {
    let end = find(s, at + 2, marker)?;
    let words = &s[at + 2..end];
    let c = marker.as_bytes()[0] as char;
    (!words.is_empty() && !words.contains(c)).then_some((words, end + 2))
}

/// The spans of a paragraph's (or heading's, or cell's) words.
pub fn inline(s: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    spans(s, 0, &mut out);
    out
}

fn push(out: &mut Vec<Span>, text: &str, style: u8) {
    if text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(last) if last.style == style => last.text.push_str(text),
        _ => out.push(Span { text: String::from(text), style }),
    }
}

fn spans(s: &str, style: u8, out: &mut Vec<Span>) {
    let b = s.as_bytes();
    let (mut i, mut plain) = (0, 0);
    while i < b.len() {
        let next = b.get(i + 1).copied();
        // What's at i, if it's something: its words, their style, whether they're formatted further, and where the
        // text goes on.
        let found = match b[i] {
            b'`' => find(s, i + 1, "`").filter(|&end| end > i + 1).map(|end| (&s[i + 1..end], style | CODE, false, end + 1)),
            b'!' if next == Some(b'[') => link(s, i + 1, true).map(|(w, after)| (w, style, true, after)),
            b'[' => link(s, i, false).map(|(w, after)| (w, style | LINK, true, after)),
            b'*' if next == Some(b'*') => pair(s, i, "**").map(|(w, after)| (w, style | BOLD, true, after)),
            b'_' if next == Some(b'_') => pair(s, i, "__").map(|(w, after)| (w, style | BOLD, true, after)),
            b'~' if next == Some(b'~') => pair(s, i, "~~").map(|(w, after)| (w, style | STRUCK, true, after)),
            m @ (b'*' | b'_') => emphasis(s, i, m).map(|(w, after)| (w, style | ITALIC, true, after)),
            _ => None,
        };
        match found {
            Some((words, inner, further, after)) => {
                push(out, &s[plain..i], style);
                if further {
                    spans(words, inner, out);
                } else {
                    push(out, words, inner);
                }
                i = after;
                plain = after;
            }
            None => i += 1,
        }
    }
    push(out, &s[plain..], style);
}
