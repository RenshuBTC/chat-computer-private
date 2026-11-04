//! Words into the model's tokens and back: Qwen's byte-level BPE, as its tokenizer.json describes it. The text is cut
//! into pieces by the model's own splitting rule (written out here as code: no regular expressions), each piece's
//! UTF-8 bytes start as one token each, and the pair of neighbours that comes first in the merge list is merged, again
//! and again, until no pair is in it.
//!
//! The model's special tokens (<|im_start|>, <tool_call>, <think>...) are only ever made from the chat's own markup
//! (`encode_marked`): words from a person, a web page or a file never become one (`encode`), so nothing they say can
//! pretend to be a new turn of the conversation.

use crate::unicode::{Class, class};
use alloc::vec;
use alloc::vec::Vec;

/// The tokenizer: each token's bytes, the merges (in order), the token of each single byte, and the special tokens.
pub struct Tokenizer {
    /// Token i's bytes are blob[offsets[i]..offsets[i + 1]].
    offsets: Vec<u32>,
    blob: Vec<u8>,
    merges: PairMap,
    byte_token: [u32; 256],
    /// Special tokens: their text and id, longest first.
    specials: Vec<(Vec<u8>, u32)>,
}

impl Tokenizer {
    /// From its parts: every token's bytes, the merges in order (left, right, the token they make), and the special
    /// tokens (text, id).
    pub fn new(tokens: Vec<Vec<u8>>, merges: &[(u32, u32, u32)], specials: Vec<(Vec<u8>, u32)>) -> Tokenizer {
        let mut offsets = Vec::with_capacity(tokens.len() + 1);
        let mut blob = Vec::new();
        offsets.push(0);
        let mut byte_token = [u32::MAX; 256];
        for (i, t) in tokens.iter().enumerate() {
            blob.extend_from_slice(t);
            offsets.push(blob.len() as u32);
            if t.len() == 1 && !specials.iter().any(|(_, id)| *id == i as u32) && byte_token[t[0] as usize] == u32::MAX {
                byte_token[t[0] as usize] = i as u32;
            }
        }
        let mut map = PairMap::new(merges.len());
        for (rank, &(l, r, m)) in merges.iter().enumerate() {
            map.insert(l, r, rank as u32, m);
        }
        let mut specials = specials;
        specials.sort_by_key(|s| core::cmp::Reverse(s.0.len()));
        Tokenizer { offsets, blob, merges: map, byte_token, specials }
    }

    /// How many tokens there are.
    pub fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Token `id`'s bytes (empty for an id past the end).
    pub fn bytes(&self, id: u32) -> &[u8] {
        let i = id as usize;
        if i + 1 >= self.offsets.len() {
            return &[];
        }
        &self.blob[self.offsets[i] as usize..self.offsets[i + 1] as usize]
    }

    /// A special token's id, by its text ("<|im_end|>").
    pub fn special(&self, text: &str) -> Option<u32> {
        self.specials.iter().find(|(t, _)| t == text.as_bytes()).map(|&(_, id)| id)
    }

    /// Whether `id` is a special token.
    pub fn is_special(&self, id: u32) -> bool {
        self.specials.iter().any(|&(_, s)| s == id)
    }

    /// Words, as tokens: any special token's text in them is only words.
    pub fn encode(&self, text: &str, out: &mut Vec<u32>) {
        // (In Unicode's composed form first, as the models' tokenizer reads it.)
        let text = crate::nfc::nfc(text);
        let text = text.as_ref();
        let mut pieces = Vec::new();
        split(text, &mut pieces);
        let mut work = Work::default();
        for p in pieces {
            self.bpe(&text.as_bytes()[p.0..p.1], out, &mut work);
        }
    }

