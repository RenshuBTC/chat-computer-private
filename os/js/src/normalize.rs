//! Unicode normalization (String.prototype.normalize, and collation's canonical equivalence) for the letters with
//! accents that Latin, Greek and Cyrillic writing use: each such letter is its base letter and one combining mark
//! (é is e and ◌́), composed (NFC) or decomposed (NFD); the compatibility forms (NFKC, NFKD) also turn full-width
//! letters and digits, ligatures, superscripts and the like into what they're compatible with. Letters with two marks
//! (Vietnamese's ế) and Hangul are left as they are.

use alloc::string::String;
use alloc::vec::Vec;

/// Letters with an accent (lower case) and their base letter and combining mark.
const PAIRS: &[(char, char, char)] = &[
    ('à', 'a', '\u{300}'), ('è', 'e', '\u{300}'), ('ì', 'i', '\u{300}'), ('ò', 'o', '\u{300}'), ('ù', 'u', '\u{300}'),
    ('ǹ', 'n', '\u{300}'), ('ẁ', 'w', '\u{300}'), ('ỳ', 'y', '\u{300}'),
    ('á', 'a', '\u{301}'), ('é', 'e', '\u{301}'), ('í', 'i', '\u{301}'), ('ó', 'o', '\u{301}'), ('ú', 'u', '\u{301}'),
    ('ý', 'y', '\u{301}'), ('ć', 'c', '\u{301}'), ('ń', 'n', '\u{301}'), ('ś', 's', '\u{301}'), ('ź', 'z', '\u{301}'),
    ('ĺ', 'l', '\u{301}'), ('ŕ', 'r', '\u{301}'), ('ǵ', 'g', '\u{301}'), ('ẃ', 'w', '\u{301}'), ('ḱ', 'k', '\u{301}'),
    ('ḿ', 'm', '\u{301}'), ('ṕ', 'p', '\u{301}'),
    ('â', 'a', '\u{302}'), ('ê', 'e', '\u{302}'), ('î', 'i', '\u{302}'), ('ô', 'o', '\u{302}'), ('û', 'u', '\u{302}'),
    ('ĉ', 'c', '\u{302}'), ('ĝ', 'g', '\u{302}'), ('ĥ', 'h', '\u{302}'), ('ĵ', 'j', '\u{302}'), ('ŝ', 's', '\u{302}'),
    ('ŵ', 'w', '\u{302}'), ('ŷ', 'y', '\u{302}'), ('ẑ', 'z', '\u{302}'),
    ('ã', 'a', '\u{303}'), ('ñ', 'n', '\u{303}'), ('õ', 'o', '\u{303}'), ('ĩ', 'i', '\u{303}'), ('ũ', 'u', '\u{303}'),
    ('ẽ', 'e', '\u{303}'), ('ỹ', 'y', '\u{303}'), ('ṽ', 'v', '\u{303}'),
    ('ā', 'a', '\u{304}'), ('ē', 'e', '\u{304}'), ('ī', 'i', '\u{304}'), ('ō', 'o', '\u{304}'), ('ū', 'u', '\u{304}'),
    ('ȳ', 'y', '\u{304}'), ('ḡ', 'g', '\u{304}'),
    ('ă', 'a', '\u{306}'), ('ĕ', 'e', '\u{306}'), ('ĭ', 'i', '\u{306}'), ('ŏ', 'o', '\u{306}'), ('ŭ', 'u', '\u{306}'),
    ('ğ', 'g', '\u{306}'), ('й', 'и', '\u{306}'), ('ў', 'у', '\u{306}'),
    ('ċ', 'c', '\u{307}'), ('ė', 'e', '\u{307}'), ('ġ', 'g', '\u{307}'), ('ż', 'z', '\u{307}'), ('ȧ', 'a', '\u{307}'),
    ('ȯ', 'o', '\u{307}'), ('ḃ', 'b', '\u{307}'), ('ḋ', 'd', '\u{307}'), ('ḟ', 'f', '\u{307}'), ('ṁ', 'm', '\u{307}'),
    ('ṅ', 'n', '\u{307}'), ('ṗ', 'p', '\u{307}'), ('ṙ', 'r', '\u{307}'), ('ṡ', 's', '\u{307}'), ('ṫ', 't', '\u{307}'),
    ('ẋ', 'x', '\u{307}'), ('ẏ', 'y', '\u{307}'),
    ('ä', 'a', '\u{308}'), ('ë', 'e', '\u{308}'), ('ï', 'i', '\u{308}'), ('ö', 'o', '\u{308}'), ('ü', 'u', '\u{308}'),
    ('ÿ', 'y', '\u{308}'), ('ẅ', 'w', '\u{308}'), ('ẍ', 'x', '\u{308}'), ('ḧ', 'h', '\u{308}'), ('ẗ', 't', '\u{308}'),
    ('ё', 'е', '\u{308}'), ('ї', 'і', '\u{308}'), ('ӧ', 'о', '\u{308}'), ('ӱ', 'у', '\u{308}'),
    ('ϊ', 'ι', '\u{308}'), ('ϋ', 'υ', '\u{308}'),
    ('å', 'a', '\u{30A}'), ('ů', 'u', '\u{30A}'), ('ẘ', 'w', '\u{30A}'), ('ẙ', 'y', '\u{30A}'),
    ('ő', 'o', '\u{30B}'), ('ű', 'u', '\u{30B}'),
    ('ǎ', 'a', '\u{30C}'), ('č', 'c', '\u{30C}'), ('ď', 'd', '\u{30C}'), ('ě', 'e', '\u{30C}'), ('ǧ', 'g', '\u{30C}'),
    ('ȟ', 'h', '\u{30C}'), ('ǐ', 'i', '\u{30C}'), ('ǰ', 'j', '\u{30C}'), ('ǩ', 'k', '\u{30C}'), ('ľ', 'l', '\u{30C}'),
    ('ň', 'n', '\u{30C}'), ('ǒ', 'o', '\u{30C}'), ('ř', 'r', '\u{30C}'), ('š', 's', '\u{30C}'), ('ť', 't', '\u{30C}'),
    ('ǔ', 'u', '\u{30C}'), ('ž', 'z', '\u{30C}'),
    ('ȁ', 'a', '\u{30F}'), ('ȅ', 'e', '\u{30F}'), ('ȉ', 'i', '\u{30F}'), ('ȍ', 'o', '\u{30F}'), ('ȑ', 'r', '\u{30F}'),
    ('ȕ', 'u', '\u{30F}'),
    ('ạ', 'a', '\u{323}'), ('ẹ', 'e', '\u{323}'), ('ị', 'i', '\u{323}'), ('ọ', 'o', '\u{323}'), ('ụ', 'u', '\u{323}'),
    ('ỵ', 'y', '\u{323}'), ('ḅ', 'b', '\u{323}'), ('ḍ', 'd', '\u{323}'), ('ḥ', 'h', '\u{323}'), ('ḳ', 'k', '\u{323}'),
    ('ḷ', 'l', '\u{323}'), ('ṃ', 'm', '\u{323}'), ('ṇ', 'n', '\u{323}'), ('ṛ', 'r', '\u{323}'), ('ṣ', 's', '\u{323}'),
    ('ṭ', 't', '\u{323}'), ('ṿ', 'v', '\u{323}'), ('ẉ', 'w', '\u{323}'), ('ẓ', 'z', '\u{323}'),
    ('ç', 'c', '\u{327}'), ('ş', 's', '\u{327}'), ('ţ', 't', '\u{327}'), ('ģ', 'g', '\u{327}'), ('ķ', 'k', '\u{327}'),
    ('ļ', 'l', '\u{327}'), ('ņ', 'n', '\u{327}'), ('ŗ', 'r', '\u{327}'), ('ȩ', 'e', '\u{327}'), ('ḑ', 'd', '\u{327}'),
    ('ḩ', 'h', '\u{327}'),
    ('ą', 'a', '\u{328}'), ('ę', 'e', '\u{328}'), ('į', 'i', '\u{328}'), ('ų', 'u', '\u{328}'), ('ǫ', 'o', '\u{328}'),
    ('ș', 's', '\u{326}'), ('ț', 't', '\u{326}'),
    ('ά', 'α', '\u{301}'), ('έ', 'ε', '\u{301}'), ('ή', 'η', '\u{301}'), ('ί', 'ι', '\u{301}'), ('ό', 'ο', '\u{301}'),
    ('ύ', 'υ', '\u{301}'), ('ώ', 'ω', '\u{301}'), ('ѓ', 'г', '\u{301}'), ('ќ', 'к', '\u{301}'),
];

