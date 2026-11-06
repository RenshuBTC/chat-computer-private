//! CSS read as browsers read it (CSS Syntax Level 3's tokens, then rules): style sheets' rules, each its selectors
//! and declarations; a style attribute's declarations; @media's rules kept where its query matches the page's
//! window (its width and height in CSS pixels), @supports', @layer's and @container's kept; @import's addresses
//! noted, to be fetched; @font-face's fonts (font_faces: read from the sheet's text, as its unicode-range has to be);
//! @keyframes, @page and the like skipped. A rule whose selectors can't be read is dropped whole, as the standard
//! says. What the declarations' values mean is style.rs's.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    /// A function's name (lower case) and "(".
    Function(String),
    AtKeyword(String),
    /// "#...", and whether what follows the "#" could be an id.
    Hash(String, bool),
    Str(String),
    Url(String),
    Number(f32),
    Percent(f32),
    /// A number and its unit (lower case).
    Dimension(f32, String),
    Delim(char),
    /// A string a line ended before its closing quote: nothing it's in is valid.
    BadString,
    Ws,
    Colon,
    Semicolon,
    Comma,
    OpenSquare,
    CloseSquare,
    OpenParen,
    CloseParen,
    OpenCurly,
    CloseCurly,
}

/// Tokens written out as CSS again (a custom property's value, as getComputedStyle gives it), trimmed.
pub fn serialize(t: &[Tok]) -> String {
    let mut out = String::new();
    for tok in t {
        match tok {
            Tok::Ident(s) => out.push_str(s),
            Tok::Function(f) => {
                out.push_str(f);
                out.push('(');
            }
            Tok::AtKeyword(s) => {
                out.push('@');
                out.push_str(s);
            }
            Tok::Hash(s, _) => {
                out.push('#');
                out.push_str(s);
            }
            Tok::Str(s) => {
                out.push('"');
                for c in s.chars() {
                    if c == '"' || c == '\\' {
                        out.push('\\');
                    }
                    out.push(c);
                }
                out.push('"');
            }
            Tok::Url(u) => {
                out.push_str("url(");
                out.push_str(u);
                out.push(')');
            }
            Tok::Number(n) => out.push_str(&alloc::format!("{n}")),
            Tok::Percent(n) => out.push_str(&alloc::format!("{n}%")),
            Tok::Dimension(n, u) => out.push_str(&alloc::format!("{n}{u}")),
            Tok::Delim(c) => out.push(*c),
            Tok::BadString => {}
            Tok::Ws => out.push(' '),
            Tok::Colon => out.push(':'),
            Tok::Semicolon => out.push(';'),
            Tok::Comma => out.push(','),
            Tok::OpenSquare => out.push('['),
            Tok::CloseSquare => out.push(']'),
            Tok::OpenParen => out.push('('),
            Tok::CloseParen => out.push(')'),
            Tok::OpenCurly => out.push('{'),
            Tok::CloseCurly => out.push('}'),
        }
    }
    String::from(out.trim())
}

/// CSS's tokens (comments dropped).
/// CSS's white space: spaces, tabs and line ends (not the em space or the no-break space: those are letters to it).
fn css_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

pub fn tokens(css: &str) -> Vec<Tok> {
    let chars: Vec<char> = css.chars().collect();
    let mut t = Tokenizer { c: chars, i: 0 };
    let mut out = Vec::new();
    while let Some(tok) = t.next() {
        out.push(tok);
    }
    out
}

struct Tokenizer {
    c: Vec<char>,
    i: usize,
}

fn name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

fn name_char(c: char) -> bool {
    name_start(c) || c.is_ascii_digit() || c == '-'
}

impl Tokenizer {
    fn at(&self, k: usize) -> char {
        self.c.get(self.i + k).copied().unwrap_or('\0')
    }

    fn valid_escape(&self, k: usize) -> bool {
        self.at(k) == '\\' && self.at(k + 1) != '\n' && self.i + k + 1 < self.c.len()
    }

    /// Whether an identifier starts `k` characters on.
    fn ident_start(&self, k: usize) -> bool {
        let c = self.at(k);
        if c == '-' {
            let d = self.at(k + 1);
            return name_start(d) || d == '-' || self.valid_escape(k + 1);
        }
        name_start(c) || self.valid_escape(k)
    }

    fn number_start(&self) -> bool {
        let (a, b, c) = (self.at(0), self.at(1), self.at(2));
        if a.is_ascii_digit() {
            return true;
        }
        if a == '.' {
            return b.is_ascii_digit();
        }
        if a == '+' || a == '-' {
            return b.is_ascii_digit() || (b == '.' && c.is_ascii_digit());
        }
        false
    }

    /// An escape (after its "\"): a character by its hex code (and one space after it), or the character itself.
    fn escape(&mut self) -> char {
        let mut hex = 0u32;
        let mut n = 0;
        while n < 6 && self.at(0).is_ascii_hexdigit() {
            hex = hex * 16 + self.at(0).to_digit(16).unwrap_or(0);
            self.i += 1;
            n += 1;
        }
        if n > 0 {
            if css_space(self.at(0)) {
                self.i += 1;
            }
            return match char::from_u32(hex) {
                Some(c) if hex != 0 => c,
                _ => '\u{FFFD}',
            };
        }
        let c = self.at(0);
        self.i += 1;
        c
    }

    fn name(&mut self) -> String {
        let mut s = String::new();
        loop {
            let c = self.at(0);
            if name_char(c) && self.i < self.c.len() {
                s.push(c);
                self.i += 1;
            } else if self.valid_escape(0) {
                self.i += 1;
                s.push(self.escape());
            } else {
                return s;
            }
        }
    }

    fn number(&mut self) -> f32 {
        let start = self.i;
        if matches!(self.at(0), '+' | '-') {
            self.i += 1;
        }
        while self.at(0).is_ascii_digit() {
            self.i += 1;
        }
        if self.at(0) == '.' && self.at(1).is_ascii_digit() {
            self.i += 1;
            while self.at(0).is_ascii_digit() {
                self.i += 1;
            }
        }
        if matches!(self.at(0), 'e' | 'E')
            && (self.at(1).is_ascii_digit() || (matches!(self.at(1), '+' | '-') && self.at(2).is_ascii_digit()))
        {
            self.i += 2;
            while self.at(0).is_ascii_digit() {
                self.i += 1;
            }
        }
        let s: String = self.c[start..self.i].iter().collect();
        parse_f32(&s)
    }

    fn string(&mut self, quote: char) -> Tok {
        let mut s = String::new();
        loop {
            if self.i >= self.c.len() {
                return Tok::Str(s);
            }
            let c = self.at(0);
            self.i += 1;
            match c {
                c if c == quote => return Tok::Str(s),
                '\n' => {
                    // (The line's end is left for what follows.)
                    self.i -= 1;
                    return Tok::BadString;
                }
                '\\' => {
                    if self.at(0) == '\n' {
                        self.i += 1;
                    } else if self.i < self.c.len() {
                        s.push(self.escape());
                    }
                }
                c => s.push(c),
            }
        }
    }

