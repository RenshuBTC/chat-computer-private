//! A page's words, from its content streams (ISO 32000-1, 9.4 and 8.4): the text it shows, each string's codes as
//! text by its font (font.rs). Where each string is on the page is followed (the text matrix through Td, TD, T*, Tm
//! and TJ's adjustments, each glyph's width, and the graphics' own matrix through cm, q and Q): a string whose baseline
//! is well above or below the last one's starts a new line; a line's strings are put left to right, with a space
//! where one ends well before the next starts. A line further below the last than the page's lines usually are, or
//! above it, starts a new paragraph (after a blank line). Right-to-left words (Hebrew's, Arabic's), drawn as they're
//! seen, are put in the order they're read. Form XObjects' text is read where they're drawn (a few deep). Pictures
//! are passed over.

use crate::file::File;
use crate::font::Font;
use crate::lex::{Dict, Lexer, Obj};
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

/// How deep forms may be drawn inside forms.
const MOST_FORMS: u32 = 8;
/// The most work done on a page, and in a whole document (a page that's a huge drawing stops there): an operation's
/// one, and looking through a list of fonts or forms as many as it has.
const MOST_OPS: usize = 4_000_000;
const MOST_OPS_ALL: usize = 150_000_000;
/// The most fonts kept read (beyond, they're read again).
const MOST_FONTS: usize = 512;
/// The most pieces a line has (more are another line), and the most lines a page has.
const MOST_PIECES: usize = 10_000;
const MOST_LINES: usize = 100_000;
/// The most bytes of forms' contents kept decoded (a form drawn on every page is decoded once).
const MOST_FORM_BYTES: usize = 32 << 20;

