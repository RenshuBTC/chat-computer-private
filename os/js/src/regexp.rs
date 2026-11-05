//! Regular expressions: a pattern parsed (ECMAScript's grammar; outside Unicode mode, Annex B's too: the web's
//! patterns rely on it), compiled to a small program, and run by a backtracking matcher with a stack of its own (so
//! a long input can't overflow the machine's). Lookarounds run as matches of their own (they're atomic).

use crate::unicode_gc::{CATEGORIES, CATEGORY_RANGES};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Flags {
    pub global: bool,
    pub ignore_case: bool,
    pub multiline: bool,
    pub dot_all: bool,
    pub unicode: bool,
    pub unicode_sets: bool,
    pub sticky: bool,
    pub has_indices: bool,
}

impl Flags {
    /// The flags of a flags string (None: one it doesn't know, or one twice).
    pub fn parse(s: &str) -> Option<Flags> {
        let mut f = Flags::default();
        for c in s.chars() {
            let slot = match c {
                'g' => &mut f.global,
                'i' => &mut f.ignore_case,
                'm' => &mut f.multiline,
                's' => &mut f.dot_all,
                'u' => &mut f.unicode,
                'v' => &mut f.unicode_sets,
                'y' => &mut f.sticky,
                'd' => &mut f.has_indices,
                _ => return None,
            };
            if *slot {
                return None;
            }
            *slot = true;
        }
        if f.unicode && f.unicode_sets {
            return None;
        }
        Some(f)
    }

    /// Unicode mode (u or v).
    pub fn full_unicode(&self) -> bool {
        self.unicode || self.unicode_sets
    }
}

/// A set of code points, as sorted ranges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Set {
    pub ranges: Vec<(u32, u32)>,
}

impl Set {
    fn add(&mut self, a: u32, b: u32) {
        self.ranges.push((a, b));
    }

    fn add_set(&mut self, o: &Set) {
        self.ranges.extend_from_slice(&o.ranges);
    }

    fn normalize(&mut self) {
        self.ranges.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::with_capacity(self.ranges.len());
        for &(a, b) in &self.ranges {
            match out.last_mut() {
                Some(l) if a <= l.1.saturating_add(1) => l.1 = l.1.max(b),
                _ => out.push((a, b)),
            }
        }
        self.ranges = out;
    }

    fn invert(&self) -> Set {
        let mut out = Vec::new();
        let mut next = 0u32;
        for &(a, b) in &self.ranges {
            if a > next {
                out.push((next, a - 1));
            }
            next = b + 1;
        }
        if next <= 0x10FFFF {
            out.push((next, 0x10FFFF));
        }
        Set { ranges: out }
    }

    fn intersect(&self, o: &Set) -> Set {
        let mut out = Vec::new();
        for &(a, b) in &self.ranges {
            for &(c, d) in &o.ranges {
                let (lo, hi) = (a.max(c), b.min(d));
                if lo <= hi {
                    out.push((lo, hi));
                }
            }
        }
        let mut s = Set { ranges: out };
        s.normalize();
        s
    }

    pub fn contains(&self, c: u32) -> bool {
        self.ranges.binary_search_by(|&(a, b)| if c < a { core::cmp::Ordering::Greater } else if c > b { core::cmp::Ordering::Less } else { core::cmp::Ordering::Equal }).is_ok()
    }
}

#[derive(Debug)]
enum Node {
    Empty,
    Char(u32),
    Any,
    /// A class (and whether it's negated).
    Class(Set, bool),
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    /// A group: its capture number (None: non-capturing).
    Group(Box<Node>, Option<usize>),
    Repeat(Box<Node>, u32, u32, bool),
    Bol,
    Eol,
    WordBoundary(bool),
    BackRef(usize),
    NamedRef(Vec<u16>),
    Look(Box<Node>, bool, bool),
}

pub struct Error(pub String);

fn err<T>(m: &str) -> Result<T, Error> {
    Err(Error(String::from(m)))
}

const INF: u32 = u32::MAX;

struct Parser<'a> {
    p: &'a [u16],
    at: usize,
    u: bool,
    v: bool,
    groups: usize,
    total_groups: usize,
    names: Vec<(Vec<u16>, usize)>,
    has_names: bool,
}

