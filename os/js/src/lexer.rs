//! JavaScript's tokens, read from a script's text: names (Unicode letters and escapes among them), keywords,
//! punctuation, numbers (decimal, hexadecimal, octal and binary, with separators, BigInts), strings (every escape),
//! template literals (their parts read as the parser comes to each), regular expressions (where an expression may
//! start: told apart from division by what came before), comments, and whether a line ended before each token (for
//! the automatic semicolons).

use crate::string::{is_line_terminator, is_space, Atom, Atoms, JsStr};
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum P {
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    Ellipsis,
    Semi,
    Comma,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    Plus,
    Minus,
    Star,
    Percent,
    StarStar,
    PlusPlus,
    MinusMinus,
    Shl,
    Shr,
    UShr,
    Amp,
    Pipe,
    Caret,
    Bang,
    Tilde,
    AmpAmp,
    PipePipe,
    QQ,
    Question,
    QDot,
    Colon,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    PercentAssign,
    StarStarAssign,
    ShlAssign,
    ShrAssign,
    UShrAssign,
    AmpAssign,
    PipeAssign,
    CaretAssign,
    AmpAmpAssign,
    PipePipeAssign,
    QQAssign,
    Arrow,
    Slash,
    SlashAssign,
    At,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Eof,
    /// A name or keyword, and whether it was written with an escape (then it can't be a keyword).
    Name(Atom, bool),
    /// `#name`.
    Private(Atom),
    Number(f64),
    /// A BigInt's digits (in base 10, as text) and sign-free.
    BigInt(String),
    /// A string, and whether it had a legacy octal escape (not allowed in strict code).
    Str(JsStr, bool),
    /// A template's part: its value (None where an escape is invalid: only allowed in tagged templates), its raw
    /// text, whether it's the last part, and whether it's the first (after "`": a template starts; else it goes on
    /// after a substitution's "}").
    Template(Option<JsStr>, JsStr, bool, bool),
    Regex(JsStr, String),
    Punct(P),
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub start: usize,
    pub end: usize,
    /// A line terminator came between the token before and this one.
    pub newline: bool,
}

#[derive(Debug, Clone)]
pub struct SyntaxError {
    pub message: String,
    pub at: usize,
}

/// Where a lexer is (`Lexer::state`).
#[derive(Clone, Debug)]
pub struct LexState {
    pos: usize,
    templates: Vec<u32>,
    regex_ok: bool,
}

pub struct Lexer<'a> {
    src: &'a str,
    pub pos: usize,
    /// Templates open: for each, how many braces are open in its current substitution.
    templates: Vec<u32>,
    /// Whether the token before allows a regular expression next (where an expression may start).
    regex_ok: bool,
}

fn is_id_start(c: char) -> bool {
    if c.is_ascii() {
        return c == '$' || c == '_' || c.is_ascii_alphabetic();
    }
    crate::unicode_id::within(&crate::unicode_id::ID_START, c as u32)
}

fn is_id_part(c: char) -> bool {
    if c.is_ascii() {
        return c == '$' || c == '_' || c.is_ascii_alphanumeric();
    }
    crate::unicode_id::within(&crate::unicode_id::ID_CONTINUE, c as u32)
}

