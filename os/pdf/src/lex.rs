//! PDF's objects as they're written (ISO 32000-1, 7.2 and 7.3): null, booleans, numbers, strings (literal and hex),
//! names, arrays, dictionaries, indirect references ("12 0 R"), and, in content streams, operators. Nothing is
//! trusted: anything that can't be read ends the object, nesting is limited, and a stray delimiter is skipped.

use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq)]
pub enum Obj {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Str(Vec<u8>),
    Name(Vec<u8>),
    Array(Vec<Obj>),
    Dict(Dict),
    Ref(u32, u16),
    /// A stream: its dictionary, and where its data is in the file.
    Stream(Dict, core::ops::Range<usize>),
    /// An operator of a content stream (or any other bare word).
    Op(Vec<u8>),
}

/// A dictionary: its keys (names, without "/") and values, in order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Dict(pub Vec<(Vec<u8>, Obj)>);

impl Dict {
    pub fn get(&self, key: &str) -> Option<&Obj> {
        self.0.iter().rev().find(|(k, _)| k == key.as_bytes()).map(|(_, v)| v)
    }
}

impl Obj {
    pub fn int(&self) -> Option<i64> {
        match *self {
            Obj::Int(i) => Some(i),
            Obj::Real(r) if r.is_finite() => Some(r as i64),
            _ => None,
        }
    }
    pub fn num(&self) -> Option<f64> {
        match *self {
            Obj::Int(i) => Some(i as f64),
            Obj::Real(r) if r.is_finite() => Some(r),
            _ => None,
        }
    }
    pub fn name(&self) -> Option<&[u8]> {
        match self {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }
    pub fn dict(&self) -> Option<&Dict> {
        match self {
            Obj::Dict(d) | Obj::Stream(d, _) => Some(d),
            _ => None,
        }
    }
    pub fn array(&self) -> Option<&[Obj]> {
        match self {
            Obj::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn string(&self) -> Option<&[u8]> {
        match self {
            Obj::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// How deep arrays and dictionaries may nest.
const MOST_DEPTH: usize = 48;

pub fn is_white(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

pub fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

pub struct Lexer<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> Lexer<'a> {
        Lexer { data, pos }
    }

    /// Past white space and comments.
    pub fn skip_space(&mut self) {
        while let Some(&b) = self.data.get(self.pos) {
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(&c) = self.data.get(self.pos) {
                    if c == b'\n' || c == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// Whether `word` comes next, as a word of its own (then it's passed).
    pub fn keyword(&mut self, word: &[u8]) -> bool {
        self.skip_space();
        let end = self.pos + word.len();
        if self.data.get(self.pos..end) == Some(word) && self.data.get(end).is_none_or(|&b| is_white(b) || is_delim(b)) {
            self.pos = end;
            return true;
        }
        false
    }

    /// A non-negative integer, if one comes next (else nothing is passed).
    pub fn unsigned(&mut self) -> Option<u64> {
        self.skip_space();
        let start = self.pos;
        let mut n: u64 = 0;
        while let Some(&b) = self.data.get(self.pos) {
            if !b.is_ascii_digit() {
                break;
            }
            let Some(m) = n.checked_mul(10).and_then(|m| m.checked_add((b - b'0') as u64)) else {
                self.pos = start;
                return None;
            };
            n = m;
            self.pos += 1;
        }
        if self.pos == start || self.data.get(self.pos).is_some_and(|&b| !(is_white(b) || is_delim(b))) {
            self.pos = start;
            return None;
        }
        Some(n)
    }

    /// The next object; None at the end, or where nothing can be read.
    pub fn object(&mut self) -> Option<Obj> {
        self.object_in(0)
    }

    fn object_in(&mut self, depth: usize) -> Option<Obj> {
        if depth > MOST_DEPTH {
            return None;
        }
        self.skip_space();
        let b = *self.data.get(self.pos)?;
        match b {
            b'/' => {
                self.pos += 1;
                Some(Obj::Name(self.name_body()))
            }
            b'(' => {
                self.pos += 1;
                Some(Obj::Str(self.literal()))
            }
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                Some(Obj::Dict(self.dict_body(depth)))
            }
            b'<' => {
                self.pos += 1;
                Some(Obj::Str(self.hex()))
            }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_space();
                    match self.data.get(self.pos) {
                        None => break,
                        Some(b']') => {
                            self.pos += 1;
                            break;
                        }
                        _ => {}
                    }
                    let at = self.pos;
                    match self.object_in(depth + 1) {
                        Some(o) => items.push(o),
                        None => break,
                    }
                    if self.pos == at {
                        break;
                    }
                }
                Some(Obj::Array(items))
            }
            b'0'..=b'9' | b'+' | b'-' | b'.' => Some(self.number()),
            b')' | b'>' | b']' | b'{' | b'}' => {
                self.pos += 1;
                Some(Obj::Op(vec![b]))
            }
            _ => {
                let start = self.pos;
                while self.data.get(self.pos).is_some_and(|&c| !is_white(c) && !is_delim(c)) {
                    self.pos += 1;
                }
                if self.pos == start {
                    self.pos += 1;
                    return Some(Obj::Op(vec![b]));
                }
                let word = &self.data[start..self.pos];
                Some(match word {
                    b"true" => Obj::Bool(true),
                    b"false" => Obj::Bool(false),
                    b"null" => Obj::Null,
                    w => Obj::Op(w.to_vec()),
                })
            }
        }
    }

    /// A number; or, for an integer followed by another and "R", a reference.
    fn number(&mut self) -> Obj {
        let start = self.pos;
        let mut dot = false;
        if matches!(self.data.get(self.pos), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        while let Some(&b) = self.data.get(self.pos) {
            if b.is_ascii_digit() {
                self.pos += 1;
            } else if b == b'.' && !dot {
                dot = true;
                self.pos += 1;
            } else if b == b'-' || b == b'+' {
                // ("1-2" is written by some programs: the rest is dropped.)
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = core::str::from_utf8(&self.data[start..self.pos]).unwrap_or("0");
        let clean: alloc::string::String = text.chars().enumerate().filter(|&(i, c)| i == 0 || (c != '-' && c != '+')).map(|(_, c)| c).collect();
        if dot {
            return Obj::Real(clean.parse::<f64>().unwrap_or(0.0));
        }
        let Ok(n) = clean.parse::<i64>() else { return Obj::Real(clean.parse::<f64>().unwrap_or(0.0)) };
        // A reference: "n g R".
        if n >= 0 && n <= u32::MAX as i64 && !text.starts_with(['+', '-']) {
            let after = self.pos;
            if let Some(g) = self.unsigned()
                && g <= u16::MAX as u64
                && self.keyword(b"R")
            {
                return Obj::Ref(n as u32, g as u16);
            }
            self.pos = after;
        }
        Obj::Int(n)
    }

    /// A name's characters after its "/", with #XX read.
    fn name_body(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(&b) = self.data.get(self.pos) {
            if is_white(b) || is_delim(b) {
                break;
            }
            if b == b'#'
                && let (Some(h), Some(l)) = (self.hex_at(self.pos + 1), self.hex_at(self.pos + 2))
            {
                out.push(h * 16 + l);
                self.pos += 3;
                continue;
            }
            out.push(b);
            self.pos += 1;
        }
        out
    }

    fn hex_at(&self, at: usize) -> Option<u8> {
        (*self.data.get(at)? as char).to_digit(16).map(|d| d as u8)
    }

    /// A literal string's bytes after its "(": nested parentheses kept, escapes read, its line ends as "\n".
    fn literal(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 1;
        while let Some(&b) = self.data.get(self.pos) {
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(b);
                }
                b'\\' => {
                    let Some(&e) = self.data.get(self.pos) else { break };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut v = (e - b'0') as u32;
                            for _ in 0..2 {
                                match self.data.get(self.pos) {
                                    Some(&d @ b'0'..=b'7') => {
                                        v = v * 8 + (d - b'0') as u32;
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(v as u8);
                        }
                        b'\r' => {
                            if self.data.get(self.pos) == Some(&b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                b'\r' => {
                    if self.data.get(self.pos) == Some(&b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                _ => out.push(b),
            }
        }
        out
    }

    /// A hex string's bytes after its "<" (white space skipped; an odd last digit counts as followed by 0).
    fn hex(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut high: Option<u8> = None;
        while let Some(&b) = self.data.get(self.pos) {
            self.pos += 1;
            if b == b'>' {
                break;
            }
            let Some(d) = (b as char).to_digit(16) else { continue };
            match high.take() {
                Some(h) => out.push(h * 16 + d as u8),
                None => high = Some(d as u8),
            }
        }
        if let Some(h) = high {
            out.push(h * 16);
        }
        out
    }

    /// A dictionary's keys and values after its "<<", up to ">>".
    fn dict_body(&mut self, depth: usize) -> Dict {
        let mut d = Dict::default();
        loop {
            self.skip_space();
            match (self.data.get(self.pos), self.data.get(self.pos + 1)) {
                (None, _) => break,
                (Some(b'>'), Some(b'>')) => {
                    self.pos += 2;
                    break;
                }
                (Some(b'/'), _) => {}
                _ => {
                    // (Something that isn't a key: skipped, to find the next.)
                    let at = self.pos;
                    if self.object_in(depth + 1).is_none() || self.pos == at {
                        break;
                    }
                    continue;
                }
            }
            self.pos += 1;
            let key = self.name_body();
            self.skip_space();
            if matches!((self.data.get(self.pos), self.data.get(self.pos + 1)), (Some(b'>'), Some(b'>'))) {
                d.0.push((key, Obj::Null));
                continue;
            }
            match self.object_in(depth + 1) {
                Some(v) => d.0.push((key, v)),
                None => break,
            }
        }
        d
    }
}