type Matrix = [f64; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `a` then `b`: a × b.
fn times(a: &Matrix, b: &Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

#[derive(Clone)]
struct Graphics {
    ctm: Matrix,
    font: Option<Rc<Font>>,
    size: f64,
    char_space: f64,
    word_space: f64,
    scale: f64,
    leading: f64,
    rise: f64,
}

/// A document's pages being read: the page's lines so far (each with how far below the last it is, in its letters'
/// heights), and its line being read; the fonts read so far (those that are objects of their own kept for every page, by their
/// number, and those written in a page's resources for that page, by name).
pub struct Reader<'f, 'a> {
    file: &'f File<'a>,
    lines: Vec<(f64, String)>,
    line: Vec<Piece>,
    gap: f64,
    fonts: BTreeMap<Vec<u8>, Rc<Font>>,
    forms: BTreeMap<u32, Rc<Vec<u8>>>,
    form_bytes: usize,
    ops: usize,
    ops_all: usize,
}

impl<'f, 'a> Reader<'f, 'a> {
    pub fn new(file: &'f File<'a>) -> Reader<'f, 'a> {
        Reader {
            file,
            lines: Vec::new(),
            line: Vec::new(),
            gap: 0.0,
            fonts: BTreeMap::new(),
            forms: BTreeMap::new(),
            form_bytes: 0,
            ops: 0,
            ops_all: 0,
        }
    }

    /// A page's text, read from its contents (with its resources).
    pub fn page(&mut self, contents: &[u8], resources: &Dict) -> String {
        self.lines.clear();
        self.line.clear();
        self.gap = 0.0;
        self.ops_all += self.ops;
        self.ops = 0;
        if self.ops_all > MOST_OPS_ALL {
            return String::new();
        }
        self.fonts.retain(|k, _| !k.starts_with(b"/"));
        let g = Graphics { ctm: IDENTITY, font: None, size: 0.0, char_space: 0.0, word_space: 0.0, scale: 1.0, leading: 0.0, rise: 0.0 };
        self.run(contents, resources, g, 0);
        self.end_line();
        // (How far apart the page's lines usually are: further, or upwards, is a new paragraph.)
        let mut downs: Vec<f64> = self.lines.iter().skip(1).map(|l| l.0).filter(|&g| g > 0.6).collect();
        downs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
        let usual = downs.get(downs.len() / 2).copied().unwrap_or(1.2);
        let paragraph = (usual * 1.35).max(usual + 0.45);
        let mut out = String::new();
        for (i, (gap, text)) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push_str(if *gap > paragraph || *gap < -0.6 { "\n\n" } else { "\n" });
            }
            out.push_str(text);
        }
        tidy(&out)
    }

    fn font(&mut self, resources: &Dict, name: &[u8]) -> Rc<Font> {
        let fonts = self.file.get_in(resources, "Font");
        self.ops += fonts.dict().map_or(0, |d| d.0.len());
        let entry = fonts.dict().and_then(|d| d.0.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.clone()));
        // (Fonts are kept by what they are, a reference's number, so one font named alike in two resources isn't
        // mistaken for the other.)
        let key: Vec<u8> = match &entry {
            Some(Obj::Ref(n, g)) => alloc::format!("{n} {g} R").into_bytes(),
            _ => {
                let mut k = b"/".to_vec();
                k.extend_from_slice(name);
                k
            }
        };
        if let Some(f) = self.fonts.get(&key) {
            return f.clone();
        }
        let font = Rc::new(match entry.map(|e| self.file.resolve(&e)) {
            Some(Obj::Dict(d)) => Font::load(self.file, &d),
            _ => Font::fallback(),
        });
        if self.fonts.len() >= MOST_FONTS {
            self.fonts.clear();
        }
        self.fonts.insert(key, font.clone());
        font
    }

    fn run(&mut self, contents: &[u8], resources: &Dict, mut g: Graphics, depth: u32) {
        let mut saved: Vec<Graphics> = Vec::new();
        // (Fonts by their names in these resources, as they've been found.)
        let mut named: BTreeMap<Vec<u8>, Rc<Font>> = BTreeMap::new();
        let (mut tm, mut tlm) = (IDENTITY, IDENTITY);
        let mut args: Vec<Obj> = Vec::new();
        let mut lex = Lexer::new(contents, 0);
        loop {
            let at = lex.pos;
            let Some(o) = lex.object() else { break };
            if lex.pos == at {
                break;
            }
            let Obj::Op(op) = o else {
                if args.len() < 64 {
                    args.push(o);
                }
                continue;
            };
            self.ops += 1;
            if self.ops > MOST_OPS {
                break;
            }
            let num = |i: usize| args.get(i).and_then(Obj::num).unwrap_or(0.0);
            match op.as_slice() {
                b"q" => {
                    if saved.len() < 64 {
                        saved.push(g.clone());
                    }
                }
                b"Q" => {
                    if let Some(s) = saved.pop() {
                        g = s;
                    }
                }
                b"cm" if args.len() >= 6 => {
                    let m = [num(0), num(1), num(2), num(3), num(4), num(5)];
                    g.ctm = times(&m, &g.ctm);
                }
                b"BT" => {
                    tm = IDENTITY;
                    tlm = IDENTITY;
                }
                b"Tf" if args.len() >= 2 => {
                    if let Some(name) = args[0].name() {
                        let font = match named.get(name) {
                            Some(f) => f.clone(),
                            None => {
                                let name = name.to_vec();
                                let f = self.font(resources, &name);
                                if named.len() < MOST_FONTS {
                                    named.insert(name, f.clone());
                                }
                                f
                            }
                        };
                        g.font = Some(font);
                    }
                    g.size = num(1);
                }
                b"Tc" => g.char_space = num(0),
                b"Tw" => g.word_space = num(0),
                b"Tz" => g.scale = num(0) / 100.0,
                b"TL" => g.leading = num(0),
                b"Ts" => g.rise = num(0),
                b"Td" => {
                    tlm = times(&[1.0, 0.0, 0.0, 1.0, num(0), num(1)], &tlm);
                    tm = tlm;
                }
                b"TD" => {
                    g.leading = -num(1);
                    tlm = times(&[1.0, 0.0, 0.0, 1.0, num(0), num(1)], &tlm);
                    tm = tlm;
                }
                b"Tm" if args.len() >= 6 => {
                    tlm = [num(0), num(1), num(2), num(3), num(4), num(5)];
                    tm = tlm;
                }
                b"T*" => {
                    tlm = times(&[1.0, 0.0, 0.0, 1.0, 0.0, -g.leading], &tlm);
                    tm = tlm;
                }
                b"Tj" => {
                    if let Some(s) = args.first().and_then(Obj::string) {
                        let s = s.to_vec();
                        self.show(&s, &g, &mut tm);
                    }
                }
                b"'" => {
                    tlm = times(&[1.0, 0.0, 0.0, 1.0, 0.0, -g.leading], &tlm);
                    tm = tlm;
                    if let Some(s) = args.first().and_then(Obj::string) {
                        let s = s.to_vec();
                        self.show(&s, &g, &mut tm);
                    }
                }
                b"\"" => {
                    g.word_space = num(0);
                    g.char_space = num(1);
                    tlm = times(&[1.0, 0.0, 0.0, 1.0, 0.0, -g.leading], &tlm);
                    tm = tlm;
                    if let Some(s) = args.get(2).and_then(Obj::string) {
                        let s = s.to_vec();
                        self.show(&s, &g, &mut tm);
                    }
                }
                b"TJ" => {
                    if let Some(items) = args.first().and_then(Obj::array) {
                        let items = items.to_vec();
                        for item in &items {
                            match item {
                                Obj::Str(s) => self.show(s, &g, &mut tm),
                                o => {
                                    if let Some(n) = o.num() {
                                        let tx = -n / 1000.0 * g.size * g.scale;
                                        tm = times(&[1.0, 0.0, 0.0, 1.0, tx, 0.0], &tm);
                                    }
                                }
                            }
                        }
                    }
                }
                b"Do" if depth < MOST_FORMS => {
                    if let Some(name) = args.first().and_then(Obj::name) {
                        let name = name.to_vec();
                        self.form(resources, &name, &g, depth);
                    }
                }
                b"BI" => skip_inline_image(&mut lex),
                _ => {}
            }
            args.clear();
        }
    }

    /// A Form XObject drawn with Do: its contents read with its resources (else the page's), its matrix applied.
    fn form(&mut self, resources: &Dict, name: &[u8], g: &Graphics, depth: u32) {
        let objects = self.file.get_in(resources, "XObject");
        self.ops += objects.dict().map_or(0, |d| d.0.len());
        let Some(entry) = objects.dict().and_then(|d| d.0.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.clone())) else {
            return;
        };
        let form = self.file.resolve(&entry);
        let Obj::Stream(dict, _) = &form else { return };
        if self.file.get_in(dict, "Subtype").name() != Some(b"Form") {
            return;
        }
        let number = match entry {
            Obj::Ref(n, _) => Some(n),
            _ => None,
        };
        let data = match number.and_then(|n| self.forms.get(&n)) {
            Some(d) => d.clone(),
            None => {
                let Some(d) = self.file.stream(&form) else { return };
                let d = Rc::new(d);
                if let Some(n) = number
                    && self.form_bytes + d.len() <= MOST_FORM_BYTES
                {
                    self.form_bytes += d.len();
                    self.forms.insert(n, d.clone());
                }
                d
            }
        };
        let own = self.file.get_in(dict, "Resources");
        let res = own.dict().cloned().unwrap_or_else(|| resources.clone());
        let mut inner = g.clone();
        if let Some(m) = self.file.get_in(dict, "Matrix").array() {
            let v: Vec<f64> = m.iter().map(|x| x.num().unwrap_or(0.0)).collect();
            if v.len() == 6 {
                inner.ctm = times(&[v[0], v[1], v[2], v[3], v[4], v[5]], &g.ctm);
            }
        }
        self.run(&data, &res, inner, depth + 1);
    }

    /// A string shown: its codes' text, as a piece of the line it's on (a string whose baseline is well above or below
    /// the last one's starts a new line); the text matrix moved on by each glyph's width.
    fn show(&mut self, s: &[u8], g: &Graphics, tm: &mut Matrix) {
        let Some(font) = g.font.clone() else { return };
        let start = times(&times(&[g.size * g.scale, 0.0, 0.0, g.size, 0.0, g.rise], tm), &g.ctm);
        let size = sqrt(start[2] * start[2] + start[3] * start[3]).max(0.001);
        let (x, y) = (start[4], start[5]);
        if let Some(last) = self.line.last() {
            let height = size.max(last.size);
            let below = last.y - y;
            if below.abs() > height * 0.6 {
                self.end_line();
                self.gap = below / height;
            }
        }
        let mut text = String::new();
        for (code, single) in font.codes(s) {
            match font.text(code) {
                Some(t) => push(&mut text, &t),
                None => {
                    if let Some(c) = font.utf16(code) {
                        let mut b = [0u8; 4];
                        push(&mut text, c.encode_utf8(&mut b));
                    }
                }
            }
            let w = font.width(code) / 1000.0;
            let tx = (w * g.size + g.char_space + if single { g.word_space } else { 0.0 }) * g.scale;
            *tm = times(&[1.0, 0.0, 0.0, 1.0, tx, 0.0], tm);
        }
        let end = times(&times(&[g.size * g.scale, 0.0, 0.0, g.size, 0.0, g.rise], tm), &g.ctm)[4];
        if self.line.len() >= MOST_PIECES {
            self.end_line();
        }
        // (A string drawn leftwards, as on a mirrored page: its ends the other way round.)
        let (x0, x1) = if end >= x { (x, end) } else { (end, x) };
        self.line.push(Piece { x0, x1, y, size, text });
    }

    /// The line's pieces written out, left to right (programs draw a line's parts in any order: a run in another
    /// font, a table's cells), a space where one piece ends well before the next starts; a piece drawn again over
    /// itself (as programs draw text bold) once.
    fn end_line(&mut self) {
        if self.line.is_empty() {
            return;
        }
        let mut pieces = core::mem::take(&mut self.line);
        pieces.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(core::cmp::Ordering::Equal));
        let mut text = String::new();
        let mut kept: Option<usize> = None;
        for (i, p) in pieces.iter().enumerate() {
            if let Some(k) = kept {
                let q = &pieces[k];
                if p.text == q.text && (p.x0 - q.x0).abs() < p.size.max(q.size) * 0.3 {
                    continue;
                }
                if p.x0 - q.x1 > p.size.max(q.size) * 0.15 && !text.ends_with(' ') && !p.text.starts_with(' ') {
                    text.push(' ');
                }
            }
            text.push_str(&p.text);
            kept = Some(i);
        }
        self.line = pieces;
        self.line.clear();
        if !text.trim().is_empty() && self.lines.len() < MOST_LINES {
            self.lines.push((self.gap, in_reading_order(&text)));
        }
    }
}