/// Capitals that aren't their small letter's upper case with the same mark (İ: I and a dot above).
const CAPITAL_PAIRS: &[(char, char, char)] = &[('İ', 'I', '\u{307}')];

/// What a character decomposes into, canonically (a base letter and a mark), if anything.
pub fn decompose(c: char) -> Option<(char, char)> {
    if (c as u32) < 0xC0 {
        return None;
    }
    if let Some(&(_, b, m)) = CAPITAL_PAIRS.iter().find(|p| p.0 == c) {
        return Some((b, m));
    }
    if let Some(&(_, b, m)) = PAIRS.iter().find(|p| p.0 == c) {
        return Some((b, m));
    }
    // A capital: its small letter's base, in capitals.
    if c.is_uppercase() {
        let mut lower = c.to_lowercase();
        if let (Some(l), None) = (lower.next(), lower.next())
            && let Some(&(_, b, m)) = PAIRS.iter().find(|p| p.0 == l)
        {
            let mut up = b.to_uppercase();
            if let (Some(u), None) = (up.next(), up.next()) {
                return Some((u, m));
            }
        }
    }
    None
}

/// The letter a base and a mark compose into, if there's one.
pub fn compose(base: char, mark: char) -> Option<char> {
    if let Some(&(c, _, _)) = CAPITAL_PAIRS.iter().find(|p| p.1 == base && p.2 == mark) {
        return Some(c);
    }
    if let Some(&(c, _, _)) = PAIRS.iter().find(|p| p.1 == base && p.2 == mark) {
        return Some(c);
    }
    if base.is_uppercase() {
        let mut lower = base.to_lowercase();
        if let (Some(l), None) = (lower.next(), lower.next())
            && let Some(&(c, _, _)) = PAIRS.iter().find(|p| p.1 == l && p.2 == mark)
        {
            let mut up = c.to_uppercase();
            if let (Some(u), None) = (up.next(), up.next())
                && decompose(u) == Some((base, mark))
            {
                return Some(u);
            }
        }
    }
    None
}

