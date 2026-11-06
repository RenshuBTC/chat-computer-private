//! Counter styles (CSS Counter Styles): how a counter's value is written ("3", "c", "iii", "•", "三"), CSS's
//! predefined styles and those a page's @counter-style rules make, each written as Chrome writes it (checked against
//! Chromium's own list markers: its Hebrew numbers up to 999,999, its square "■", the Chinese, Japanese and Korean
//! styles' ways with ones and zeros).

use crate::css::{Decl, Tok};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

/// How a style writes values.
#[derive(Clone, Debug, PartialEq)]
pub enum System {
    Cyclic,
    Numeric,
    Alphabetic,
    Symbolic,
    Additive,
    /// Its symbols for the values from this one on, one each.
    Fixed(i64),
    /// One of CSS's styles with a way of its own.
    Special(Special),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Special {
    /// Hebrew numbers (thousands before a geresh, 15 and 16 as ט״ו and ט״ז are written).
    Hebrew,
    /// Chinese numbers: simplified or traditional characters, formal (banking) or informal.
    Chinese { simplified: bool, formal: bool },
    Japanese { formal: bool },
    Korean(Korean),
    Ethiopic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Korean {
    Hangul,
    HanjaInformal,
    HanjaFormal,
}

/// A counter style.
#[derive(Clone, Debug)]
pub struct CounterStyle {
    pub system: System,
    pub symbols: Vec<String>,
    /// Additive's symbols, by weight (largest first).
    pub additive: Vec<(i64, String)>,
    /// What goes before and after a negative value.
    pub negative: (String, String),
    /// What goes before and after the value in a list's marker.
    pub prefix: String,
    pub suffix: String,
    /// The values it writes (None: its system's own); others are written in its fallback.
    pub range: Option<Vec<(i64, i64)>>,
    /// The fewest symbols it writes, and what it fills up to them with.
    pub pad: (usize, String),
    pub fallback: String,
}

impl CounterStyle {
    fn new(system: System, symbols: &[&str], suffix: &str) -> CounterStyle {
        CounterStyle {
            system,
            symbols: symbols.iter().map(|s| String::from(*s)).collect(),
            additive: Vec::new(),
            negative: (String::from("-"), String::new()),
            prefix: String::new(),
            suffix: String::from(suffix),
            range: None,
            pad: (0, String::new()),
            fallback: String::from("decimal"),
        }
    }

    /// Each character of a string a symbol.
    fn chars(system: System, symbols: &str, suffix: &str) -> CounterStyle {
        let mut s = CounterStyle::new(system, &[], suffix);
        s.symbols = symbols.chars().map(String::from).collect();
        s
    }

    /// Ten digits from a code point on.
    fn digits(zero: u32) -> CounterStyle {
        let mut s = CounterStyle::new(System::Numeric, &[], ". ");
        s.symbols = (0..10).filter_map(|i| char::from_u32(zero + i)).map(String::from).collect();
        s
    }

    fn additive(pairs: &[(i64, &str)], range: (i64, i64)) -> CounterStyle {
        let mut s = CounterStyle::new(System::Additive, &[], ". ");
        s.additive = pairs.iter().map(|&(w, t)| (w, String::from(t))).collect();
        s.range = Some(alloc::vec![range]);
        s
    }

    fn special(special: Special, suffix: &str, negative: &str, range: (i64, i64), fallback: &str) -> CounterStyle {
        let mut s = CounterStyle::new(System::Special(special), &[], suffix);
        s.negative = (String::from(negative), String::new());
        s.range = Some(alloc::vec![range]);
        s.fallback = String::from(fallback);
        s
    }

    /// Whether it writes a value (rather than its fallback).
    fn writes(&self, v: i64) -> bool {
        match &self.range {
            Some(r) => r.iter().any(|&(lo, hi)| v >= lo && v <= hi),
            None => match self.system {
                System::Alphabetic | System::Symbolic => v >= 1,
                System::Additive => v >= 0,
                _ => true,
            },
        }
    }

    /// Whether a negative value is written as its absolute value with the negative sign around it.
    fn signed(&self) -> bool {
        !matches!(self.system, System::Cyclic | System::Fixed(_))
    }

    /// A value (zero or more, when signed) written in its symbols; None when it can't be.
    fn initial(&self, v: i64) -> Option<String> {
        let n = self.symbols.len() as i64;
        match &self.system {
            System::Cyclic => (n > 0).then(|| self.symbols[(v - 1).rem_euclid(n) as usize].clone()),
            System::Fixed(first) => {
                let i = v.checked_sub(*first)?;
                (i >= 0 && i < n).then(|| self.symbols[i as usize].clone())
            }
            System::Symbolic => {
                if n == 0 || v < 1 {
                    return None;
                }
                let times = (v - 1) / n + 1;
                // (A very long run is written in the fallback, as Chrome limits it.)
                if times > 60 {
                    return None;
                }
                Some(self.symbols[((v - 1) % n) as usize].repeat(times as usize))
            }
            System::Alphabetic => {
                if n < 2 || v < 1 {
                    return None;
                }
                let mut parts = Vec::new();
                let mut v = v;
                while v > 0 {
                    v -= 1;
                    parts.push(self.symbols[(v % n) as usize].as_str());
                    v /= n;
                }
                Some(parts.iter().rev().copied().collect())
            }
            System::Numeric => {
                if n < 2 {
                    return None;
                }
                if v == 0 {
                    return Some(self.symbols[0].clone());
                }
                let mut parts = Vec::new();
                let mut v = v;
                while v > 0 {
                    parts.push(self.symbols[(v % n) as usize].as_str());
                    v /= n;
                }
                Some(parts.iter().rev().copied().collect())
            }
            System::Additive => {
                if v == 0 {
                    return self.additive.iter().find(|(w, _)| *w == 0).map(|(_, s)| s.clone());
                }
                let mut out = String::new();
                let mut v = v;
                for (w, sym) in &self.additive {
                    if *w <= 0 || *w > v {
                        continue;
                    }
                    let times = v / w;
                    if times > 1000 {
                        return None;
                    }
                    for _ in 0..times {
                        out.push_str(sym);
                    }
                    v -= w * times;
                    if v == 0 {
                        return Some(out);
                    }
                }
                None
            }
            System::Special(sp) => Some(special(*sp, v)),
        }
    }
}

/// A predefined style, by its name in lower case.
pub fn predefined(name: &str) -> Option<CounterStyle> {
    use System::{Alphabetic, Cyclic, Fixed, Numeric};
    Some(match name {
        "decimal" => CounterStyle::digits('0' as u32),
        "decimal-leading-zero" => {
            let mut s = CounterStyle::digits('0' as u32);
            s.pad = (2, String::from("0"));
            s
        }
        "arabic-indic" => CounterStyle::digits(0x0660),
        "bengali" => CounterStyle::digits(0x09E6),
        "cambodian" | "khmer" => CounterStyle::digits(0x17E0),
        "devanagari" => CounterStyle::digits(0x0966),
        "gujarati" => CounterStyle::digits(0x0AE6),
        "gurmukhi" => CounterStyle::digits(0x0A66),
        "kannada" => CounterStyle::digits(0x0CE6),
        "lao" => CounterStyle::digits(0x0ED0),
        "malayalam" => CounterStyle::digits(0x0D66),
        "mongolian" => CounterStyle::digits(0x1810),
        "myanmar" => CounterStyle::digits(0x1040),
        "oriya" => CounterStyle::digits(0x0B66),
        "persian" => CounterStyle::digits(0x06F0),
        "tamil" => CounterStyle::digits(0x0BE6),
        "telugu" => CounterStyle::digits(0x0C66),
        "thai" => CounterStyle::digits(0x0E50),
        "tibetan" => CounterStyle::digits(0x0F20),
        "cjk-decimal" => {
            let mut s = CounterStyle::chars(Numeric, "〇一二三四五六七八九", "、");
            s.range = Some(alloc::vec![(0, i64::MAX)]);
            s
        }
        "lower-roman" | "upper-roman" => {
            let upper = name == "upper-roman";
            let pairs: [(i64, &str); 13] = [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
            let mut s = CounterStyle::additive(&pairs, (1, 3999));
            if upper {
                for (_, t) in s.additive.iter_mut() {
                    *t = t.to_ascii_uppercase();
                }
            }
            s
        }
        "armenian" | "upper-armenian" | "lower-armenian" => {
            // (Ա to Ք, or ա to ք: 1 to 9, the tens, the hundreds and the thousands, in order.)
            let first = if name == "lower-armenian" { 0x0561 } else { 0x0531 };
            let mut pairs: Vec<(i64, String)> = Vec::new();
            for i in 0..36u32 {
                let weight = (i % 9 + 1) as i64 * 10i64.pow(i / 9);
                pairs.push((weight, char::from_u32(first + i).map(String::from).unwrap_or_default()));
            }
            pairs.reverse();
            let mut s = CounterStyle::additive(&[], (1, 9999));
            s.additive = pairs;
            s
        }
        "georgian" => {
            let pairs: [(i64, u32); 37] = [
                (10000, 0x10F5), (9000, 0x10F0), (8000, 0x10EF), (7000, 0x10F4), (6000, 0x10EE), (5000, 0x10ED), (4000, 0x10EC),
                (3000, 0x10EB), (2000, 0x10EA), (1000, 0x10E9), (900, 0x10E8), (800, 0x10E7), (700, 0x10E6), (600, 0x10E5),
                (500, 0x10E4), (400, 0x10F3), (300, 0x10E2), (200, 0x10E1), (100, 0x10E0), (90, 0x10DF), (80, 0x10DE),
                (70, 0x10DD), (60, 0x10F2), (50, 0x10DC), (40, 0x10DB), (30, 0x10DA), (20, 0x10D9), (10, 0x10D8), (9, 0x10D7),
                (8, 0x10F1), (7, 0x10D6), (6, 0x10D5), (5, 0x10D4), (4, 0x10D3), (3, 0x10D2), (2, 0x10D1), (1, 0x10D0),
            ];
            let mut s = CounterStyle::additive(&[], (1, 19999));
            s.additive = pairs.iter().map(|&(w, c)| (w, char::from_u32(c).map(String::from).unwrap_or_default())).collect();
            s
        }
        "hebrew" => CounterStyle::special(Special::Hebrew, ". ", "-", (1, 999_999), "decimal"),
        "lower-alpha" | "lower-latin" => CounterStyle::chars(Alphabetic, "abcdefghijklmnopqrstuvwxyz", ". "),
        "upper-alpha" | "upper-latin" => CounterStyle::chars(Alphabetic, "ABCDEFGHIJKLMNOPQRSTUVWXYZ", ". "),
        "lower-greek" => CounterStyle::chars(Alphabetic, "αβγδεζηθικλμνξοπρστυφχψω", ". "),
        "hiragana" => CounterStyle::chars(Alphabetic, "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをん", "、"),
        "hiragana-iroha" => CounterStyle::chars(Alphabetic, "いろはにほへとちりぬるをわかよたれそつねならむうゐのおくやまけふこえてあさきゆめみしゑひもせす", "、"),
        "katakana" => CounterStyle::chars(Alphabetic, "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン", "、"),
        "katakana-iroha" => CounterStyle::chars(Alphabetic, "イロハニホヘトチリヌルヲワカヨタレソツネナラムウヰノオクヤマケフコエテアサキユメミシヱヒモセス", "、"),
        "disc" => CounterStyle::new(Cyclic, &["\u{2022}"], " "),
        "circle" => CounterStyle::new(Cyclic, &["\u{25E6}"], " "),
        "square" => CounterStyle::new(Cyclic, &["\u{25A0}"], " "),
        "disclosure-open" => CounterStyle::new(Cyclic, &["\u{25BE}"], " "),
        "disclosure-closed" => CounterStyle::new(Cyclic, &["\u{25B8}"], " "),
        "cjk-earthly-branch" | "cjk-heavenly-stem" => {
            let symbols = if name == "cjk-earthly-branch" { "子丑寅卯辰巳午未申酉戌亥" } else { "甲乙丙丁戊己庚辛壬癸" };
            let mut s = CounterStyle::chars(Fixed(1), symbols, "、");
            s.fallback = String::from("cjk-decimal");
            s
        }
        "japanese-informal" | "japanese-formal" => {
            CounterStyle::special(Special::Japanese { formal: name == "japanese-formal" }, "、", "マイナス", (-9999, 9999), "cjk-decimal")
        }
        "korean-hangul-formal" | "korean-hanja-informal" | "korean-hanja-formal" => {
            let k = match name {
                "korean-hangul-formal" => Korean::Hangul,
                "korean-hanja-informal" => Korean::HanjaInformal,
                _ => Korean::HanjaFormal,
            };
            CounterStyle::special(Special::Korean(k), ", ", "마이너스 ", (-9_999_999_999_999_999, 9_999_999_999_999_999), "cjk-decimal")
        }
        "simp-chinese-informal" | "simp-chinese-formal" | "trad-chinese-informal" | "trad-chinese-formal" | "cjk-ideographic" => {
            let simplified = name.starts_with("simp");
            let formal = name.ends_with("formal") && !name.ends_with("informal");
            let negative = if simplified { "负" } else { "負" };
            CounterStyle::special(Special::Chinese { simplified, formal }, "、", negative, (-9_999_999_999_999_999, 9_999_999_999_999_999), "cjk-decimal")
        }
        "ethiopic-numeric" => {
            let mut s = CounterStyle::special(Special::Ethiopic, "/ ", "-", (1, i64::MAX), "decimal");
            s.negative = (String::from("-"), String::new());
            s
        }
        _ => return None,
    })
}

/// Whether a name is one of CSS's predefined styles' (in any case).
pub fn is_predefined(name: &str) -> bool {
    predefined(&name.to_ascii_lowercase()).is_some()
}

/// A page's counter styles (its @counter-style rules), by name.
pub type Styles = BTreeMap<String, Arc<CounterStyle>>;

/// A style by name: the page's own, else CSS's (predefined names in any case); decimal when there's no such style.
pub fn find(name: &str, page: &Styles) -> CounterStyle {
    let lower = name.to_ascii_lowercase();
    let builtin = predefined(&lower);
    // (A predefined style's name is matched in any case; a page's own in its case. A page may take over the
    // predefined styles but decimal, disc, circle, square and the disclosure triangles.)
    let name = if builtin.is_some() { lower.as_str() } else { name };
    if !fixed_name(name)
        && let Some(s) = page.get(name)
    {
        return (**s).clone();
    }
    builtin.unwrap_or_else(|| CounterStyle::digits('0' as u32))
}

/// The styles a page can't take over.
fn fixed_name(name: &str) -> bool {
    matches!(name, "decimal" | "disc" | "circle" | "square" | "disclosure-open" | "disclosure-closed")
}

/// A value written in a style, without its prefix and suffix (as counter() writes it): in the style's fallback where
/// the style doesn't write it.
pub fn represent(name: &str, value: i64, page: &Styles) -> String {
    let style = find(name, page);
    write(&style, value, page, 0)
}

/// A list item's marker for a value: the style's prefix, the value, its suffix.
pub fn marker(name: &str, value: i64, page: &Styles) -> String {
    let style = find(name, page);
    let mut out = style.prefix.clone();
    out.push_str(&write(&style, value, page, 0));
    out.push_str(&style.suffix);
    out
}

fn write(style: &CounterStyle, value: i64, page: &Styles, depth: u32) -> String {
    let fall = |page: &Styles| {
        if depth >= 8 {
            return alloc::format!("{value}");
        }
        let f = find(&style.fallback, page);
        write(&f, value, page, depth + 1)
    };
    if !style.writes(value) {
        return fall(page);
    }
    let negative = value < 0 && style.signed();
    let Some(body) = style.initial(if negative { value.saturating_abs() } else { value }) else { return fall(page) };
    let mut body = body;
    // Padded to its fewest symbols (the negative sign counting towards them).
    let have = body.chars().count() + if negative { style.negative.0.chars().count() + style.negative.1.chars().count() } else { 0 };
    if style.pad.0 > have && !style.pad.1.is_empty() && style.pad.0 - have < 100 {
        body = style.pad.1.repeat(style.pad.0 - have) + &body;
    }
    if negative {
        let mut out = style.negative.0.clone();
        out.push_str(&body);
        out.push_str(&style.negative.1);
        return out;
    }
    body
}

/// The styles with ways of their own, for values of zero or more.
fn special(sp: Special, v: i64) -> String {
    match sp {
        Special::Hebrew => {
            if v >= 1000 {
                let mut out = hebrew_below_1000(v / 1000);
                out.push('\u{5F3}');
                out.push_str(&hebrew_below_1000(v % 1000));
                out
            } else {
                hebrew_below_1000(v)
            }
        }
        Special::Chinese { simplified, formal } => chinese(v, simplified, formal),
        Special::Japanese { formal } => japanese(v, formal),
        Special::Korean(k) => korean(v, k),
        Special::Ethiopic => ethiopic(v),
    }
}

fn hebrew_below_1000(v: i64) -> String {
    const ONES: [char; 9] = ['א', 'ב', 'ג', 'ד', 'ה', 'ו', 'ז', 'ח', 'ט'];
    const TENS: [char; 9] = ['י', 'כ', 'ל', 'מ', 'נ', 'ס', 'ע', 'פ', 'צ'];
    const HUNDREDS: [char; 3] = ['ק', 'ר', 'ש'];
    let mut out = String::new();
    let mut h = v / 100;
    while h >= 4 {
        out.push('ת');
        h -= 4;
    }
    if h > 0 {
        out.push(HUNDREDS[(h - 1) as usize]);
    }
    let rest = v % 100;
    match rest {
        15 => out.push_str("טו"),
        16 => out.push_str("טז"),
        _ => {
            if rest >= 10 {
                out.push(TENS[(rest / 10 - 1) as usize]);
            }
            if rest % 10 > 0 {
                out.push(ONES[(rest % 10 - 1) as usize]);
            }
        }
    }
    out
}

/// Chinese numbers: in groups of four digits (万 or 萬, 亿 or 億, 兆), a digit's place after it (十, 百, 千), a run of
/// zeros between digits as one 零; informally, a group of ten to nineteen without its 一.
fn chinese(v: i64, simplified: bool, formal: bool) -> String {
    let digits: [&str; 10] = match (simplified, formal) {
        (_, false) => ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"],
        (true, true) => ["零", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖"],
        (false, true) => ["零", "壹", "貳", "參", "肆", "伍", "陸", "柒", "捌", "玖"],
    };
    let places: [&str; 4] = if formal { ["", "拾", "佰", "仟"] } else { ["", "十", "百", "千"] };
    let groups_marks: [&str; 4] = if simplified { ["", "万", "亿", "兆"] } else { ["", "萬", "億", "兆"] };
    if v == 0 {
        return String::from(digits[0]);
    }
    let mut groups = Vec::new();
    let mut n = v;
    while n > 0 {
        groups.push(n % 10000);
        n /= 10000;
    }
    // Pieces, with None for a zero that may be written.
    let mut pieces: Vec<Option<String>> = Vec::new();
    for (gi, &g) in groups.iter().enumerate().rev() {
        if g == 0 {
            pieces.push(None);
            continue;
        }
        let drop_one = !formal && (10..=19).contains(&g);
        let mut group: Vec<Option<String>> = Vec::new();
        for place in (0..4).rev() {
            let d = (g / 10i64.pow(place)) % 10;
            if d == 0 {
                group.push(None);
            } else if place == 1 && d == 1 && drop_one {
                group.push(Some(String::from(places[1])));
            } else {
                group.push(Some(String::from(digits[d as usize]) + places[place as usize]));
            }
        }
        // (A group's own trailing zeros go.)
        while group.last().is_some_and(|p| p.is_none()) {
            group.pop();
        }
        pieces.extend(group);
        pieces.push(Some(String::from(groups_marks[gi.min(3)])));
    }
    let mut out = String::new();
    let mut zero = false;
    for p in pieces {
        match p {
            None => zero = true,
            Some(t) => {
                if zero && !out.is_empty() {
                    out.push_str(digits[0]);
                }
                zero = false;
                out.push_str(&t);
            }
        }
    }
    out
}

/// Japanese numbers below 10,000: each non-zero digit and its place (十, 百, 千, or 拾, 百, 阡 formally); informally a
/// one before a place left out.
fn japanese(v: i64, formal: bool) -> String {
    let digits: [&str; 10] = if formal { ["零", "壱", "弐", "参", "四", "伍", "六", "七", "八", "九"] } else { ["〇", "一", "二", "三", "四", "五", "六", "七", "八", "九"] };
    let places: [&str; 4] = if formal { ["", "拾", "百", "阡"] } else { ["", "十", "百", "千"] };
    if v == 0 {
        return String::from(digits[0]);
    }
    let mut out = String::new();
    for place in (0..4).rev() {
        let d = (v / 10i64.pow(place)) % 10;
        if d == 0 {
            continue;
        }
        if !(d == 1 && place > 0 && !formal) {
            out.push_str(digits[d as usize]);
        }
        out.push_str(places[place as usize]);
    }
    out
}

/// Korean numbers: groups of four digits (만, 억, 조 or 萬, 億, 兆) apart, each non-zero digit and its place; the
/// informal hanja leave out a one before any place.
fn korean(v: i64, k: Korean) -> String {
    let (digits, places, groups_marks): ([&str; 10], [&str; 4], [&str; 4]) = match k {
        Korean::Hangul => (["영", "일", "이", "삼", "사", "오", "육", "칠", "팔", "구"], ["", "십", "백", "천"], ["", "만", "억", "조"]),
        Korean::HanjaInformal => (["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"], ["", "十", "百", "千"], ["", "萬", "億", "兆"]),
        Korean::HanjaFormal => (["零", "壹", "貳", "參", "四", "五", "六", "七", "八", "九"], ["", "拾", "百", "仟"], ["", "萬", "億", "兆"]),
    };
    if v == 0 {
        return String::from(digits[0]);
    }
    let mut groups = Vec::new();
    let mut n = v;
    while n > 0 {
        groups.push(n % 10000);
        n /= 10000;
    }
    let mut words: Vec<String> = Vec::new();
    for (gi, &g) in groups.iter().enumerate().rev() {
        if g == 0 {
            continue;
        }
        let mut w = String::new();
        for place in (0..4).rev() {
            let d = (g / 10i64.pow(place)) % 10;
            if d == 0 {
                continue;
            }
            // (Informally, a one is left out before a place: the group's own, or its group's mark.)
            let before_mark = place > 0 || gi > 0;
            if !(k == Korean::HanjaInformal && d == 1 && before_mark) {
                w.push_str(digits[d as usize]);
            }
            w.push_str(places[place as usize]);
        }
        w.push_str(groups_marks[gi.min(3)]);
        words.push(w);
    }
    words.join(" ")
}

/// Ethiopic numbers: pairs of digits, ፻ after the odd pairs and ፼ after the even ones (as CSS Counter Styles has it).
fn ethiopic(v: i64) -> String {
    const ONES: [char; 9] = ['፩', '፪', '፫', '፬', '፭', '፮', '፯', '፰', '፱'];
    const TENS: [char; 9] = ['፲', '፳', '፴', '፵', '፶', '፷', '፸', '፹', '፺'];
    if v == 1 {
        return String::from("፩");
    }
    let mut groups = Vec::new();
    let mut n = v;
    while n > 0 {
        groups.push(n % 100);
        n /= 100;
    }
    let top = groups.len() - 1;
    let mut out = String::new();
    for (i, &g) in groups.iter().enumerate().rev() {
        let drop = g == 0 || (g == 1 && (i == top || i % 2 == 1));
        if !drop {
            if g / 10 > 0 {
                out.push(TENS[(g / 10 - 1) as usize]);
            }
            if g % 10 > 0 {
                out.push(ONES[(g % 10 - 1) as usize]);
            }
        }
        if i % 2 == 1 && g != 0 {
            out.push('፻');
        } else if i % 2 == 0 && i > 0 {
            out.push('፼');
        }
    }
    out
}

/// A page's @counter-style rule read (its name and descriptors); `extends` taken from the styles already read.
pub fn from_rule(name: &str, decls: &[Decl], page: &Styles) -> Option<CounterStyle> {
    // (CSS's own names, and none, can't be taken.)
    let lower = name.to_ascii_lowercase();
    if fixed_name(&lower) || matches!(lower.as_str(), "none" | "inherit" | "initial" | "unset" | "default") {
        return None;
    }
    let get = |n: &str| decls.iter().rev().find(|d| d.name == n).map(|d| trim(&d.value));
    let mut style = CounterStyle::new(System::Symbolic, &[], ". ");
    let system = get("system").unwrap_or(&[]);
    let mut extends = false;
    match system {
        [] => {}
        [Tok::Ident(i)] => {
            style.system = match i.to_ascii_lowercase().as_str() {
                "cyclic" => System::Cyclic,
                "numeric" => System::Numeric,
                "alphabetic" => System::Alphabetic,
                "symbolic" => System::Symbolic,
                "additive" => System::Additive,
                "fixed" => System::Fixed(1),
                _ => return None,
            }
        }
        [Tok::Ident(i), rest @ ..] if i.eq_ignore_ascii_case("fixed") => {
            let n = rest.iter().find_map(|t| if let Tok::Number(n) = t { Some(*n as i64) } else { None })?;
            style.system = System::Fixed(n);
        }
        [Tok::Ident(i), rest @ ..] if i.eq_ignore_ascii_case("extends") => {
            let other = rest.iter().find_map(|t| if let Tok::Ident(n) = t { Some(n.clone()) } else { None })?;
            let base = if page.contains_key(other.as_str()) { page.get(other.as_str()).map(|s| (**s).clone()) } else { predefined(&other.to_ascii_lowercase()) };
            style = base.unwrap_or_else(|| find("decimal", page));
            extends = true;
        }
        _ => return None,
    }
    if !extends {
        if let Some(v) = get("symbols") {
            style.symbols = symbols(v);
        }
        if let Some(v) = get("additive-symbols") {
            let mut pairs = Vec::new();
            for part in crate::css::split_commas(v) {
                let part = trim(part);
                let w = part.iter().find_map(|t| if let Tok::Number(n) = t { Some(*n as i64) } else { None });
                let s = symbols(part).into_iter().next();
                if let (Some(w), Some(s)) = (w, s) {
                    pairs.push((w, s));
                }
            }
            style.additive = pairs;
        }
        // (Not enough symbols for the system: the rule is invalid.)
        let n = style.symbols.len();
        let ok = match style.system {
            System::Alphabetic | System::Numeric => n >= 2,
            System::Additive => !style.additive.is_empty(),
            _ => n >= 1,
        };
        if !ok {
            return None;
        }
    }
    if let Some(v) = get("negative") {
        let s = symbols(v);
        style.negative = (s.first().cloned().unwrap_or_default(), s.get(1).cloned().unwrap_or_default());
    }
    if let Some(v) = get("prefix") {
        style.prefix = symbols(v).into_iter().next().unwrap_or_default();
    }
    if let Some(v) = get("suffix") {
        style.suffix = symbols(v).into_iter().next().unwrap_or_default();
    }
    if let Some(v) = get("range") {
        if matches!(v, [Tok::Ident(i)] if i.eq_ignore_ascii_case("auto")) {
            style.range = None;
        } else {
            let mut ranges = Vec::new();
            for part in crate::css::split_commas(v) {
                // (Each range's two ends: numbers, or infinite: below everything at the start, above it at the end.)
                let ends: Vec<Option<i64>> = trim(part)
                    .iter()
                    .filter_map(|t| match t {
                        Tok::Number(n) => Some(Some(*n as i64)),
                        Tok::Ident(i) if i.eq_ignore_ascii_case("infinite") => Some(None),
                        _ => None,
                    })
                    .collect();
                if let [lo, hi] = ends[..] {
                    ranges.push((lo.unwrap_or(i64::MIN), hi.unwrap_or(i64::MAX)));
                }
            }
            if !ranges.is_empty() {
                style.range = Some(ranges);
            }
        }
    }
    if let Some(v) = get("pad") {
        let n = v.iter().find_map(|t| if let Tok::Number(n) = t { Some(*n as usize) } else { None });
        let s = symbols(v).into_iter().next();
        if let (Some(n), Some(s)) = (n, s) {
            style.pad = (n, s);
        }
    }
    if let Some(v) = get("fallback")
        && let Some(Tok::Ident(f)) = v.first()
    {
        style.fallback = f.clone();
    }
    Some(style)
}

fn trim(v: &[Tok]) -> &[Tok] {
    crate::values::trim(v)
}

/// The symbols in a descriptor's value: strings, and identifiers as written.
fn symbols(v: &[Tok]) -> Vec<String> {
    v.iter()
        .filter_map(|t| match t {
            Tok::Str(s) => Some(s.clone()),
            Tok::Ident(i) => Some(i.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str, v: i64) -> String {
        marker(name, v, &Styles::new())
    }

    #[test]
    fn as_chrome_writes_them() {
        // (Each as Chromium's own markers have it.)
        assert_eq!(m("decimal", 1), "1. ");
        assert_eq!(m("hiragana", 0), "0、");
        assert_eq!(m("lower-roman", 4000), "4000. ");
        assert_eq!(m("decimal-leading-zero", -5), "-5. ");
        assert_eq!(m("decimal-leading-zero", 0), "00. ");
        assert_eq!(m("lower-alpha", -1), "-1. ");
        assert_eq!(m("lower-greek", 25), "αα. ");
        assert_eq!(m("upper-latin", 27), "AA. ");
        assert_eq!(m("cjk-decimal", -12), "-12、");
        assert_eq!(m("hebrew", 15), "טו. ");
        assert_eq!(m("hebrew", 1999), "א׳תתקצט. ");
        assert_eq!(m("hebrew", 11000), "יא׳. ");
        assert_eq!(m("hebrew", 999_999), "תתקצט׳תתקצט. ");
        assert_eq!(m("hebrew", 1_000_000), "1000000. ");
        assert_eq!(m("armenian", 9999), "ՔՋՂԹ. ");
        assert_eq!(m("lower-armenian", 1234), "ռմլդ. ");
        assert_eq!(m("georgian", 19999), "ჵჰშჟთ. ");
        assert_eq!(m("georgian", 20000), "20000. ");
        assert_eq!(m("square", -3), "■ ");
        assert_eq!(m("arabic-indic", -109), "-١٠٩. ");
        assert_eq!(m("katakana-iroha", 48), "イイ、");
        assert_eq!(m("cjk-ideographic", -12), "負十二、");
        assert_eq!(m("cjk-ideographic", 101), "一百零一、");
        assert_eq!(m("simp-chinese-informal", 10010), "一万零十、");
        assert_eq!(m("simp-chinese-informal", 1000000001), "十亿零一、");
        assert_eq!(m("simp-chinese-informal", 1010101010), "十亿一千零一十万一千零一十、");
        assert_eq!(m("simp-chinese-informal", 2147483647), "二十一亿四千七百四十八万三千六百四十七、");
        assert_eq!(m("trad-chinese-formal", 100010000), "壹億零壹萬、");
        assert_eq!(m("trad-chinese-formal", 10019), "壹萬零壹拾玖、");
        assert_eq!(m("japanese-informal", 1100), "千百、");
        assert_eq!(m("japanese-informal", -12), "マイナス十二、");
        assert_eq!(m("japanese-informal", 10000), "一〇〇〇〇、");
        assert_eq!(m("japanese-informal", -10000), "-10000、");
        assert_eq!(m("japanese-formal", 2345), "弐阡参百四拾伍、");
        assert_eq!(m("korean-hangul-formal", 12345), "일만 이천삼백사십오, ");
        assert_eq!(m("korean-hangul-formal", -10000), "마이너스 일만, ");
        assert_eq!(m("korean-hanja-informal", 2147483647), "二十億 四千七百四十八萬 三千六百四十七, ");
        assert_eq!(m("korean-hanja-informal", 1010101010), "十億 千十萬 千十, ");
        assert_eq!(m("korean-hanja-formal", 2345), "貳仟參百四拾五, ");
        assert_eq!(m("ethiopic-numeric", 12345), "፼፳፫፻፵፭/ ");
        assert_eq!(m("ethiopic-numeric", 1000000), "፻፼/ ");
        assert_eq!(m("ethiopic-numeric", 2147483647), "፳፩፼፵፯፻፵፰፼፴፮፻፵፯/ ");
        assert_eq!(m("ethiopic-numeric", 0), "0/ ");
        assert_eq!(m("cjk-earthly-branch", 19), "一九、");
        assert_eq!(m("cjk-heavenly-stem", 10), "癸、");
        assert_eq!(m("DECIMAL", 3), "3. ");
        assert_eq!(m("foo", 3), "3. ");
        assert_eq!(represent("upper-roman", 4, &Styles::new()), "IV");
    }
}