fn is_syntax(c: u32) -> bool {
    matches!(char::from_u32(c), Some('^' | '$' | '\\' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '/'))
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u32> {
        self.p.get(self.at).map(|c| *c as u32)
    }

    fn peek_at(&self, k: usize) -> Option<u32> {
        self.p.get(self.at + k).map(|c| *c as u32)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c as u32) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// The next code point (a surrogate pair is one in Unicode mode).
    fn next_char(&mut self) -> Option<u32> {
        let c = self.peek()?;
        self.at += 1;
        if self.u
            && (0xD800..0xDC00).contains(&c)
            && let Some(d) = self.peek()
            && (0xDC00..0xE000).contains(&d)
        {
            self.at += 1;
            return Some(0x10000 + ((c - 0xD800) << 10) + (d - 0xDC00));
        }
        Some(c)
    }

    fn disjunction(&mut self) -> Result<Node, Error> {
        let mut alts = vec![self.alternative()?];
        while self.eat('|') {
            alts.push(self.alternative()?);
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap_or(Node::Empty) } else { Node::Alt(alts) })
    }

    fn alternative(&mut self) -> Result<Node, Error> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' as u32 || c == ')' as u32 {
                break;
            }
            items.push(self.term()?);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap_or(Node::Empty),
            _ => Node::Seq(items),
        })
    }

    fn term(&mut self) -> Result<Node, Error> {
        let c = self.peek().ok_or(Error(String::from("unexpected end")))?;
        // Assertions.
        match char::from_u32(c) {
            Some('^') => {
                self.at += 1;
                return Ok(Node::Bol);
            }
            Some('$') => {
                self.at += 1;
                return Ok(Node::Eol);
            }
            Some('\\') if self.peek_at(1) == Some('b' as u32) => {
                self.at += 2;
                return Ok(Node::WordBoundary(false));
            }
            Some('\\') if self.peek_at(1) == Some('B' as u32) => {
                self.at += 2;
                return Ok(Node::WordBoundary(true));
            }
            Some('(') if self.peek_at(1) == Some('?' as u32) => {
                let k = self.peek_at(2);
                let look = match (k.and_then(char::from_u32), self.peek_at(3).and_then(char::from_u32)) {
                    (Some('='), _) => Some((false, false, 3)),
                    (Some('!'), _) => Some((false, true, 3)),
                    (Some('<'), Some('=')) => Some((true, false, 4)),
                    (Some('<'), Some('!')) => Some((true, true, 4)),
                    _ => None,
                };
                if let Some((behind, negate, len)) = look {
                    self.at += len;
                    let d = self.disjunction()?;
                    if !self.eat(')') {
                        return err("Unterminated group");
                    }
                    let node = Node::Look(Box::new(d), behind, negate);
                    // (Annex B: a lookahead may be quantified outside Unicode mode.)
                    if !self.u && !behind {
                        return self.quantifier(node);
                    }
                    if self.at_quantifier() {
                        return err("Nothing to repeat");
                    }
                    return Ok(node);
                }
            }
            _ => {}
        }
        let atom = self.atom()?;
        self.quantifier(atom)
    }

    fn at_quantifier(&self) -> bool {
        match self.peek().and_then(char::from_u32) {
            Some('*' | '+' | '?') => true,
            Some('{') => self.braces().is_some(),
            _ => false,
        }
    }

    /// A {n,m} quantifier at the cursor: (min, max, its length).
    fn braces(&self) -> Option<(u32, u32, usize)> {
        let mut i = self.at + 1;
        let digits = |i: &mut usize| -> Option<u32> {
            let start = *i;
            let mut v: u64 = 0;
            while let Some(&c) = self.p.get(*i) {
                if !(0x30..=0x39).contains(&c) {
                    break;
                }
                v = (v * 10 + (c - 0x30) as u64).min(INF as u64);
                *i += 1;
            }
            (*i > start).then_some(v as u32)
        };
        let min = digits(&mut i)?;
        let mut max = min;
        if self.p.get(i) == Some(&(b',' as u16)) {
            i += 1;
            max = digits(&mut i).unwrap_or(INF);
        }
        if self.p.get(i) != Some(&(b'}' as u16)) {
            return None;
        }
        Some((min, max, i + 1 - self.at))
    }

    fn quantifier(&mut self, atom: Node) -> Result<Node, Error> {
        let (min, max) = match self.peek().and_then(char::from_u32) {
            Some('*') => {
                self.at += 1;
                (0, INF)
            }
            Some('+') => {
                self.at += 1;
                (1, INF)
            }
            Some('?') => {
                self.at += 1;
                (0, 1)
            }
            Some('{') => match self.braces() {
                Some((a, b, len)) => {
                    self.at += len;
                    if a > b {
                        return err("numbers out of order in {} quantifier");
                    }
                    (a, b)
                }
                None => {
                    if self.u {
                        return err("Incomplete quantifier");
                    }
                    return Ok(atom);
                }
            },
            _ => return Ok(atom),
        };
        let greedy = !self.eat('?');
        if matches!(atom, Node::Bol | Node::Eol | Node::WordBoundary(_)) || matches!(atom, Node::Look(_, true, _)) {
            return err("Nothing to repeat");
        }
        Ok(Node::Repeat(Box::new(atom), min, max, greedy))
    }

    fn atom(&mut self) -> Result<Node, Error> {
        let c = self.peek().ok_or(Error(String::from("unexpected end")))?;
        match char::from_u32(c) {
            Some('.') => {
                self.at += 1;
                Ok(Node::Any)
            }
            Some('(') => {
                self.at += 1;
                if self.eat('?') {
                    if self.eat(':') {
                        let d = self.disjunction()?;
                        if !self.eat(')') {
                            return err("Unterminated group");
                        }
                        return Ok(Node::Group(Box::new(d), None));
                    }
                    if self.eat('<') {
                        let name = self.group_name()?;
                        self.groups += 1;
                        let idx = self.groups;
                        if self.names.iter().any(|(n, _)| *n == name) {
                            return err("Duplicate capture group name");
                        }
                        self.names.push((name, idx));
                        let d = self.disjunction()?;
                        if !self.eat(')') {
                            return err("Unterminated group");
                        }
                        return Ok(Node::Group(Box::new(d), Some(idx)));
                    }
                    return err("Invalid group");
                }
                self.groups += 1;
                let idx = self.groups;
                let d = self.disjunction()?;
                if !self.eat(')') {
                    return err("Unterminated group");
                }
                Ok(Node::Group(Box::new(d), Some(idx)))
            }
            Some('[') => {
                self.at += 1;
                let (set, neg) = self.class()?;
                Ok(Node::Class(set, neg))
            }
            Some('\\') => {
                self.at += 1;
                self.atom_escape()
            }
            Some('*' | '+' | '?') => err("Nothing to repeat"),
            Some('{') => {
                if self.u || self.braces().is_some() {
                    return err("Nothing to repeat");
                }
                self.at += 1;
                Ok(Node::Char(c))
            }
            Some(')') => err("Unmatched ')'"),
            Some(']' | '}') if self.u => err("Lone quantifier brackets"),
            _ => {
                let ch = self.next_char().unwrap_or(c);
                Ok(Node::Char(ch))
            }
        }
    }

    fn group_name(&mut self) -> Result<Vec<u16>, Error> {
        let mut name = Vec::new();
        loop {
            let Some(c) = self.peek() else { return err("Invalid capture group name") };
            if c == '>' as u32 {
                self.at += 1;
                break;
            }
            let cp = if c == '\\' as u32 {
                self.at += 1;
                if !self.eat('u') {
                    return err("Invalid capture group name");
                }
                let save_u = self.u;
                self.u = true;
                let r = self.unicode_escape_body();
                self.u = save_u;
                r.ok_or(Error(String::from("Invalid Unicode escape in group name")))?
            } else {
                // (A name may be written with surrogate pairs even outside Unicode mode.)
                let save_u = self.u;
                self.u = true;
                let r = self.next_char();
                self.u = save_u;
                r.unwrap_or(c)
            };
            let ok = if name.is_empty() { crate::unicode_id::is_start(cp) || cp == '$' as u32 || cp == '_' as u32 } else { crate::unicode_id::is_part(cp) || cp == '$' as u32 || cp == 0x200C || cp == 0x200D };
            if !ok {
                return err("Invalid capture group name");
            }
            let mut buf = [0u16; 2];
            if let Some(ch) = char::from_u32(cp) {
                name.extend_from_slice(ch.encode_utf16(&mut buf));
            }
        }
        if name.is_empty() {
            return err("Invalid capture group name");
        }
        Ok(name)
    }

    fn hex(&mut self, n: usize) -> Option<u32> {
        let mut v = 0u32;
        for k in 0..n {
            let d = char::from_u32(self.peek_at(k)?)?.to_digit(16)?;
            v = v * 16 + d;
        }
        self.at += n;
        Some(v)
    }

    /// After "\u": the code unit or point (a surrogate pair of escapes joined in Unicode mode).
    fn unicode_escape_body(&mut self) -> Option<u32> {
        if self.u && self.peek() == Some('{' as u32) {
            let save = self.at;
            self.at += 1;
            let mut v: u32 = 0;
            let mut n = 0;
            while let Some(d) = self.peek().and_then(char::from_u32).and_then(|c| c.to_digit(16)) {
                v = v.saturating_mul(16).saturating_add(d);
                self.at += 1;
                n += 1;
            }
            if n == 0 || !self.eat('}') || v > 0x10FFFF {
                self.at = save;
                return None;
            }
            return Some(v);
        }
        let v = self.hex(4)?;
        if self.u
            && (0xD800..0xDC00).contains(&v)
            && self.peek() == Some('\\' as u32)
            && self.peek_at(1) == Some('u' as u32)
        {
            let save = self.at;
            self.at += 2;
            if let Some(w) = self.hex(4)
                && (0xDC00..0xE000).contains(&w)
            {
                return Some(0x10000 + ((v - 0xD800) << 10) + (w - 0xDC00));
            }
            self.at = save;
        }
        Some(v)
    }

    fn atom_escape(&mut self) -> Result<Node, Error> {
        let c = self.peek().ok_or(Error(String::from("\\ at end of pattern")))?;
        let ch = char::from_u32(c).unwrap_or('\0');
        // Backreferences.
        if ('1'..='9').contains(&ch) {
            let save = self.at;
            let mut n: usize = 0;
            while let Some(d) = self.peek().and_then(char::from_u32).and_then(|c| c.to_digit(10)) {
                n = n.saturating_mul(10).saturating_add(d as usize);
                self.at += 1;
            }
            if n <= self.total_groups {
                return Ok(Node::BackRef(n));
            }
            if self.u {
                return err("Invalid escape");
            }
            // Annex B: an octal escape (or the digit itself).
            self.at = save;
            if ch >= '8' {
                self.at += 1;
                return Ok(Node::Char(c));
            }
            return Ok(Node::Char(self.octal()));
        }
        if ch == 'k' {
            self.at += 1;
            if self.u || self.has_names {
                if !self.eat('<') {
                    return err("Invalid named reference");
                }
                let name = self.group_name()?;
                return Ok(Node::NamedRef(name));
            }
            return Ok(Node::Char('k' as u32));
        }
        if let Some(set) = self.class_escape()? {
            return Ok(Node::Class(set, false));
        }
        let c = self.char_escape(false)?;
        Ok(Node::Char(c))
    }

    fn octal(&mut self) -> u32 {
        let mut v = 0u32;
        let first = self.peek().unwrap_or(0x30);
        let most = if first <= '3' as u32 { 3 } else { 2 };
        for _ in 0..most {
            match self.peek() {
                Some(d) if ('0' as u32..='7' as u32).contains(&d) => {
                    v = v * 8 + (d - '0' as u32);
                    self.at += 1;
                }
                _ => break,
            }
        }
        v
    }

    /// \d \D \s \S \w \W \p{..} \P{..} (the cursor after the backslash).
    fn class_escape(&mut self) -> Result<Option<Set>, Error> {
        let c = self.peek().and_then(char::from_u32);
        let set = match c {
            Some('d') => digits(),
            Some('D') => digits().invert(),
            Some('s') => spaces(),
            Some('S') => spaces().invert(),
            Some('w') => word(false),
            Some('W') => word(false).invert(),
            Some('p' | 'P') if self.u => {
                let neg = c == Some('P');
                self.at += 1;
                if !self.eat('{') {
                    return err("Invalid property name");
                }
                let start = self.at;
                while let Some(x) = self.peek() {
                    if x == '}' as u32 {
                        break;
                    }
                    self.at += 1;
                }
                let name: String = self.p[start..self.at].iter().map(|u| char::from_u32(*u as u32).unwrap_or('?')).collect();
                if !self.eat('}') {
                    return err("Invalid property name");
                }
                let set = property(&name).ok_or(Error(String::from("Invalid property name")))?;
                return Ok(Some(if neg { set.invert() } else { set }));
            }
            _ => return Ok(None),
        };
        self.at += 1;
        Ok(Some(set))
    }

    /// A character escape (the cursor after the backslash).
    fn char_escape(&mut self, in_class: bool) -> Result<u32, Error> {
        let c = self.peek().ok_or(Error(String::from("\\ at end of pattern")))?;
        self.at += 1;
        Ok(match char::from_u32(c).unwrap_or('\0') {
            't' => 9,
            'n' => 10,
            'v' => 11,
            'f' => 12,
            'r' => 13,
            'c' => match self.peek().and_then(char::from_u32) {
                Some(l) if l.is_ascii_alphabetic() => {
                    self.at += 1;
                    l as u32 % 32
                }
                Some(l) if in_class && !self.u && (l.is_ascii_digit() || l == '_') => {
                    self.at += 1;
                    l as u32 % 32
                }
                _ => {
                    if self.u {
                        return err("Invalid unicode escape");
                    }
                    // Annex B: the backslash itself, and "c" read next.
                    self.at -= 1;
                    '\\' as u32
                }
            },
            '0' if !self.peek().is_some_and(|d| ('0' as u32..='9' as u32).contains(&d)) => 0,
            '0'..='7' if !self.u => {
                self.at -= 1;
                self.octal()
            }
            'x' => match self.hex(2) {
                Some(v) => v,
                None if self.u => return err("Invalid escape"),
                None => 'x' as u32,
            },
            'u' => match self.unicode_escape_body() {
                Some(v) => v,
                None if self.u => return err("Invalid Unicode escape"),
                None => 'u' as u32,
            },
            '-' if in_class && self.u => '-' as u32,
            other => {
                let o = other as u32;
                if self.u {
                    if is_syntax(o) {
                        return Ok(o);
                    }
                    return err("Invalid escape");
                }
                if other == 'k' && self.has_names {
                    return err("Invalid escape");
                }
                // (A surrogate pair after a backslash, outside Unicode mode: one unit at a time.)
                o
            }
        })
    }

    /// A class's contents (after its "["): the set, and whether it's negated.
    fn class(&mut self) -> Result<(Set, bool), Error> {
        let neg = self.eat('^');
        if self.v {
            let s = self.class_set()?;
            return Ok((s, neg));
        }
        let mut set = Set::default();
        loop {
            let Some(c) = self.peek() else { return err("Unterminated character class") };
            if c == ']' as u32 {
                self.at += 1;
                break;
            }
            let a = self.class_atom()?;
            if self.peek() == Some('-' as u32) && self.peek_at(1).is_some_and(|x| x != ']' as u32) {
                self.at += 1;
                let b = self.class_atom()?;
                match (a, b) {
                    (ClassAtom::Char(x), ClassAtom::Char(y)) => {
                        if x > y {
                            return err("Range out of order in character class");
                        }
                        set.add(x, y);
                    }
                    (a, b) => {
                        if self.u {
                            return err("Invalid character class");
                        }
                        // Annex B: a class escape in a range: both, and "-".
                        for x in [a, b] {
                            match x {
                                ClassAtom::Char(c) => set.add(c, c),
                                ClassAtom::Set(s) => set.add_set(&s),
                            }
                        }
                        set.add('-' as u32, '-' as u32);
                    }
                }
                continue;
            }
            match a {
                ClassAtom::Char(c) => set.add(c, c),
                ClassAtom::Set(s) => set.add_set(&s),
            }
        }
        set.normalize();
        Ok((set, neg))
    }

    fn class_atom(&mut self) -> Result<ClassAtom, Error> {
        let c = self.peek().ok_or(Error(String::from("Unterminated character class")))?;
        if c == '\\' as u32 {
            self.at += 1;
            if let Some(s) = self.class_escape()? {
                return Ok(ClassAtom::Set(s));
            }
            if self.peek() == Some('b' as u32) {
                self.at += 1;
                return Ok(ClassAtom::Char(8));
            }
            if self.u && self.peek() == Some('-' as u32) {
                self.at += 1;
                return Ok(ClassAtom::Char('-' as u32));
            }
            // (Outside Unicode mode, "\1" in a class is an octal escape; "\8" is "8".)
            if !self.u
                && let Some(d) = self.peek()
                && ('1' as u32..='9' as u32).contains(&d)
            {
                if d >= '8' as u32 {
                    self.at += 1;
                    return Ok(ClassAtom::Char(d));
                }
                return Ok(ClassAtom::Char(self.octal()));
            }
            return Ok(ClassAtom::Char(self.char_escape(true)?));
        }
        Ok(ClassAtom::Char(self.next_char().unwrap_or(c)))
    }

    /// A v-mode class: unions, intersections (&&), subtractions (--) and nested classes.
    fn class_set(&mut self) -> Result<Set, Error> {
        let mut acc = Set::default();
        let mut op: Option<u8> = None;
        let mut first = true;
        loop {
            let Some(c) = self.peek() else { return err("Unterminated character class") };
            if c == ']' as u32 {
                self.at += 1;
                break;
            }
            if c == '&' as u32 && self.peek_at(1) == Some('&' as u32) {
                self.at += 2;
                op = Some(b'&');
                continue;
            }
            if c == '-' as u32 && self.peek_at(1) == Some('-' as u32) {
                self.at += 2;
                op = Some(b'-');
                continue;
            }
            let mut operand = Set::default();
            if c == '[' as u32 {
                self.at += 1;
                let neg = self.eat('^');
                let inner = self.class_set()?;
                operand = if neg { inner.invert() } else { inner };
            } else if c == '\\' as u32 && self.peek_at(1) == Some('q' as u32) {
                // \q{strings}: single characters only.
                self.at += 2;
                if !self.eat('{') {
                    return err("Invalid escape");
                }
                while let Some(x) = self.peek() {
                    if x == '}' as u32 {
                        break;
                    }
                    if x == '|' as u32 {
                        self.at += 1;
                        continue;
                    }
                    let ch = if x == '\\' as u32 {
                        self.at += 1;
                        self.char_escape(true)?
                    } else {
                        self.next_char().unwrap_or(x)
                    };
                    operand.add(ch, ch);
                }
                self.eat('}');
            } else {
                let a = self.class_atom()?;
                match a {
                    ClassAtom::Char(x) => {
                        if self.peek() == Some('-' as u32) && self.peek_at(1) != Some('-' as u32) {
                            self.at += 1;
                            match self.class_atom()? {
                                ClassAtom::Char(y) if y >= x => operand.add(x, y),
                                _ => return err("Invalid character class"),
                            }
                        } else {
                            operand.add(x, x);
                        }
                    }
                    ClassAtom::Set(s) => operand = s,
                }
            }
            operand.normalize();
            if first {
                acc = operand;
                first = false;
            } else {
                match op {
                    Some(b'&') => acc = acc.intersect(&operand),
                    Some(b'-') => acc = acc.intersect(&operand.invert()),
                    _ => {
                        acc.add_set(&operand);
                        acc.normalize();
                    }
                }
            }
            op = None;
        }
        acc.normalize();
        Ok(acc)
    }
}