/// A piece of text where it was shown on the page: where it starts and ends along its line, its baseline, and its
/// font's size there.
struct Piece {
    x0: f64,
    x1: f64,
    y: f64,
    size: f64,
    text: String,
}

/// Text into a piece: ligatures as their letters, a few symbol fonts' bullets as bullets, tabs and no-break spaces as
/// spaces; controls and the "no character" mark left out.
fn push(out: &mut String, t: &str) {
    for c in t.chars() {
        match c {
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{FB06}' => out.push_str("st"),
            // (Symbol's and Wingdings' bullets, where Word's lists put them, at Unicode's private codes.)
            '\u{F0B7}' | '\u{F06C}' => out.push('•'),
            '\u{F0A7}' | '\u{F06E}' => out.push('▪'),
            '\u{F0D8}' => out.push('➢'),
            '\u{F0FC}' => out.push('✓'),
            '\u{F076}' => out.push('❖'),
            '\u{F0A8}' | '\u{F071}' => out.push('□'),
            '\u{A0}' | '\t' => out.push(' '),
            '\r' | '\n' => out.push(' '),
            c if (c as u32) < 32 || c == '\u{FFFD}' || c == '\u{FEFF}' => {}
            c => out.push(c),
        }
    }
}

/// Right-to-left letters (Hebrew's, Arabic's and the like, and their presentation forms), but not Arabic's digits.
fn rtl(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x065F | 0x066A..=0x06EF | 0x06FA..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFC)
}