    fn url(&mut self) -> Tok {
        while css_space(self.at(0)) {
            self.i += 1;
        }
        let mut s = String::new();
        loop {
            if self.i >= self.c.len() {
                return Tok::Url(s);
            }
            let c = self.at(0);
            self.i += 1;
            match c {
                ')' => return Tok::Url(s),
                c if css_space(c) => {
                    while css_space(self.at(0)) {
                        self.i += 1;
                    }
                    if self.at(0) == ')' {
                        self.i += 1;
                    } else {
                        // A bad url(): up to its ")".
                        while self.i < self.c.len() && self.at(0) != ')' {
                            self.i += 1;
                        }
                        self.i += 1;
                    }
                    return Tok::Url(s);
                }
                '\\' => s.push(self.escape()),
                c => s.push(c),
            }
        }
    }

    fn next(&mut self) -> Option<Tok> {
        loop {
            if self.i >= self.c.len() {
                return None;
            }
            // Comments.
            if self.at(0) == '/' && self.at(1) == '*' {
                self.i += 2;
                while self.i < self.c.len() && !(self.at(0) == '*' && self.at(1) == '/') {
                    self.i += 1;
                }
                self.i += 2;
                continue;
            }
            break;
        }
        let c = self.at(0);
        if css_space(c) {
            while css_space(self.at(0)) && self.i < self.c.len() {
                self.i += 1;
            }
            return Some(Tok::Ws);
        }
        if c == '"' || c == '\'' {
            self.i += 1;
            return Some(self.string(c));
        }
        if c == '#' {
            if name_char(self.at(1)) || self.valid_escape(1) {
                self.i += 1;
                let id = self.ident_start(0);
                return Some(Tok::Hash(self.name(), id));
            }
            self.i += 1;
            return Some(Tok::Delim('#'));
        }
        if self.number_start() {
            let n = self.number();
            if self.ident_start(0) {
                let unit = self.name().to_ascii_lowercase();
                return Some(Tok::Dimension(n, unit));
            }
            if self.at(0) == '%' {
                self.i += 1;
                return Some(Tok::Percent(n));
            }
            return Some(Tok::Number(n));
        }
        if c == '-' && self.at(1) == '-' && self.at(2) == '>' {
            self.i += 3;
            return Some(Tok::Ws);
        }
        if c == '<' && self.at(1) == '!' && self.at(2) == '-' && self.at(3) == '-' {
            self.i += 4;
            return Some(Tok::Ws);
        }
        if self.ident_start(0) {
            let name = self.name();
            if self.at(0) == '(' {
                self.i += 1;
                let lower = name.to_ascii_lowercase();
                if lower == "url" {
                    let mut k = 0;
                    while css_space(self.at(k)) {
                        k += 1;
                    }
                    if self.at(k) != '"' && self.at(k) != '\'' {
                        return Some(self.url());
                    }
                }
                return Some(Tok::Function(lower));
            }
            return Some(Tok::Ident(name));
        }
        if c == '@' && self.ident_start(1) {
            self.i += 1;
            return Some(Tok::AtKeyword(self.name().to_ascii_lowercase()));
        }
        if c == '\\' && self.valid_escape(0) {
            return Some(Tok::Ident(self.name()));
        }
        self.i += 1;
        Some(match c {
            ':' => Tok::Colon,
            ';' => Tok::Semicolon,
            ',' => Tok::Comma,
            '[' => Tok::OpenSquare,
            ']' => Tok::CloseSquare,
            '(' => Tok::OpenParen,
            ')' => Tok::CloseParen,
            '{' => Tok::OpenCurly,
            '}' => Tok::CloseCurly,
            c => Tok::Delim(c),
        })
    }
}

/// A number from its digits (core has no parser for floats that's also in no_std's... it does: str::parse).
fn parse_f32(s: &str) -> f32 {
    s.parse::<f32>().unwrap_or(0.0)
}

// ------------------------------------------------------------------------------------------------ rules

#[derive(Clone, Debug)]
pub struct Decl {
    /// Its property's name (lower case, but for custom properties: "--name" as written).
    pub name: String,
    pub value: Vec<Tok>,
    pub important: bool,
}

pub struct Rule {
    pub selectors: Vec<Selector>,
    pub decls: Vec<Decl>,
}

#[derive(Default)]
pub struct Sheet {
    pub rules: Vec<Rule>,
    /// @import's addresses (as written), with whether their media matched.
    pub imports: Vec<String>,
    /// The shadow root whose tree it styles (a <style> inside one: its rules are for that tree alone, its host
    /// through :host, and what's put in its slots through ::slotted()); None for the page's.
    pub scope: Option<usize>,
    /// Its @font-face rules' fonts.
    pub fonts: Vec<FontFace>,
    /// Its @keyframes rules.
    pub keyframes: Vec<Keyframes>,
    /// Its @counter-style rules: each one's name and descriptors.
    pub counter_styles: Vec<(String, Vec<Decl>)>,
}

/// An @keyframes rule: its name, and each keyframe's places (0 to 1: "from" 0, "to" 1, percentages) and
/// declarations.
#[derive(Clone, Debug, Default)]
pub struct Keyframes {
    pub name: String,
    pub frames: Vec<(Vec<f32>, Vec<Decl>)>,
}

/// An @keyframes rule's name and block read.
fn keyframes(prelude: &[Tok], inner: &[Tok]) -> Option<Keyframes> {
    let name = prelude.iter().find_map(|t| match t {
        Tok::Ident(i) | Tok::Str(i) => Some(i.clone()),
        _ => None,
    })?;
    let mut frames = Vec::new();
    let mut i = 0;
    while i < inner.len() {
        let start = i;
        while i < inner.len() && inner[i] != Tok::OpenCurly {
            i += 1;
        }
        if i >= inner.len() {
            break;
        }
        let end = block_end(inner, i);
        let places: Vec<f32> = split_commas(&inner[start..i])
            .iter()
            .filter_map(|p| match crate::values::trim(p) {
                [Tok::Ident(w)] if w.eq_ignore_ascii_case("from") => Some(0.0),
                [Tok::Ident(w)] if w.eq_ignore_ascii_case("to") => Some(1.0),
                [Tok::Percent(x)] => Some(x / 100.0),
                _ => None,
            })
            .collect();
        let decls = declarations(&inner[(i + 1).min(inner.len())..end.min(inner.len())]);
        if !places.is_empty() {
            frames.push((places, decls));
        }
        i = end + 1;
    }
    Some(Keyframes { name, frames })
}

/// A font an @font-face rule names: for a family, at weights and a style, for some characters, fetched from where
/// it says.
#[derive(Clone, Debug, PartialEq)]
pub struct FontFace {
    /// Its family's name, in lower case.
    pub family: String,
    /// Where it can be fetched from, in order: each address (as written) and its format() (lower case; "" if none).
    pub sources: Vec<(String, String)>,
    /// The weights it's for (the least and the most), and whether it's italic (or oblique).
    pub weight: (u16, u16),
    pub italic: bool,
    /// The characters it's for (unicode-range's ranges): none, all of them.
    pub ranges: Vec<(u32, u32)>,
    /// The widths it's for (font-stretch: percentages of normal, the least and the most).
    pub stretch: (u16, u16),
}