    /// The chat's own markup and words: special tokens' text becomes them.
    pub fn encode_marked(&self, text: &str, out: &mut Vec<u32>) {
        let b = text.as_bytes();
        let mut start = 0;
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'<'
                && let Some((t, id)) = self.specials.iter().find(|(t, _)| b[i..].starts_with(t))
            {
                self.encode(&text[start..i], out);
                out.push(*id);
                i += t.len();
                start = i;
                continue;
            }
            i += 1;
        }
        self.encode(&text[start..], out);
    }

    /// One piece of text: its bytes as tokens, merged as far as the merges go.
    fn bpe(&self, bytes: &[u8], out: &mut Vec<u32>, w: &mut Work) {
        if bytes.is_empty() {
            return;
        }
        if bytes.len() == 1 {
            out.push(self.byte_token[bytes[0] as usize]);
            return;
        }
        // The symbols, as a linked list (prev, next, token; u32::MAX token: merged away), and the pairs that can
        // merge in a heap by rank (stale ones are skipped when they come up).
        let n = bytes.len();
        w.tok.clear();
        w.prev.clear();
        w.next.clear();
        w.heap.clear();
        for (i, &b) in bytes.iter().enumerate() {
            w.tok.push(self.byte_token[b as usize]);
            w.prev.push(if i == 0 { u32::MAX } else { i as u32 - 1 });
            w.next.push(if i + 1 == n { u32::MAX } else { i as u32 + 1 });
        }
        for i in 0..n - 1 {
            if let Some((rank, _)) = self.merges.get(w.tok[i], w.tok[i + 1]) {
                heap_push(&mut w.heap, (rank, i as u32));
            }
        }
        while let Some((rank, i)) = heap_pop(&mut w.heap) {
            let i = i as usize;
            let t = w.tok[i];
            let j = w.next[i];
            if t == u32::MAX || j == u32::MAX {
                continue;
            }
            let j = j as usize;
            // Still this pair, at this rank?
            let Some((r, merged)) = self.merges.get(t, w.tok[j]) else { continue };
            if r != rank {
                continue;
            }
            w.tok[i] = merged;
            w.tok[j] = u32::MAX;
            let after = w.next[j];
            w.next[i] = after;
            if after != u32::MAX {
                w.prev[after as usize] = i as u32;
                if let Some((r2, _)) = self.merges.get(merged, w.tok[after as usize]) {
                    heap_push(&mut w.heap, (r2, i as u32));
                }
            }
            let before = w.prev[i];
            if before != u32::MAX
                && let Some((r2, _)) = self.merges.get(w.tok[before as usize], merged)
            {
                heap_push(&mut w.heap, (r2, before));
            }
        }
        let mut i = 0usize;
        loop {
            out.push(w.tok[i]);
            let j = w.next[i];
            if j == u32::MAX {
                break;
            }
            i = j as usize;
        }
    }
}

#[derive(Default)]
struct Work {
    tok: Vec<u32>,
    prev: Vec<u32>,
    next: Vec<u32>,
    heap: Vec<(u32, u32)>,
}

fn heap_push(h: &mut Vec<(u32, u32)>, v: (u32, u32)) {
    h.push(v);
    let mut i = h.len() - 1;
    while i > 0 {
        let p = (i - 1) / 2;
        if h[p] <= h[i] {
            break;
        }
        h.swap(p, i);
        i = p;
    }
}

fn heap_pop(h: &mut Vec<(u32, u32)>) -> Option<(u32, u32)> {
    if h.is_empty() {
        return None;
    }
    let top = h.swap_remove(0);
    let n = h.len();
    let mut i = 0;
    loop {
        let (l, r) = (2 * i + 1, 2 * i + 2);
        let mut m = i;
        if l < n && h[l] < h[m] {
            m = l;
        }
        if r < n && h[r] < h[m] {
            m = r;
        }
        if m == i {
            break;
        }
        h.swap(i, m);
        i = m;
    }
    Some(top)
}

/// Merges by pair: an open-addressing table of (left, right) to (rank, merged).
struct PairMap {
    keys: Vec<u64>,
    values: Vec<(u32, u32)>,
    mask: usize,
}

const EMPTY: u64 = u64::MAX;

impl PairMap {
    fn new(count: usize) -> PairMap {
        let size = (count * 2).next_power_of_two().max(16);
        PairMap { keys: vec![EMPTY; size], values: vec![(0, 0); size], mask: size - 1 }
    }

    fn slot(&self, key: u64) -> usize {
        // (A multiplicative hash: the pairs' ids are small and close together.)
        (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 20) as usize & self.mask
    }

    fn insert(&mut self, l: u32, r: u32, rank: u32, merged: u32) {
        let key = (l as u64) << 32 | r as u64;
        let mut i = self.slot(key);
        loop {
            if self.keys[i] == EMPTY {
                self.keys[i] = key;
                self.values[i] = (rank, merged);
                return;
            }
            if self.keys[i] == key {
                return; // (the first of a pair listed twice is the one that counts)
            }
            i = (i + 1) & self.mask;
        }
    }

