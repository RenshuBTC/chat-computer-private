//! Text in Lora (its regular and bold, both from the one variable font file) and, for code, Atkinson Hyperlegible
//! Mono, set with Chat Computer's own font engine: measuring, wrapping and drawing lines.
//! Characters they don't have (arrows, check marks, maths, Greek and Cyrillic, emoji, and box drawing in code; Chinese,
//! Japanese and Korean) come from fallback fonts made from Google's Noto fonts (`os/fonts/`), as a browser would take
//! them from the system's. Lines break at spaces, and between the characters of Chinese, Japanese and Korean (but not
//! before closing punctuation, nor after opening), as a browser breaks them.
//! Letters side by side are kerned as the font says ("AV", "To" and "y." closer), at its regular weight and its bold,
//! as a browser does: within a run of one font and face, so not across a change to bold or to a fallback font.
//! Accents typed as characters of their own (a letter, then a combining mark: "e" and "\u{301}", as Macs name files)
//! are set as HarfBuzz sets them: composed into one letter where the font has it ("é"), else placed on the letter
//! where the font's anchors put them (GPOS's mark positioning), else over or under it by its ink (as HarfBuzz does
//! for a font without them).
//! Sizes are in 1/256 of a pixel.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod compose;

use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;
use font::raster::{Pen, Raster, ONE};
use font::{Font, Instance, Kerning, Marks, Outline};

static LORA: &[u8] = include_bytes!("../../../chatcomputer/ui/assets/fonts/Lora.ttf");
static MONO: &[u8] = include_bytes!("../../../chatcomputer/ui/assets/fonts/AtkinsonHyperlegibleMono.ttf");
static FALLBACK: &[u8] = include_bytes!("../../fonts/ChatComputerFallback.ttf");
static FALLBACK_MONO: &[u8] = include_bytes!("../../fonts/ChatComputerFallbackMono.ttf");
static CJK: &[u8] = include_bytes!("../../fonts/ChatComputerCJK.ttf");

/// Some letters reach a little past their own width (an italic f, an accent); the drawn line keeps
/// this much room on each side, in pixels.
const PAD: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub size: i32,
    pub line_height: i32,
}

/// Which of the fonts: the text's own, bold (weight 700), slanted ("italic": the regular or bold glyphs leaned 12°,
/// as browsers do for a font without its italic), or the code font.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Face {
    Regular,
    Bold,
    Italic,
    BoldItalic,
    Mono,
}

impl Face {
    fn bold(self) -> bool {
        matches!(self, Face::Bold | Face::BoldItalic)
    }
    fn slanted(self) -> bool {
        matches!(self, Face::Italic | Face::BoldItalic)
    }
}

/// tan 12°, in 1/65536: how far a slanted glyph leans right for each unit up.
const SLANT: i64 = 13930;

/// How far apart a fallback font's glyph is drawn twice in bold (the fallbacks have no bold of their own), in 1/256
/// pixel at `size`: as Skia makes a bold up, its strokes a 24th of the size thicker at 9 pixels and below, a 32nd at
/// 36 and above, and in between, in between.
fn made_up_bold(size: i32) -> i32 {
    let px = (size / ONE).clamp(9, 36);
    // size × (1/24 + (px − 9)/27 × (1/32 − 1/24)) = size × (108 − (px − 9)) / 2592
    (size as i64 * (108 - (px - 9)) as i64 / 2592) as i32
}

/// Leans an outline right as it goes up (font units × 64, y up), on its way to `to`.
struct Slant<'a> {
    to: &'a mut dyn Outline,
}

impl Slant<'_> {
    fn x(x: i32, y: i32) -> i32 {
        x + ((y as i64 * SLANT) >> 16) as i32
    }
}

impl Outline for Slant<'_> {
    fn move_to(&mut self, x: i32, y: i32) {
        self.to.move_to(Self::x(x, y), y);
    }
    fn line_to(&mut self, x: i32, y: i32) {
        self.to.line_to(Self::x(x, y), y);
    }
    fn quad_to(&mut self, cx: i32, cy: i32, x: i32, y: i32) {
        self.to.quad_to(Self::x(cx, cy), cy, Self::x(x, y), y);
    }
    fn close(&mut self) {
        self.to.close();
    }
}