enum ClassAtom {
    Char(u32),
    Set(Set),
}

fn digits() -> Set {
    Set { ranges: vec![(0x30, 0x39)] }
}

fn word(unicode_ic: bool) -> Set {
    let mut s = Set { ranges: vec![(0x30, 0x39), (0x41, 0x5A), (0x5F, 0x5F), (0x61, 0x7A)] };
    if unicode_ic {
        s.add(0x17F, 0x17F);
        s.add(0x212A, 0x212A);
        s.normalize();
    }
    s
}

pub fn spaces() -> Set {
    let mut s = Set { ranges: vec![(9, 13), (0x20, 0x20), (0xA0, 0xA0), (0x1680, 0x1680), (0x2000, 0x200A), (0x2028, 0x2029), (0x202F, 0x202F), (0x205F, 0x205F), (0x3000, 0x3000), (0xFEFF, 0xFEFF)] };
    s.normalize();
    s
}

/// A Unicode property's set (General_Category values, and some binary properties).
fn property(name: &str) -> Option<Set> {
    let (key, value) = match name.split_once('=') {
        Some((k, v)) => (Some(k), v),
        None => (None, name),
    };
    let cat = |names: &[&str]| -> Set {
        let mut s = Set::default();
        for n in names {
            if let Some(i) = CATEGORIES.iter().position(|c| c == n) {
                for &(a, b) in CATEGORY_RANGES[i] {
                    s.add(a, b);
                }
            }
        }
        s.normalize();
        s
    };
    let gc = |v: &str| -> Option<Set> {
        let short: &[&str] = match v {
            "L" | "Letter" => &["Lu", "Ll", "Lt", "Lm", "Lo"],
            "LC" | "Cased_Letter" => &["Lu", "Ll", "Lt"],
            "Lu" | "Uppercase_Letter" => &["Lu"],
            "Ll" | "Lowercase_Letter" => &["Ll"],
            "Lt" | "Titlecase_Letter" => &["Lt"],
            "Lm" | "Modifier_Letter" => &["Lm"],
            "Lo" | "Other_Letter" => &["Lo"],
            "M" | "Mark" | "Combining_Mark" => &["Mn", "Mc", "Me"],
            "Mn" | "Nonspacing_Mark" => &["Mn"],
            "Mc" | "Spacing_Mark" => &["Mc"],
            "Me" | "Enclosing_Mark" => &["Me"],
            "N" | "Number" => &["Nd", "Nl", "No"],
            "Nd" | "Decimal_Number" | "digit" => &["Nd"],
            "Nl" | "Letter_Number" => &["Nl"],
            "No" | "Other_Number" => &["No"],
            "P" | "Punctuation" | "punct" => &["Pc", "Pd", "Ps", "Pe", "Pi", "Pf", "Po"],
            "Pc" | "Connector_Punctuation" => &["Pc"],
            "Pd" | "Dash_Punctuation" => &["Pd"],
            "Ps" | "Open_Punctuation" => &["Ps"],
            "Pe" | "Close_Punctuation" => &["Pe"],
            "Pi" | "Initial_Punctuation" => &["Pi"],
            "Pf" | "Final_Punctuation" => &["Pf"],
            "Po" | "Other_Punctuation" => &["Po"],
            "S" | "Symbol" => &["Sm", "Sc", "Sk", "So"],
            "Sm" | "Math_Symbol" => &["Sm"],
            "Sc" | "Currency_Symbol" => &["Sc"],
            "Sk" | "Modifier_Symbol" => &["Sk"],
            "So" | "Other_Symbol" => &["So"],
            "Z" | "Separator" => &["Zs", "Zl", "Zp"],
            "Zs" | "Space_Separator" => &["Zs"],
            "Zl" | "Line_Separator" => &["Zl"],
            "Zp" | "Paragraph_Separator" => &["Zp"],
            "C" | "Other" => &["Cc", "Cf", "Cs", "Co", "Cn"],
            "Cc" | "Control" | "cntrl" => &["Cc"],
            "Cf" | "Format" => &["Cf"],
            "Cs" | "Surrogate" => &["Cs"],
            "Co" | "Private_Use" => &["Co"],
            "Cn" | "Unassigned" => &["Cn"],
            _ => return None,
        };
        Some(cat(short))
    };
    match key {
        Some("General_Category" | "gc") => return gc(value),
        Some("Script" | "sc" | "Script_Extensions" | "scx") => return script(value),
        Some(_) => return None,
        None => {}
    }
    if let Some(s) = gc(value) {
        return Some(s);
    }
    let mut s = Set::default();
    match value {
        "Any" => s.add(0, 0x10FFFF),
        "ASCII" => s.add(0, 0x7F),
        "ASCII_Hex_Digit" | "AHex" => {
            s.add(0x30, 0x39);
            s.add(0x41, 0x46);
            s.add(0x61, 0x66);
        }
        "Alphabetic" | "Alpha" => {
            s = cat(&["Lu", "Ll", "Lt", "Lm", "Lo", "Nl"]);
            s.add_set(&cat(&["Mn", "Mc"]).intersect(&Set { ranges: vec![(0x345, 0x345), (0x5B0, 0x5C7), (0x610, 0x61A), (0x64B, 0x657), (0x900, 0xDFF)] }));
        }
        "White_Space" | "space" => s = spaces().intersect(&Set { ranges: vec![(0, 0xFEFE)] }),
        "Uppercase" | "Upper" => s = cat(&["Lu"]),
        "Lowercase" | "Lower" => s = cat(&["Ll"]),
        "ID_Start" | "IDS" => {
            s.ranges = crate::unicode_id::ID_START.to_vec();
            s.add(0x41, 0x5A);
            s.add(0x61, 0x7A);
        }
        "ID_Continue" | "IDC" => {
            s.ranges = crate::unicode_id::ID_CONTINUE.to_vec();
            s.add(0x30, 0x39);
            s.add(0x41, 0x5A);
            s.add(0x5F, 0x5F);
            s.add(0x61, 0x7A);
        }
        "Emoji" | "Emoji_Presentation" | "Extended_Pictographic" | "EPres" | "ExtPict" => {
            for (a, b) in [(0x203C, 0x203C), (0x2049, 0x2049), (0x2122, 0x2122), (0x2139, 0x2139), (0x2194, 0x21AA), (0x231A, 0x23FF), (0x24C2, 0x24C2), (0x25AA, 0x27BF), (0x2934, 0x2935), (0x2B05, 0x2B55), (0x3030, 0x303D), (0x3297, 0x3299), (0x1F000, 0x1FAFF)] {
                s.add(a, b);
            }
            if value == "Emoji" {
                s.add(0x23, 0x23);
                s.add(0x2A, 0x2A);
                s.add(0x30, 0x39);
                s.add(0xA9, 0xA9);
                s.add(0xAE, 0xAE);
            }
        }
        "Emoji_Modifier" | "EMod" => s.add(0x1F3FB, 0x1F3FF),
        "Emoji_Component" | "EComp" => {
            s.add(0x1F3FB, 0x1F3FF);
            s.add(0x200D, 0x200D);
            s.add(0xFE0F, 0xFE0F);
            s.add(0x1F1E6, 0x1F1FF);
        }
        "Regional_Indicator" | "RI" => s.add(0x1F1E6, 0x1F1FF),
        _ => return None,
    }
    s.normalize();
    Some(s)
}