    fn get(&self, l: u32, r: u32) -> Option<(u32, u32)> {
        if l == u32::MAX || r == u32::MAX {
            return None;
        }
        let key = (l as u64) << 32 | r as u64;
        let mut i = self.slot(key);
        loop {
            let k = self.keys[i];
            if k == key {
                return Some(self.values[i]);
            }
            if k == EMPTY {
                return None;
            }
            i = (i + 1) & self.mask;
        }
    }
}

/// The model's splitting rule, as code: the pieces of `text` (byte ranges), as its pattern finds them, each
/// alternative tried in order at each place:
///
///   (?i:'s|'t|'re|'ve|'m|'ll|'d) | [^\r\n\p{L}\p{N}]?[\p{L}\p{M}]+ | \p{N} | ?[^\s\p{L}\p{M}\p{N}]+[\r\n]* |
///   \s*[\r\n]+ | \s+(?!\S) | \s+
pub fn split(text: &str, out: &mut Vec<(usize, usize)>) {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();
    let at = |i: usize| if i < n { chars[i].0 } else { text.len() };
    let cls = |i: usize| class(chars[i].1);
    let is_nl = |c: char| c == '\r' || c == '\n';
    let letter_or_mark = |k: Class| k == Class::Letter || k == Class::Mark;
    let mut i = 0;
    while i < n {
        let c = chars[i].1;
        // 's 't 're 've 'm 'll 'd, in any case.
        if c == '\'' && i + 1 < n {
            let low = |k: usize| if k < n { chars[k].1.to_ascii_lowercase() } else { '\0' };
            let len = match (low(i + 1), low(i + 2)) {
                ('s' | 't' | 'm' | 'd', _) => 2,
                ('r', 'e') | ('v', 'e') | ('l', 'l') => 3,
                _ => 0,
            };
            if len > 0 {
                out.push((at(i), at(i + len)));
                i += len;
                continue;
            }
        }
        // [^\r\n\p{L}\p{N}]? then letters and marks.
        {
            let k = cls(i);
            let mut j = i;
            if !is_nl(c) && k != Class::Letter && k != Class::Number && i + 1 < n && letter_or_mark(cls(i + 1)) {
                j = i + 1;
            }
            if letter_or_mark(cls(j)) {
                let mut e = j;
                while e < n && letter_or_mark(cls(e)) {
                    e += 1;
                }
                out.push((at(i), at(e)));
                i = e;
                continue;
            }
        }
        // One number.
        if cls(i) == Class::Number {
            out.push((at(i), at(i + 1)));
            i += 1;
            continue;
        }
        // An optional space, then anything but white space, letters, marks and numbers; then line breaks.
        {
            let other = |k: usize| k < n && cls(k) == Class::Other;
            let j = if c == ' ' && other(i + 1) { i + 1 } else { i };
            if other(j) {
                let mut e = j;
                while other(e) {
                    e += 1;
                }
                while e < n && is_nl(chars[e].1) {
                    e += 1;
                }
                out.push((at(i), at(e)));
                i = e;
                continue;
            }
        }
        // White space: up to its last line break, if it has one; else all of it but the last character before
        // something that isn't white space (which goes with it); else all of it.
        if cls(i) == Class::Space {
            let mut e = i;
            while e < n && cls(e) == Class::Space {
                e += 1;
            }
            if let Some(last_nl) = (i..e).rev().find(|&k| is_nl(chars[k].1)) {
                out.push((at(i), at(last_nl + 1)));
                i = last_nl + 1;
                continue;
            }
            let end = if e < n && e - i >= 2 { e - 1 } else { e };
            out.push((at(i), at(end)));
            i = end;
            continue;
        }
        // (Nothing else is left: every character is one of those classes.)
        out.push((at(i), at(i + 1)));
        i += 1;
    }
}

/// The bytes of the byte-level alphabet's characters: tokenizer.json writes each byte as a printable character
/// (the GPT-2 table: bytes that print are themselves, the rest from U+0100 on).
pub fn byte_level_table() -> [char; 256] {
    let mut table = ['\0'; 256];
    let mut extra = 0u32;
    for b in 0..256u32 {
        let prints = (0x21..=0x7E).contains(&b) || (0xA1..=0xAC).contains(&b) || (0xAE..=0xFF).contains(&b);
        table[b as usize] = if prints {
            char::from_u32(b).unwrap()
        } else {
            let c = char::from_u32(256 + extra).unwrap();
            extra += 1;
            c
        };
    }
    table
}
