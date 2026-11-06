//! Character references in HTML (&amp;, &nbsp;, &#8217;, &#x2014;...), read as the HTML standard reads them: all
//! of its named ones (some without their ";"), numbers (those in the C1 range as windows-1252 has them), and in an
//! attribute's value a name without its ";" left as it is before "=" or a letter or digit ("?a=1&copy=2").

use alloc::string::String;

use crate::entities_data::{LEGACY, NAMED};

/// A text's character references read.
pub fn decode_text(s: &str) -> String {
    decode(s, false)
}

/// `s` with its character references read (`attribute`: it's an attribute's value).
pub fn decode(s: &str, attribute: bool) -> String {
    if !s.contains('&') {
        return String::from(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match reference(rest, attribute) {
            Some((Chars::One(c), used)) => {
                out.push(c);
                rest = &rest[used..];
            }
            Some((Chars::Text(t), used)) => {
                out.push_str(t);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// What the reference at the start of `s` ("&...") stands for, and how many bytes of `s` it is. None: it's not one
/// (the "&" is just that).
fn reference(s: &str, attribute: bool) -> Option<(Chars, usize)> {
    let b = s.as_bytes();
    if b.get(1) == Some(&b'#') {
        // A number: decimal, or hexadecimal after "x".
        let (hex, start) = if matches!(b.get(2), Some(b'x' | b'X')) { (true, 3) } else { (false, 2) };
        let digits = b[start..].iter().take_while(|c| if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() }).count();
        if digits == 0 {
            return None;
        }
        let text = &s[start..start + digits];
        let n = u32::from_str_radix(text.trim_start_matches('0'), if hex { 16 } else { 10 }).unwrap_or(if text.trim_start_matches('0').is_empty() { 0 } else { u32::MAX });
        let mut used = start + digits;
        if b.get(used) == Some(&b';') {
            used += 1;
        }
        return Some((Chars::One(number(n)), used));
    }
    // A name: the letters and digits after the "&", and a ";" if one ends it.
    let run = b[1..].iter().take_while(|c| c.is_ascii_alphanumeric()).count();
    if run == 0 {
        return None;
    }
    let name = &s[1..1 + run];
    if b.get(1 + run) == Some(&b';')
        && let Ok(i) = NAMED.binary_search_by(|(n, _)| (*n).cmp(name))
    {
        return Some((Chars::Text(NAMED[i].1), run + 2));
    }
    // (Else the longest of the old names that need no ";" it starts with: "&notit;" is "¬it;".)
    for len in (1..=run.min(6)).rev() {
        let head = &name[..len];
        if LEGACY.binary_search(&head).is_ok() {
            let next = b.get(1 + len).copied();
            if attribute && next.is_some_and(|c| c == b'=' || c.is_ascii_alphanumeric()) {
                return None;
            }
            let i = NAMED.binary_search_by(|(n, _)| (*n).cmp(head)).ok()?;
            return Some((Chars::Text(NAMED[i].1), 1 + len));
        }
    }
    None
}

/// The character a number stands for: none, a surrogate's or one past Unicode's are U+FFFD; those from 0x80 to 0x9F
/// as windows-1252 has them.
fn number(n: u32) -> char {
    match n {
        0 => '\u{FFFD}',
        0x80..=0x9F => match n {
            0x80 => '€',
            0x82 => '‚',
            0x83 => 'ƒ',
            0x84 => '„',
            0x85 => '…',
            0x86 => '†',
            0x87 => '‡',
            0x88 => 'ˆ',
            0x89 => '‰',
            0x8A => 'Š',
            0x8B => '‹',
            0x8C => 'Œ',
            0x8E => 'Ž',
            0x91 => '‘',
            0x92 => '’',
            0x93 => '“',
            0x94 => '”',
            0x95 => '•',
            0x96 => '–',
            0x97 => '—',
            0x98 => '˜',
            0x99 => '™',
            0x9A => 'š',
            0x9B => '›',
            0x9C => 'œ',
            0x9E => 'ž',
            0x9F => 'Ÿ',
            n => char::from_u32(n).unwrap_or('\u{FFFD}'),
        },
        n => char::from_u32(n).unwrap_or('\u{FFFD}'),
    }
}

/// A reference's characters: a number's one, or a name's (one or two).
enum Chars {
    One(char),
    Text(&'static str),
}