/// A combining mark's class (0: none), for putting marks in their canonical order.
fn mark_class(c: char) -> u8 {
    match c as u32 {
        0x0300..=0x0314 | 0x033D..=0x0344 | 0x0346 | 0x034A..=0x034C | 0x0350..=0x0352 | 0x0357 | 0x035B | 0x0363..=0x036F => 230,
        0x0315 | 0x031A | 0x0358 => 232,
        0x0316..=0x0319 | 0x031C..=0x0320 | 0x0323..=0x0326 | 0x0329..=0x0333 | 0x0339..=0x033C | 0x0347..=0x0349 | 0x034D..=0x034E | 0x0353..=0x0356 | 0x0359..=0x035A => 220,
        0x031B => 216,
        0x0321..=0x0322 | 0x0327..=0x0328 => 202,
        0x0334..=0x0338 => 1,
        0x0345 => 240,
        _ => 0,
    }
}

/// What a compatibility form also changes a character into.
fn compatible(c: char, out: &mut String) -> bool {
    let s: &str = match c as u32 {
        0xFF01..=0xFF5E => {
            out.push(char::from_u32(c as u32 - 0xFEE0).unwrap_or(c));
            return true;
        }
        0x3000 | 0xA0 | 0x2000..=0x200A | 0x202F | 0x205F => " ",
        0xFB00 => "ff",
        0xFB01 => "fi",
        0xFB02 => "fl",
        0xFB03 => "ffi",
        0xFB04 => "ffl",
        0xFB05 | 0xFB06 => "st",
        0x0132 => "IJ",
        0x0133 => "ij",
        0x01C4 => "DŽ",
        0x01C6 => "dž",
        0x01C7 => "LJ",
        0x01C9 => "lj",
        0x01CA => "NJ",
        0x01CC => "nj",
        0x00B9 | 0x2081 => "1",
        0x00B2 | 0x2082 => "2",
        0x00B3 | 0x2083 => "3",
        0x2070 | 0x2080 => "0",
        0x2074..=0x2079 => {
            out.push(char::from_u32(c as u32 - 0x2074 + '4' as u32).unwrap_or(c));
            return true;
        }
        0x2084..=0x2089 => {
            out.push(char::from_u32(c as u32 - 0x2084 + '4' as u32).unwrap_or(c));
            return true;
        }
        0x00BC => "1⁄4",
        0x00BD => "1⁄2",
        0x00BE => "3⁄4",
        0x2122 => "TM",
        0x2026 => "...",
        0x2025 => "..",
        0x2024 => ".",
        0x00B5 => "μ",
        0x017F => "s",
        0x2160..=0x216B => ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X", "XI", "XII"][(c as u32 - 0x2160) as usize],
        0x2460..=0x2468 => {
            out.push(char::from_u32(c as u32 - 0x2460 + '1' as u32).unwrap_or(c));
            return true;
        }
        0x00A8 => " \u{308}",
        0x00B4 => " \u{301}",
        0x00AA => "a",
        0x00BA => "o",
        _ => return false,
    };
    out.push_str(s);
    true
}