/// A font-stretch (font-width) keyword's or percentage's width, as a percentage of normal.
pub fn stretch_value(w: &str) -> Option<u16> {
    Some(match w {
        "ultra-condensed" => 50,
        "extra-condensed" => 62,
        "condensed" => 75,
        "semi-condensed" => 87,
        "normal" => 100,
        "semi-expanded" => 112,
        "expanded" => 125,
        "extra-expanded" => 150,
        "ultra-expanded" => 200,
        p => crate::num::round(p.strip_suffix('%')?.parse::<f32>().ok().filter(|v| *v >= 0.0)?).min(1000.0) as u16,
    })
}

impl FontFace {
    /// Whether it has `c` in its unicode-range.
    pub fn covers(&self, c: u32) -> bool {
        self.ranges.is_empty() || self.ranges.iter().any(|&(a, b)| a <= c && c <= b)
    }
}

/// Text with its comments taken out (outside strings).
fn without_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let b = css.as_bytes();
    let mut i = 0;
    let mut quote = 0u8;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == b'\\' && i + 1 < b.len() {
                out.push_str(&css[i..i + 2]);
                i += 2;
                continue;
            }
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i = css[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
            continue;
        }
        // (A character whole: its bytes are copied as they come.)
        let len = css[i..].chars().next().map_or(1, |ch| ch.len_utf8());
        out.push_str(&css[i..i + len]);
        i += len;
    }
    out
}

/// Splits text at `sep` where it isn't inside parentheses or quotes.
fn split_top(text: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut quote, mut start) = (0i32, None::<char>, 0usize);
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match quote {
            Some(q) => {
                if c == '\\' {
                    chars.next();
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '(' => depth += 1,
                ')' => depth -= 1,
                c if c == sep && depth <= 0 => {
                    out.push(&text[start..i]);
                    start = i + c.len_utf8();
                }
                _ => {}
            },
        }
    }
    out.push(&text[start..]);
    out
}

/// A name as written, its quotes taken off.
fn unquote(v: &str) -> String {
    let v = v.trim();
    let inner = if v.len() >= 2 && (v.starts_with('"') && v.ends_with('"') || v.starts_with('\'') && v.ends_with('\'')) { &v[1..v.len() - 1] } else { v };
    String::from(inner.trim())
}

/// The fonts a style sheet's @font-face rules name, from its text (unicode-range's "U+1E00" isn't CSS's tokens).
pub fn font_faces(css: &str) -> Vec<FontFace> {
    let mut out = Vec::new();
    if !css.contains("@font-face") && !css.to_ascii_lowercase().contains("@font-face") {
        return out;
    }
    let text = without_comments(css);
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("@font-face") {
        let start = from + at + 10;
        let Some(open) = text[start..].find('{').map(|o| start + o) else { break };
        // Its block: to its closing brace (none inside strings or url()s counts).
        let (mut depth, mut quote, mut end) = (0i32, None::<char>, text.len());
        let mut chars = text[open..].char_indices();
        while let Some((i, c)) = chars.next() {
            match quote {
                Some(q) => {
                    if c == '\\' {
                        chars.next();
                    } else if c == q {
                        quote = None;
                    }
                }
                None => match c {
                    '"' | '\'' => quote = Some(c),
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + i;
                            break;
                        }
                    }
                    _ => {}
                },
            }
        }
        let block = &text[open + 1..end.min(text.len())];
        from = (end + 1).min(text.len());
        let mut face = FontFace { family: String::new(), sources: Vec::new(), weight: (400, 400), italic: false, ranges: Vec::new(), stretch: (100, 100) };
        for decl in split_top(block, ';') {
            let Some((name, value)) = decl.split_once(':') else { continue };
            let value = value.trim().trim_end_matches("!important").trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "font-family" => face.family = unquote(value).to_ascii_lowercase(),
                "src" => {
                    for part in split_top(value, ',') {
                        let lower = part.to_ascii_lowercase();
                        let Some(u) = lower.find("url(") else { continue };
                        let rest = &part[u + 4..];
                        // (To its ")": the first outside quotes.)
                        let mut quote = None::<char>;
                        let close = rest
                            .char_indices()
                            .find(|&(_, c)| match quote {
                                Some(q) => {
                                    if c == q {
                                        quote = None;
                                    }
                                    false
                                }
                                None if c == '"' || c == '\'' => {
                                    quote = Some(c);
                                    false
                                }
                                None => c == ')',
                            })
                            .map_or(rest.len(), |(i, _)| i);
                        let address = unquote(&rest[..close]);
                        let format = lower.find("format(").map(|f| {
                            let r = &part[f + 7..];
                            unquote(&r[..r.find(')').unwrap_or(r.len())]).to_ascii_lowercase()
                        });
                        if !address.is_empty() {
                            face.sources.push((address, format.unwrap_or_default()));
                        }
                    }
                }
                "font-weight" => {
                    let ws: Vec<u16> = value
                        .split_ascii_whitespace()
                        .filter_map(|w| match w.to_ascii_lowercase().as_str() {
                            "normal" => Some(400),
                            "bold" => Some(700),
                            n => n.parse::<f32>().ok().map(|v| v.clamp(1.0, 1000.0) as u16),
                        })
                        .collect();
                    if let Some(&a) = ws.first() {
                        let b = ws.get(1).copied().unwrap_or(a);
                        face.weight = (a.min(b), a.max(b));
                    }
                }
                "font-stretch" | "font-width" => {
                    let ws: Vec<u16> = value.split_ascii_whitespace().filter_map(|w| stretch_value(&w.to_ascii_lowercase())).collect();
                    if let Some(&a) = ws.first() {
                        let b = ws.get(1).copied().unwrap_or(a);
                        face.stretch = (a.min(b), a.max(b));
                    }
                }
                "font-style" => {
                    let v = value.to_ascii_lowercase();
                    face.italic = v.starts_with("italic") || v.starts_with("oblique");
                }
                "unicode-range" => {
                    for r in value.split(',') {
                        let r = r.trim();
                        let Some(hex) = r.strip_prefix("U+").or_else(|| r.strip_prefix("u+")) else { continue };
                        let parse = |h: &str| u32::from_str_radix(h, 16).ok();
                        let range = match hex.split_once('-') {
                            Some((a, b)) => parse(a).zip(parse(b)),
                            None if hex.contains('?') => parse(&hex.replace('?', "0")).zip(parse(&hex.replace('?', "F"))),
                            None => parse(hex).map(|a| (a, a)),
                        };
                        if let Some((a, b)) = range {
                            face.ranges.push((a.min(b), a.max(b)));
                        }
                    }
                }
                _ => {}
            }
        }
        if !face.family.is_empty() && !face.sources.is_empty() {
            out.push(face);
        }
    }
    out
}

