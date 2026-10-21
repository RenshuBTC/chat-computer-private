//! JSON (RFC 8259): reading what Claude's API sends, and writing strings into requests. Numbers are kept as
//! their text. Nesting is limited (64 deep), so nothing sent can exhaust the stack.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn path(&self, keys: &[&str]) -> Option<&Json> {
        let mut v = self;
        for k in keys {
            v = v.get(k)?;
        }
        Some(v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Json> {
        match self {
            Json::Obj(fields) => fields.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Sets an object's field (added at the end if it isn't there).
    pub fn set(&mut self, key: &str, value: Json) {
        if let Json::Obj(fields) = self {
            match fields.iter_mut().find(|(k, _)| k == key) {
                Some((_, v)) => *v = value,
                None => fields.push((String::from(key), value)),
            }
        }
    }

    /// Adds text to a string field (making it if it isn't there).
    pub fn append(&mut self, key: &str, more: &str) {
        match self.get_mut(key) {
            Some(Json::Str(s)) => s.push_str(more),
            _ => self.set(key, Json::Str(String::from(more))),
        }
    }

    /// Adds a value to an array field (making it if it isn't there).
    pub fn push(&mut self, key: &str, value: Json) {
        match self.get_mut(key) {
            Some(Json::Arr(items)) => items.push(value),
            _ => self.set(key, Json::Arr(alloc::vec![value])),
        }
    }

    /// As JSON text again: what was read comes out the same (numbers as they were written).
    pub fn write(&self) -> String {
        let mut out = String::new();
        self.write_to(&mut out);
        out
    }

    fn write_to(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => out.push_str(n),
            Json::Str(s) => out.push_str(&quote(s)),
            Json::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write_to(out);
                }
                out.push(']');
            }
            Json::Obj(fields) => {
                out.push('{');
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&quote(k));
                    out.push(':');
                    v.write_to(out);
                }
                out.push('}');
            }
        }
    }
}

struct Reader<'a> {
    s: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn space(&mut self) {
        while self.at < self.s.len() && matches!(self.s[self.at], b' ' | b'\t' | b'\n' | b'\r') {
            self.at += 1;
        }
    }

    fn byte(&mut self) -> Option<u8> {
        let b = *self.s.get(self.at)?;
        self.at += 1;
        Some(b)
    }

    fn literal(&mut self, word: &[u8], v: Json) -> Option<Json> {
        if self.s.get(self.at..self.at + word.len())? == word {
            self.at += word.len();
            Some(v)
        } else {
            None
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..4 {
            v = v * 16 + (self.byte()? as char).to_digit(16)?;
        }
        Some(v)
    }

    fn string(&mut self) -> Option<String> {
        let mut out = Vec::new();
        loop {
            match self.byte()? {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => match self.byte()? {
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    b'/' => out.push(b'/'),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'u' => {
                        let mut c = self.hex4()?;
                        if (0xD800..0xDC00).contains(&c) {
                            // A surrogate pair: 😀.
                            if self.byte()? != b'\\' || self.byte()? != b'u' {
                                return None;
                            }
                            let low = self.hex4()?;
                            if !(0xDC00..0xE000).contains(&low) {
                                return None;
                            }
                            c = 0x10000 + ((c - 0xD800) << 10) + (low - 0xDC00);
                        }
                        let ch = char::from_u32(c).unwrap_or('\u{FFFD}');
                        let mut b = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                    }
                    _ => return None,
                },
                c if c < 0x20 => return None,
                c => out.push(c),
            }
        }
    }

    fn value(&mut self, depth: usize) -> Option<Json> {
        if depth > 64 {
            return None;
        }
        self.space();
        match *self.s.get(self.at)? {
            b'n' => self.literal(b"null", Json::Null),
            b't' => self.literal(b"true", Json::Bool(true)),
            b'f' => self.literal(b"false", Json::Bool(false)),
            b'"' => {
                self.at += 1;
                self.string().map(Json::Str)
            }
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if self.s.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Some(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.space();
                    match self.byte()? {
                        b',' => continue,
                        b']' => return Some(Json::Arr(items)),
                        _ => return None,
                    }
                }
            }
            b'{' => {
                self.at += 1;
                let mut fields = Vec::new();
                self.space();
                if self.s.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Some(Json::Obj(fields));
                }
                loop {
                    self.space();
                    if self.byte()? != b'"' {
                        return None;
                    }
                    let k = self.string()?;
                    self.space();
                    if self.byte()? != b':' {
                        return None;
                    }
                    let v = self.value(depth + 1)?;
                    fields.push((k, v));
                    self.space();
                    match self.byte()? {
                        b',' => continue,
                        b'}' => return Some(Json::Obj(fields)),
                        _ => return None,
                    }
                }
            }
            b'-' | b'0'..=b'9' => {
                let start = self.at;
                while self.at < self.s.len() && matches!(self.s[self.at], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.at += 1;
                }
                core::str::from_utf8(&self.s[start..self.at]).ok().map(|n| Json::Num(String::from(n)))
            }
            _ => None,
        }
    }
}

pub fn parse(s: &str) -> Option<Json> {
    let mut r = Reader { s: s.as_bytes(), at: 0 };
    let v = r.value(0)?;
    r.space();
    (r.at == r.s.len()).then_some(v)
}

/// `s` as a JSON string, quotes included.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str("\\u00");
                out.push(char::from_digit(c as u32 >> 4, 16).unwrap());
                out.push(char::from_digit(c as u32 & 15, 16).unwrap());
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
