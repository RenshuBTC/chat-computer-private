//! NFC, Unicode's composed form, which the models' tokenizer puts text in before reading it: an "é" written as
//! "e" and an accent (as Macs name files) reads the same as the one character. (Unicode Standard Annex #15:
//! decompose, put marks in their canonical order, compose again.)

use crate::nfc_table::{CLASSES, COMPOSE, DECOMPOSE};
use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

// Hangul syllables are made of their letters by arithmetic (the Unicode Standard, section 3.12).
const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = L_COUNT * N_COUNT;

pub fn nfc(s: &str) -> Cow<'_, str> {
    // Below U+0300 (ASCII and the Latin letters with accents) everything is already composed: in UTF-8 those
    // characters' bytes are all below 0xCC.
    if s.bytes().all(|b| b < 0xCC) {
        return Cow::Borrowed(s);
    }
    let mut d: Vec<(u32, u8)> = Vec::with_capacity(s.len());
    for c in s.chars() {
        decompose(c as u32, &mut d);
    }
    // Each run of marks in the order of their classes (a stable sort: equal classes keep their order).
    let mut i = 0;
    while i < d.len() {
        if d[i].1 == 0 {
            i += 1;
            continue;
        }
        let start = i;
        while i < d.len() && d[i].1 != 0 {
            i += 1;
        }
        let run = &mut d[start..i];
        for j in 1..run.len() {
            let mut k = j;
            while k > 0 && run[k - 1].1 > run[k].1 {
                run.swap(k - 1, k);
                k -= 1;
            }
        }
    }
    let mut out: Vec<u32> = Vec::with_capacity(d.len());
    let mut starter: Option<usize> = None;
    let mut last = 0u8;
    for &(c, class) in &d {
        if let Some(at) = starter {
            // A character joins the last starter unless something between them has a class of its own as high.
            let blocked = out.len() > at + 1 && last >= class;
            if !blocked && let Some(x) = compose(out[at], c) {
                out[at] = x;
                continue;
            }
        }
        if class == 0 {
            starter = Some(out.len());
        }
        last = class;
        out.push(c);
    }
    let composed: String = out.iter().map(|&c| char::from_u32(c).unwrap_or('\u{FFFD}')).collect();
    if composed == s { Cow::Borrowed(s) } else { Cow::Owned(composed) }
}

fn class(c: u32) -> u8 {
    match CLASSES.binary_search_by(|&(a, b, _)| {
        if c < a {
            core::cmp::Ordering::Greater
        } else if c > b {
            core::cmp::Ordering::Less
        } else {
            core::cmp::Ordering::Equal
        }
    }) {
        Ok(i) => CLASSES[i].2,
        Err(_) => 0,
    }
}

fn decompose(c: u32, out: &mut Vec<(u32, u8)>) {
    if (S_BASE..S_BASE + S_COUNT).contains(&c) {
        let s = c - S_BASE;
        out.push((L_BASE + s / N_COUNT, 0));
        out.push((V_BASE + s % N_COUNT / T_COUNT, 0));
        if !s.is_multiple_of(T_COUNT) {
            out.push((T_BASE + s % T_COUNT, 0));
        }
        return;
    }
    match DECOMPOSE.binary_search_by_key(&c, |e| e.0) {
        Ok(i) => {
            let (_, a, b) = DECOMPOSE[i];
            decompose(a, out);
            if b != 0 {
                decompose(b, out);
            }
        }
        Err(_) => out.push((c, class(c))),
    }
}

fn compose(a: u32, b: u32) -> Option<u32> {
    if (L_BASE..L_BASE + L_COUNT).contains(&a) && (V_BASE..V_BASE + V_COUNT).contains(&b) {
        return Some(S_BASE + ((a - L_BASE) * V_COUNT + (b - V_BASE)) * T_COUNT);
    }
    if (S_BASE..S_BASE + S_COUNT).contains(&a) && (a - S_BASE).is_multiple_of(T_COUNT) && (T_BASE + 1..T_BASE + T_COUNT).contains(&b) {
        return Some(a + (b - T_BASE));
    }
    COMPOSE.binary_search_by(|e| (e.0, e.1).cmp(&(a, b))).ok().map(|i| COMPOSE[i].2)
}

#[cfg(test)]
mod tests {
    use super::nfc;

    #[test]
    fn composes_as_unicode_says() {
        assert_eq!(nfc("Cafe\u{301}"), "Café");
        assert_eq!(nfc("plain ascii"), "plain ascii");
        // Marks in canonical order: dot below (220) before the acute (230), then composed as far as they go.
        assert_eq!(nfc("a\u{301}\u{323}"), "\u{1EA1}\u{301}");
        // Hangul: letters into a syllable.
        assert_eq!(nfc("\u{1100}\u{1161}\u{11A8}"), "\u{AC01}");
        // A singleton (the Kelvin sign) becomes its letter, and an excluded composite stays apart.
        assert_eq!(nfc("\u{212A}"), "K");
        assert_eq!(nfc("\u{0958}"), "\u{0915}\u{093C}");
    }
}