/// The page's window, for @media: its size in CSS pixels.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

/// A style sheet, read.
pub fn sheet(css: &str, view: Viewport) -> Sheet {
    let toks = closed(tokens(css));
    let mut s = Sheet::default();
    rules(&toks, view, &mut s, 0);
    s.fonts = font_faces(css);
    s
}

/// A style attribute's declarations.
pub fn inline_style(css: &str) -> Vec<Decl> {
    declarations(&closed(tokens(css)))
}

/// Tokens with what's still open at their end closed, as CSS closes blocks and functions at the end of a style sheet:
/// "color: rgb(0, 128, 0" is rgb(0, 128, 0).
fn closed(mut t: Vec<Tok>) -> Vec<Tok> {
    let mut open: Vec<Tok> = Vec::new();
    for x in &t {
        match x {
            Tok::OpenParen | Tok::Function(_) => open.push(Tok::CloseParen),
            Tok::OpenSquare => open.push(Tok::CloseSquare),
            Tok::OpenCurly => open.push(Tok::CloseCurly),
            Tok::CloseParen | Tok::CloseSquare | Tok::CloseCurly => {
                if open.last() == Some(x) {
                    open.pop();
                }
            }
            _ => {}
        }
    }
    while let Some(c) = open.pop() {
        t.push(c);
    }
    t
}

/// Where the block (or bracket) opened at `open` ends: the place of its closing token (or the end).
fn block_end(t: &[Tok], open: usize) -> usize {
    let mut depth = 0i32;
    let mut i = open;
    while i < t.len() {
        match t[i] {
            Tok::OpenCurly | Tok::OpenParen | Tok::OpenSquare | Tok::Function(_) => depth += 1,
            Tok::CloseCurly | Tok::CloseParen | Tok::CloseSquare => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    t.len()
}

fn rules(t: &[Tok], view: Viewport, out: &mut Sheet, depth: u32) {
    let mut i = 0;
    // (@import counts only at a sheet's top, before any rule but @charset and @layer's statements.)
    let mut imports_allowed = depth == 0;
    while i < t.len() {
        match &t[i] {
            Tok::Ws | Tok::Semicolon | Tok::CloseCurly => i += 1,
            Tok::AtKeyword(name) => {
                let keeps_imports = matches!(name.as_str(), "import" | "charset" | "layer");
                // Its prelude, up to ";" or its block.
                let start = i + 1;
                let mut j = start;
                while j < t.len() && !matches!(t[j], Tok::Semicolon | Tok::OpenCurly) {
                    if matches!(t[j], Tok::OpenParen | Tok::Function(_) | Tok::OpenSquare) {
                        j = block_end(t, j);
                    }
                    j += 1;
                }
                let prelude = &t[start..j.min(t.len())];
                if j < t.len() && t[j] == Tok::OpenCurly {
                    let end = block_end(t, j);
                    let inner = &t[(j + 1).min(t.len())..end.min(t.len())];
                    if depth < 16 {
                        match name.as_str() {
                            "media" => {
                                if media_matches(prelude, view) {
                                    rules(inner, view, out, depth + 1);
                                }
                            }
                            "supports" => {
                                if supports(prelude) {
                                    rules(inner, view, out, depth + 1);
                                }
                            }
                            "keyframes" | "-webkit-keyframes" | "-moz-keyframes" | "-o-keyframes" => {
                                if let Some(k) = keyframes(prelude, inner) {
                                    out.keyframes.push(k);
                                }
                            }
                            "counter-style" => {
                                if let [Tok::Ident(n)] = crate::values::trim(prelude) {
                                    out.counter_styles.push((n.clone(), declarations(inner)));
                                }
                            }
                            "layer" | "container" | "scope" | "document" | "-moz-document" | "starting-style" => {
                                if name != "starting-style" {
                                    rules(inner, view, out, depth + 1);
                                }
                            }
                            _ => {}
                        }
                    }
                    i = end + 1;
                } else {
                    if name == "import" && imports_allowed {
                        import(prelude, view, out);
                    }
                    i = j + 1;
                }
                if !keeps_imports || (name == "layer" && j < t.len() && t[j] == Tok::OpenCurly) {
                    imports_allowed = false;
                }
            }
            _ => {
                imports_allowed = false;
                // A style rule: its selectors, then its block.
                let start = i;
                let mut j = i;
                while j < t.len() && t[j] != Tok::OpenCurly {
                    if matches!(t[j], Tok::OpenParen | Tok::Function(_) | Tok::OpenSquare) {
                        j = block_end(t, j);
                    }
                    j += 1;
                }
                if j >= t.len() {
                    return;
                }
                let end = block_end(t, j);
                let prelude = &t[start..j];
                let block = &t[j + 1..end.min(t.len())];
                if let Some(selectors) = selector_list(prelude) {
                    let decls = declarations(block);
                    if !decls.is_empty() {
                        out.rules.push(Rule { selectors, decls });
                    }
                    // Rules nested in it (CSS nesting), their selectors relative to its.
                    nested(block, prelude, view, out, depth);
                }
                i = end + 1;
            }
        }
    }
}

/// Rules nested in a style rule's block (CSS nesting: "a { &:hover {...} .b & {...} }"), each read with "&" as its
/// parent's selectors (as :is() of them), or in front of them where it has none.
fn nested(block: &[Tok], parent: &[Tok], view: Viewport, out: &mut Sheet, depth: u32) {
    if depth >= 8 {
        return;
    }
    let mut i = 0;
    while i < block.len() {
        // Declarations end at ";"; a nested rule is a prelude and a block.
        let start = i;
        let mut j = i;
        while j < block.len() && !matches!(block[j], Tok::Semicolon | Tok::OpenCurly) {
            if matches!(block[j], Tok::OpenParen | Tok::Function(_) | Tok::OpenSquare) {
                j = block_end(block, j);
            }
            j += 1;
        }
        if j >= block.len() {
            return;
        }
        if block[j] == Tok::Semicolon {
            i = j + 1;
            continue;
        }
        let end = block_end(block, j);
        let prelude = &block[start..j];
        let inner = &block[j + 1..end.min(block.len())];
        let first = prelude.iter().find(|t| **t != Tok::Ws);
        // (A block with no selector before it is no rule: "{ color: red }" in a rule's block goes.)
        if first.is_none() {
            i = end + 1;
            continue;
        }
        if let Some(Tok::AtKeyword(name)) = first {
            if (name == "media" && media_matches(&prelude[1..], view)) || name == "supports" || name == "layer" {
                // Its declarations are the parent's, under the condition.
                let decls = declarations(inner);
                if !decls.is_empty()
                    && let Some(selectors) = selector_list(parent)
                {
                    out.rules.push(Rule { selectors, decls });
                }
                nested(inner, parent, view, out, depth + 1);
            }
        } else {
            let mut joined: Vec<Tok> = Vec::new();
            let has_amp = prelude.iter().any(|t| *t == Tok::Delim('&'));
            for (k, part) in split_commas(prelude).into_iter().enumerate() {
                if k > 0 {
                    joined.push(Tok::Comma);
                }
                let mut wrapped = alloc::vec![Tok::Colon, Tok::Function(String::from("is"))];
                wrapped.extend(parent.iter().cloned());
                wrapped.push(Tok::CloseParen);
                if has_amp {
                    for t in part {
                        if *t == Tok::Delim('&') {
                            joined.extend(wrapped.iter().cloned());
                        } else {
                            joined.push(t.clone());
                        }
                    }
                } else {
                    joined.extend(wrapped);
                    joined.push(Tok::Ws);
                    joined.extend(part.iter().cloned());
                }
            }
            if let Some(selectors) = selector_list(&joined) {
                let decls = declarations(inner);
                if !decls.is_empty() {
                    out.rules.push(Rule { selectors, decls });
                }
                nested(inner, &joined, view, out, depth + 1);
            }
        }
        i = end + 1;
    }
}

fn import(prelude: &[Tok], view: Viewport, out: &mut Sheet) {
    let mut it = prelude.iter().filter(|t| **t != Tok::Ws);
    let url = match it.next() {
        Some(Tok::Str(s)) | Some(Tok::Url(s)) => s.clone(),
        Some(Tok::Function(f)) if f == "url" => match it.next() {
            Some(Tok::Str(s)) => s.clone(),
            _ => return,
        },
        _ => return,
    };
    // (Its media, if it gives any, after the address.)
    let pos = prelude.iter().position(|t| matches!(t, Tok::Str(_) | Tok::Url(_))).map_or(prelude.len(), |p| p + 1);
    let rest = &prelude[pos.min(prelude.len())..];
    let media: Vec<Tok> = rest
        .iter()
        .skip_while(|t| matches!(t, Tok::CloseParen | Tok::Ws))
        .filter(|t| !matches!(t, Tok::Ident(i) if i.eq_ignore_ascii_case("layer")))
        .cloned()
        .collect();
    if media.iter().all(|t| *t == Tok::Ws) || media_matches(&media, view) {
        out.imports.push(url);
    }
}

/// A block's declarations ("name: value [!important]"), those that can't be read left out.
pub fn declarations(t: &[Tok]) -> Vec<Decl> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        let start = i;
        let mut j = i;
        let mut nested_block = false;
        while j < t.len() && t[j] != Tok::Semicolon {
            if t[j] == Tok::OpenCurly {
                // A nested rule: none of this is a declaration.
                nested_block = true;
                j = block_end(t, j) + 1;
                break;
            }
            if matches!(t[j], Tok::OpenParen | Tok::Function(_) | Tok::OpenSquare) {
                j = block_end(t, j);
            }
            j += 1;
        }
        if nested_block {
            i = j;
            continue;
        }
        if let Some(d) = declaration(&t[start..j.min(t.len())]) {
            out.push(d);
        }
        i = j + 1;
    }
    out
}