/// A string decomposed (NFD; NFKD if `compat`): letters with accents as their base and marks, marks in order.
pub fn nfd(s: &str, compat: bool) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    let push = |c: char, out: &mut String| {
        if compat {
            let mut tmp = String::new();
            if compatible(c, &mut tmp) {
                for t in tmp.chars() {
                    decompose_into(t, out);
                }
                return;
            }
        }
        decompose_into(c, out);
    };
    for c in s.chars() {
        push(c, &mut out);
    }
    order_marks(out)
}

/// Hangul's syllables: their leading consonant, vowel and trailing consonant, by Unicode's arithmetic.
const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const S_COUNT: u32 = 11172;

fn decompose_into(c: char, out: &mut String) {
    let cp = c as u32;
    if (S_BASE..S_BASE + S_COUNT).contains(&cp) {
        let s = cp - S_BASE;
        out.push(char::from_u32(L_BASE + s / 588).unwrap_or(c));
        out.push(char::from_u32(V_BASE + (s % 588) / 28).unwrap_or(c));
        if s % 28 != 0 {
            out.push(char::from_u32(T_BASE + s % 28).unwrap_or(c));
        }
        return;
    }
    match decompose(c) {
        Some((b, m)) => {
            decompose_into(b, out);
            out.push(m);
        }
        None => out.push(c),
    }
}

/// Marks after a letter in their canonical order (by class; stable within one).
fn order_marks(s: String) -> String {
    let chars: Vec<char> = s.chars().collect();
    if !chars.iter().any(|c| mark_class(*c) > 0) {
        return s;
    }
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if mark_class(chars[i]) == 0 {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && mark_class(chars[i]) > 0 {
            i += 1;
        }
        let mut run: Vec<char> = chars[start..i].to_vec();
        run.sort_by_key(|c| mark_class(*c));
        out.extend(run);
    }
    out.into_iter().collect()
}

/// A string composed (NFC; NFKC if `compat`): a letter and a mark that have a letter of their own become it.
pub fn nfc(s: &str, compat: bool) -> String {
    let d = nfd(s, compat);
    let chars: Vec<char> = d.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    // Where the last starter is, and the class of the last mark after it that wasn't composed.
    let mut starter: Option<usize> = None;
    let mut last_class: u8 = 0;
    for &c in &chars {
        let cp = c as u32;
        // Hangul: a leading consonant and a vowel make a syllable; a syllable and a trailing consonant, another.
        if let Some(&last) = out.last() {
            let l = last as u32;
            if (L_BASE..L_BASE + 19).contains(&l) && (V_BASE..V_BASE + 21).contains(&cp) {
                let n = out.len();
                out[n - 1] = char::from_u32(S_BASE + ((l - L_BASE) * 21 + (cp - V_BASE)) * 28).unwrap_or(last);
                continue;
            }
            if (S_BASE..S_BASE + S_COUNT).contains(&l) && (l - S_BASE) % 28 == 0 && (T_BASE + 1..T_BASE + 28).contains(&cp) {
                let n = out.len();
                out[n - 1] = char::from_u32(l + (cp - T_BASE)).unwrap_or(last);
                continue;
            }
        }
        let class = mark_class(c);
        if let Some(st) = starter
            && class > 0
            && (last_class == 0 || last_class < class)
            && let Some(composed) = compose(out[st], c)
        {
            out[st] = composed;
            continue;
        }
        if class == 0 {
            starter = Some(out.len());
            last_class = 0;
        } else {
            last_class = class;
        }
        out.push(c);
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_with_accents_decompose_and_compose() {
        assert_eq!(nfd("Café Ångström", false), "Cafe\u{301} A\u{30A}ngstro\u{308}m");
        assert_eq!(nfc("Cafe\u{301} A\u{30A}ngstro\u{308}m", false), "Café Ångström");
        assert_eq!(nfc("é", false), "é");
        assert_eq!(nfd("İstanbul", false), "I\u{307}stanbul");
        assert_eq!(nfd("Ǎ", false), "A\u{30C}");
        assert_eq!(nfc("a\u{323}\u{302}", false).chars().count(), 2);
        assert_eq!(nfkc("ﬁ１２", true), "fi12");
        assert_eq!(nfd("ş", false), "s\u{327}");
        assert_eq!(nfd("\u{d4db}", false), "\u{1111}\u{1171}\u{11b6}");
        assert_eq!(nfc("\u{1111}\u{1171}\u{11b6}", false), "\u{d4db}");
    }

    fn nfkc(s: &str, compat: bool) -> String {
        nfc(s, compat)
    }
}
