//! JavaScript's strings: sequences of UTF-16 code units (so `length`, `charCodeAt` and the rest count as the language
//! says), shared and never changed. A string made by joining two long ones is a rope, its two halves kept until its
//! units are first asked for (then flattened once, its halves let go), so building a long string a piece at a time
//! (`s += x` in a loop) takes time in proportion to its length, not its square.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{OnceCell, RefCell};
use core::fmt;
use core::hash::{Hash, Hasher};

/// Joins shorter than this are copied at once; longer ones make a rope.
const ROPE_FROM: usize = 64;
/// The deepest a rope may be before its halves are flattened (so nothing that walks one goes deep).
const DEEPEST: u16 = 200;

#[derive(Clone)]
pub struct JsStr(Rc<Node>);

enum Node {
    Flat(Box<[u16]>),
    Rope { halves: RefCell<Option<(JsStr, JsStr)>>, flat: OnceCell<Box<[u16]>>, len: usize, depth: u16 },
}

use alloc::boxed::Box;

impl JsStr {
    pub fn empty() -> JsStr {
        JsStr(Rc::new(Node::Flat(Box::new([]))))
    }

    pub fn from_units(units: Vec<u16>) -> JsStr {
        JsStr(Rc::new(Node::Flat(units.into_boxed_slice())))
    }

    pub fn from_slice(units: &[u16]) -> JsStr {
        JsStr(Rc::new(Node::Flat(units.into())))
    }

    pub fn from_str(s: &str) -> JsStr {
        JsStr::from_units(s.encode_utf16().collect())
    }

    pub fn len(&self) -> usize {
        match &*self.0 {
            Node::Flat(u) => u.len(),
            Node::Rope { len, .. } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn depth(&self) -> u16 {
        match &*self.0 {
            Node::Flat(_) => 0,
            Node::Rope { depth, .. } => *depth,
        }
    }

    /// The code units.
    pub fn units(&self) -> &[u16] {
        match &*self.0 {
            Node::Flat(u) => u,
            Node::Rope { halves, flat, len, .. } => flat.get_or_init(|| {
                let mut out = Vec::with_capacity(*len);
                // (Without recursion: the halves still to copy, the rightmost first on the stack.)
                let (l, r) = halves.borrow_mut().take().expect("a rope not yet flat has its halves");
                let mut stack: Vec<JsStr> = alloc::vec![r, l];
                while let Some(s) = stack.pop() {
                    match &*s.0 {
                        Node::Flat(u) => out.extend_from_slice(u),
                        Node::Rope { halves, flat, .. } => {
                            if let Some(f) = flat.get() {
                                out.extend_from_slice(f);
                            } else if let Some((l, r)) = halves.borrow().as_ref() {
                                stack.push(r.clone());
                                stack.push(l.clone());
                            }
                        }
                    }
                }
                out.into_boxed_slice()
            }),
        }
    }

    /// `self` then `other`.
    pub fn concat(&self, other: &JsStr) -> JsStr {
        if other.is_empty() {
            return self.clone();
        }
        if self.is_empty() {
            return other.clone();
        }
        let len = self.len() + other.len();
        if len < ROPE_FROM {
            let mut v = Vec::with_capacity(len);
            v.extend_from_slice(self.units());
            v.extend_from_slice(other.units());
            return JsStr::from_units(v);
        }
        let (mut l, mut r) = (self.clone(), other.clone());
        // (Too deep: the deeper half made flat first.)
        if l.depth() >= DEEPEST {
            l = JsStr::from_slice(l.units());
        }
        if r.depth() >= DEEPEST {
            r = JsStr::from_slice(r.units());
        }
        let depth = l.depth().max(r.depth()) + 1;
        JsStr(Rc::new(Node::Rope { halves: RefCell::new(Some((l, r))), flat: OnceCell::new(), len, depth }))
    }

    pub fn at(&self, i: usize) -> Option<u16> {
        self.units().get(i).copied()
    }

    pub fn slice(&self, from: usize, to: usize) -> JsStr {
        let u = self.units();
        let to = to.min(u.len());
        let from = from.min(to);
        if from == 0 && to == u.len() {
            return self.clone();
        }
        JsStr::from_slice(&u[from..to])
    }

    /// The string as Rust text (unpaired surrogates as U+FFFD).
    pub fn to_rust(&self) -> String {
        char::decode_utf16(self.units().iter().copied()).map(|c| c.unwrap_or('\u{FFFD}')).collect()
    }

    pub fn ptr_eq(a: &JsStr, b: &JsStr) -> bool {
        Rc::ptr_eq(&a.0, &b.0)
    }

    /// Whether it's `s` (ASCII text compared unit by unit).
    pub fn is(&self, s: &str) -> bool {
        let u = self.units();
        u.len() == s.len() && s.bytes().zip(u.iter()).all(|(b, &c)| b as u16 == c)
    }

    pub fn starts_with(&self, prefix: &[u16]) -> bool {
        self.units().starts_with(prefix)
    }

    /// Where `needle` first is, from `from`.
    pub fn find(&self, needle: &[u16], from: usize) -> Option<usize> {
        let h = self.units();
        if needle.is_empty() {
            return (from <= h.len()).then_some(from);
        }
        if needle.len() > h.len() {
            return None;
        }
        (from..=h.len() - needle.len()).find(|&i| &h[i..i + needle.len()] == needle)
    }

    pub fn rfind(&self, needle: &[u16], from: usize) -> Option<usize> {
        let h = self.units();
        if needle.len() > h.len() {
            return None;
        }
        let last = (h.len() - needle.len()).min(from);
        (0..=last).rev().find(|&i| &h[i..i + needle.len()] == needle)
    }
}

impl PartialEq for JsStr {
    fn eq(&self, other: &JsStr) -> bool {
        JsStr::ptr_eq(self, other) || (self.len() == other.len() && self.units() == other.units())
    }
}

impl Eq for JsStr {}

impl PartialOrd for JsStr {
    fn partial_cmp(&self, other: &JsStr) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for JsStr {
    fn cmp(&self, other: &JsStr) -> core::cmp::Ordering {
        self.units().cmp(other.units())
    }
}

impl Hash for JsStr {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.units().hash(h)
    }
}

impl fmt::Debug for JsStr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.to_rust())
    }
}