fn declaration(t: &[Tok]) -> Option<Decl> {
    let mut i = 0;
    while i < t.len() && t[i] == Tok::Ws {
        i += 1;
    }
    let Some(Tok::Ident(name)) = t.get(i) else { return None };
    i += 1;
    while i < t.len() && t[i] == Tok::Ws {
        i += 1;
    }
    if t.get(i) != Some(&Tok::Colon) {
        return None;
    }
    let mut value: Vec<Tok> = t[i + 1..].to_vec();
    if value.contains(&Tok::BadString) {
        return None;
    }
    // "!important" at its end.
    let mut important = false;
    while value.last() == Some(&Tok::Ws) {
        value.pop();
    }
    if let Some(Tok::Ident(last)) = value.last()
        && last.eq_ignore_ascii_case("important")
    {
        let mut k = value.len() - 1;
        while k > 0 && value[k - 1] == Tok::Ws {
            k -= 1;
        }
        if k > 0 && value[k - 1] == Tok::Delim('!') {
            important = true;
            value.truncate(k - 1);
        }
    }
    while value.first() == Some(&Tok::Ws) {
        value.remove(0);
    }
    while value.last() == Some(&Tok::Ws) {
        value.pop();
    }
    let name = if name.starts_with("--") { name.clone() } else { name.to_ascii_lowercase() };
    Some(Decl { name, value, important })
}

/// Tokens split at their top-level commas.
pub fn split_commas(t: &[Tok]) -> Vec<&[Tok]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < t.len() {
        match t[i] {
            Tok::Comma => {
                out.push(&t[start..i]);
                start = i + 1;
            }
            Tok::OpenParen | Tok::Function(_) | Tok::OpenSquare | Tok::OpenCurly => i = block_end(t, i),
            _ => {}
        }
        i += 1;
    }
    out.push(&t[start.min(t.len())..]);
    out
}

// ------------------------------------------------------------------------------------------------ @media, @supports

/// Whether a media query list matches the page's window (a comma between queries is "or").
pub fn media_matches(t: &[Tok], view: Viewport) -> bool {
    let t: Vec<Tok> = t.iter().filter(|x| **x != Tok::Ws).cloned().collect();
    if t.is_empty() {
        return true;
    }
    split_commas(&t).iter().any(|q| query(q, view))
}

fn query(q: &[Tok], view: Viewport) -> bool {
    let mut i = 0;
    let mut negate = false;
    if let Some(Tok::Ident(w)) = q.first() {
        match w.to_ascii_lowercase().as_str() {
            "not" => {
                negate = true;
                i = 1;
            }
            "only" => i = 1,
            _ => {}
        }
    }
    let mut ok = true;
    let mut first = true;
    while i < q.len() {
        match &q[i] {
            Tok::Ident(w) => {
                let w = w.to_ascii_lowercase();
                match w.as_str() {
                    "and" => {}
                    "or" => {
                        // ("or" between conditions: any of them.)
                        if ok {
                            return !negate;
                        }
                        ok = true;
                    }
                    "screen" | "all" => {}
                    "not" => {
                        // "not (condition)".
                        if let Some(Tok::OpenParen) = q.get(i + 1) {
                            let end = block_end(q, i + 1);
                            ok &= !feature(&q[i + 2..end.min(q.len())], view);
                            i = end;
                        }
                    }
                    _ => ok = false,
                }
                first = false;
            }
            Tok::OpenParen => {
                let end = block_end(q, i);
                ok &= feature(&q[i + 1..end.min(q.len())], view);
                i = end;
                first = false;
            }
            _ => {
                let _ = first;
                ok = false;
            }
        }
        i += 1;
    }
    ok != negate
}

/// A length in a media query, in CSS pixels.
fn media_length(t: &Tok) -> Option<f32> {
    match t {
        Tok::Dimension(n, u) => Some(match u.as_str() {
            "px" => *n,
            "em" | "rem" => n * 16.0,
            "pt" => n * 4.0 / 3.0,
            "cm" => n * 96.0 / 2.54,
            "mm" => n * 96.0 / 25.4,
            "in" => n * 96.0,
            _ => return None,
        }),
        Tok::Number(n) => Some(*n),
        _ => None,
    }
}

