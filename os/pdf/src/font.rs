//! What a font's codes are as characters, and how wide each is (ISO 32000-1, 9.6 to 9.10): its ToUnicode CMap first;
//! else, for a simple font, its encoding (WinAnsi, MacRoman or Standard, with /Differences' glyph names read as
//! Adobe's glyph list names them); for a composite font, a UCS-2 or UTF-16 CMap's codes as the characters
//! themselves. Codes are split by the font's code space (one byte each in a simple font).

use crate::file::File;
use crate::lex::{Dict, Lexer, Obj};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub struct Font {
    /// Code space ranges (low, high, bytes); none: one byte a code.
    ranges: Vec<(u32, u32, u8)>,
    unicode: ToUnicode,
    /// A simple font's codes as text.
    simple: Vec<Option<String>>,
    /// A composite font whose codes are UTF-16 themselves.
    utf16: bool,
    widths: Widths,
    /// A simple font (its code 32 is a space, for word spacing).
    pub one_byte: bool,
}

enum Widths {
    Simple { first: u32, list: Vec<f64>, missing: f64 },
    /// A composite font's widths: codes' own, then ranges' (low, high, width), then the default.
    Composite { default: f64, map: BTreeMap<u32, f64>, ranges: Vec<(u32, u32, f64)> },
}

/// A ToUnicode CMap: codes' own text (bfchar, and bfrange's arrays), then ranges of codes whose text counts up from
/// the low code's (low, high, its text as UTF-16).
#[derive(Default)]
pub struct ToUnicode {
    pub chars: BTreeMap<u32, String>,
    pub ranges: Vec<(u32, u32, Vec<u16>)>,
}

impl ToUnicode {
    fn get(&self, code: u32) -> Option<String> {
        if let Some(t) = self.chars.get(&code) {
            return Some(t.clone());
        }
        let &(lo, _, ref base) = in_ranges(&self.ranges, code, |r| (r.0, r.1))?;
        let mut units = base.clone();
        if let Some(last) = units.last_mut() {
            *last = last.wrapping_add((code - lo) as u16);
        }
        Some(String::from_utf16_lossy(&units))
    }
}

/// The most codes, or ranges, a CMap's or a font's widths list; the most ranges of codes a CMap's code space has.
const MOST_MAPPED: usize = 200_000;
const MOST_SPACES: usize = 256;

/// The range (in `ranges`, in order of where they start) that `code` is in, if any: the last that starts at or
/// before it, or one a little before that (ranges written overlapping).
fn in_ranges<T>(ranges: &[T], code: u32, ends: impl Fn(&T) -> (u32, u32)) -> Option<&T> {
    let after = ranges.partition_point(|r| ends(r).0 <= code);
    ranges[..after].iter().rev().take(16).find(|r| {
        let (lo, hi) = ends(r);
        lo <= code && code <= hi
    })
}

impl Font {
    /// A font that can't be read: bytes as Latin-1.
    pub fn fallback() -> Font {
        Font {
            ranges: Vec::new(),
            simple: (0..256u32).map(|c| (c >= 32).then(|| String::from(char::from_u32(c).unwrap_or(' ')))).collect(),
            unicode: ToUnicode::default(),
            utf16: false,
            widths: Widths::Simple { first: 0, list: Vec::new(), missing: 500.0 },
            one_byte: true,
        }
    }