/// Written left to right even among right-to-left words: other letters, and digits (Arabic's too).
fn ltr(c: char) -> bool {
    !rtl(c) && c.is_alphanumeric()
}

/// A line's text in the order it's read. A PDF draws right-to-left words (Hebrew, Arabic) as they're seen, left to
/// right: a line that starts (on the left) with a right-to-left letter is read right to left as a whole, its
/// left-to-right words and numbers kept as they are; in a line that starts left to right, each right-to-left phrase
/// is turned round, numbers in it kept as they are. Brackets in what's turned round face the other way (a font draws
/// "(" mirrored among right-to-left words). Then Arabic's presentation forms (letters as they're drawn alone, first,
/// in the middle or last) as the letters themselves.
fn in_reading_order(line: &str) -> String {
    if !line.chars().any(rtl) {
        return String::from(line);
    }
    let mut c: Vec<char> = line.chars().collect();
    if c.iter().find(|&&x| rtl(x) || ltr(x)).is_some_and(|&x| rtl(x)) {
        turn(&mut c);
    } else {
        let mut i = 0;
        while i < c.len() {
            if !rtl(c[i]) {
                i += 1;
                continue;
            }
            // The phrase: to its last right-to-left letter before a left-to-right one (numbers and spaces in it).
            let (mut end, mut j) = (i, i + 1);
            while j < c.len() && !(ltr(c[j]) && !c[j].is_numeric()) {
                if rtl(c[j]) {
                    end = j;
                }
                j += 1;
            }
            turn(&mut c[i..=end]);
            i = end + 1;
        }
    }
    let mut out = String::with_capacity(line.len());
    for ch in c {
        match arabic_letter(ch) {
            Some(s) => out.push_str(s),
            None => out.push(ch),
        }
    }
    out
}