/// One media feature: "min-width: 600px", "width >= 600px", "400px < width <= 800px", "prefers-color-scheme: dark"...
fn feature(t: &[Tok], view: Viewport) -> bool {
    let t: Vec<Tok> = t.iter().filter(|x| **x != Tok::Ws).cloned().collect();
    if t.first().is_some_and(|x| matches!(x, Tok::OpenParen)) || t.iter().any(|x| matches!(x, Tok::Ident(w) if w == "and" || w == "or")) {
        // A nested condition: "((a) and (b))".
        return query(&t, view);
    }
    if let Some(colon) = t.iter().position(|x| *x == Tok::Colon) {
        let Some(Tok::Ident(name)) = t.first() else { return false };
        let name = name.to_ascii_lowercase();
        let value = t.get(colon + 1);
        let len = value.and_then(media_length);
        let ident = match value {
            Some(Tok::Ident(v)) => v.to_ascii_lowercase(),
            _ => String::new(),
        };
        let ratio = |w: f32, h: f32| -> Option<bool> {
            let (a, b) = match (t.get(colon + 1), t.get(colon + 3)) {
                (Some(Tok::Number(a)), Some(Tok::Number(b))) if t.get(colon + 2) == Some(&Tok::Delim('/')) => (*a, *b),
                (Some(Tok::Number(a)), _) => (*a, 1.0),
                _ => return None,
            };
            Some(w * b >= h * a)
        };
        return match name.as_str() {
            "min-width" | "min-device-width" => len.is_some_and(|l| view.width >= l),
            "max-width" | "max-device-width" => len.is_some_and(|l| view.width <= l),
            "width" | "device-width" => len.is_some_and(|l| view.width == l),
            "min-height" | "min-device-height" => len.is_some_and(|l| view.height >= l),
            "max-height" | "max-device-height" => len.is_some_and(|l| view.height <= l),
            "height" => len.is_some_and(|l| view.height == l),
            "orientation" => (ident == "landscape") == (view.width >= view.height),
            "prefers-color-scheme" => ident == "light",
            "prefers-reduced-motion" | "prefers-reduced-transparency" | "prefers-reduced-data" => ident == "no-preference",
            "prefers-contrast" => ident == "no-preference",
            "forced-colors" | "inverted-colors" => ident == "none",
            "hover" | "any-hover" => ident == "hover",
            "pointer" | "any-pointer" => ident == "fine",
            "scripting" => ident == "none",
            "display-mode" => ident == "browser",
            "color" | "min-color" => true,
            "monochrome" => matches!(value, Some(Tok::Number(n)) if *n == 0.0),
            "min-aspect-ratio" => ratio(view.width, view.height).unwrap_or(false),
            "max-aspect-ratio" => ratio(view.width, view.height).map(|r| !r).unwrap_or(false),
            "min-resolution" | "-webkit-min-device-pixel-ratio" | "min--moz-device-pixel-ratio" => match value {
                Some(Tok::Number(n)) => *n <= 1.0,
                Some(Tok::Dimension(n, u)) => match u.as_str() {
                    "dppx" | "x" => *n <= 1.0,
                    "dpi" => *n <= 96.0,
                    "dpcm" => *n <= 96.0 / 2.54,
                    _ => false,
                },
                _ => false,
            },
            "max-resolution" | "-webkit-max-device-pixel-ratio" => match value {
                Some(Tok::Number(n)) => *n >= 1.0,
                Some(Tok::Dimension(n, u)) => match u.as_str() {
                    "dppx" | "x" => *n >= 1.0,
                    "dpi" => *n >= 96.0,
                    _ => true,
                },
                _ => false,
            },
            _ => false,
        };
    }
    // Range syntax: "width >= 600px", "600px <= width", "400px < width <= 800px".
    let mut ok = true;
    let mut found = false;
    let mut k = 0;
    while k < t.len() {
        if let Tok::Ident(name) = &t[k] {
            let name = name.to_ascii_lowercase();
            let size = match name.as_str() {
                "width" => view.width,
                "height" => view.height,
                _ => return false,
            };
            found = true;
            // What's after it, then what's before it.
            if let Some((op, l)) = comparison(&t[k + 1..]) {
                ok &= compare(size, &op, l);
            }
            if k >= 1 {
                let before: Vec<Tok> = t[..k].to_vec();
                if let Some((op, l)) = comparison_before(&before) {
                    ok &= compare(l, &op, size);
                }
            }
        }
        k += 1;
    }
    found && ok
}

fn compare(a: f32, op: &str, b: f32) -> bool {
    match op {
        "<" => a < b,
        "<=" => a <= b,
        ">" => a > b,
        ">=" => a >= b,
        "=" => a == b,
        _ => false,
    }
}

/// An operator and a length after a feature's name ("<= 800px").
fn comparison(t: &[Tok]) -> Option<(String, f32)> {
    let mut op = String::new();
    let mut i = 0;
    while let Some(Tok::Delim(c)) = t.get(i) {
        op.push(*c);
        i += 1;
    }
    Some((op, media_length(t.get(i)?)?))
}

/// A length and an operator before a feature's name ("400px <").
fn comparison_before(t: &[Tok]) -> Option<(String, f32)> {
    let mut i = t.len();
    let mut op = String::new();
    while i > 0 {
        if let Tok::Delim(c) = t[i - 1] {
            op.insert(0, c);
            i -= 1;
        } else {
            break;
        }
    }
    Some((op, media_length(t.get(i.checked_sub(1)?)?)?))
}

/// Whether an @supports condition holds: everything is taken as supported but what "not" asks about and selector()s
/// that can't be read.
fn supports(t: &[Tok]) -> bool {
    let t: Vec<&Tok> = t.iter().filter(|x| **x != Tok::Ws).collect();
    match t.first() {
        Some(Tok::Ident(w)) if w.eq_ignore_ascii_case("not") => false,
        _ => true,
    }
}

// ------------------------------------------------------------------------------------------------ selectors

#[derive(Clone, Debug, PartialEq)]
pub enum Combinator {
    /// " "
    Descendant,
    /// ">"
    Child,
    /// "+"
    Next,
    /// "~"
    Later,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttrOp {
    Exists,
    Equals,
    Includes,
    Dash,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Clone, Debug)]
pub struct AttrSel {
    pub name: String,
    pub op: AttrOp,
    pub value: String,
    /// Whether the value's case doesn't matter (" i").
    pub ci: bool,
}