/// Which font a glyph is from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Text,
    Mono,
    /// Characters the text font lacks (in its weight only: there's no bold of them).
    Fallback,
    /// Characters the code font lacks, a code character wide (box drawing lines meet).
    FallbackMono,
    /// Chinese, Japanese and Korean (in their regular weight only too).
    Cjk,
}

/// Characters that show nothing and take no room: joiners and zero-width spaces, variation selectors (the "as an
/// emoji" after ❤ in ❤️), the byte order mark, and the soft hyphen. (Nor do they come between letters that kern, as
/// HarfBuzz leaves them out.)
fn invisible(c: char) -> bool {
    matches!(c, '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}')
}

/// Chinese, Japanese and Korean: ideographs, kana, Hangul, Bopomofo, their punctuation and full-width forms. A line
/// can break between any two characters where one of these is (UAX #14's ideographs, and Hangul as browsers break
/// it), unless one closes or opens.
fn cjk(c: char) -> bool {
    matches!(c as u32, 0x2E80..=0x2FFF | 0x3000..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xA960..=0xA97F
        | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

/// Characters a line doesn't start with: closing punctuation (UAX #14's CL, CP, EX and IS), and what belongs to the
/// character before (NS and IN: iteration marks, the middle dot, ellipses; signs after a number). Small kana and
/// the long vowel mark can start one, as browsers break by default (CSS's normal line breaking).
fn closing(c: char) -> bool {
    matches!(c, ')' | ']' | '}' | ',' | '.' | ':' | ';' | '!' | '?' | '%' | '、' | '。' | '〉' | '》' | '」' | '』' | '】' | '〕'
        | '〗' | '〙' | '〛' | '〞' | '〟' | '・' | '゠' | '〜' | '々' | '〻' | 'ゝ' | 'ゞ' | 'ヽ' | 'ヾ' | '…' | '‥' | '‼' | '⁇'
        | '⁈' | '⁉' | '°' | '′' | '″' | '℃' | '‰' | '！' | '）' | '，' | '．' | '：' | '；' | '？' | '］' | '｝' | '｠' | '｡'
        | '｣' | '､' | '･' | '％')
}

/// Characters a line doesn't end with: opening punctuation (OP), and signs before a number (PR).
fn opening(c: char) -> bool {
    matches!(c, '(' | '[' | '{' | '〈' | '《' | '「' | '『' | '【' | '〔' | '〖' | '〘' | '〚' | '〝' | '（' | '［' | '｛' | '｟'
        | '｢' | '$' | '£' | '¥' | '€' | '₩' | '＄' | '￡' | '￥' | '￦')
}

/// Quotation marks (QU), which a line doesn't break before or after: whether one opens or closes isn't known, as
/// UAX #14 had it (browsers keep "说“好”了" whole).
fn quote(c: char) -> bool {
    matches!(c, '"' | '\'' | '“' | '”' | '‘' | '’' | '«' | '»' | '‹' | '›')
}

/// Spaces a line breaks after, which go when it does: the space, and the ideographic space (a browser lets it hang
/// past the line's end, where it can't be seen).
fn space(c: char) -> bool {
    matches!(c, ' ' | '\u{3000}')
}

/// Whether a line can break between two characters side by side (neither a space): where either is Chinese, Japanese
/// or Korean, unless the second closes or the first opens, or either is a quotation mark.
fn breaks_between(a: char, b: char) -> bool {
    (cjk(a) || cjk(b)) && !closing(b) && !opening(a) && !quote(a) && !quote(b)
}

/// A character as it's set: its glyph (which font's, and which), its face, and how far it moves the pen (font units).
#[derive(Clone, Copy)]
struct Set {
    source: Source,
    glyph: u16,
    face: Face,
    advance: i32,
}

pub struct Type {
    font: Font<'static>,
    mono: Font<'static>,
    fallback: Font<'static>,
    fallback_mono: Font<'static>,
    cjk: Font<'static>,
    /// The bold instance, and every glyph's advance in it (worked out once: they come from the glyphs' variations).
    bold: Option<Instance>,
    bold_advances: Vec<u16>,
    /// The text font's kerning, at its regular weight and its bold. (The code font has none, nor have the fallbacks.)
    kerning: Kerning<'static>,
    bold_kerning: Kerning<'static>,
    /// The text font's mark positioning, at its regular weight and its bold (the others have none).
    marks: Marks<'static>,
    bold_marks: Marks<'static>,
}

/// How a character is set on a line: where the pen is at it (font units from the line's start, kerned with the
/// letter before), how far it moves the pen, the kerning with that letter (in `pen` already), whether it's a mark on
/// the letter before it (a line doesn't break before one), and its glyph as drawn, if it shows (where: from the
/// line's start, and up from the baseline). A mark composed into its letter shows nothing of its own.
#[derive(Clone, Copy)]
struct Placed {
    pen: i32,
    advance: i32,
    kern: i32,
    mark: bool,
    drawn: Option<(Set, i32, i32)>,
}

/// A mark's combining class (Unicode's: 0 for letters, which aren't marks).
fn class(c: char) -> u8 {
    let c = c as u32;
    let i = compose::CLASSES.partition_point(|&(_, end, _)| end < c);
    match compose::CLASSES.get(i) {
        Some(&(start, _, class)) if start <= c => class,
        _ => 0,
    }
}

/// The one character a letter and a mark compose into, if Unicode has one.
fn composed(a: char, b: char) -> Option<char> {
    let key = (a as u32, b as u32);
    let i = compose::COMPOSE.binary_search_by(|&(x, y, _)| (x, y).cmp(&key)).ok()?;
    char::from_u32(compose::COMPOSE[i].2)
}

/// Whether a mark's class puts it under the letter (and whether it touches it), or over it.
fn below(class: u8) -> bool {
    matches!(class, 200 | 202 | 218 | 220 | 222 | 233)
}
fn above(class: u8) -> bool {
    matches!(class, 214 | 216 | 228 | 230 | 232 | 234)
}
fn attached(class: u8) -> bool {
    matches!(class, 200 | 202 | 214 | 216)
}

/// A line of text drawn as ink coverage, ready to put on the screen.
pub struct Ink {
    pub width: usize,
    pub height: usize,
    pub coverage: Vec<u8>,
    /// How far left of the text's own start the coverage begins, in pixels.
    pub left: usize,
}

impl Type {
    /// Chat Computer's own fonts: Lora, Atkinson Hyperlegible Mono for code, and the fallbacks.
    pub fn builtin() -> Type {
        let font = Font::parse(LORA).expect("the built-in font is damaged");
        let mono = Font::parse(MONO).expect("the built-in code font is damaged");
        let fallback = Font::parse(FALLBACK).expect("the built-in fallback font is damaged");
        let fallback_mono = Font::parse(FALLBACK_MONO).expect("the built-in fallback code font is damaged");
        let cjk = Font::parse(CJK).expect("the built-in Chinese, Japanese and Korean font is damaged");
        // (Glyphs from every font are drawn to the same scale: all have 1000 units to the em.)
        let upem = font.metrics.units_per_em;
        assert!([&mono, &fallback, &fallback_mono, &cjk].iter().all(|f| f.metrics.units_per_em == upem), "the fonts' sizes differ");
        let bold = font.instance(b"wght", 700);
        let bold_advances = match &bold {
            Some(at) => font.advances_at(at, 0..font.glyph_count()),
            None => Vec::new(),
        };
        let (kerning, bold_kerning) = (font.kerning(None), font.kerning(bold.as_ref()));
        let (marks, bold_marks) = (font.marks(None), font.marks(bold.as_ref()));
        Type { font, mono, fallback, fallback_mono, cjk, bold, bold_advances, kerning, bold_kerning, marks, bold_marks }
    }

    fn scale(&self, units: i32, size: i32) -> i32 {
        (units as i64 * size as i64 / self.font.metrics.units_per_em as i64) as i32
    }

    fn font_of(&self, source: Source) -> &Font<'static> {
        match source {
            Source::Text => &self.font,
            Source::Mono => &self.mono,
            Source::Fallback => &self.fallback,
            Source::FallbackMono => &self.fallback_mono,
            Source::Cjk => &self.cjk,
        }
    }

    /// Where a character's glyph is for `face`: in the face's own font, else in a fallback font; else the face's own
    /// "missing" glyph (a box), for characters none has.
    fn glyph(&self, c: char, face: Face) -> (Source, u16) {
        let order: [Source; 4] = if face == Face::Mono {
            [Source::Mono, Source::FallbackMono, Source::Fallback, Source::Cjk]
        } else {
            [Source::Text, Source::Fallback, Source::FallbackMono, Source::Cjk]
        };
        for source in order {
            let glyph = self.font_of(source).glyph_index(c);
            if glyph != 0 {
                return (source, glyph);
            }
        }
        (order[0], 0)
    }

    /// How a character is set in `face` (None: one that shows nothing and takes no room).
    fn set(&self, c: char, face: Face) -> Option<Set> {
        if invisible(c) {
            return None;
        }
        let (source, glyph) = self.glyph(c, face);
        let advance = match source {
            Source::Text => match self.bold_advances.get(glyph as usize) {
                Some(&a) if face.bold() => a,
                _ => self.font.advance(glyph),
            },
            _ => self.font_of(source).advance(glyph),
        };
        Some(Set { source, glyph, face, advance: advance as i32 })
    }

    /// The kerning between two characters set side by side, in font units: the text font's own, between its glyphs in
    /// one face (as a browser shapes each run of one font and style by itself).
    fn kern(&self, a: Set, b: Set) -> i32 {
        if a.face != b.face || a.source != Source::Text || b.source != Source::Text {
            return 0;
        }
        if a.face.bold() { self.bold_kerning.pair(a.glyph, b.glyph) } else { self.kerning.pair(a.glyph, b.glyph) }
    }

    /// The mark positioning of the font a glyph is from, in `face` (only the text font has any: the fallbacks are made
    /// without it).
    fn marks_of(&self, source: Source, face: Face) -> Option<&Marks<'static>> {
        match source {
            Source::Text if face.bold() => Some(&self.bold_marks),
            Source::Text => Some(&self.marks),
            _ => None,
        }
    }

    /// How each of `chars` (each with its face) is set along a line, kerned with the letter before (a line break
    /// starts over). A letter and the marks right after it as HarfBuzz sets them: the marks in their canonical order
    /// (by class), each composed into the letter where Unicode has the two as one character and the letter's font
    /// has it (unless a mark left over of its class or higher comes between), an i or j losing its dot under a mark
    /// over it, and the marks left placed on it: where the letter's font's anchors put them (on it, or on the mark
    /// before), else over or under what's there (its ink, and the marks on it), a 16th of an em apart, centred on
    /// its width, as HarfBuzz places marks for a font without anchors. Marks take no room.
    fn shape(&self, chars: &[(char, Face)]) -> Vec<Placed> {
        let mut out: Vec<Placed> = Vec::with_capacity(chars.len());
        let (mut pen, mut before): (i32, Option<Set>) = (0, None);
        let gap = (self.font.metrics.units_per_em / 16) as i32;
        let mut i = 0;
        while i < chars.len() {
            let (c, face) = chars[i];
            if c == '\n' {
                before = None;
            }
            let Some(set) = self.set(c, face).filter(|_| c != '\n') else {
                out.push(Placed { pen, advance: 0, kern: 0, mark: false, drawn: None });
                i += 1;
                continue;
            };
            let mut end = i + 1;
            while end < chars.len() && class(chars[end].0) != 0 {
                end += 1;
            }
            let mut order: Vec<usize> = (i + 1..end).collect();
            order.sort_by_key(|&k| class(chars[k].0));
            // Composed into the letter where they can be.
            let font = self.font_of(set.source);
            let (mut letter, mut kept, mut last) = (c, Vec::new(), 0u8);
            for &k in &order {
                let (m, cc) = (chars[k].0, class(chars[k].0));
                let free = kept.is_empty() || last < cc;
                match composed(letter, m).filter(|&x| free && font.glyph_index(x) != 0) {
                    Some(x) => letter = x,
                    None => {
                        kept.push(k);
                        last = cc;
                    }
                }
            }
            if kept.iter().any(|&k| above(class(chars[k].0))) {
                let dotless = match letter {
                    'i' => 'ı',
                    'j' => '\u{237}',
                    other => other,
                };
                if font.glyph_index(dotless) != 0 {
                    letter = dotless;
                }
            }
            let base = if letter == c { set } else { self.set(letter, face).unwrap_or(set) };
            let kern = before.map_or(0, |b| self.kern(b, base));
            pen += kern;
            let start = pen;
            out.push(Placed { pen, advance: base.advance, kern, mark: false, drawn: Some((base, pen, 0)) });
            pen += base.advance;
            before = Some(base);
            for _ in i + 1..end {
                out.push(Placed { pen, advance: 0, kern: 0, mark: true, drawn: None });
            }
            // The marks left, placed in their canonical order.
            let base_font = self.font_of(base.source);
            let ink = base_font.bounds(base.glyph);
            let (mut top, mut bottom) = ink.map_or((0, 0), |(_, y0, _, y1)| (y1, y0));
            let mut stack: Vec<(u16, (i32, i32))> = Vec::new();
            for &k in &kept {
                let (m, cc) = (chars[k].0, class(chars[k].0));
                // From the letter's font if it has the mark, else from whichever has it.
                let mark = match base_font.glyph_index(m) {
                    0 => self.set(m, face),
                    g => Some(Set { source: base.source, glyph: g, face, advance: 0 }),
                };
                let Some(mut mark) = mark else { continue };
                mark.advance = 0;
                let same = mark.source == base.source;
                let anchored = self
                    .marks_of(base.source, face)
                    .filter(|_| same)
                    .and_then(|marks| marks.on_mark(&stack, mark.glyph).or_else(|| marks.on_base(base.glyph, mark.glyph)));
                let bounds = self.font_of(mark.source).bounds(mark.glyph);
                let (dx, dy) = match (anchored, ink, bounds) {
                    (Some(at), _, _) => at,
                    (None, Some(_), Some((mx0, my0, mx1, my1))) => {
                        let gap = if attached(cc) { 0 } else { gap };
                        let (w, mw) = (base.advance, mx1 - mx0);
                        let dx = match cc {
                            233 | 234 => w - mw / 2 - mx0,
                            200 | 218 | 228 => -mx0,
                            216 | 222 | 232 => w - mw - mx0,
                            _ => (w - mw) / 2 - mx0,
                        };
                        let dy = if below(cc) {
                            (bottom - gap - my1).min(0)
                        } else if above(cc) {
                            // (Not moved down much: a mark that would go down goes half as far.)
                            let dy = top + gap - my0;
                            if dy < 0 { dy / 2 } else { dy }
                        } else {
                            0
                        };
                        (dx, dy)
                    }
                    // (A letter without ink, a space: where the pen is after it.)
                    _ => (base.advance, 0),
                };
                if let Some((_, my0, _, my1)) = bounds {
                    top = top.max(my1 + dy);
                    bottom = bottom.min(my0 + dy);
                }
                if same {
                    stack.push((mark.glyph, (dx, dy)));
                }
                out[k].drawn = Some((mark, start + dx, dy));
            }
            i = end;
        }
        out
    }

    /// Each glyph of `text` in `face` as it's drawn along the line: where (font units from its start, and up from
    /// the baseline), and how it's set.
    fn line(&self, text: &str, face: Face) -> Vec<(i32, i32, Set)> {
        let chars: Vec<(char, Face)> = text.chars().map(|c| (c, face)).collect();
        self.shape(&chars).into_iter().filter_map(|p| p.drawn.map(|(s, x, y)| (x, y, s))).collect()
    }

    /// The width of `text` at `size`.
    pub fn width(&self, text: &str, size: i32) -> i32 {
        self.width_in(text, Face::Regular, size)
    }

    /// The width of `text` in `face` at `size`.
    pub fn width_in(&self, text: &str, face: Face, size: i32) -> i32 {
        let chars: Vec<(char, Face)> = text.chars().map(|c| (c, face)).collect();
        self.scale(self.shape(&chars).last().map_or(0, |p| p.pen + p.advance), size)
    }

    /// Where the character at `at` (a byte in `text`, set in the regular face) starts along the line, at `size`:
    /// where the caret goes before it (after the kerning with the one before: it starts where it's drawn; before a
    /// mark, after its letter).
    pub fn x_at(&self, text: &str, at: usize, size: i32) -> i32 {
        let chars: Vec<(char, Face)> = text.chars().map(|c| (c, Face::Regular)).collect();
        let placed = self.shape(&chars);
        let index = text[..text.len().min(at)].chars().count();
        let width = placed.last().map_or(0, |p| p.pen + p.advance);
        let x = match placed.get(index) {
            Some(p) if p.mark => p.pen,
            // (Before what shows nothing, where the next that shows is.)
            _ => placed[index.min(placed.len())..].iter().find(|p| p.drawn.is_some() || p.mark).map_or(width, |p| p.pen),
        };
        self.scale(x, size)
    }

    /// The kerning between two styled characters side by side, at `size` (for text drawn in runs of one style, to
    /// put each run where the line was measured to have it).
    pub fn kern_between(&self, a: (char, Face), b: (char, Face), size: i32) -> i32 {
        match (self.set(a.0, a.1), self.set(b.0, b.1)) {
            (Some(x), Some(y)) => self.scale(self.kern(x, y), size),
            _ => 0,
        }
    }

    /// Where styled text (each character with its face) breaks into lines no wider than `max` at `size`, as
    /// `breaks` does: ranges of `chars`, without the line breaks and the spaces lines broke at.
    pub fn breaks_styled(&self, chars: &[(char, Face)], size: i32, max: i32) -> Vec<Range<usize>> {
        self.break_lines(chars.iter().enumerate().map(|(i, &(c, f))| (i, i + 1, c, f)), chars.len(), size, max)
    }

    /// Where `text` breaks into lines no wider than `max` (at `size`): at spaces, and between the characters of
    /// Chinese, Japanese and Korean (`breaks_between`); inside a word only if it's longer than a line; and after every
    /// line break ('\n', not part of any line). One pass, however long.
    pub fn breaks(&self, text: &str, size: i32, max: i32) -> Vec<Range<usize>> {
        self.break_lines(text.char_indices().map(|(i, c)| (i, i + c.len_utf8(), c, Face::Regular)), text.len(), size, max)
    }

    /// Lines no wider than `max` from characters (each where it is and where the next is, as `end` counts, and its
    /// face): ranges of those places, without the line breaks and the spaces lines broke at. Measured as they're
    /// drawn, kerning, accents and all.
    fn break_lines(&self, chars: impl Iterator<Item = (usize, usize, char, Face)>, end: usize, size: i32, max: i32) -> Vec<Range<usize>> {
        let items: Vec<(usize, usize, char, Face)> = chars.collect();
        let styled: Vec<(char, Face)> = items.iter().map(|&(_, _, c, f)| (c, f)).collect();
        let placed = self.shape(&styled);
        let upem = self.font.metrics.units_per_em as i64;
        let max = (max.max(0) as i64 * upem / size.max(1) as i64) as i32; // in the font's units
        let mut lines = Vec::new();
        let mut start = 0;
        let mut width = 0; // of the line so far, in font units
        // The last place this line can break: where it would end, where the next would start (after a space, which
        // goes; or before a character, between Chinese, Japanese or Korean ones), and the line's width up to there;
        // and the kerning between the character after it and the one before (gone if the line breaks there).
        let mut can: Option<(usize, usize, i32)> = None;
        let mut kern_after = 0;
        // The character before this one on the line, and whether this one is the first after the last place the
        // line can break.
        let mut before: Option<char> = None;
        let mut first = false;
        for (&(i, after, c, _), p) in items.iter().zip(&placed) {
            if c == '\n' {
                lines.push(start..i);
                (start, width, can, before, first) = (i + 1, 0, None, None, false);
                continue;
            }
            // (What shows nothing takes no room; a mark goes with its letter.)
            if p.mark || p.drawn.is_none() {
                continue;
            }
            let mut k = p.kern;
            // (Not right after a space: the break there is the space's, the space going with it.)
            if let Some(b) = before
                && !space(b)
                && !space(c)
                && i > start
                && breaks_between(b, c)
            {
                can = Some((i, i, width));
                first = true;
            }
            let after_break = core::mem::take(&mut first);
            before = Some(c);
            if space(c) {
                width += k + p.advance;
                can = Some((i, after, width));
                first = true;
                continue;
            }
            if after_break {
                kern_after = k;
            }
            if width + k + p.advance > max && i > start {
                match can {
                    Some((at, next, upto)) if at > start => {
                        lines.push(start..at);
                        start = next;
                        // (What's after the break is the line now, without its kerning with what was before it.)
                        width -= upto + kern_after;
                        if after_break {
                            (width, k) = (0, 0);
                        }
                    }
                    _ => {
                        lines.push(start..i);
                        start = i;
                        (width, k) = (0, 0);
                    }
                }
                can = None;
            }
            width += k + p.advance;
        }
        lines.push(start..end);
        lines
    }

    /// Where the baseline is, below the top of a line box in `style`, in 1/256 pixel.
    pub fn baseline(&self, style: Style) -> i32 {
        let m = self.font.metrics;
        let content = self.scale(m.ascender as i32 - m.descender as i32, style.size);
        (style.line_height - content) / 2 + self.scale(m.ascender as i32, style.size)
    }

    /// Splits `text` into lines no wider than `max`, breaking at spaces.
    #[allow(dead_code)]
    pub fn wrap<'t>(&self, text: &'t str, size: i32, max: i32) -> Vec<&'t str> {
        let mut lines = Vec::new();
        let mut start = 0; // where the current line starts
        let mut end = 0; // where it ends if broken at the last space seen
        for (i, c) in text.char_indices() {
            if c == ' ' {
                if self.width(&text[start..i], size) > max && end > start {
                    lines.push(&text[start..end]);
                    start = end + 1;
                }
                end = i;
            }
        }
        if self.width(&text[start..], size) > max && end > start {
            lines.push(&text[start..end]);
            start = end + 1;
        }
        lines.push(&text[start..]);
        lines
    }

    /// Draws one line of text. `top` is the fraction of a pixel (0..256) the line box starts below the
    /// pixel row it will be placed on.
    pub fn ink(&self, text: &str, style: Style, top: i32) -> Ink {
        self.ink_in(text, Face::Regular, style, top)
    }

    /// Draws one line of text in `face`. (All of them sit on the text font's baseline: the code font has the same
    /// ascender and descender.)
    pub fn ink_in(&self, text: &str, face: Face, style: Style, top: i32) -> Ink {
        let m = self.font.metrics;
        let width = (self.width_in(text, face, style.size) + ONE - 1) / ONE;
        // (A slanted line's tops lean past its last letter: about a fifth of the size.)
        let lean = if face.slanted() { (style.size / 5 + ONE - 1) / ONE } else { 0 } as usize;
        let w = width as usize + 2 * PAD + lean;
        let h = ((top + style.line_height + ONE - 1) / ONE) as usize;
        // The font's own height (ascender to descender), centred in the line box, as CSS does.
        let content = self.scale(m.ascender as i32 - m.descender as i32, style.size);
        let baseline = top + (style.line_height - content) / 2 + self.scale(m.ascender as i32, style.size);
        let mut raster = Raster::new(w, h);
        let mut pen = Pen::new(&mut raster, style.size, m.units_per_em);
        pen.y = baseline;
        let apart = made_up_bold(style.size);
        for (units, up, s) in self.line(text, face) {
            let font = self.font_of(s.source);
            let draw = |out: &mut dyn Outline| match &self.bold {
                Some(at) if face.bold() && s.source == Source::Text => font.outline_at(s.glyph, at, out),
                _ => font.outline(s.glyph, out),
            };
            // (In bold, a fallback font's glyph drawn twice, a little apart, as browsers make a bold up: it has none.)
            let strikes: &[i32] = if face.bold() && s.source != Source::Text { &[0, apart] } else { &[0] };
            for &dx in strikes {
                pen.x = PAD as i32 * ONE + self.scale(units, style.size) + dx;
                pen.y = baseline - self.scale(up, style.size);
                if face.slanted() {
                    draw(&mut Slant { to: &mut pen });
                } else {
                    draw(&mut pen);
                }
            }
        }
        let mut coverage = vec![0; w * h];
        raster.coverage(&mut coverage);
        Ink { width: w, height: h, coverage, left: PAD }
    }
}

#[cfg(test)]
mod tests;