/// Right-to-left text turned round: its brackets the other way, its left-to-right runs (words, numbers and what's
/// between them) turned back.
fn turn(c: &mut [char]) {
    c.reverse();
    for ch in c.iter_mut() {
        *ch = match *ch {
            '(' => ')',
            ')' => '(',
            '[' => ']',
            ']' => '[',
            '{' => '}',
            '}' => '{',
            '<' => '>',
            '>' => '<',
            '«' => '»',
            '»' => '«',
            x => x,
        };
    }
    let mut i = 0;
    while i < c.len() {
        if !ltr(c[i]) {
            i += 1;
            continue;
        }
        let (mut end, mut j) = (i, i + 1);
        while j < c.len() && !rtl(c[j]) {
            if ltr(c[j]) {
                end = j;
            }
            j += 1;
        }
        // (Its brackets back as they were, too.)
        let run = &mut c[i..=end];
        run.reverse();
        for ch in run.iter_mut() {
            *ch = match *ch {
                '(' => ')',
                ')' => '(',
                '[' => ']',
                ']' => '[',
                '{' => '}',
                '}' => '{',
                '<' => '>',
                '>' => '<',
                '«' => '»',
                '»' => '«',
                x => x,
            };
        }
        i = end + 1;
    }
}

/// Arabic's presentation forms B (Unicode's FE70 to FEFC), each with the letter or marks it's a form of.
const ARABIC_FORMS: &[(u32, u32, &str)] = &[
    (0xFE70, 0xFE71, "\u{064B}"),
    (0xFE72, 0xFE72, "\u{064C}"),
    (0xFE74, 0xFE74, "\u{064D}"),
    (0xFE76, 0xFE77, "\u{064E}"),
    (0xFE78, 0xFE79, "\u{064F}"),
    (0xFE7A, 0xFE7B, "\u{0650}"),
    (0xFE7C, 0xFE7D, "\u{0651}"),
    (0xFE7E, 0xFE7F, "\u{0652}"),
    (0xFE80, 0xFE80, "\u{0621}"),
    (0xFE81, 0xFE82, "\u{0622}"),
    (0xFE83, 0xFE84, "\u{0623}"),
    (0xFE85, 0xFE86, "\u{0624}"),
    (0xFE87, 0xFE88, "\u{0625}"),
    (0xFE89, 0xFE8C, "\u{0626}"),
    (0xFE8D, 0xFE8E, "\u{0627}"),
    (0xFE8F, 0xFE92, "\u{0628}"),
    (0xFE93, 0xFE94, "\u{0629}"),
    (0xFE95, 0xFE98, "\u{062A}"),
    (0xFE99, 0xFE9C, "\u{062B}"),
    (0xFE9D, 0xFEA0, "\u{062C}"),
    (0xFEA1, 0xFEA4, "\u{062D}"),
    (0xFEA5, 0xFEA8, "\u{062E}"),
    (0xFEA9, 0xFEAA, "\u{062F}"),
    (0xFEAB, 0xFEAC, "\u{0630}"),
    (0xFEAD, 0xFEAE, "\u{0631}"),
    (0xFEAF, 0xFEB0, "\u{0632}"),
    (0xFEB1, 0xFEB4, "\u{0633}"),
    (0xFEB5, 0xFEB8, "\u{0634}"),
    (0xFEB9, 0xFEBC, "\u{0635}"),
    (0xFEBD, 0xFEC0, "\u{0636}"),
    (0xFEC1, 0xFEC4, "\u{0637}"),
    (0xFEC5, 0xFEC8, "\u{0638}"),
    (0xFEC9, 0xFECC, "\u{0639}"),
    (0xFECD, 0xFED0, "\u{063A}"),
    (0xFED1, 0xFED4, "\u{0641}"),
    (0xFED5, 0xFED8, "\u{0642}"),
    (0xFED9, 0xFEDC, "\u{0643}"),
    (0xFEDD, 0xFEE0, "\u{0644}"),
    (0xFEE1, 0xFEE4, "\u{0645}"),
    (0xFEE5, 0xFEE8, "\u{0646}"),
    (0xFEE9, 0xFEEC, "\u{0647}"),
    (0xFEED, 0xFEEE, "\u{0648}"),
    (0xFEEF, 0xFEF0, "\u{0649}"),
    (0xFEF1, 0xFEF4, "\u{064A}"),
    (0xFEF5, 0xFEF6, "\u{0644}\u{0622}"),
    (0xFEF7, 0xFEF8, "\u{0644}\u{0623}"),
    (0xFEF9, 0xFEFA, "\u{0644}\u{0625}"),
    (0xFEFB, 0xFEFC, "\u{0644}\u{0627}"),
];

