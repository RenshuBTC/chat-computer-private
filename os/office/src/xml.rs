//! Just enough XML for Office's files: tags with their attributes, end tags and text, in order, as they are in the
//! file (no checking that tags pair up: the readers keep their own track); character references and the five
//! named entities put back in text and attribute values. Comments, processing instructions and declarations are
//! passed over; CDATA sections are text as they are.

use alloc::string::String;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Token<'a> {
    /// `<name attributes>`, or `<name attributes/>` (empty: no end tag follows).
    Start(Tag<'a>),
    /// `</name>`.
    End(&'a str),
    /// Text between tags, still escaped (`text` unescapes it).
    Text(&'a str),
    /// A CDATA section's text, as it is.
    Raw(&'a str),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tag<'a> {
    /// Its name, with its prefix ("w:p").
    pub name: &'a str,
    attributes: &'a str,
    pub empty: bool,
}

impl<'a> Tag<'a> {
    /// Its name without the prefix ("p" for "w:p").
    pub fn local(&self) -> &'a str {
        local(self.name)
    }

    /// The value of the attribute called exactly `name` ("w:val", "t"), unescaped.
    pub fn attr(&self, name: &str) -> Option<String> {
        self.attributes().find(|(key, _)| *key == name).map(|(_, v)| text(v))
    }

    /// The value of the first attribute with a prefix and this local name: `id` finds "r:id" (whatever the
    /// relationships' prefix is), not "id".
    pub fn attr_prefixed(&self, name: &str) -> Option<String> {
        self.attributes().find(|(key, _)| key.contains(':') && local(key) == name).map(|(_, v)| text(v))
    }

    /// Each attribute: its name, and its value still escaped.
    fn attributes(&self) -> impl Iterator<Item = (&'a str, &'a str)> {
        let mut rest = self.attributes;
        core::iter::from_fn(move || {
            rest = rest.trim_start();
            let eq = rest.find('=')?;
            let key = rest[..eq].trim();
            let after = rest[eq + 1..].trim_start();
            let quote = after.chars().next().filter(|&q| q == '"' || q == '\'')?;
            let end = after[1..].find(quote)? + 1;
            rest = &after[end + 1..];
            Some((key, &after[1..end]))
        })
    }
}

/// A name without its prefix.
pub fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

pub struct Reader<'a> {
    s: &'a str,
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(s: &'a str) -> Reader<'a> {
        Reader { s, at: 0 }
    }
}

impl<'a> Iterator for Reader<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            let rest = self.s.get(self.at..)?;
            if rest.is_empty() {
                return None;
            }
            if !rest.starts_with('<') {
                let end = rest.find('<').unwrap_or(rest.len());
                self.at += end;
                return Some(Token::Text(&rest[..end]));
            }
            // What isn't a tag: a CDATA section's text, as it is; processing instructions, comments and
            // declarations, passed over.
            if let Some(after) = rest.strip_prefix("<![CDATA[") {
                let end = after.find("]]>");
                let inner = &after[..end.unwrap_or(after.len())];
                self.at += 9 + inner.len() + if end.is_some() { 3 } else { 0 };
                return Some(Token::Raw(inner));
            }
            if rest.starts_with("<?") {
                self.at += rest.find("?>").map_or(rest.len(), |e| e + 2);
                continue;
            }
            if let Some(after) = rest.strip_prefix("<!--") {
                self.at += after.find("-->").map_or(rest.len(), |e| 4 + e + 3);
                continue;
            }
            if rest.starts_with("<!") {
                self.at += rest.find('>').map_or(rest.len(), |e| e + 1);
                continue;
            }
            // The tag's end: the first > outside quotes.
            let bytes = rest.as_bytes();
            let mut quote = 0u8;
            let mut end = None;
            for (i, &b) in bytes.iter().enumerate().skip(1) {
                if quote != 0 {
                    if b == quote {
                        quote = 0;
                    }
                } else if b == b'"' || b == b'\'' {
                    quote = b;
                } else if b == b'>' {
                    end = Some(i);
                    break;
                }
            }
            let Some(end) = end else {
                self.at = self.s.len();
                return None;
            };
            self.at += end + 1;
            let inside = &rest[1..end];
            if let Some(name) = inside.strip_prefix('/') {
                return Some(Token::End(name.trim()));
            }
            let empty = inside.ends_with('/');
            let inside = inside.strip_suffix('/').unwrap_or(inside);
            let split = inside.find(|c: char| c.is_ascii_whitespace()).unwrap_or(inside.len());
            return Some(Token::Start(Tag { name: &inside[..split], attributes: &inside[split..], empty }));
        }
    }
}

/// Escaped text as it reads: `&lt;` `&gt;` `&amp;` `&quot;` `&apos;` and character references (`&#233;` `&#xE9;`)
/// put back; anything else left as it is.
pub fn text(s: &str) -> String {
    if !s.contains('&') {
        return String::from(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        // (A reference is short: its ; within a dozen bytes, found byte by byte, as text after & needn't be ASCII.)
        let Some(semi) = rest.as_bytes()[..rest.len().min(12)].iter().position(|&b| b == b';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..semi];
        let c = match name {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
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