/// The scripts most text is in (their main blocks; a script's own code points spread wider than these).
fn script(name: &str) -> Option<Set> {
    let r: &[(u32, u32)] = match name {
        "Latin" | "Latn" => &[(0x41, 0x5A), (0x61, 0x7A), (0xAA, 0xAA), (0xBA, 0xBA), (0xC0, 0xD6), (0xD8, 0xF6), (0xF8, 0x24F), (0x1E00, 0x1EFF), (0x2C60, 0x2C7F), (0xA720, 0xA7FF), (0xFF21, 0xFF3A), (0xFF41, 0xFF5A)],
        "Greek" | "Grek" => &[(0x370, 0x3FF), (0x1F00, 0x1FFF)],
        "Cyrillic" | "Cyrl" => &[(0x400, 0x52F), (0x2DE0, 0x2DFF), (0xA640, 0xA69F)],
        "Armenian" | "Armn" => &[(0x531, 0x58F)],
        "Hebrew" | "Hebr" => &[(0x591, 0x5F4), (0xFB1D, 0xFB4F)],
        "Arabic" | "Arab" => &[(0x600, 0x6FF), (0x750, 0x77F), (0x8A0, 0x8FF), (0xFB50, 0xFDFF), (0xFE70, 0xFEFF)],
        "Devanagari" | "Deva" => &[(0x900, 0x97F)],
        "Bengali" | "Beng" => &[(0x980, 0x9FF)],
        "Thai" => &[(0xE00, 0xE7F)],
        "Hangul" | "Hang" => &[(0x1100, 0x11FF), (0x3130, 0x318F), (0xAC00, 0xD7AF)],
        "Hiragana" | "Hira" => &[(0x3041, 0x309F)],
        "Katakana" | "Kana" => &[(0x30A0, 0x30FF), (0x31F0, 0x31FF), (0xFF66, 0xFF9F)],
        "Han" | "Hani" => &[(0x2E80, 0x2FDF), (0x3005, 0x3007), (0x3021, 0x3029), (0x3400, 0x4DBF), (0x4E00, 0x9FFF), (0xF900, 0xFAFF), (0x20000, 0x3134F)],
        _ => return None,
    };
    Some(Set { ranges: r.to_vec() })
}