/// Words after which an expression (so a regular expression) may start.
const BEFORE_EXPRESSION: [&str; 17] =
    ["return", "typeof", "instanceof", "in", "of", "new", "delete", "void", "throw", "case", "do", "else", "yield", "await", "extends", "export", "default"];

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Lexer<'a> {
        let mut l = Lexer { src, pos: 0, templates: Vec::new(), regex_ok: true };
        // (A hashbang line first is a comment.)
        if src.starts_with("#!") {
            l.pos = src.find(['\n', '\r', '\u{2028}', '\u{2029}']).unwrap_or(src.len());
        }
        l
    }

    fn err(&self, message: &str) -> SyntaxError {
        SyntaxError { message: String::from(message), at: self.pos }
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.src[self.pos..].chars().nth(n)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }

    /// Skips white space and comments; true if a line terminator was among them.
    fn skip(&mut self) -> Result<bool, SyntaxError> {
        let mut newline = false;
        let at_start = self.pos == 0;
        loop {
            let Some(c) = self.peek() else { break };
            if is_space(c as u32) || c == '\u{FEFF}' {
                self.pos += c.len_utf8();
            } else if is_line_terminator(c as u32) {
                self.pos += c.len_utf8();
                newline = true;
                // (An HTML-like closing comment at a line's start: the rest of the line.)
                let rest = &self.src[self.pos..];
                let trimmed = rest.trim_start_matches([' ', '\t']);
                if trimmed.starts_with("-->") {
                    self.pos += rest.len() - trimmed.len() + 3;
                    self.line_comment();
                }
            } else if self.src[self.pos..].starts_with("//") || self.src[self.pos..].starts_with("<!--") {
                self.line_comment();
            } else if self.src[self.pos..].starts_with("/*") {
                let Some(end) = self.src[self.pos + 2..].find("*/") else { return Err(self.err("a comment isn't closed")) };
                let body = &self.src[self.pos + 2..self.pos + 2 + end];
                if body.chars().any(|c| is_line_terminator(c as u32)) {
                    newline = true;
                }
                self.pos += 2 + end + 2;
            } else if at_start && self.pos == 0 && self.src.starts_with("-->") {
                self.line_comment();
            } else {
                break;
            }
        }
        Ok(newline)
    }

    fn line_comment(&mut self) {
        let rest = &self.src[self.pos..];
        let n = rest.find(['\n', '\r', '\u{2028}', '\u{2029}']).unwrap_or(rest.len());
        self.pos += n;
    }

    /// The next token.
    pub fn next(&mut self, atoms: &mut Atoms) -> Result<Token, SyntaxError> {
        let newline = self.skip()?;
        let start = self.pos;
        let tok = self.read(atoms)?;
        self.regex_ok = match &tok {
            Tok::Name(a, _) => {
                let s = atoms.get(*a);
                BEFORE_EXPRESSION.iter().any(|w| s.is(w))
            }
            Tok::Punct(p) => !matches!(p, P::RParen | P::RBracket | P::RBrace | P::PlusPlus | P::MinusMinus),
            Tok::Template(_, _, tail, _) => !*tail,
            _ => false,
        };
        Ok(Token { tok, start, end: self.pos, newline })
    }

    /// The parser says a regular expression may come here (after `)` of `if (...)`, say), or division.
    pub fn set_regex_ok(&mut self, ok: bool) {
        self.regex_ok = ok;
    }

    /// Reads again from `pos` (the parser went back), with a regular expression allowed or not.
    pub fn rewind(&mut self, pos: usize, regex_ok: bool) {
        self.pos = pos;
        self.regex_ok = regex_ok;
    }

    /// Where it is, to come back to (`restore`): a parser looking ahead.
    pub fn state(&self) -> LexState {
        LexState { pos: self.pos, templates: self.templates.clone(), regex_ok: self.regex_ok }
    }

    pub fn restore(&mut self, s: LexState) {
        self.pos = s.pos;
        self.templates = s.templates;
        self.regex_ok = s.regex_ok;
    }

    /// The source.
    pub fn source(&self) -> &'a str {
        self.src
    }

    fn read(&mut self, atoms: &mut Atoms) -> Result<Tok, SyntaxError> {
        let Some(c) = self.peek() else { return Ok(Tok::Eof) };
        if is_id_start(c) || c == '\\' {
            let (name, escaped) = self.name()?;
            return Ok(Tok::Name(atoms.intern_units(&name, None), escaped));
        }
        if c == '#' {
            self.pos += 1;
            let (name, _) = self.name()?;
            if name.is_empty() {
                return Err(self.err("# isn't followed by a name"));
            }
            return Ok(Tok::Private(atoms.intern_units(&name, None)));
        }
        if c.is_ascii_digit() || (c == '.' && self.peek_at(1).is_some_and(|d| d.is_ascii_digit())) {
            return self.number();
        }
        if c == '"' || c == '\'' {
            return self.string(c);
        }
        if c == '`' {
            self.pos += 1;
            return self.template(true);
        }
        if c == '}' && let Some(depth) = self.templates.last_mut() {
            if *depth == 0 {
                self.templates.pop();
                self.pos += 1;
                return self.template(false);
            }
            *depth -= 1;
        }
        if c == '{' && let Some(depth) = self.templates.last_mut() {
            *depth += 1;
        }
        if c == '/' && self.regex_ok {
            return self.regex();
        }
        self.punct()
    }

    /// A name (with \u escapes): its UTF-16 units, and whether it had an escape.
    fn name(&mut self) -> Result<(Vec<u16>, bool), SyntaxError> {
        let mut out: Vec<u16> = Vec::new();
        let mut escaped = false;
        let mut first = true;
        loop {
            let Some(c) = self.peek() else { break };
            let ch = if c == '\\' {
                self.pos += 1;
                if !self.eat('u') {
                    return Err(self.err("a name has a backslash that isn't \\u"));
                }
                escaped = true;
                let code = self.unicode_escape()?;
                let ch = char::from_u32(code).ok_or_else(|| self.err("a name's escape isn't a character"))?;
                if !(if first { is_id_start(ch) } else { is_id_part(ch) }) {
                    return Err(self.err("a name's escape isn't a letter of a name"));
                }
                ch
            } else if (first && is_id_start(c)) || (!first && is_id_part(c)) {
                self.pos += c.len_utf8();
                c
            } else {
                break;
            };
            let mut buf = [0u16; 2];
            out.extend_from_slice(ch.encode_utf16(&mut buf));
            first = false;
        }
        Ok((out, escaped))
    }

    /// After `\u`: XXXX or {X...}.
    fn unicode_escape(&mut self) -> Result<u32, SyntaxError> {
        if self.eat('{') {
            let mut v: u32 = 0;
            let mut n = 0;
            while let Some(c) = self.peek() {
                if c == '}' {
                    break;
                }
                let d = c.to_digit(16).ok_or_else(|| self.err("a \\u{...} escape has a letter that isn't hexadecimal"))?;
                v = v.saturating_mul(16).saturating_add(d);
                self.pos += 1;
                n += 1;
            }
            if !self.eat('}') || n == 0 || v > 0x10FFFF {
                return Err(self.err("a \\u{...} escape isn't right"));
            }
            Ok(v)
        } else {
            let mut v = 0;
            for _ in 0..4 {
                let d = self.peek().and_then(|c| c.to_digit(16)).ok_or_else(|| self.err("a \\u escape needs four hexadecimal digits"))?;
                v = v * 16 + d;
                self.pos += 1;
            }
            Ok(v)
        }
    }

    fn number(&mut self) -> Result<Tok, SyntaxError> {
        let start = self.pos;
        let c = self.bump().unwrap_or('0');
        // 0x, 0o, 0b.
        if c == '0' && let Some(k) = self.peek() {
            let radix = match k {
                'x' | 'X' => 16,
                'o' | 'O' => 8,
                'b' | 'B' => 2,
                _ => 0,
            };
            if radix != 0 {
                self.pos += 1;
                let digits = self.digits(radix)?;
                if digits.is_empty() {
                    return Err(self.err("a number has no digits after its prefix"));
                }
                if self.eat('n') {
                    return Ok(Tok::BigInt(big_to_decimal(&digits, radix)));
                }
                self.after_number()?;
                return Ok(Tok::Number(digits_value(&digits, radix)));
            }
            // A legacy octal number (017), or a decimal one with a leading zero (089).
            if k.is_ascii_digit() {
                let digits = self.digits(10)?;
                if digits.chars().all(|d| d < '8') {
                    self.after_number()?;
                    return Ok(Tok::Number(digits_value(&digits, 8)));
                }
                // (Decimal: on with the rest below.)
            }
        }
        // Decimal: digits, a fraction, an exponent.
        self.pos = start;
        let mut text = String::new();
        let int = self.digits(10)?;
        text.push_str(&int);
        if self.eat('n') {
            let t = int.trim_start_matches('0');
            return Ok(Tok::BigInt(String::from(if t.is_empty() { "0" } else { t })));
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            text.push('.');
            text.push_str(&self.digits(10)?);
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let save = self.pos;
            self.pos += 1;
            let mut exp = String::from("e");
            if let Some(s @ ('+' | '-')) = self.peek() {
                self.pos += 1;
                exp.push(s);
            }
            let d = self.digits(10)?;
            if d.is_empty() {
                self.pos = save;
                return Err(self.err("a number's exponent has no digits"));
            }
            exp.push_str(&d);
            text.push_str(&exp);
        }
        self.after_number()?;
        let text = if text.starts_with('.') { alloc::format!("0{text}") } else { text };
        let text = if text.ends_with('.') { alloc::format!("{text}0") } else { text };
        let v: f64 = text.parse().map_err(|_| self.err("a number isn't right"))?;
        Ok(Tok::Number(v))
    }

    /// Digits of `radix`, with `_` separators between them (taken out).
    fn digits(&mut self, radix: u32) -> Result<String, SyntaxError> {
        let mut out = String::new();
        let mut last_sep = false;
        while let Some(c) = self.peek() {
            if c == '_' {
                if out.is_empty() || last_sep {
                    return Err(self.err("a number's separator isn't between digits"));
                }
                last_sep = true;
                self.pos += 1;
                continue;
            }
            if !c.is_digit(radix) {
                break;
            }
            out.push(c);
            last_sep = false;
            self.pos += 1;
        }
        if last_sep {
            return Err(self.err("a number ends with a separator"));
        }
        Ok(out)
    }

    /// A number can't run straight into a name.
    fn after_number(&self) -> Result<(), SyntaxError> {
        match self.peek() {
            Some(c) if is_id_start(c) || c.is_ascii_digit() => Err(self.err("a number runs into a name")),
            _ => Ok(()),
        }
    }

    fn string(&mut self, quote: char) -> Result<Tok, SyntaxError> {
        self.pos += 1;
        let mut out: Vec<u16> = Vec::new();
        let mut octal = false;
        loop {
            let Some(c) = self.bump() else { return Err(self.err("a string isn't closed")) };
            if c == quote {
                break;
            }
            match c {
                '\\' => {
                    if let Some(v) = self.escape(&mut octal, false)? {
                        out.extend(v);
                    }
                }
                '\n' | '\r' => return Err(self.err("a string isn't closed on its line")),
                c => {
                    let mut buf = [0u16; 2];
                    out.extend_from_slice(c.encode_utf16(&mut buf));
                }
            }
        }
        Ok(Tok::Str(JsStr::from_units(out), octal))
    }

    /// After a backslash in a string or template: the units it stands for (None for a line continuation).
    /// `template`: octal escapes (but \0 alone) aren't allowed, and an invalid escape is an error the caller handles.
    fn escape(&mut self, octal: &mut bool, template: bool) -> Result<Option<Vec<u16>>, SyntaxError> {
        let Some(c) = self.bump() else { return Err(self.err("a string ends with a backslash")) };
        let unit = |u: u32| -> Vec<u16> {
            match char::from_u32(u) {
                Some(ch) => {
                    let mut buf = [0u16; 2];
                    ch.encode_utf16(&mut buf).to_vec()
                }
                // (A lone surrogate, written as an escape: kept as it is.)
                None => alloc::vec![u as u16],
            }
        };
        Ok(Some(match c {
            'n' => alloc::vec![10],
            't' => alloc::vec![9],
            'r' => alloc::vec![13],
            'b' => alloc::vec![8],
            'f' => alloc::vec![12],
            'v' => alloc::vec![11],
            '\r' => {
                self.eat('\n');
                return Ok(None);
            }
            '\n' | '\u{2028}' | '\u{2029}' => return Ok(None),
            'x' => {
                let mut v = 0;
                for _ in 0..2 {
                    let d = self.peek().and_then(|c| c.to_digit(16)).ok_or_else(|| self.err("a \\x escape needs two hexadecimal digits"))?;
                    v = v * 16 + d;
                    self.pos += 1;
                }
                alloc::vec![v as u16]
            }
            'u' => {
                let v = self.unicode_escape()?;
                if v > 0xFFFF {
                    unit(v)
                } else {
                    alloc::vec![v as u16]
                }
            }
            '0' if !self.peek().is_some_and(|d| d.is_ascii_digit()) => alloc::vec![0],
            '0'..='7' => {
                if template {
                    return Err(self.err("an octal escape in a template"));
                }
                *octal = true;
                let mut v = c.to_digit(8).unwrap_or(0);
                let max_len = if c <= '3' { 3 } else { 2 };
                for _ in 1..max_len {
                    match self.peek().and_then(|d| d.to_digit(8)) {
                        Some(d) => {
                            v = v * 8 + d;
                            self.pos += 1;
                        }
                        None => break,
                    }
                }
                alloc::vec![v as u16]
            }
            '8' | '9' => {
                if template {
                    return Err(self.err("\\8 or \\9 in a template"));
                }
                *octal = true;
                alloc::vec![c as u16]
            }
            c => unit(c as u32),
        }))
    }

    /// A template's part, after "`" or the "}" that ends a substitution: up to "${" (pushed: a substitution
    /// follows) or "`".
    fn template(&mut self, head: bool) -> Result<Tok, SyntaxError> {
        let mut cooked: Option<Vec<u16>> = Some(Vec::new());
        let mut raw: Vec<u16> = Vec::new();
        loop {
            let Some(c) = self.peek() else { return Err(self.err("a template isn't closed")) };
            if c == '`' {
                self.pos += 1;
                return Ok(Tok::Template(cooked.map(JsStr::from_units), JsStr::from_units(raw), true, head));
            }
            if c == '$' && self.peek_at(1) == Some('{') {
                self.pos += 2;
                self.templates.push(0);
                return Ok(Tok::Template(cooked.map(JsStr::from_units), JsStr::from_units(raw), false, head));
            }
            if c == '\\' {
                let at = self.pos;
                self.pos += 1;
                let mut octal = false;
                match self.escape(&mut octal, true) {
                    Ok(v) => {
                        if let (Some(cv), Some(v)) = (cooked.as_mut(), v) {
                            cv.extend(v);
                        }
                    }
                    Err(_) => {
                        // (Not a valid escape: no value, only the raw text; the parser refuses it but in a tag.)
                        cooked = None;
                        self.pos = at + 1;
                        if let Some(ch) = self.bump() {
                            let _ = ch;
                        }
                    }
                }
                // The raw text: as written, line terminators made \n.
                let written = &self.src[at..self.pos];
                push_raw(&mut raw, written);
                continue;
            }
            self.pos += c.len_utf8();
            if c == '\r' {
                self.eat('\n');
                if let Some(cv) = cooked.as_mut() {
                    cv.push(10);
                }
                raw.push(10);
                continue;
            }
            let mut buf = [0u16; 2];
            let u = c.encode_utf16(&mut buf);
            if let Some(cv) = cooked.as_mut() {
                cv.extend_from_slice(u);
            }
            raw.extend_from_slice(u);
        }
    }

    fn regex(&mut self) -> Result<Tok, SyntaxError> {
        self.pos += 1;
        let start = self.pos;
        let mut class = false;
        loop {
            let Some(c) = self.bump() else { return Err(self.err("a regular expression isn't closed")) };
            match c {
                '\\' => {
                    match self.bump() {
                        Some(n) if !is_line_terminator(n as u32) => {}
                        _ => return Err(self.err("a regular expression isn't closed")),
                    }
                }
                '[' => class = true,
                ']' => class = false,
                '/' if !class => break,
                c if is_line_terminator(c as u32) => return Err(self.err("a regular expression isn't closed on its line")),
                _ => {}
            }
        }
        let body = JsStr::from_str(&self.src[start..self.pos - 1]);
        let mut flags = String::new();
        while let Some(c) = self.peek() {
            if !is_id_part(c) {
                break;
            }
            flags.push(c);
            self.pos += c.len_utf8();
        }
        Ok(Tok::Regex(body, flags))
    }

    fn punct(&mut self) -> Result<Tok, SyntaxError> {
        let rest = &self.src[self.pos..];
        const TABLE: [(&str, P); 57] = [
            (">>>=", P::UShrAssign),
            ("...", P::Ellipsis),
            ("===", P::StrictEq),
            ("!==", P::StrictNe),
            ("**=", P::StarStarAssign),
            ("<<=", P::ShlAssign),
            (">>=", P::ShrAssign),
            (">>>", P::UShr),
            ("&&=", P::AmpAmpAssign),
            ("||=", P::PipePipeAssign),
            ("??=", P::QQAssign),
            ("=>", P::Arrow),
            ("==", P::Eq),
            ("!=", P::Ne),
            ("<=", P::Le),
            (">=", P::Ge),
            ("**", P::StarStar),
            ("++", P::PlusPlus),
            ("--", P::MinusMinus),
            ("<<", P::Shl),
            (">>", P::Shr),
            ("&&", P::AmpAmp),
            ("||", P::PipePipe),
            ("??", P::QQ),
            ("+=", P::PlusAssign),
            ("-=", P::MinusAssign),
            ("*=", P::StarAssign),
            ("%=", P::PercentAssign),
            ("&=", P::AmpAssign),
            ("|=", P::PipeAssign),
            ("^=", P::CaretAssign),
            ("/=", P::SlashAssign),
            ("{", P::LBrace),
            ("}", P::RBrace),
            ("(", P::LParen),
            (")", P::RParen),
            ("[", P::LBracket),
            ("]", P::RBracket),
            (".", P::Dot),
            (";", P::Semi),
            (",", P::Comma),
            ("<", P::Lt),
            (">", P::Gt),
            ("+", P::Plus),
            ("-", P::Minus),
            ("*", P::Star),
            ("%", P::Percent),
            ("&", P::Amp),
            ("|", P::Pipe),
            ("^", P::Caret),
            ("!", P::Bang),
            ("~", P::Tilde),
            ("?", P::Question),
            (":", P::Colon),
            ("=", P::Assign),
            ("/", P::Slash),
            ("@", P::At),
        ];
        // "?." isn't optional chaining before a digit (a ? .5 : b).
        if rest.starts_with("?.") && !rest[2..].starts_with(|c: char| c.is_ascii_digit()) {
            self.pos += 2;
            return Ok(Tok::Punct(P::QDot));
        }
        for (s, p) in TABLE {
            if rest.starts_with(s) {
                self.pos += s.len();
                return Ok(Tok::Punct(p));
            }
        }
        Err(self.err(&alloc::format!("a character JavaScript doesn't have here: {:?}", rest.chars().next().unwrap_or(' '))))
    }
}