#[derive(Clone, Debug)]
pub enum Pseudo {
    FirstChild,
    LastChild,
    OnlyChild,
    /// :nth-child(an+b), and whether it counts from the end.
    Nth(i32, i32, bool),
    /// :nth-of-type(an+b) and the like, and whether it counts from the end.
    NthOfType(i32, i32, bool),
    OnlyOfType,
    Not(Vec<Selector>),
    /// :is() and :where() (the second counts nothing towards specificity), :matches(), :any().
    Is(Vec<Selector>),
    Has(Vec<Selector>),
    Root,
    Empty,
    Link,
    Checked,
    Disabled,
    Enabled,
    Required,
    Optional,
    PlaceholderShown,
    /// :popover-open: a popover showing.
    PopoverOpen,
    /// :host, :host(S): a shadow tree's host (from the tree's own style sheets).
    Host(Vec<Selector>),
    /// ::slotted(S): an element the host has that its shadow tree shows in a slot.
    Slotted(Vec<Selector>),
    /// What never matches here (:hover, :focus, :visited, :active, :target...).
    Never,
}

#[derive(Clone, Debug, Default)]
pub struct Compound {
    /// Its element's name (lower case), or None for any.
    pub tag: Option<String>,
    pub ids: Vec<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<AttrSel>,
    pub pseudos: Vec<Pseudo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PseudoElement {
    Before,
    After,
    FirstLetter,
    /// A list item's marker.
    Marker,
}

#[derive(Clone, Debug)]
pub struct Selector {
    /// Its compound selectors, left to right, and the combinators between them (one fewer).
    pub compounds: Vec<Compound>,
    pub combinators: Vec<Combinator>,
    /// (ids, classes and the like, elements' names), as one number: a << 20 | b << 10 | c.
    pub specificity: u32,
    pub pseudo_element: Option<PseudoElement>,
}

/// A selector list, or None if any of it can't be read (the rule is dropped then).
pub fn selector_list(t: &[Tok]) -> Option<Vec<Selector>> {
    let mut out = Vec::new();
    for part in split_commas(t) {
        out.push(selector(part)?);
    }
    if out.is_empty() { None } else { Some(out) }
}

/// A forgiving selector list (:is(), :where()): the parts that can be read.
fn forgiving(t: &[Tok]) -> Vec<Selector> {
    split_commas(t).into_iter().filter_map(selector).collect()
}

fn selector(t: &[Tok]) -> Option<Selector> {
    let mut i = 0;
    let mut compounds: Vec<Compound> = Vec::new();
    let mut combinators: Vec<Combinator> = Vec::new();
    let mut pending: Option<Combinator> = None;
    let mut cur: Option<Compound> = None;
    let mut pseudo_element = None;
    let mut spec = (0u32, 0u32, 0u32);
    // (A leading combinator, as nesting's relative selectors have, is a descendant of what's around.)
    while i < t.len() {
        match &t[i] {
            Tok::Ws => {
                if let Some(c) = cur.take() {
                    compounds.push(c);
                    pending = Some(Combinator::Descendant);
                }
                i += 1;
            }
            Tok::Delim(d) if matches!(*d, '>' | '+' | '~') => {
                if let Some(c) = cur.take() {
                    compounds.push(c);
                }
                if compounds.is_empty() {
                    // A relative selector (":has(> a)", or nesting's "> a"): from anything.
                    compounds.push(Compound::default());
                }
                pending = Some(match *d {
                    '>' => Combinator::Child,
                    '+' => Combinator::Next,
                    _ => Combinator::Later,
                });
                i += 1;
            }
            _ => {
                if pseudo_element.is_some() {
                    // Nothing comes after ::before but its own pseudo-classes, which never match here.
                    return None;
                }
                let c = cur.get_or_insert_with(Compound::default);
                if cur_is_new(c) && let Some(p) = pending.take() {
                    combinators.push(p);
                }
                let used = simple(&t[i..], c, &mut spec, &mut pseudo_element)?;
                i += used;
            }
        }
    }
    if let Some(c) = cur.take() {
        compounds.push(c);
    } else if pending.is_some() && pending != Some(Combinator::Descendant) {
        return None;
    }
    if compounds.is_empty() {
        return None;
    }
    // (Combinators pushed must be one fewer than compounds.)
    while combinators.len() + 1 > compounds.len() {
        combinators.pop();
    }
    while combinators.len() + 1 < compounds.len() {
        combinators.insert(0, Combinator::Descendant);
    }
    let specificity = (spec.0.min(1023) << 20) | (spec.1.min(1023) << 10) | spec.2.min(1023);
    Some(Selector { compounds, combinators, specificity, pseudo_element })
}

/// Whether nothing has been put in this compound yet (so the combinator before it is still to be noted).
fn cur_is_new(c: &Compound) -> bool {
    c.tag.is_none() && c.ids.is_empty() && c.classes.is_empty() && c.attrs.is_empty() && c.pseudos.is_empty()
}

/// One simple selector at the start of `t`, put in `c`: how many tokens it took, or None if it can't be read.
fn simple(t: &[Tok], c: &mut Compound, spec: &mut (u32, u32, u32), pe: &mut Option<PseudoElement>) -> Option<usize> {
    match t.first()? {
        Tok::Ident(name) => {
            c.tag = Some(name.to_ascii_lowercase());
            spec.2 += 1;
            Some(1)
        }
        Tok::Delim('*') => {
            // "*" and "*|*" and the like: any.
            Some(1)
        }
        Tok::Delim('&') => {
            // Nesting's "&" outside a nested rule: the page's root.
            c.pseudos.push(Pseudo::Root);
            Some(1)
        }
        Tok::Hash(id, true) => {
            c.ids.push(id.clone());
            spec.0 += 1;
            Some(1)
        }
        Tok::Delim('.') => match t.get(1) {
            Some(Tok::Ident(class)) => {
                c.classes.push(class.clone());
                spec.1 += 1;
                Some(2)
            }
            _ => None,
        },
        Tok::OpenSquare => {
            let end = block_end(t, 0);
            let inner: Vec<&Tok> = t[1..end.min(t.len())].iter().filter(|x| **x != Tok::Ws).collect();
            let name = match inner.first() {
                Some(Tok::Ident(n)) => n.to_ascii_lowercase(),
                _ => return None,
            };
            let mut k = 1;
            let op = match (inner.get(k), inner.get(k + 1)) {
                (None, _) => AttrOp::Exists,
                (Some(Tok::Delim('=')), _) => {
                    k += 1;
                    AttrOp::Equals
                }
                (Some(Tok::Delim(d)), Some(Tok::Delim('='))) => {
                    k += 2;
                    match d {
                        '~' => AttrOp::Includes,
                        '|' => AttrOp::Dash,
                        '^' => AttrOp::Prefix,
                        '$' => AttrOp::Suffix,
                        '*' => AttrOp::Substring,
                        _ => return None,
                    }
                }
                _ => return None,
            };
            let value = match inner.get(k) {
                Some(Tok::Ident(v)) | Some(Tok::Str(v)) => v.clone(),
                Some(Tok::Number(n)) => alloc::format!("{n}"),
                None if op == AttrOp::Exists => String::new(),
                _ => return None,
            };
            let ci = matches!(inner.get(k + 1), Some(Tok::Ident(f)) if f.eq_ignore_ascii_case("i"));
            c.attrs.push(AttrSel { name, op, value, ci });
            spec.1 += 1;
            Some(end + 1)
        }
        Tok::Colon => {
            // "::" a pseudo-element, ":" a pseudo-class.
            if t.get(1) == Some(&Tok::Colon) {
                let (name, used) = match t.get(2) {
                    Some(Tok::Ident(n)) => (n.to_ascii_lowercase(), 3),
                    Some(Tok::Function(n)) => (n.clone(), block_end(t, 2) + 1),
                    _ => return None,
                };
                spec.2 += 1;
                match name.as_str() {
                    "before" => *pe = Some(PseudoElement::Before),
                    "after" => *pe = Some(PseudoElement::After),
                    "first-letter" => *pe = Some(PseudoElement::FirstLetter),
                    // (A ::before's or ::after's own marker isn't styled here: never matched.)
                    "marker" if pe.is_some() => c.pseudos.push(Pseudo::Never),
                    "marker" => *pe = Some(PseudoElement::Marker),
                    "slotted" => {
                        // (What's slotted; what the compound before it says of the slot isn't checked.)
                        let end = block_end(t, 2);
                        let list = selector_list(&t[3..end.min(t.len())])?;
                        let best = list.iter().map(|s| s.specificity).max().unwrap_or(0);
                        spec.1 += (best >> 10) & 1023;
                        spec.2 += best & 1023;
                        *c = Compound { pseudos: alloc::vec![Pseudo::Slotted(list)], ..Compound::default() };
                    }
                    // What isn't drawn here (::placeholder, ::marker, ::selection...): never the element itself.
                    _ => c.pseudos.push(Pseudo::Never),
                }
                return Some(used);
            }
            match t.get(1)? {
                Tok::Ident(n) => {
                    let n = n.to_ascii_lowercase();
                    spec.1 += 1;
                    let p = match n.as_str() {
                        "first-child" => Pseudo::FirstChild,
                        "last-child" => Pseudo::LastChild,
                        "only-child" => Pseudo::OnlyChild,
                        "first-of-type" => Pseudo::NthOfType(0, 1, false),
                        "last-of-type" => Pseudo::NthOfType(0, 1, true),
                        "only-of-type" => Pseudo::OnlyOfType,
                        "root" => Pseudo::Root,
                        "empty" => Pseudo::Empty,
                        "link" | "any-link" => Pseudo::Link,
                        "checked" => Pseudo::Checked,
                        "disabled" => Pseudo::Disabled,
                        "enabled" => Pseudo::Enabled,
                        "required" => Pseudo::Required,
                        "optional" => Pseudo::Optional,
                        "placeholder-shown" => Pseudo::PlaceholderShown,
                        "popover-open" => Pseudo::PopoverOpen,
                        "host" => Pseudo::Host(Vec::new()),
                        // Old pseudo-elements written with one colon.
                        "before" | "after" => {
                            spec.1 -= 1;
                            spec.2 += 1;
                            *pe = Some(if n == "before" { PseudoElement::Before } else { PseudoElement::After });
                            return Some(2);
                        }
                        "first-letter" => {
                            spec.1 -= 1;
                            spec.2 += 1;
                            *pe = Some(PseudoElement::FirstLetter);
                            return Some(2);
                        }
                        "first-line" => Pseudo::Never,
                        "defined" | "scope" | "read-write" | "in-range" | "valid" => {
                            if n == "scope" {
                                Pseudo::Root
                            } else {
                                return Some(2);
                            }
                        }
                        _ => Pseudo::Never,
                    };
                    c.pseudos.push(p);
                    Some(2)
                }
                Tok::Function(f) => {
                    let end = block_end(t, 1);
                    let args = &t[2..end.min(t.len())];
                    let p = match f.as_str() {
                        "not" => {
                            let list = selector_list(args)?;
                            spec.1 += list.iter().map(|s| (s.specificity >> 10) & 1023).max().unwrap_or(0);
                            spec.0 += list.iter().map(|s| s.specificity >> 20).max().unwrap_or(0);
                            Pseudo::Not(list)
                        }
                        "is" | "matches" | "-webkit-any" | "-moz-any" | "any" => {
                            let list = forgiving(args);
                            let best = list.iter().map(|s| s.specificity).max().unwrap_or(0);
                            spec.0 += best >> 20;
                            spec.1 += (best >> 10) & 1023;
                            spec.2 += best & 1023;
                            Pseudo::Is(list)
                        }
                        "where" => Pseudo::Is(forgiving(args)),
                        "has" => {
                            let list = forgiving(args);
                            spec.1 += 1;
                            Pseudo::Has(list)
                        }
                        "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" => {
                            spec.1 += 1;
                            let (a, b) = an_plus_b(args)?;
                            let last = f.contains("last");
                            if f.ends_with("of-type") { Pseudo::NthOfType(a, b, last) } else { Pseudo::Nth(a, b, last) }
                        }
                        "host" => {
                            let list = selector_list(args)?;
                            let best = list.iter().map(|s| s.specificity).max().unwrap_or(0);
                            spec.1 += 1 + ((best >> 10) & 1023);
                            spec.2 += best & 1023;
                            Pseudo::Host(list)
                        }
                        "lang" | "dir" | "host-context" | "state" => {
                            spec.1 += 1;
                            Pseudo::Never
                        }
                        _ => {
                            spec.1 += 1;
                            Pseudo::Never
                        }
                    };
                    c.pseudos.push(p);
                    Some(end + 1)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// "an+b" in :nth-child(): odd, even, 3, 2n+1, -n+3, n... (an " of S" after it isn't kept.)
fn an_plus_b(t: &[Tok]) -> Option<(i32, i32)> {
    let mut s = String::new();
    for tok in t {
        match tok {
            Tok::Ws => {}
            Tok::Ident(i) if i.eq_ignore_ascii_case("of") => break,
            Tok::Ident(i) => s.push_str(&i.to_ascii_lowercase()),
            Tok::Number(n) => {
                if *n >= 0.0 && !s.is_empty() && !s.ends_with(['+', '-']) {
                    s.push('+');
                }
                s.push_str(&alloc::format!("{}", *n as i32));
            }
            Tok::Dimension(n, u) => {
                if *n >= 0.0 && !s.is_empty() && !s.ends_with(['+', '-']) {
                    s.push('+');
                }
                s.push_str(&alloc::format!("{}{u}", *n as i32));
            }
            Tok::Delim(c) => s.push(*c),
            _ => return None,
        }
    }
    match s.as_str() {
        "odd" => return Some((2, 1)),
        "even" => return Some((2, 0)),
        _ => {}
    }
    match s.find('n') {
        None => Some((0, s.parse().ok()?)),
        Some(at) => {
            let a = match &s[..at] {
                "" | "+" => 1,
                "-" => -1,
                n => n.parse().ok()?,
            };
            let rest = &s[at + 1..];
            let b = if rest.is_empty() { 0 } else { rest.trim_start_matches('+').parse().ok()? };
            Some((a, b))
        }
    }
}

/// A selector, boxed (for the recursive kinds' sake).
pub type BoxedSelector = Box<Selector>;
