//! Where a line of text may break: the Unicode line breaking algorithm (UAX #14, for Unicode 15.1), with CSS's
//! tailorings of it (line-break: strict, normal, loose, anywhere; word-break: break-all, keep-all), as Chrome has
//! them through ICU.

use alloc::vec::Vec;

use crate::breaks_data::{CLASSES, STARTS};

/// A character's line breaking class (UAX #14), with the distinctions the rules need: OP and CP that are East Asian
/// wide (OPW, CPW: not held to letters by LB30), QU that are initial or final punctuation (QUPI, QUPF: LB15a,
/// LB15b), SA that are combining marks or not (SACM, SAAL).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Class {
    XX, BK, CR, LF, NL, SP, ZW, WJ, GL, CM, ZWJ, OP, OPW, CL, CP, CPW, QU, QUPI, QUPF, BA, BB, HY, B2, IS, NS, EX, SY,
    AL, HL, NU, PR, PO, ID, EB, EM, IN, H2, H3, JL, JV, JT, RI, CB, AI, CJ, SACM, SAAL, SG, AK, AP, AS, VF, VI,
}

use Class::*;

const ALL: [Class; 53] = [
    XX, BK, CR, LF, NL, SP, ZW, WJ, GL, CM, ZWJ, OP, OPW, CL, CP, CPW, QU, QUPI, QUPF, BA, BB, HY, B2, IS, NS, EX, SY,
    AL, HL, NU, PR, PO, ID, EB, EM, IN, H2, H3, JL, JV, JT, RI, CB, AI, CJ, SACM, SAAL, SG, AK, AP, AS, VF, VI,
];

/// A character's class, as Unicode gives it.
pub fn class(c: char) -> Class {
    let c = c as u32;
    let i = match STARTS.binary_search(&c) {
        Ok(i) => i,
        Err(i) => i - 1,
    };
    ALL[CLASSES[i] as usize]
}

/// CSS's line-break.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Strictness {
    #[default]
    Normal,
    Strict,
    Loose,
    /// Between any two characters (not inside one made of several: a letter and its accents).
    Anywhere,
}

/// How a text's lines may break: its line-break and word-break.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rules {
    pub strictness: Strictness,
    /// word-break: break-all: letters and digits break as ideographs do.
    pub break_all: bool,
    /// word-break: keep-all: ideographs (and Korean) don't break between themselves, as letters don't.
    pub keep_all: bool,
    /// The text's language is Chinese or Japanese: a wave dash or a double hyphen may start a line (unless
    /// line-break is strict).
    pub cjk: bool,
    /// word-break: manual: no breaks inside the words of scripts written without spaces (Thai...) but where the
    /// text says (a zero width space).
    pub manual: bool,
}

/// Whether a line may break at a place.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Break {
    No,
    May,
    /// It must: after a line break (a line feed, a line separator...).
    Must,
}

fn op(k: Class) -> bool {
    matches!(k, OP | OPW)
}

fn cp(k: Class) -> bool {
    matches!(k, CP | CPW)
}

fn qu(k: Class) -> bool {
    matches!(k, QU | QUPI | QUPF)
}

/// Characters CSS's line-break: loose lets lines break before, whatever the language: inseparable characters
/// (ellipses) and iteration marks.
fn loose(c: char) -> bool {
    matches!(c, '\u{2024}' | '\u{2025}' | '\u{2026}' | '\u{22EF}' | '\u{FE19}' | '\u{3005}' | '\u{303B}' | '\u{309D}' | '\u{309E}' | '\u{30FD}' | '\u{30FE}')
}

/// And those it lets lines break before (or, signs before numbers, after) in Chinese and Japanese: centred
/// punctuation, wave dashes and double hyphens, and postfix and prefix signs.
fn loose_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{30FB}' | '\u{FF1A}' | '\u{FF1B}' | '\u{FF65}' | '\u{203C}' | '\u{2047}' | '\u{2048}' | '\u{2049}' | '\u{FF01}' | '\u{FF1F}' | '\u{301C}' | '\u{30A0}'
            | '\u{00B0}' | '\u{2030}' | '\u{2032}' | '\u{2033}' | '\u{2035}' | '\u{2103}' | '\u{2109}' | '\u{FE6A}' | '\u{FF05}' | '\u{FFE0}' | '\u{00B1}'
            | '\u{20AC}' | '\u{2116}' | '\u{FE69}' | '\u{FF04}' | '\u{FFE1}' | '\u{FFE5}' | '\u{FFE6}'
    )
}