/// A template's raw text as written (a CR or CRLF taken as LF).
fn push_raw(raw: &mut Vec<u16>, written: &str) {
    let mut chars = written.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            raw.push(10);
            continue;
        }
        let mut buf = [0u16; 2];
        raw.extend_from_slice(c.encode_utf16(&mut buf));
    }
}

/// Digits of `radix` as a number (rounded as JavaScript rounds a long literal: to the nearest double).
fn digits_value(digits: &str, radix: u32) -> f64 {
    // (Exactly while it fits in 128 bits; beyond, in floating point.)
    let mut v: u128 = 0;
    let mut f: f64 = 0.0;
    let mut exact = true;
    for c in digits.chars() {
        let d = c.to_digit(radix).unwrap_or(0);
        if exact {
            match v.checked_mul(radix as u128).and_then(|x| x.checked_add(d as u128)) {
                Some(n) => v = n,
                None => {
                    exact = false;
                    f = v as f64 * radix as f64 + d as f64;
                }
            }
        } else {
            f = f * radix as f64 + d as f64;
        }
    }
    if exact { v as f64 } else { f }
}

/// A BigInt literal's digits in another radix, as decimal text.
fn big_to_decimal(digits: &str, radix: u32) -> String {
    // Base-10^9 limbs, least first.
    let mut limbs: Vec<u64> = alloc::vec![0];
    for c in digits.chars() {
        let mut carry = c.to_digit(radix).unwrap_or(0) as u64;
        for l in limbs.iter_mut() {
            let v = *l * radix as u64 + carry;
            *l = v % 1_000_000_000;
            carry = v / 1_000_000_000;
        }
        while carry > 0 {
            limbs.push(carry % 1_000_000_000);
            carry /= 1_000_000_000;
        }
    }
    let mut out = String::new();
    for (i, l) in limbs.iter().rev().enumerate() {
        if i == 0 {
            out.push_str(&alloc::format!("{l}"));
        } else {
            out.push_str(&alloc::format!("{l:09}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        let mut atoms = Atoms::new();
        let mut l = Lexer::new(src);
        let mut out = Vec::new();
        loop {
            let t = l.next(&mut atoms).unwrap();
            if t.tok == Tok::Eof {
                break;
            }
            out.push(t.tok);
        }
        out
    }

    #[test]
    fn numbers() {
        assert_eq!(toks("0x1F 0o17 0b101 1_000 .5 5. 1e3 017 089 1.5e-2"), [
            Tok::Number(31.0),
            Tok::Number(15.0),
            Tok::Number(5.0),
            Tok::Number(1000.0),
            Tok::Number(0.5),
            Tok::Number(5.0),
            Tok::Number(1000.0),
            Tok::Number(15.0),
            Tok::Number(89.0),
            Tok::Number(0.015)
        ]);
        assert_eq!(toks("0x10n 123n"), [Tok::BigInt("16".into()), Tok::BigInt("123".into())]);
    }

    #[test]
    fn regex_or_division() {
        let t = toks("a / b / c; x = /ab+c/gi; f(/[/]/)");
        assert!(matches!(t[1], Tok::Punct(P::Slash)));
        assert!(t.iter().any(|t| matches!(t, Tok::Regex(b, f) if b.to_rust() == "ab+c" && f == "gi")));
        assert!(t.iter().any(|t| matches!(t, Tok::Regex(b, _) if b.to_rust() == "[/]")));
    }

    #[test]
    fn strings_and_templates() {
        let t = toks(r#"'a\nb' "\x41B\u{1F600}" `x${1 + {a: 2}.a}y${`z`}`"#);
        assert!(matches!(&t[0], Tok::Str(s, false) if s.units() == [97, 10, 98]));
        assert!(matches!(&t[1], Tok::Str(s, false) if s.to_rust() == "AB😀"));
        assert!(matches!(&t[2], Tok::Template(Some(s), _, false, true) if s.to_rust() == "x"));
        assert!(t.iter().any(|t| matches!(t, Tok::Template(Some(s), _, false, false) if s.to_rust() == "y")));
        assert!(matches!(t.last(), Some(Tok::Template(Some(s), _, true, false)) if s.is_empty()));
    }

    #[test]
    fn comments_and_newlines() {
        let mut atoms = Atoms::new();
        let mut l = Lexer::new("a /* x\n */ b // c\nd");
        let a = l.next(&mut atoms).unwrap();
        let b = l.next(&mut atoms).unwrap();
        let d = l.next(&mut atoms).unwrap();
        assert!(!a.newline && b.newline && d.newline);
    }
}