fn arabic_letter(c: char) -> Option<&'static str> {
    let c = c as u32;
    if !(0xFE70..=0xFEFC).contains(&c) {
        return None;
    }
    ARABIC_FORMS.iter().find(|&&(lo, hi, _)| lo <= c && c <= hi).map(|f| f.2)
}

/// √x, near enough (core has no square root): Newton's method from a guess halving the exponent.
fn sqrt(x: f64) -> f64 {
    if !(x > 0.0) || !x.is_finite() {
        return 0.0;
    }
    let mut r = f64::from_bits((x.to_bits() >> 1) + (1023u64 << 51));
    for _ in 0..6 {
        r = 0.5 * (r + x / r);
    }
    r
}

/// Past an inline picture: its dictionary, "ID", its data, up to an "EI" on its own.
fn skip_inline_image(lex: &mut Lexer) {
    while let Some(o) = lex.object() {
        if o == Obj::Op(b"ID".to_vec()) {
            break;
        }
    }
    let d = lex.data;
    let mut i = lex.pos + 1;
    while i + 2 <= d.len() {
        if &d[i..i + 2] == b"EI"
            && crate::lex::is_white(d[i - 1])
            && d.get(i + 2).is_none_or(|&b| crate::lex::is_white(b) || crate::lex::is_delim(b))
        {
            lex.pos = i + 2;
            return;
        }
        i += 1;
    }
    lex.pos = d.len();
}

/// Each line's spaces at its ends taken off, spaces run together, at most one blank line in a row.
fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = false;
    for line in s.split('\n') {
        let mut l = String::with_capacity(line.len());
        for w in line.split(' ').filter(|w| !w.is_empty()) {
            if !l.is_empty() {
                l.push(' ');
            }
            l.push_str(w);
        }
        if l.is_empty() {
            blank = !out.is_empty();
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank { "\n\n" } else { "\n" });
        }
        blank = false;
        out.push_str(&l);
    }
    out
}