impl fmt::Display for JsStr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_rust())
    }
}

impl From<&str> for JsStr {
    fn from(s: &str) -> JsStr {
        JsStr::from_str(s)
    }
}

/// FNV-1a over code units: the hash of the engine's own tables.
pub fn hash_units(u: &[u16]) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for &c in u {
        h ^= c as u32 & 0xFF;
        h = h.wrapping_mul(0x0100_0193);
        h ^= (c >> 8) as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// The names properties are looked up by, interned: each distinct string once, by number.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Atom(pub u32);

#[derive(Default)]
pub struct Atoms {
    strings: Vec<JsStr>,
    /// Open addressing: each slot an atom's number + 1 (0: empty), by its string's hash.
    slots: Vec<u32>,
}

impl Atoms {
    /// A table with the predefined names first (names.rs: each its constant's number).
    pub fn new() -> Atoms {
        let mut a = Atoms { strings: Vec::new(), slots: alloc::vec![0; 1024] };
        for t in crate::names::TEXTS {
            a.intern_str(t);
        }
        a
    }

    pub fn intern(&mut self, s: &JsStr) -> Atom {
        self.intern_units(s.units(), Some(s))
    }

    pub fn intern_str(&mut self, s: &str) -> Atom {
        let u: Vec<u16> = s.encode_utf16().collect();
        self.intern_units(&u, None)
    }

    pub fn intern_units(&mut self, u: &[u16], have: Option<&JsStr>) -> Atom {
        let mask = self.slots.len() - 1;
        let mut i = hash_units(u) as usize & mask;
        loop {
            let slot = self.slots[i];
            if slot == 0 {
                break;
            }
            if self.strings[slot as usize - 1].units() == u {
                return Atom(slot - 1);
            }
            i = (i + 1) & mask;
        }
        let id = self.strings.len() as u32;
        self.strings.push(have.cloned().unwrap_or_else(|| JsStr::from_slice(u)));
        self.slots[i] = id + 1;
        if self.strings.len() * 2 > self.slots.len() {
            self.grow();
        }
        Atom(id)
    }

    /// The atom for `u`, if it's interned already.
    pub fn lookup(&self, u: &[u16]) -> Option<Atom> {
        let mask = self.slots.len() - 1;
        let mut i = hash_units(u) as usize & mask;
        loop {
            let slot = self.slots[i];
            if slot == 0 {
                return None;
            }
            if self.strings[slot as usize - 1].units() == u {
                return Some(Atom(slot - 1));
            }
            i = (i + 1) & mask;
        }
    }

    fn grow(&mut self) {
        let size = self.slots.len() * 2;
        let mut slots = alloc::vec![0u32; size];
        for (id, s) in self.strings.iter().enumerate() {
            let mut i = hash_units(s.units()) as usize & (size - 1);
            while slots[i] != 0 {
                i = (i + 1) & (size - 1);
            }
            slots[i] = id as u32 + 1;
        }
        self.slots = slots;
    }

    pub fn get(&self, a: Atom) -> &JsStr {
        &self.strings[a.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.strings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

/// Whether a code unit is a JavaScript white space (not a line terminator).
pub fn is_space(c: u32) -> bool {
    matches!(c, 0x09 | 0x0B | 0x0C | 0x20 | 0xA0 | 0xFEFF | 0x1680 | 0x2000..=0x200A | 0x202F | 0x205F | 0x3000)
}

pub fn is_line_terminator(c: u32) -> bool {
    matches!(c, 0x0A | 0x0D | 0x2028 | 0x2029)
}

/// The string with JavaScript's white space and line terminators taken off its ends (String.prototype.trim).
pub fn trim(u: &[u16]) -> &[u16] {
    let ws = |c: &u16| is_space(*c as u32) || is_line_terminator(*c as u32);
    let start = u.iter().position(|c| !ws(c)).unwrap_or(u.len());
    let end = u.iter().rposition(|c| !ws(c)).map_or(start, |e| e + 1);
    &u[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ropes_flatten_once_and_deep_ones_stay_shallow() {
        let mut s = JsStr::empty();
        for i in 0..10_000 {
            s = s.concat(&JsStr::from_str(if i % 2 == 0 { "abcdefghij" } else { "0123456789" }));
        }
        assert_eq!(s.len(), 100_000);
        assert!(s.depth() <= DEEPEST);
        assert_eq!(&s.units()[..12], &"abcdefghij01".encode_utf16().collect::<Vec<_>>()[..]);
        assert_eq!(s.slice(99_990, 100_000).to_rust(), "0123456789");
    }

    #[test]
    fn atoms_are_one_each() {
        let mut a = Atoms::new();
        let x = a.intern_str("length");
        let y = a.intern(&JsStr::from_str("length"));
        assert_eq!((x, y), (crate::names::LENGTH, crate::names::LENGTH));
        let base = a.len() as u32;
        for i in 0..5000 {
            a.intern_str(&alloc::format!("p{i}"));
        }
        assert_eq!(a.lookup(&"p4999".encode_utf16().collect::<Vec<_>>()), Some(Atom(base + 4999)));
        assert_eq!(a.get(x).to_rust(), "length");
    }
}