/// A character's class with LB1 and CSS's tailorings applied.
fn resolve(c: char, rules: Rules) -> Class {
    let k = match class(c) {
        AI | SG | XX => AL,
        // (Thai, Lao, Khmer, Burmese: letters, but see `breaks`.)
        SACM => CM,
        SAAL => AL,
        CJ => {
            if rules.strictness == Strictness::Strict {
                NS
            } else {
                ID
            }
        }
        k => k,
    };
    let k = if rules.strictness == Strictness::Loose && (loose(c) || (rules.cjk && loose_cjk(c))) { ID } else { k };
    // (A wave dash and a double hyphen start lines in Chinese and Japanese, but for line-break: strict.)
    let k = if rules.cjk && rules.strictness != Strictness::Strict && matches!(c, '\u{301C}' | '\u{30A0}') { ID } else { k };
    if rules.break_all && matches!(k, AL | HL | NU) {
        return ID;
    }
    if rules.keep_all && matches!(k, ID | H2 | H3 | JL | JV | JT) {
        return AL;
    }
    k
}

/// Where lines may break in `text`: for each i in 0..=len, at the place before `text[i]` (0, its start: never; len,
/// its end: Must, as the end of a text is).
pub fn breaks(text: &[char], rules: Rules) -> Vec<Break> {
    let n = text.len();
    let mut out = alloc::vec![Break::No; n + 1];
    if n == 0 {
        return out;
    }
    out[n] = Break::Must;
    let raw: Vec<Class> = text.iter().map(|&c| resolve(c, rules)).collect();
    // LB9, LB10: a combining mark (or a zero width joiner) is of the class of what it's on (none: a letter).
    let mut cls = raw.clone();
    let mut attached = alloc::vec![false; n];
    for i in 0..n {
        if matches!(raw[i], CM | ZWJ) {
            if i > 0 && !matches!(cls[i - 1], BK | CR | LF | NL | SP | ZW) {
                cls[i] = cls[i - 1];
                attached[i] = true;
            } else {
                cls[i] = AL;
            }
        }
    }
    // (The last character before a place that isn't a space: `cls`'s.)
    let before_spaces = |i: usize| -> Option<usize> {
        let mut j = i;
        while j > 0 {
            j -= 1;
            if cls[j] != SP {
                return Some(j);
            }
        }
        None
    };
    // (Whether each character is (on) a letter of Thai, Lao, Khmer or Burmese.)
    let mut sa = alloc::vec![false; n];
    for i in 0..n {
        sa[i] = class(text[i]) == SAAL || (attached[i] && i > 0 && sa[i - 1]);
    }
    for i in 1..n {
        out[i] = decide(&raw, &cls, &attached, i, rules, &before_spaces, text);
        // (Between two letters of Thai, Lao, Khmer or Burmese, where Chrome looks the words up in a dictionary:
        // without one, between any two of them (and their marks), as CSS has it.)
        if !rules.manual && out[i] == Break::No && !attached[i] && sa[i - 1] && class(text[i]) == SAAL {
            out[i] = Break::May;
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn decide(raw: &[Class], cls: &[Class], attached: &[bool], i: usize, rules: Rules, before_spaces: &dyn Fn(usize) -> Option<usize>, text: &[char]) -> Break {
    let n = cls.len();
    let (ra, rb) = (raw[i - 1], raw[i]);
    // LB4, LB5: after a line break, always.
    if ra == BK {
        return Break::Must;
    }
    if ra == CR && rb == LF {
        return Break::No;
    }
    if matches!(ra, CR | LF | NL) {
        return Break::Must;
    }
    // LB6, LB7: never before a line break, a space or a zero width space.
    if matches!(rb, BK | CR | LF | NL) {
        return Break::No;
    }
    if rules.strictness == Strictness::Anywhere {
        return if attached[i] || ra == ZWJ { Break::No } else { Break::May };
    }
    if matches!(rb, SP | ZW) {
        return Break::No;
    }
    // LB8: after a zero width space (and the spaces after it).
    {
        let mut j = i - 1;
        while j > 0 && raw[j] == SP {
            j -= 1;
        }
        if raw[j] == ZW {
            return Break::May;
        }
    }
    // LB8a, LB9: not after a zero width joiner, nor before a combining mark.
    if ra == ZWJ || attached[i] {
        return Break::No;
    }
    let (a, b) = (cls[i - 1], cls[i]);
    // LB11, LB12, LB12a: word joiners and glue.
    if a == WJ || b == WJ || a == GL {
        return Break::No;
    }
    if b == GL && !matches!(a, SP | BA | HY) {
        return Break::No;
    }
    // LB13: not before closing punctuation, "!" or "/".
    if b == CL || cp(b) || b == EX || b == SY {
        return Break::No;
    }
    let last = before_spaces(i);
    let last_cls = last.map(|j| cls[j]);
    // LB14: not after opening punctuation, spaces or not.
    if last_cls.is_some_and(op) {
        return Break::No;
    }
    // LB15a: not after an opening quotation mark (where one opens).
    if let Some(j) = last
        && cls[j] == QUPI
        && (j == 0 || matches!(cls[j - 1], BK | CR | LF | NL | OP | OPW | QU | QUPI | QUPF | GL | SP | ZW))
    {
        return Break::No;
    }
    // LB15b: not before a closing one (where one closes).
    if b == QUPF && (i + 1 == n || matches!(cls[i + 1], SP | GL | WJ | CL | QU | QUPI | QUPF | CP | CPW | EX | IS | SY | BK | CR | LF | NL | ZW)) {
        return Break::No;
    }
    // LB15c, LB15d: before a "." that starts a number after a space; else not before "." and ",".
    if a == SP && b == IS && i + 1 < n && cls[i + 1] == NU {
        return Break::May;
    }
    if b == IS {
        return Break::No;
    }
    // LB16, LB17.
    if b == NS && last_cls.is_some_and(|k| k == CL || cp(k)) {
        return Break::No;
    }
    if b == B2 && last_cls == Some(B2) {
        return Break::No;
    }
    // LB18: after spaces.
    if a == SP {
        return Break::May;
    }
    // LB19: not either side of a quotation mark (as Chrome has it still: Unicode 16's LB19a, letting them break
    // between East Asian characters, not yet).
    if qu(a) || qu(b) {
        return Break::No;
    }
    // LB20.
    if a == CB || b == CB {
        return Break::May;
    }
    // LB21, LB21a, LB21b, LB22.
    if matches!(b, BA | HY | NS) || a == BB {
        return Break::No;
    }
    if matches!(a, HY | BA) && i >= 2 && cls[i - 2] == HL {
        return Break::No;
    }
    if a == SY && b == HL {
        return Break::No;
    }
    if b == IN {
        return Break::No;
    }
    // LB23, LB23a, LB24, LB25: letters, numbers and their signs.
    let letter = |k: Class| matches!(k, AL | HL);
    if (letter(a) && b == NU) || (a == NU && letter(b)) {
        return Break::No;
    }
    if (a == PR && matches!(b, ID | EB | EM)) || (matches!(a, ID | EB | EM) && b == PO) {
        return Break::No;
    }
    if (matches!(a, PR | PO) && letter(b)) || (letter(a) && matches!(b, PR | PO)) {
        return Break::No;
    }
    // (LB25 as a regular expression, as ICU has it: a number with its signs and brackets, "$(1.5)%".)
    let number_ends_at = |j: usize| -> bool {
        // A number ends at j: NU (NU | SY | IS)*.
        let mut k = j;
        loop {
            match cls[k] {
                NU => return true,
                SY | IS if k > 0 => k -= 1,
                _ => return false,
            }
        }
    };
    if matches!(a, PR | PO) && (b == NU || ((op(b) || b == HY) && i + 1 < n && cls[i + 1] == NU)) {
        return Break::No;
    }
    if (op(a) || a == HY || a == IS) && b == NU {
        return Break::No;
    }
    if (matches!(b, NU | SY | IS | CL) || cp(b)) && number_ends_at(i - 1) {
        return Break::No;
    }
    if matches!(b, PO | PR) && (number_ends_at(i - 1) || ((a == CL || cp(a)) && i >= 2 && number_ends_at(i - 2))) {
        return Break::No;
    }
    // LB26, LB27: Korean syllables.
    if (a == JL && matches!(b, JL | JV | H2 | H3)) || (matches!(a, JV | H2) && matches!(b, JV | JT)) || (matches!(a, JT | H3) && b == JT) {
        return Break::No;
    }
    if (matches!(a, JL | JV | JT | H2 | H3) && b == PO) || (a == PR && matches!(b, JL | JV | JT | H2 | H3)) {
        return Break::No;
    }
    // LB28: between letters.
    if letter(a) && letter(b) {
        return Break::No;
    }
    // LB28a: Brahmic orthographic syllables (U+25CC, the dotted circle, as a consonant).
    let base = |j: usize| matches!(cls[j], AK | AS) || text[j] == '\u{25CC}';
    if a == AP && (base(i) || b == AS) {
        return Break::No;
    }
    if base(i - 1) && matches!(b, VF | VI) {
        return Break::No;
    }
    if a == VI && i >= 2 && base(i - 2) && (matches!(b, AK) || text[i] == '\u{25CC}') {
        return Break::No;
    }
    if base(i - 1) && base(i) && i + 1 < n && cls[i + 1] == VF {
        return Break::No;
    }
    // LB29, LB30: "." before letters; letters and numbers against brackets (narrow ones).
    if a == IS && letter(b) {
        return Break::No;
    }
    if (matches!(a, AL | HL | NU) && b == OP) || (a == CP && matches!(b, AL | HL | NU)) {
        return Break::No;
    }
    // LB30a: regional indicators in pairs (flags).
    if a == RI && b == RI {
        let mut k = 0;
        let mut j = i;
        while j > 0 && cls[j - 1] == RI {
            k += 1;
            j -= 1;
        }
        if k % 2 == 1 {
            return Break::No;
        }
    }
    // LB30b: an emoji and its skin tone.
    if a == EB && b == EM {
        return Break::No;
    }
    // LB31: anywhere else.
    Break::May
}

/// Chrome's own rule for two printable ASCII characters side by side (a table it keeps for compatibility with older
/// browsers, in place of the Unicode algorithm's): a line may break before an opening bracket after most
/// punctuation, and after "-" and "?" before most characters; nowhere else (not after "/", not between letters).
fn ascii_pair(a: char, b: char) -> bool {
    let wide = |b: char| matches!(b, '#' | '%' | '&' | '(' | '*' | '+' | '-' | '0'..='9' | '<' | '=' | '>' | '@' | 'A'..='Z' | '[' | '\\' | '^' | '_' | '`' | 'a'..='z' | '{' | '|' | '~');
    match a {
        '-' => wide(b) || matches!(b, '"' | '\''),
        '?' => wide(b) || b == '$',
        '!' | '"' | '#' | '%' | '&' | ')' | '*' | '+' | ',' | '.' | ':' | ';' | '=' | '>' | '\\' | ']' | '|' | '}' | '~' => matches!(b, '(' | '<' | '[' | '{'),
        _ => false,
    }
}

fn ascii(c: char) -> bool {
    ('!'..='~').contains(&c)
}

/// Where lines may break in `text`, as Chrome finds the places: after a space, always (and never before one);
/// between two printable ASCII characters, by its own table (and between "-" and a digit only after a letter or
/// digit: "ABCD-1234", not "-1"); where either is beyond ASCII (a no-break space aside), by the Unicode algorithm.
pub fn chrome(text: &[char], rules: Rules) -> Vec<Break> {
    let mut out = breaks(text, rules);
    if rules.strictness == Strictness::Anywhere {
        return out;
    }
    for i in 1..text.len() {
        if out[i] == Break::Must {
            continue;
        }
        let (a, b) = (text[i - 1], text[i]);
        // (word-break: break-all: a letter or digit breaks from a hyphen after it too, as Chrome has it.)
        if rules.break_all && (a.is_alphanumeric() && matches!(class(b), HY | BA)) {
            out[i] = Break::May;
            continue;
        }
        out[i] = if matches!(b, ' ' | '\t') {
            Break::No
        } else if matches!(a, ' ' | '\t') {
            Break::May
        } else if a == '-' && b.is_ascii_digit() && !rules.break_all {
            if i >= 2 && text[i - 2].is_ascii_alphanumeric() { Break::May } else { Break::No }
        } else if ascii(a) && ascii(b) && !rules.break_all {
            if ascii_pair(a, b) {
                Break::May
            } else {
                Break::No
            }
        } else if (a == '\u{a0}' && (ascii(b) || b == '\u{a0}')) || (b == '\u{a0}' && ascii(a)) {
            Break::No
        } else {
            out[i]
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text with "|" where a line may break (and "!" where it must).
    fn marked(text: &str, rules: Rules) -> alloc::string::String {
        let chars: Vec<char> = text.chars().collect();
        let b = breaks(&chars, rules);
        let mut out = alloc::string::String::new();
        for (i, &c) in chars.iter().enumerate() {
            match b[i] {
                Break::May if i > 0 => out.push('|'),
                Break::Must if i > 0 => out.push('!'),
                _ => {}
            }
            out.push(c);
        }
        out
    }

    fn marked_chrome(text: &str, rules: Rules) -> alloc::string::String {
        let chars: Vec<char> = text.chars().collect();
        let b = chrome(&chars, rules);
        let mut out = alloc::string::String::new();
        for (i, &c) in chars.iter().enumerate() {
            if i > 0 && b[i] != Break::No {
                out.push('|');
            }
            out.push(c);
        }
        out
    }

    #[test]
    fn lines_break_where_unicode_and_chrome_break_them() {
        let n = Rules::default();
        assert_eq!(marked("Hello, world. A well-known (fact)!", n), "Hello, |world. |A |well-|known |(fact)!");
        assert_eq!(marked("http://example.com/a/b?c=1", n), "http://|example.com/|a/|b?|c=1");
        // (Chrome: its own rules between ASCII characters: not after "/", but after "?" and "-"; after any space.)
        assert_eq!(marked_chrome("http://example.com/a/b?c=1 x-1 -1 (!a)", n), "http://example.com/a/b?|c=1 |x-|1 |-1 |(!a)");
        assert_eq!(marked("$-1.50 or 25% off, 1,000", n), "$-1.50 |or |25% |off, |1,000");
        // (Chinese and Japanese: between ideographs, not before closing punctuation and small kana (strict), not
        // after opening.)
        assert_eq!(marked("日本語。「テスト」です", n), "日|本|語。|「テ|ス|ト」|で|す");
        assert_eq!(marked("ちょっと", n), "ち|ょ|っ|と");
        assert_eq!(marked("ちょっと", Rules { strictness: Strictness::Strict, ..n }), "ちょっ|と");
        // (word-break.)
        assert_eq!(marked("abc def", Rules { break_all: true, ..n }), "a|b|c |d|e|f");
        assert_eq!(marked("日本語 한국어", Rules { keep_all: true, ..n }), "日本語 |한국어");
        // (A letter and its accent stay together; no break before or after a no-break space.)
        assert_eq!(marked("e\u{301}x a\u{a0}b", Rules { strictness: Strictness::Anywhere, ..n }), "e\u{301}|x| |a|\u{a0}|b");
        assert_eq!(marked("a\u{a0}b c\nd", n), "a\u{a0}b |c\n!d");
    }
}
