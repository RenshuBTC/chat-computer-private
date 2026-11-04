//! Just enough JSON for the model's published files (config.json, tokenizer.json, the weights files' headers).

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Json>),
    /// In the order written.
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        if let Json::Str(s) = self { Some(s) } else { None }
    }

    pub fn num(&self) -> Option<f64> {
        if let Json::Number(n) = self { Some(*n) } else { None }
    }

    pub fn usize(&self) -> Option<usize> {
        self.num().filter(|n| *n >= 0.0 && *n == (*n as u64) as f64).map(|n| n as usize)
    }

    pub fn array(&self) -> Option<&[Json]> {
        if let Json::Array(a) = self { Some(a) } else { None }
    }

    pub fn object(&self) -> Option<&[(String, Json)]> {
        if let Json::Object(o) = self { Some(o) } else { None }
    }

    pub fn bool(&self) -> Option<bool> {
        if let Json::Bool(b) = self { Some(*b) } else { None }
    }
}

pub fn parse(text: &str) -> Option<Json> {
    let mut p = Parser { b: text.as_bytes(), i: 0, depth: 0 };
    let v = p.value()?;
    p.space();
    (p.i == p.b.len()).then_some(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.space();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.depth += 1;
        if self.depth > 256 {
            return None;
        }
        self.space();
        let v = match *self.b.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut items = Vec::new();
                if !self.eat(b'}') {
                    loop {
                        self.space();
                        let k = self.string()?;
                        if !self.eat(b':') {
                            return None;
                        }
                        items.push((k, self.value()?));
                        if self.eat(b',') {
                            continue;
                        }
                        if self.eat(b'}') {
                            break;
                        }
                        return None;
                    }
                }
                Json::Object(items)
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                if !self.eat(b']') {
                    loop {
                        items.push(self.value()?);
                        if self.eat(b',') {
                            continue;
                        }
                        if self.eat(b']') {
                            break;
                        }
                        return None;
                    }
                }
                Json::Array(items)
            }
            b'"' => Json::Str(self.string()?),
            b't' if self.b[self.i..].starts_with(b"true") => {
                self.i += 4;
                Json::Bool(true)
            }
            b'f' if self.b[self.i..].starts_with(b"false") => {
                self.i += 5;
                Json::Bool(false)
            }
            b'n' if self.b[self.i..].starts_with(b"null") => {
                self.i += 4;
                Json::Null
            }
            _ => {
                let start = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                let s = core::str::from_utf8(&self.b[start..self.i]).ok()?;
                // ("NaN" and "Infinity", which Python writes, aren't JSON: not taken.)
                Json::Number(s.parse().ok()?)
            }
        };
        self.depth -= 1;
        Some(v)
    }

    fn string(&mut self) -> Option<String> {
        if self.b.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.b.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.b.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) && self.b.get(self.i..self.i + 2) == Some(b"\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                }
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return None,
                    }
                }
                _ => out.push(c),
            }
        }
        String::from_utf8(out).ok()
    }

    fn hex4(&mut self) -> Option<u32> {
        let s = core::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u32::from_str_radix(s, 16).ok()
    }
}

/// Written out the way Python's json.dumps writes it by default (", " and ": " between items, letters beyond ASCII
/// as they are), which is how the models' own templates write a tool's description into the prompt.
pub fn dumps(v: &Json) -> String {
    let mut out = String::new();
    write(v, ", ", ": ", &mut out);
    out
}

/// Written out with nothing between items, for answers that only programs read.
pub fn compact(v: &Json) -> String {
    let mut out = String::new();
    write(v, ",", ":", &mut out);
    out
}

fn write(v: &Json, comma: &str, colon: &str, out: &mut String) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Number(n) => number(*n, out),
        Json::Str(s) => quote(s, out),
        Json::Array(items) => {
            out.push('[');
            for (i, x) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(comma);
                }
                write(x, comma, colon, out);
            }
            out.push(']');
        }
        Json::Object(items) => {
            out.push('{');
            for (i, (k, x)) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(comma);
                }
                quote(k, out);
                out.push_str(colon);
                write(x, comma, colon, out);
            }
            out.push('}');
        }
    }
}

fn number(n: f64, out: &mut String) {
    use core::fmt::Write;
    if !n.is_finite() {
        // (JSON has no way to say these.)
        out.push_str("null");
    } else if n == (n as i64) as f64 && n.abs() < 9.007_199_254_740_992e15 {
        let _ = write!(out, "{}", n as i64);
    } else {
        let _ = write!(out, "{n}");
    }
}

/// A string in quotes, escaped as Python escapes it: \" \\ \n \r \t \b \f, other control characters as \u00xx.
pub fn quote(s: &str, out: &mut String) {
    use core::fmt::Write;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn written_as_python_writes_it() {
        let v = parse(r#"{"a": [1, 2.5, true, null], "b": "x\"y\\z\n\u0001é", "c": {}}"#).unwrap();
        assert_eq!(dumps(&v), "{\"a\": [1, 2.5, true, null], \"b\": \"x\\\"y\\\\z\\n\\u0001é\", \"c\": {}}");
        assert_eq!(compact(&v), "{\"a\":[1,2.5,true,null],\"b\":\"x\\\"y\\\\z\\n\\u0001é\",\"c\":{}}");
        assert_eq!(parse(&dumps(&v)), Some(v));
        assert_eq!(dumps(&Json::Number(-3.0)), "-3".to_string());
    }
}