/// Counts a pattern's capturing groups (before parsing it, so \10 can be told from \1 then 0) and whether it names
/// any.
fn count_groups(p: &[u16]) -> (usize, bool) {
    let (mut n, mut named) = (0, false);
    let mut i = 0;
    let mut in_class = false;
    while i < p.len() {
        match p[i] {
            0x5C => i += 1,
            0x5B => in_class = true,
            0x5D => in_class = false,
            0x28 if !in_class => {
                if p.get(i + 1) != Some(&(b'?' as u16)) {
                    n += 1;
                } else if p.get(i + 2) == Some(&(b'<' as u16)) && !matches!(p.get(i + 3), Some(&0x3D) | Some(&0x21)) {
                    n += 1;
                    named = true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (n, named)
}

// ------------------------------------------------------------------------------------------------- the program

#[derive(Clone, Copy, Debug)]
enum Inst {
    Char(u32, bool),
    Any(bool),
    Set(u32, bool, bool),
    Split(u32, u32),
    Jmp(u32),
    Save(u32),
    Reset(u32, u32),
    Bol,
    Eol,
    Word(bool),
    BackRef(u32, bool),
    Look(u32, bool, bool),
    /// A loop's head: (counter register, position register, min, max, greedy, the loop's exit).
    RepInit(u32),
    Rep(u32, u32, u32, u32, bool, u32),
    /// A loop's tail: (counter register, position register, min, the head).
    RepEnd(u32, u32, u32, u32),
    /// A one-character loop: (its character test's instruction, min, max, greedy).
    Star(u32, u32, u32, bool),
    Match,
}

pub struct Program {
    insts: Vec<Inst>,
    sets: Vec<Set>,
    pub groups: usize,
    pub names: Vec<(Vec<u16>, usize)>,
    pub flags: Flags,
    registers: usize,
}

struct Compiler<'a> {
    insts: Vec<Inst>,
    sets: Vec<Set>,
    registers: usize,
    flags: Flags,
    names: &'a [(Vec<u16>, usize)],
}

/// The capture groups a node holds: (first, one past the last).
fn group_range(n: &Node) -> Option<(usize, usize)> {
    let mut lo = usize::MAX;
    let mut hi = 0;
    fn walk(n: &Node, lo: &mut usize, hi: &mut usize) {
        match n {
            Node::Group(inner, idx) => {
                if let Some(i) = idx {
                    *lo = (*lo).min(*i);
                    *hi = (*hi).max(*i + 1);
                }
                walk(inner, lo, hi);
            }
            Node::Seq(v) | Node::Alt(v) => v.iter().for_each(|x| walk(x, lo, hi)),
            Node::Repeat(x, ..) | Node::Look(x, ..) => walk(x, lo, hi),
            _ => {}
        }
    }
    walk(n, &mut lo, &mut hi);
    (hi > 0).then_some((lo, hi))
}

impl<'a> Compiler<'a> {
    fn emit(&mut self, i: Inst) -> u32 {
        self.insts.push(i);
        self.insts.len() as u32 - 1
    }

    fn here(&self) -> u32 {
        self.insts.len() as u32
    }

    fn set(&mut self, s: Set) -> u32 {
        self.sets.push(s);
        self.sets.len() as u32 - 1
    }

    /// A single-character node's test (for one-character loops).
    fn single(&mut self, n: &Node, back: bool) -> Option<Inst> {
        Some(match n {
            Node::Char(c) => Inst::Char(self.canon(*c), back),
            Node::Any => Inst::Any(back),
            Node::Class(s, neg) => {
                let i = self.set(s.clone());
                Inst::Set(i, *neg, back)
            }
            _ => return None,
        })
    }

    fn canon(&self, c: u32) -> u32 {
        if self.flags.ignore_case { canonicalize(c, self.flags.full_unicode()) } else { c }
    }

    fn node(&mut self, n: &Node, back: bool) -> Result<(), Error> {
        match n {
            Node::Empty => {}
            Node::Char(c) => {
                let c = self.canon(*c);
                self.emit(Inst::Char(c, back));
            }
            Node::Any => {
                self.emit(Inst::Any(back));
            }
            Node::Class(s, neg) => {
                let i = self.set(s.clone());
                self.emit(Inst::Set(i, *neg, back));
            }
            Node::Seq(items) => {
                if back {
                    for x in items.iter().rev() {
                        self.node(x, back)?;
                    }
                } else {
                    for x in items {
                        self.node(x, back)?;
                    }
                }
            }
            Node::Alt(alts) => {
                let mut ends = Vec::new();
                for (i, a) in alts.iter().enumerate() {
                    if i + 1 < alts.len() {
                        let split = self.emit(Inst::Split(0, 0));
                        self.node(a, back)?;
                        ends.push(self.emit(Inst::Jmp(0)));
                        let next = self.here();
                        self.insts[split as usize] = Inst::Split(split + 1, next);
                    } else {
                        self.node(a, back)?;
                    }
                }
                let end = self.here();
                for e in ends {
                    self.insts[e as usize] = Inst::Jmp(end);
                }
            }
            Node::Group(inner, idx) => match idx {
                Some(i) => {
                    let (open, close) = ((*i * 2) as u32, (*i * 2 + 1) as u32);
                    self.emit(Inst::Save(if back { close } else { open }));
                    self.node(inner, back)?;
                    self.emit(Inst::Save(if back { open } else { close }));
                }
                None => self.node(inner, back)?,
            },
            Node::Repeat(inner, min, max, greedy) => {
                if *max == 0 {
                    return Ok(());
                }
                if let Some(test) = self.single(inner, back) {
                    // A one-character loop: the Star runs its test (the next instruction), then goes on after it.
                    let star = self.emit(Inst::Star(0, *min, *max, *greedy));
                    let t = self.emit(test);
                    self.insts[star as usize] = Inst::Star(t, *min, *max, *greedy);
                    return Ok(());
                }
                let (g0, g1) = group_range(inner).unwrap_or((0, 0));
                let counter = self.registers as u32;
                let pos = counter + 1;
                self.registers += 2;
                self.emit(Inst::RepInit(counter));
                let head = self.emit(Inst::Rep(counter, pos, *min, *max, *greedy, 0));
                if g1 > g0 {
                    self.emit(Inst::Reset((g0 * 2) as u32, (g1 * 2) as u32));
                }
                self.node(inner, back)?;
                self.emit(Inst::RepEnd(counter, pos, *min, head));
                let exit = self.here();
                self.insts[head as usize] = Inst::Rep(counter, pos, *min, *max, *greedy, exit);
            }
            Node::Bol => {
                self.emit(Inst::Bol);
            }
            Node::Eol => {
                self.emit(Inst::Eol);
            }
            Node::WordBoundary(neg) => {
                self.emit(Inst::Word(*neg));
            }
            Node::BackRef(i) => {
                self.emit(Inst::BackRef(*i as u32, back));
            }
            Node::NamedRef(name) => {
                let Some((_, i)) = self.names.iter().find(|(n, _)| n == name) else { return err("Invalid named capture referenced") };
                self.emit(Inst::BackRef(*i as u32, back));
            }
            Node::Look(inner, behind, negate) => {
                let look = self.emit(Inst::Look(0, *behind, *negate));
                let skip = self.emit(Inst::Jmp(0));
                let start = self.here();
                self.node(inner, *behind)?;
                self.emit(Inst::Match);
                let after = self.here();
                self.insts[look as usize] = Inst::Look(start, *behind, *negate);
                self.insts[skip as usize] = Inst::Jmp(after);
            }
        }
        Ok(())
    }
}

/// Compiles a pattern (its text in UTF-16) with its flags.
pub fn compile(pattern: &[u16], flags: Flags) -> Result<Program, Error> {
    let (total, named) = count_groups(pattern);
    let mut p = Parser { p: pattern, at: 0, u: flags.full_unicode(), v: flags.unicode_sets, groups: 0, total_groups: total, names: Vec::new(), has_names: named };
    let tree = p.disjunction()?;
    if p.at < pattern.len() {
        return err(if pattern[p.at] == b')' as u16 { "Unmatched ')'" } else { "Unexpected character" });
    }
    let names = core::mem::take(&mut p.names);
    let groups = p.groups;
    let mut c = Compiler { insts: Vec::new(), sets: Vec::new(), registers: 0, flags, names: &names };
    c.emit(Inst::Save(0));
    c.node(&tree, false)?;
    c.emit(Inst::Save(1));
    c.emit(Inst::Match);
    let (insts, sets, registers) = (c.insts, c.sets, c.registers);
    Ok(Program { insts, sets, groups, names, flags, registers })
}

/// Canonicalize (for ignoring case): Unicode mode's simple case folding, else upper case (but not to ASCII from
/// beyond it).
pub fn canonicalize(c: u32, unicode: bool) -> u32 {
    let Some(ch) = char::from_u32(c) else { return c };
    if unicode {
        // (Simple case folding, mostly lower case.)
        match c {
            0x1E9E => return 0xDF,
            0x2126 => return 0x3C9,
            0x212A => return 'k' as u32,
            0x212B => return 0xE5,
            0x3C2 => return 0x3C3,
            0x3D0 => return 0x3B2,
            0x3D1 => return 0x3B8,
            0x3D5 => return 0x3C6,
            0x3D6 => return 0x3C0,
            0x3F0 => return 0x3BA,
            0x3F1 => return 0x3C1,
            0x3F5 => return 0x3B5,
            0x1E9B => return 0x1E61,
            0x17F => return 's' as u32,
            _ => {}
        }
        let mut l = ch.to_lowercase();
        if let (Some(x), None) = (l.next(), l.next()) {
            return x as u32;
        }
        return c;
    }
    let mut u = ch.to_uppercase();
    match (u.next(), u.next()) {
        (Some(x), None) => {
            let x = x as u32;
            if c >= 128 && x < 128 { c } else { x }
        }
        _ => c,
    }
}

// --------------------------------------------------------------------------------------------------- matching

enum Bt {
    /// Try again at pc, from pos.
    Alt(u32, usize),
    /// Undo a capture or register's change.
    Undo(usize, usize),
    /// A one-character loop: (its next instruction, where it began, how many it matched, min, max, greedy, test).
    Star(u32, usize, usize, u32, u32, bool, u32),
}

/// A match: each group's (start, end) in code units (None: it didn't take part).
pub type Captures = Vec<Option<(usize, usize)>>;

pub struct TooComplex;

struct Matcher<'a> {
    prog: &'a Program,
    s: &'a [u16],
    caps: Vec<usize>,
    regs: Vec<usize>,
    steps: u64,
    unicode: bool,
}

const UNSET: usize = usize::MAX;
const STEP_LIMIT: u64 = 100_000_000;

impl<'a> Matcher<'a> {
    /// The character at pos going forward (or before pos going back): (char, its length).
    fn char_at(&self, pos: usize, back: bool) -> Option<(u32, usize)> {
        if back {
            if pos == 0 {
                return None;
            }
            let c = self.s[pos - 1] as u32;
            if self.unicode && (0xDC00..0xE000).contains(&c) && pos >= 2 {
                let h = self.s[pos - 2] as u32;
                if (0xD800..0xDC00).contains(&h) {
                    return Some((0x10000 + ((h - 0xD800) << 10) + (c - 0xDC00), 2));
                }
            }
            return Some((c, 1));
        }
        let c = *self.s.get(pos)? as u32;
        if self.unicode
            && (0xD800..0xDC00).contains(&c)
            && let Some(&d) = self.s.get(pos + 1)
            && (0xDC00..0xE000).contains(&(d as u32))
        {
            return Some((0x10000 + ((c - 0xD800) << 10) + (d as u32 - 0xDC00), 2));
        }
        Some((c, 1))
    }

    fn is_word(&self, pos: usize) -> bool {
        let Some(&c) = self.s.get(pos) else { return false };
        let c = c as u32;
        let ic = self.prog.flags.ignore_case && self.unicode;
        (c < 128 && ((c as u8).is_ascii_alphanumeric() || c == '_' as u32)) || (ic && (c == 0x17F || c == 0x212A))
    }

    fn line_terminator(c: u16) -> bool {
        matches!(c, 0x0A | 0x0D | 0x2028 | 0x2029)
    }

    /// Whether a character passes a test instruction.
    fn test(&self, inst: Inst, c: u32) -> bool {
        let ic = self.prog.flags.ignore_case;
        match inst {
            Inst::Char(want, _) => {
                if ic {
                    canonicalize(c, self.unicode) == want
                } else {
                    c == want
                }
            }
            Inst::Any(_) => self.prog.flags.dot_all || !matches!(c, 0x0A | 0x0D | 0x2028 | 0x2029),
            Inst::Set(i, neg, _) => {
                let set = &self.prog.sets[i as usize];
                let mut found = set.contains(c);
                if !found && ic {
                    let k = canonicalize(c, self.unicode);
                    if let Some(ch) = char::from_u32(c) {
                        for v in ch.to_lowercase().chain(ch.to_uppercase()) {
                            let v = v as u32;
                            if set.contains(v) && canonicalize(v, self.unicode) == k {
                                found = true;
                                break;
                            }
                        }
                    }
                    if !found && set.contains(k) {
                        found = true;
                    }
                    // (The other ones that fold to the same: the few beyond simple upper and lower case.)
                    if !found {
                        for &x in &[0x17F, 0x212A, 0x212B, 0x1E9E, 0x2126, 0x3C2, 0x3D0, 0x3D1, 0x3D5, 0x3D6, 0x3F0, 0x3F1, 0x3F5, 0x1E9B, 0xDF, 0x3C9, 0xE5, 0x3C3, 0x3B2, 0x3B8, 0x3C6, 0x3C0, 0x3BA, 0x3C1, 0x3B5, 0x1E61, 0x73, 0x6B] {
                            if set.contains(x) && canonicalize(x, self.unicode) == k {
                                found = true;
                                break;
                            }
                        }
                    }
                }
                found != neg
            }
            _ => false,
        }
    }

    fn set_cap(&mut self, slot: usize, v: usize, bt: &mut Vec<Bt>) {
        bt.push(Bt::Undo(slot, self.caps[slot]));
        self.caps[slot] = v;
    }

    fn set_reg(&mut self, r: usize, v: usize, bt: &mut Vec<Bt>) {
        bt.push(Bt::Undo(usize::MAX - r, self.regs[r]));
        self.regs[r] = v;
    }

    /// Runs from pc at pos: the end position, or None.
    fn run(&mut self, mut pc: u32, mut pos: usize) -> Result<Option<usize>, TooComplex> {
        let mut bt: Vec<Bt> = Vec::new();
        loop {
            self.steps += 1;
            if self.steps > STEP_LIMIT {
                return Err(TooComplex);
            }
            let inst = self.prog.insts[pc as usize];
            let ok = match inst {
                Inst::Match => return Ok(Some(pos)),
                Inst::Char(_, back) | Inst::Any(back) | Inst::Set(_, _, back) => match self.char_at(pos, back) {
                    Some((c, l)) if self.test(inst, c) => {
                        pos = if back { pos - l } else { pos + l };
                        true
                    }
                    _ => false,
                },
                Inst::Split(a, b) => {
                    bt.push(Bt::Alt(b, pos));
                    pc = a;
                    continue;
                }
                Inst::Jmp(t) => {
                    pc = t;
                    continue;
                }
                Inst::Save(slot) => {
                    self.set_cap(slot as usize, pos, &mut bt);
                    true
                }
                Inst::Reset(a, b) => {
                    for slot in a..b {
                        if self.caps[slot as usize] != UNSET {
                            self.set_cap(slot as usize, UNSET, &mut bt);
                        }
                    }
                    true
                }
                Inst::Bol => pos == 0 || (self.prog.flags.multiline && Self::line_terminator(self.s[pos - 1])),
                Inst::Eol => pos == self.s.len() || (self.prog.flags.multiline && Self::line_terminator(self.s[pos])),
                Inst::Word(neg) => {
                    let a = pos > 0 && self.is_word(pos - 1);
                    let b = self.is_word(pos);
                    (a != b) != neg
                }
                Inst::BackRef(g, back) => {
                    let (s0, e0) = (self.caps[g as usize * 2], self.caps[g as usize * 2 + 1]);
                    if s0 == UNSET || e0 == UNSET {
                        true
                    } else {
                        let len = e0 - s0;
                        let ic = self.prog.flags.ignore_case;
                        let same = |a: u16, b: u16| if ic { canonicalize(a as u32, self.unicode) == canonicalize(b as u32, self.unicode) } else { a == b };
                        if back {
                            if pos >= len && (0..len).all(|k| same(self.s[s0 + k], self.s[pos - len + k])) {
                                pos -= len;
                                true
                            } else {
                                false
                            }
                        } else if pos + len <= self.s.len() && (0..len).all(|k| same(self.s[s0 + k], self.s[pos + k])) {
                            pos += len;
                            true
                        } else {
                            false
                        }
                    }
                }
                Inst::Look(start, behind, negate) => {
                    let saved = self.caps.clone();
                    let r = self.run(start, pos)?;
                    let _ = behind;
                    match (r.is_some(), negate) {
                        (true, false) => {
                            // Its captures stay (undone if this match backtracks past it).
                            for (slot, old) in saved.iter().enumerate() {
                                if self.caps[slot] != *old {
                                    bt.push(Bt::Undo(slot, *old));
                                }
                            }
                            true
                        }
                        (false, true) => {
                            self.caps = saved;
                            true
                        }
                        _ => {
                            self.caps = saved;
                            false
                        }
                    }
                }
                Inst::RepInit(r) => {
                    self.set_reg(r as usize, 0, &mut bt);
                    true
                }
                Inst::Rep(r, p, min, max, greedy, exit) => {
                    let count = self.regs[r as usize] as u32;
                    if count < min {
                        self.set_reg(p as usize, pos, &mut bt);
                        pc += 1;
                        continue;
                    }
                    if count >= max {
                        pc = exit;
                        continue;
                    }
                    if greedy {
                        bt.push(Bt::Alt(exit, pos));
                        self.set_reg(p as usize, pos, &mut bt);
                        pc += 1;
                    } else {
                        // Lazy: the exit first; the body when it fails.
                        let save = bt.len();
                        let _ = save;
                        self.set_reg(p as usize, pos, &mut bt);
                        bt.push(Bt::Alt(pc + 1, pos));
                        pc = exit;
                    }
                    continue;
                }
                Inst::RepEnd(r, p, min, head) => {
                    let count = self.regs[r as usize];
                    // An iteration past the minimum that matched nothing ends the loop's tries.
                    if pos == self.regs[p as usize] && count as u32 >= min {
                        false
                    } else {
                        self.set_reg(r as usize, count + 1, &mut bt);
                        pc = head;
                        continue;
                    }
                }
                Inst::Star(t, min, max, greedy) => {
                    let test = self.prog.insts[t as usize];
                    let back = matches!(test, Inst::Char(_, true) | Inst::Any(true) | Inst::Set(_, _, true));
                    let start = pos;
                    let mut n = 0u32;
                    let want = if greedy { max } else { min };
                    while n < want {
                        match self.char_at(pos, back) {
                            Some((c, l)) if self.test(test, c) => {
                                pos = if back { pos - l } else { pos + l };
                                n += 1;
                            }
                            _ => break,
                        }
                    }
                    if n < min {
                        false
                    } else {
                        bt.push(Bt::Star(pc + 2, start, n as usize, min, max, greedy, t));
                        pc += 2;
                        continue;
                    }
                }
            };
            if ok {
                pc += 1;
                continue;
            }
            // Backtrack.
            loop {
                match bt.pop() {
                    None => return Ok(None),
                    Some(Bt::Undo(slot, old)) => {
                        if slot > usize::MAX / 2 {
                            self.regs[usize::MAX - slot] = old;
                        } else {
                            self.caps[slot] = old;
                        }
                    }
                    Some(Bt::Alt(a, p)) => {
                        pc = a;
                        pos = p;
                        break;
                    }
                    Some(Bt::Star(next, start, n, min, max, greedy, t)) => {
                        let test = self.prog.insts[t as usize];
                        let back = matches!(test, Inst::Char(_, true) | Inst::Any(true) | Inst::Set(_, _, true));
                        // The position after n characters (n of them from start).
                        let at = |m: &Matcher, k: usize| -> usize {
                            let mut p = start;
                            for _ in 0..k {
                                let l = m.char_at(p, back).map_or(1, |x| x.1);
                                p = if back { p - l } else { p + l };
                            }
                            p
                        };
                        if greedy {
                            if n as u32 > min {
                                let k = n - 1;
                                bt.push(Bt::Star(next, start, k, min, max, greedy, t));
                                pos = at(self, k);
                                pc = next;
                                break;
                            }
                        } else if (n as u32) < max {
                            let p = at(self, n);
                            if let Some((c, l)) = self.char_at(p, back)
                                && self.test(test, c)
                            {
                                bt.push(Bt::Star(next, start, n + 1, min, max, greedy, t));
                                pos = if back { p - l } else { p + l };
                                pc = next;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Program {
    /// Matches at `start` (only there): each group's span, or None.
    pub fn match_at(&self, s: &[u16], start: usize) -> Result<Option<Captures>, TooComplex> {
        let mut m = Matcher { prog: self, s, caps: vec![UNSET; (self.groups + 1) * 2], regs: vec![0; self.registers], steps: 0, unicode: self.flags.full_unicode() };
        let r = m.run(0, start)?;
        Ok(r.map(|_| {
            (0..=self.groups)
                .map(|g| {
                    let (a, b) = (m.caps[g * 2], m.caps[g * 2 + 1]);
                    if a == UNSET || b == UNSET { None } else { Some((a.min(b), a.max(b))) }
                })
                .collect()
        }))
    }

    /// The first match at or after `start` (just at it if sticky).
    pub fn exec(&self, s: &[u16], start: usize, sticky: bool) -> Result<Option<Captures>, TooComplex> {
        let mut i = start;
        // (A pattern that starts with a plain character: skip to where it is.)
        let first = match self.insts.get(1) {
            Some(Inst::Char(c, false)) if !self.flags.ignore_case && *c < 0x10000 => Some(*c as u16),
            _ => None,
        };
        while i <= s.len() {
            if let Some(f) = first
                && !sticky
            {
                match s[i..].iter().position(|c| *c == f) {
                    Some(k) => i += k,
                    None => return Ok(None),
                }
            }
            if let Some(c) = self.match_at(s, i)? {
                return Ok(Some(c));
            }
            if sticky {
                return Ok(None);
            }
            i += 1;
            // (In Unicode mode, never between the halves of a pair.)
            if self.flags.full_unicode() && i < s.len() && (0xDC00..0xE000).contains(&s[i]) && (0xD800..0xDC00).contains(&s[i - 1]) {
                i += 1;
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    fn find(p: &str, f: &str, s: &str) -> Option<std::vec::Vec<Option<std::string::String>>> {
        let pu: Vec<u16> = p.encode_utf16().collect();
        let su: Vec<u16> = s.encode_utf16().collect();
        let prog = compile(&pu, Flags::parse(f).unwrap()).ok()?;
        let caps = prog.exec(&su, 0, false).ok()??;
        Some(caps.iter().map(|c| c.map(|(a, b)| std::string::String::from_utf16_lossy(&su[a..b]))).collect())
    }

    #[test]
    fn patterns_match_as_javascript_matches_them() {
        let one = |p: &str, f: &str, s: &str| find(p, f, s).map(|v| v[0].clone().unwrap_or_default());
        assert_eq!(one("a+b", "", "xxaaab"), Some("aaab".into()));
        assert_eq!(one("a*?b", "", "aab"), Some("aab".into()));
        assert_eq!(one("(a|ab)(c|bcd)(d*)", "", "abcd"), Some("abcd".into()));
        assert_eq!(one("^abc$", "m", "x\nabc\ny"), Some("abc".into()));
        assert_eq!(one("\\bfoo\\b", "", "a foo b"), Some("foo".into()));
        assert_eq!(one("[a-c]+", "i", "xxABCd"), Some("ABC".into()));
        assert_eq!(one("(?<=\\$)\\d+", "", "cost $42"), Some("42".into()));
        assert_eq!(one("(?<!\\$)\\b\\d+", "", "$4 5"), Some("5".into()));
        assert_eq!(one("x(?=y)", "", "xxy"), Some("x".into()));
        assert_eq!(one("(a)\\1", "", "aa"), Some("aa".into()));
        assert_eq!(one("(?<q>['\"]).*?\\k<q>", "", "say 'hi' now"), Some("'hi'".into()));
        assert_eq!(one("a{2,3}", "", "aaaa"), Some("aaa".into()));
        assert_eq!(one("a{,2}", "", "a{,2}"), Some("a{,2}".into()));
        assert_eq!(one("\\p{Lu}+", "u", "abcDEF"), Some("DEF".into()));
        assert_eq!(one(".", "u", "😀"), Some("😀".into()));
        assert_eq!(one("(z)((a+)?(b+)?(c))*", "", "zaacbbbcac"), Some("zaacbbbcac".into()));
        let v = find("(z)((a+)?(b+)?(c))*", "", "zaacbbbcac").unwrap();
        // (The specification's own example: the last iteration's captures, the others' cleared.)
        assert_eq!(v[2], Some("ac".into()));
        assert_eq!(v[3], Some("a".into()));
        assert_eq!(v[4], None);
        assert_eq!(v[5], Some("c".into()));
        assert_eq!(one("(a*)*", "", "b"), Some("".into()));
        assert_eq!(one("(a*)+", "", "b"), Some("".into()));
        assert_eq!(one("(?:a|b)*c", "", "ababc"), Some("ababc".into()));
        assert!(compile(&"(".encode_utf16().collect::<Vec<_>>(), Flags::default()).is_err());
        assert!(compile(&"a**".encode_utf16().collect::<Vec<_>>(), Flags::default()).is_err());
        assert!(compile(&"\\u{110000}".encode_utf16().collect::<Vec<_>>(), Flags::parse("u").unwrap()).is_err());
        // A long input doesn't overflow anything.
        let long: std::string::String = core::iter::repeat_n('a', 200_000).collect();
        assert_eq!(one("a*", "", &long).map(|s| s.len()), Some(200_000));
        assert_eq!(one("(?:a|b)*$", "", &long).map(|s| s.len()), Some(200_000));
    }
}