    pub fn load(file: &File, d: &Dict) -> Font {
        let subtype = file.get_in(d, "Subtype");
        let composite = subtype.name() == Some(b"Type0");
        let mut font = Font::fallback();
        font.simple = vec![None; 256];
        if let Some(cmap) = d.get("ToUnicode").and_then(|o| file.stream(o)) {
            let (map, ranges) = read_cmap(&cmap);
            font.unicode = map;
            if composite {
                font.ranges = ranges;
            }
        }
        if composite {
            font.one_byte = false;
            match file.get_in(d, "Encoding") {
                Obj::Name(n) => {
                    let name = String::from_utf8_lossy(&n).into_owned();
                    font.utf16 = name.contains("UCS2") || name.contains("UTF16");
                }
                s @ Obj::Stream(..) => {
                    if let Some(data) = file.stream(&s) {
                        let (_, ranges) = read_cmap(&data);
                        if !ranges.is_empty() {
                            font.ranges = ranges;
                        }
                    }
                }
                _ => {}
            }
            if font.ranges.is_empty() {
                font.ranges = vec![(0, 0xFFFF, 2)];
            }
            let descendant = file.get_in(d, "DescendantFonts");
            let cid = descendant.array().and_then(|a| a.first()).map(|f| file.resolve(f));
            let default = cid.as_ref().and_then(|c| c.dict()).map(|c| file.get_in(c, "DW")).and_then(|w| w.num()).unwrap_or(1000.0);
            let (mut map, mut ranges) = (BTreeMap::new(), Vec::new());
            if let Some(w) = cid.as_ref().and_then(|c| c.dict()).map(|c| file.get_in(c, "W")) {
                read_cid_widths(file, &w, &mut map, &mut ranges);
            }
            ranges.sort_by_key(|r| r.0);
            font.widths = Widths::Composite { default, map, ranges };
            return font;
        }
        // A simple font: its encoding, from its base and its differences.
        let encoding = file.get_in(d, "Encoding");
        let base_name: Option<Vec<u8>> = match &encoding {
            Obj::Name(n) => Some(n.clone()),
            Obj::Dict(e) => file.get_in(e, "BaseEncoding").name().map(<[u8]>::to_vec),
            _ => None,
        };
        let base_font = file.get_in(d, "BaseFont").name().map(<[u8]>::to_vec).unwrap_or_default();
        let symbolic = ["Symbol", "ZapfDingbats", "Wingdings", "Webdings"].iter().any(|s| contains(&base_font, s.as_bytes()));
        let base = match base_name.as_deref() {
            Some(b"WinAnsiEncoding") => Base::WinAnsi,
            Some(b"MacRomanEncoding") => Base::MacRoman,
            Some(b"StandardEncoding") => Base::Standard,
            _ if subtype.name() == Some(b"TrueType") => Base::WinAnsi,
            _ if symbolic => Base::None,
            _ if subtype.name() == Some(b"Type3") => Base::None,
            _ => Base::Standard,
        };
        for (code, slot) in font.simple.iter_mut().enumerate() {
            *slot = base.char(code as u8).map(String::from);
        }
        if let Obj::Dict(e) = &encoding
            && let Obj::Array(diffs) = file.get_in(e, "Differences")
        {
            let mut code: i64 = 0;
            for item in &diffs {
                match item {
                    Obj::Int(c) => code = *c,
                    Obj::Name(n) => {
                        if (0..256).contains(&code) {
                            font.simple[code as usize] = crate::glyphs::glyph(n);
                        }
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
        let first = file.get_in(d, "FirstChar").int().unwrap_or(0).clamp(0, 255) as u32;
        let list: Vec<f64> = file.get_in(d, "Widths").array().map(|a| a.iter().map(|w| file.resolve(w).num().unwrap_or(0.0)).collect()).unwrap_or_default();
        let descriptor = file.get_in(d, "FontDescriptor");
        let missing = descriptor.dict().map(|fd| file.get_in(fd, "MissingWidth")).and_then(|m| m.num()).filter(|&m| m > 0.0);
        // (A Type 3 font's widths are in its own units: its matrix makes them text space's.)
        let scale = if subtype.name() == Some(b"Type3") {
            file.get_in(d, "FontMatrix").array().and_then(|m| m.first()).and_then(|a| a.num()).map_or(1.0, |a| a * 1000.0)
        } else {
            1.0
        };
        let list = list.into_iter().map(|w| w * scale).collect();
        font.widths = Widths::Simple { first, list, missing: missing.unwrap_or(500.0) };
        font
    }

    /// A string's codes, each with whether it's a single-byte 32 (word spacing applies to it).
    pub fn codes(&self, s: &[u8]) -> Vec<(u32, bool)> {
        if self.ranges.is_empty() {
            return s.iter().map(|&b| (b as u32, b == 32)).collect();
        }
        let mut out = Vec::with_capacity(s.len() / 2 + 1);
        let mut i = 0;
        while i < s.len() {
            let mut taken = None;
            for len in 1..=4u8 {
                let Some(bytes) = s.get(i..i + len as usize) else { break };
                let v = bytes.iter().fold(0u32, |v, &b| v << 8 | b as u32);
                if self.ranges.iter().any(|&(lo, hi, l)| l == len && lo <= v && v <= hi) {
                    taken = Some((v, len as usize));
                    break;
                }
            }
            let (code, len) = taken.unwrap_or_else(|| {
                // (A code outside every range: as many bytes as the shortest range takes.)
                let len = self.ranges.iter().map(|r| r.2).min().unwrap_or(1).max(1) as usize;
                let bytes = &s[i..(i + len).min(s.len())];
                (bytes.iter().fold(0u32, |v, &b| v << 8 | b as u32), len)
            });
            out.push((code, len == 1 && code == 32));
            i += len;
        }
        out
    }

    /// A code's text, if the font says.
    pub fn text(&self, code: u32) -> Option<String> {
        if let Some(t) = self.unicode.get(code) {
            return Some(t);
        }
        if self.one_byte {
            return self.simple.get(code as usize).and_then(|s| s.clone());
        }
        None
    }

    /// A composite font's code as UTF-16 (a UCS-2 CMap's), if it is one.
    pub fn utf16(&self, code: u32) -> Option<char> {
        if self.utf16 { char::from_u32(code) } else { None }
    }

    /// A code's width, in thousandths of the font's size.
    pub fn width(&self, code: u32) -> f64 {
        match &self.widths {
            Widths::Simple { first, list, missing } => {
                code.checked_sub(*first).and_then(|i| list.get(i as usize)).copied().filter(|&w| w > 0.0).unwrap_or(*missing)
            }
            Widths::Composite { default, map, ranges } => {
                map.get(&code).copied().or_else(|| in_ranges(ranges, code, |r| (r.0, r.1)).map(|r| r.2)).unwrap_or(*default)
            }
        }
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// A composite font's /W: "c [w1 w2 ...]" (from code c on) and "c_first c_last w".
fn read_cid_widths(file: &File, w: &Obj, map: &mut BTreeMap<u32, f64>, ranges: &mut Vec<(u32, u32, f64)>) {
    let Some(items) = w.array() else { return };
    let mut i = 0;
    while i < items.len() && map.len() < MOST_MAPPED && ranges.len() < MOST_MAPPED {
        let Some(first) = file.resolve(&items[i]).int() else { break };
        match items.get(i + 1).map(|o| file.resolve(o)) {
            Some(Obj::Array(list)) => {
                for (k, v) in list.iter().enumerate() {
                    if let Some(width) = file.resolve(v).num() {
                        map.insert((first.clamp(0, u32::MAX as i64) as u32).saturating_add(k as u32), width);
                    }
                }
                i += 2;
            }
            Some(last) => {
                let (Some(last), Some(width)) = (last.int(), items.get(i + 2).and_then(|o| file.resolve(o).num())) else { break };
                let (lo, hi) = (first.clamp(0, u32::MAX as i64) as u32, last.clamp(0, u32::MAX as i64) as u32);
                if lo <= hi {
                    ranges.push((lo, hi, width));
                }
                i += 3;
            }
            None => break,
        }
    }
}

/// A CMap's mappings to text (bfchar and bfrange) and its code space ranges.
pub fn read_cmap(data: &[u8]) -> (ToUnicode, Vec<(u32, u32, u8)>) {
    let mut map = ToUnicode::default();
    let mut ranges = Vec::new();
    let mut lex = Lexer::new(data, 0);
    let value = |s: &[u8]| s.iter().fold(0u32, |v, &b| v << 8 | b as u32);
    while let Some(o) = lex.object() {
        let Obj::Op(op) = o else { continue };
        match op.as_slice() {
            // (Each pair or three read in turn: the section's end word ends it.)
            b"begincodespacerange" => loop {
                let Some(Obj::Str(lo)) = lex.object() else { break };
                let Some(Obj::Str(hi)) = lex.object() else { break };
                if !lo.is_empty() && lo.len() <= 4 && lo.len() == hi.len() && ranges.len() < MOST_SPACES {
                    ranges.push((value(&lo), value(&hi), lo.len() as u8));
                }
            },
            b"beginbfchar" => loop {
                let Some(Obj::Str(src)) = lex.object() else { break };
                let Some(dst) = lex.object() else { break };
                if src.len() > 4 || map.chars.len() >= MOST_MAPPED {
                    continue;
                }
                let text = match dst {
                    Obj::Str(d) => utf16_text(&d),
                    Obj::Name(n) => crate::glyphs::glyph(&n).unwrap_or_default(),
                    _ => continue,
                };
                map.chars.insert(value(&src), text);
            },
            b"beginbfrange" => loop {
                let Some(Obj::Str(lo)) = lex.object() else { break };
                let Some(Obj::Str(hi)) = lex.object() else { break };
                let Some(dst) = lex.object() else { break };
                if lo.len() > 4 || hi.len() > 4 {
                    continue;
                }
                let (lo, hi) = (value(&lo), value(&hi));
                if hi < lo || map.chars.len() >= MOST_MAPPED || map.ranges.len() >= MOST_MAPPED {
                    continue;
                }
                match dst {
                    Obj::Str(d) => {
                        let units: Vec<u16> = if d.len() == 1 {
                            vec![d[0] as u16]
                        } else {
                            d.chunks(2).take(32).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect()
                        };
                        map.ranges.push((lo, hi, units));
                    }
                    Obj::Array(items) => {
                        for (k, item) in items.iter().enumerate().take((hi - lo) as usize + 1) {
                            if let Obj::Str(d) = item {
                                map.chars.insert(lo.saturating_add(k as u32), utf16_text(d));
                            }
                        }
                    }
                    _ => {}
                }
            },
            _ => {}
        }
    }
    map.ranges.sort_by_key(|r| r.0);
    (map, ranges)
}

/// A CMap's destination string: UTF-16BE (a single byte, as some programs write, as that character).
fn utf16_text(d: &[u8]) -> String {
    if d.len() == 1 {
        return String::from(d[0] as char);
    }
    let units: Vec<u16> = d.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
    String::from_utf16_lossy(&units)
}

#[derive(Clone, Copy)]
enum Base {
    WinAnsi,
    MacRoman,
    Standard,
    None,
}

impl Base {
    fn char(self, b: u8) -> Option<char> {
        match self {
            Base::None => None,
            Base::WinAnsi => win_ansi(b),
            Base::MacRoman => mac_roman(b),
            Base::Standard => standard(b),
        }
    }
}

/// WinAnsiEncoding (Annex D.2): Windows' code page 1252, its unused codes from 0x7F the bullet (as the standard says,
/// and ReportLab draws its bullets).
fn win_ansi(b: u8) -> Option<char> {
    const HIGH: [u16; 32] = [
        0x20AC, 0x2022, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x2022,
        0x017D, 0x2022, 0x2022, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A,
        0x0153, 0x2022, 0x017E, 0x0178,
    ];
    match b {
        0x20..=0x7E | 0xA0..=0xFF => Some(b as char),
        0x7F => Some('\u{2022}'),
        0x80..=0x9F => char::from_u32(HIGH[(b - 0x80) as usize] as u32),
        9 | 10 | 13 => Some(' '),
        _ => None,
    }
}

fn mac_roman(b: u8) -> Option<char> {
    const HIGH: &str = "ÄÅÇÉÑÖÜáàâäãåçéèêëíìîïñóòôöõúùûü†°¢£§•¶ß®©™´¨≠ÆØ∞±≤≥¥µ∂∑∏π∫ªºΩæø¿¡¬√ƒ≈∆«»… ÀÃÕŒœ–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ\u{F8FF}ÒÚÛÙıˆ˜¯˘˙˚¸˝˛ˇ";
    match b {
        0x20..=0x7E => Some(b as char),
        0x80..=0xFF => HIGH.chars().nth((b - 0x80) as usize),
        _ => None,
    }
}

fn standard(b: u8) -> Option<char> {
    Some(match b {
        0x27 => '\u{2019}',
        0x60 => '\u{2018}',
        0x20..=0x7E => b as char,
        0xA1 => '¡', 0xA2 => '¢', 0xA3 => '£', 0xA4 => '⁄', 0xA5 => '¥', 0xA6 => 'ƒ', 0xA7 => '§', 0xA8 => '¤',
        0xA9 => '\'', 0xAA => '“', 0xAB => '«', 0xAC => '‹', 0xAD => '›', 0xAE => '\u{FB01}', 0xAF => '\u{FB02}',
        0xB1 => '–', 0xB2 => '†', 0xB3 => '‡', 0xB4 => '·', 0xB6 => '¶', 0xB7 => '•', 0xB8 => '‚', 0xB9 => '„',
        0xBA => '”', 0xBB => '»', 0xBC => '…', 0xBD => '‰', 0xBF => '¿', 0xC1 => '`', 0xC2 => '´', 0xC3 => 'ˆ',
        0xC4 => '˜', 0xC5 => '¯', 0xC6 => '˘', 0xC7 => '˙', 0xC8 => '¨', 0xCA => '˚', 0xCB => '¸', 0xCD => '˝',
        0xCE => '˛', 0xCF => 'ˇ', 0xD0 => '—', 0xE1 => 'Æ', 0xE3 => 'ª', 0xE8 => 'Ł', 0xE9 => 'Ø', 0xEA => 'Œ',
        0xEB => 'º', 0xF1 => 'æ', 0xF5 => 'ı', 0xF8 => 'ł', 0xF9 => 'ø', 0xFA => 'œ', 0xFB => 'ß',
        _ => return None,
    })
}
