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

pub mod breaks;
mod breaks_data;
mod compose;
pub mod loaded;

use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;
use font::raster::{Pen, Raster, ONE};
use font::{Font, Instance, Kerning, Marks};
pub use font::Outline;

static LORA: &[u8] = include_bytes!("../../../chatcomputer/ui/assets/fonts/Lora.ttf");
static MONO: &[u8] = include_bytes!("../../../chatcomputer/ui/assets/fonts/AtkinsonHyperlegibleMono.ttf");
static FALLBACK: &[u8] = include_bytes!("../../fonts/ChatComputerFallback.ttf");
static FALLBACK_MONO: &[u8] = include_bytes!("../../fonts/ChatComputerFallbackMono.ttf");
static CJK: &[u8] = include_bytes!("../../fonts/ChatComputerCJK.ttf");
/// The sans-serif family, for web pages that ask for one: Atkinson Hyperlegible Next (the font engine's test font,
/// and Chat Computer's own before Lora; a variable font from weight 200 to 800, OFL: `font/testdata`).
static SANS: &[u8] = include_bytes!("../../font/testdata/AtkinsonHyperlegibleNext.ttf");
/// Web pages' fonts: the metric twins of Chrome's defaults on Windows (Times New Roman, Arial, Courier New), so a
/// page's words take the room, and its lines the height, they take in Chrome (`fonts/build_web.py`: from Tinos, Arimo
/// and Cousine, OFL). Their regular, bold, italic and bold italic; the sans-serif's two are variable (its bold an
/// instance).
static WEB: [&[u8]; 10] = [
    include_bytes!("../../fonts/ChatComputerWebSerif-Regular.ttf"),
    include_bytes!("../../fonts/ChatComputerWebSerif-Bold.ttf"),
    include_bytes!("../../fonts/ChatComputerWebSerif-Italic.ttf"),
    include_bytes!("../../fonts/ChatComputerWebSerif-BoldItalic.ttf"),
    include_bytes!("../../fonts/ChatComputerWebSans.ttf"),
    include_bytes!("../../fonts/ChatComputerWebSans-Italic.ttf"),
    include_bytes!("../../fonts/ChatComputerWebMono-Regular.ttf"),
    include_bytes!("../../fonts/ChatComputerWebMono-Bold.ttf"),
    include_bytes!("../../fonts/ChatComputerWebMono-Italic.ttf"),
    include_bytes!("../../fonts/ChatComputerWebMono-BoldItalic.ttf"),
];
/// Their ascent, descent and line gap as the originals have them, in 2048ths of the em (exactly Times New Roman's,
/// Arial's and Courier New's, whose metric twins they are): a line's height is worked out from these as Chrome does.
const WEB_METRICS: [(i32, i32, i32); 3] = [(1825, 443, 87), (1854, 434, 67), (1705, 615, 0)];
/// Their x-height, the width of their "0" and their cap height, in 2048ths of the em: CSS's ex, ch and cap, as Chrome
/// on Windows measures them in Times New Roman, Arial and Courier New.
const WEB_UNITS: [(i32, i32, i32); 3] = [(916, 1024, 1356), (1062, 1139, 1467), (866, 1229, 1170)];

/// A web page's kind of font: Chrome's default serif (Times New Roman), sans-serif (Arial) or monospace (Courier New).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    Serif,
    Sans,
    Mono,
}

impl Family {
    /// A line's height in `normal` line height at `size` pixels, as Chrome works it out: the font's ascent, descent
    /// and line gap, each rounded to whole pixels.
    pub fn normal_line(self, size: f32) -> f32 {
        let (a, d, g) = self.rounded(size);
        a + d + g
    }

    /// Its ex, ch, cap and ic, as parts of the em (ic: the CJK fallback font's ideographs, an em wide).
    pub fn units(self) -> [f32; 4] {
        let (x, ch, cap) = WEB_UNITS[self as usize];
        [x as f32 / 2048.0, ch as f32 / 2048.0, cap as f32 / 2048.0, 1.0]
    }

    /// Its ascent, descent and line gap at `size` pixels, each rounded to a whole pixel (as Chrome's font metrics are).
    pub fn rounded(self, size: f32) -> (f32, f32, f32) {
        let (a, d, g) = WEB_METRICS[self as usize];
        let px = |units: i32| libm_round(units as f32 * size / 2048.0);
        (px(a), px(d), px(g))
    }
}

/// Rounds halves away from zero (f32::round isn't in core).
fn libm_round(x: f32) -> f32 {
    let t = x as i64 as f32;
    let f = x - t;
    if f >= 0.5 {
        t + 1.0
    } else if f <= -0.5 {
        t - 1.0
    } else {
        t
    }
}

/// Some letters reach a little past their own width (an italic f, an accent); the drawn line keeps
/// this much room on each side, in pixels.
const PAD: usize = 2;

/// Room added between letters and after spaces (CSS's letter-spacing and word-spacing), in 1/256 pixel.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Spacing {
    pub letter: i32,
    pub word: i32,
}

impl Spacing {
    /// What it adds to a line of characters `chars` (shaped into `placed`): after each letter (a mark goes with its
    /// letter) and after each space.
    fn added(self, chars: &[(char, Face)], placed: &[Placed]) -> i32 {
        let letters = placed.iter().filter(|p| !p.mark).count() as i32;
        let spaces = chars.iter().filter(|c| matches!(c.0, ' ' | '\u{a0}')).count() as i32;
        self.letter * letters + self.word * spaces
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub size: i32,
    pub line_height: i32,
}

/// Which of the fonts: the text's own, bold (weight 700), slanted ("italic": the regular or bold glyphs leaned 12°,
/// as browsers do for a font without its italic), or the code font; or the sans-serif font, in the same four (for
/// web pages).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Face {
    Regular,
    Bold,
    Italic,
    BoldItalic,
    Mono,
    Sans,
    SansBold,
    SansItalic,
    SansBoldItalic,
    /// A web page's: its family, and whether it's bold and italic (each a font of its own, or an instance of one).
    Web(Family, bool, bool),
    /// A font the page loaded (loaded.rs: by its number), the family whose web font has the characters it lacks, and
    /// whether it's drawn bold (its bold instance, if it's variable, else made up) and leaned (an italic made up).
    Loaded(u16, Family, bool, bool),
}

impl Face {
    fn bold(self) -> bool {
        matches!(self, Face::Bold | Face::BoldItalic | Face::SansBold | Face::SansBoldItalic | Face::Web(_, true, _) | Face::Loaded(_, _, true, _))
    }
    /// Leaned by the engine (a font without its italic): never a web face, whose italics are fonts of their own.
    fn slanted(self) -> bool {
        matches!(self, Face::Italic | Face::BoldItalic | Face::SansItalic | Face::SansBoldItalic | Face::Loaded(_, _, _, true))
    }
    fn sans(self) -> bool {
        matches!(self, Face::Sans | Face::SansBold | Face::SansItalic | Face::SansBoldItalic)
    }

    /// A number for each face (for keeping what's been drawn or measured in each).
    pub fn key(self) -> u16 {
        match self {
            Face::Regular => 0,
            Face::Bold => 1,
            Face::Italic => 2,
            Face::BoldItalic => 3,
            Face::Mono => 4,
            Face::Sans => 5,
            Face::SansBold => 6,
            Face::SansItalic => 7,
            Face::SansBoldItalic => 8,
            Face::Web(f, bold, italic) => 9 + (f as u16) * 4 + bold as u16 + 2 * italic as u16,
            Face::Loaded(id, f, bold, italic) => 32 + (id.min(4000) * 3 + f as u16) * 4 + bold as u16 + 2 * italic as u16,
        }
    }

    /// Which of the web fonts a web face is drawn in, and whether in its bold instance (a variable font's). (A loaded
    /// font's face: the web font for the characters it lacks.)
    fn web(self) -> Option<(usize, bool)> {
        let (family, bold, italic) = match self {
            Face::Web(family, bold, italic) => (family, bold, italic),
            Face::Loaded(_, family, bold, italic) => (family, bold, italic),
            _ => return None,
        };
        Some(match family {
            Family::Serif => (bold as usize + 2 * italic as usize, false),
            Family::Sans => (4 + italic as usize, bold),
            Family::Mono => (6 + bold as usize + 2 * italic as usize, false),
        })
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

/// An outline scaled (a loaded font's, from its own units to the built-in fonts' 1000 to the em).
struct Scale<'a> {
    to: &'a mut dyn Outline,
    num: i64,
    den: i64,
}

impl Scale<'_> {
    fn s(&self, v: i32) -> i32 {
        let n = v as i64 * self.num;
        ((n + if n >= 0 { self.den / 2 } else { -self.den / 2 }) / self.den) as i32
    }
}

impl Outline for Scale<'_> {
    fn move_to(&mut self, x: i32, y: i32) {
        let (x, y) = (self.s(x), self.s(y));
        self.to.move_to(x, y);
    }
    fn line_to(&mut self, x: i32, y: i32) {
        let (x, y) = (self.s(x), self.s(y));
        self.to.line_to(x, y);
    }
    fn quad_to(&mut self, cx: i32, cy: i32, x: i32, y: i32) {
        let (cx, cy, x, y) = (self.s(cx), self.s(cy), self.s(x), self.s(y));
        self.to.quad_to(cx, cy, x, y);
    }
    fn close(&mut self) {
        self.to.close();
    }
}

/// An outline moved along (a glyph to its place on the line).
struct Shift<'a> {
    to: &'a mut dyn Outline,
    dx: i32,
    dy: i32,
}

impl Outline for Shift<'_> {
    fn move_to(&mut self, x: i32, y: i32) {
        self.to.move_to(x + self.dx, y + self.dy);
    }
    fn line_to(&mut self, x: i32, y: i32) {
        self.to.line_to(x + self.dx, y + self.dy);
    }
    fn quad_to(&mut self, cx: i32, cy: i32, x: i32, y: i32) {
        self.to.quad_to(cx + self.dx, cy + self.dy, x + self.dx, y + self.dy);
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
    Sans,
    /// Characters the text font lacks (in its weight only: there's no bold of them).
    Fallback,
    /// Characters the code font lacks, a code character wide (box drawing lines meet).
    FallbackMono,
    /// Chinese, Japanese and Korean (in their regular weight only too).
    Cjk,
    /// One of the web fonts (by its place in WEB).
    Web(u8),
    /// A font a page loaded (by its number).
    Loaded(u16),
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
    /// The sans-serif font, with the same: its bold, its bold's advances, its kerning and mark positioning.
    sans: Font<'static>,
    sans_bold: Option<Instance>,
    sans_bold_advances: Vec<u16>,
    sans_kerning: Kerning<'static>,
    sans_bold_kerning: Kerning<'static>,
    sans_marks: Marks<'static>,
    sans_bold_marks: Marks<'static>,
    /// The web fonts (as WEB has them).
    web: Vec<WebFont>,
}

/// One of web pages' fonts: the font, its bold instance if it's variable (with every glyph's advance in it), and its
/// kerning and mark positioning, at its default and at its bold.
struct WebFont {
    font: Font<'static>,
    bold: Option<Instance>,
    bold_advances: Vec<u16>,
    kerning: Kerning<'static>,
    bold_kerning: Kerning<'static>,
    marks: Marks<'static>,
    bold_marks: Marks<'static>,
}

impl WebFont {
    fn new(data: &'static [u8], upem: u16) -> WebFont {
        let font = Font::parse(data).expect("a built-in web font is damaged");
        assert!(font.metrics.units_per_em == upem, "a web font's size differs");
        let bold = font.instance(b"wght", 700);
        let bold_advances = match &bold {
            Some(at) => font.advances_at(at, 0..font.glyph_count()),
            None => Vec::new(),
        };
        let (kerning, bold_kerning) = (font.kerning(None), font.kerning(bold.as_ref()));
        let (marks, bold_marks) = (font.marks(None), font.marks(bold.as_ref()));
        WebFont { font, bold, bold_advances, kerning, bold_kerning, marks, bold_marks }
    }
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
    /// How far above the line box's top it begins, in pixels (glyphs taller than the line: `ink_overflowing`).
    pub above: usize,
}

impl Type {
    /// One Type of the built-in fonts for all who share it (made the first time it's asked for): for work that
    /// isn't handed one (a page's scripts asking where an element is).
    pub fn shared() -> &'static Type {
        use core::sync::atomic::{AtomicPtr, Ordering};
        static SHARED: AtomicPtr<Type> = AtomicPtr::new(core::ptr::null_mut());
        let p = SHARED.load(Ordering::Acquire);
        if !p.is_null() {
            // SAFETY: a non-null pointer here is a Type leaked below, never freed or changed.
            return unsafe { &*p };
        }
        let fresh = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Type::builtin()));
        match SHARED.compare_exchange(core::ptr::null_mut(), fresh, Ordering::AcqRel, Ordering::Acquire) {
            // SAFETY: just made from a Box, and kept for good.
            Ok(_) => unsafe { &*fresh },
            Err(other) => {
                // (Another core made one first: this one goes, theirs is kept.)
                // SAFETY: `fresh` came from Box::into_raw above and was never shared.
                drop(unsafe { alloc::boxed::Box::from_raw(fresh) });
                // SAFETY: as above, a leaked Type.
                unsafe { &*other }
            }
        }
    }

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
        let sans = Font::parse(SANS).expect("the built-in sans-serif font is damaged");
        assert!(sans.metrics.units_per_em == upem, "the sans-serif font's size differs");
        let sans_bold = sans.instance(b"wght", 700);
        let sans_bold_advances = match &sans_bold {
            Some(at) => sans.advances_at(at, 0..sans.glyph_count()),
            None => Vec::new(),
        };
        let (sans_kerning, sans_bold_kerning) = (sans.kerning(None), sans.kerning(sans_bold.as_ref()));
        let (sans_marks, sans_bold_marks) = (sans.marks(None), sans.marks(sans_bold.as_ref()));
        let web = WEB.iter().map(|&data| WebFont::new(data, upem)).collect();
        Type {
            font,
            mono,
            fallback,
            fallback_mono,
            cjk,
            bold,
            bold_advances,
            kerning,
            bold_kerning,
            marks,
            bold_marks,
            sans,
            sans_bold,
            sans_bold_advances,
            sans_kerning,
            sans_bold_kerning,
            sans_marks,
            sans_bold_marks,
            web,
        }
    }

    fn scale(&self, units: i32, size: i32) -> i32 {
        (units as i64 * size as i64 / self.font.metrics.units_per_em as i64) as i32
    }

    fn font_of(&self, source: Source) -> &Font<'static> {
        match source {
            Source::Text => &self.font,
            Source::Mono => &self.mono,
            Source::Sans => &self.sans,
            Source::Fallback => &self.fallback,
            Source::FallbackMono => &self.fallback_mono,
            Source::Cjk => &self.cjk,
            Source::Web(i) => &self.web[i as usize].font,
            Source::Loaded(id) => match loaded::get(id) {
                Some(f) => &f.font,
                None => &self.font,
            },
        }
    }

    /// A number in the units of a glyph's font in the built-in fonts' units (1000 to the em): a loaded font's scaled.
    fn units(&self, source: Source, v: i32) -> i32 {
        match source {
            Source::Loaded(id) => match loaded::get(id) {
                Some(f) if f.upem != 1000 => ((v as i64 * 1000 + if v >= 0 { f.upem as i64 / 2 } else { -(f.upem as i64) / 2 }) / f.upem as i64) as i32,
                _ => v,
            },
            _ => v,
        }
    }

    /// A glyph's ink's bounds (x0, y0, x1, y1), in the built-in fonts' units.
    fn bounds_of(&self, source: Source, glyph: u16) -> Option<(i32, i32, i32, i32)> {
        let (a, b, c, d) = self.font_of(source).bounds(glyph)?;
        Some((self.units(source, a), self.units(source, b), self.units(source, c), self.units(source, d)))
    }

    /// A glyph's outline, at the face's instance, in the built-in fonts' units.
    fn draw_glyph(&self, source: Source, glyph: u16, face: Face, out: &mut dyn Outline) -> bool {
        let font = self.font_of(source);
        let at = self.instance_of(source, face);
        let go = |o: &mut dyn Outline| match at {
            Some(at) => font.outline_at(glyph, at, o),
            None => font.outline(glyph, o),
        };
        match source {
            Source::Loaded(id) => match loaded::get(id) {
                Some(f) if f.upem != 1000 => go(&mut Scale { to: out, num: 1000, den: f.upem as i64 }),
                _ => go(out),
            },
            _ => go(out),
        }
    }

    /// The variable instance a glyph from `source` is drawn at in `face` (its bold), if there's one.
    fn instance_of(&self, source: Source, face: Face) -> Option<&Instance> {
        if !face.bold() {
            return None;
        }
        match source {
            Source::Text => self.bold.as_ref(),
            Source::Sans => self.sans_bold.as_ref(),
            Source::Web(i) => self.web[i as usize].bold.as_ref(),
            Source::Loaded(id) => loaded::get(id).and_then(|f| f.bold.as_ref()),
            _ => None,
        }
    }

    /// Whether a glyph from `source` in `face` is drawn twice, a little apart, to make it bold: a bold face's, from a
    /// font with no bold of its own (a fallback).
    fn made_bold(&self, source: Source, face: Face) -> bool {
        face.bold()
            && match source {
                Source::Text => self.bold.is_none(),
                Source::Sans => self.sans_bold.is_none(),
                // (A web face's bold is a font of its own, or an instance of its variable one; a loaded font's, its
                // instance if it's variable, else made up.)
                Source::Web(_) => false,
                Source::Loaded(id) => loaded::get(id).is_none_or(|f| f.bold.is_none()),
                _ => true,
            }
    }

    /// Where a character's glyph is for `face`: in the face's own font, else in a fallback font; else the face's own
    /// "missing" glyph (a box), for characters none has.
    fn glyph(&self, c: char, face: Face) -> (Source, u16) {
        // A loaded font: its own glyphs, then its family's web font's, then the fallbacks'.
        if let Face::Loaded(id, family, ..) = face
            && let Some((i, _)) = face.web()
        {
            let own = Source::Loaded(id);
            if loaded::get(id).is_some() {
                let glyph = self.font_of(own).glyph_index(c);
                if glyph != 0 {
                    return (own, glyph);
                }
            }
            let web = Source::Web(i as u8);
            let order = if family == Family::Mono {
                [web, Source::FallbackMono, Source::Fallback, Source::Text, Source::Cjk]
            } else {
                [web, Source::Fallback, Source::Text, Source::FallbackMono, Source::Cjk]
            };
            for source in order {
                let glyph = self.font_of(source).glyph_index(c);
                if glyph != 0 {
                    return (source, glyph);
                }
            }
            return (web, 0);
        }
        // A web face: its own font, then the fallbacks (the code font's first, for monospace), then the text font's.
        if let Some((i, _)) = face.web() {
            let own = Source::Web(i as u8);
            let order = if matches!(face, Face::Web(Family::Mono, ..)) {
                [own, Source::FallbackMono, Source::Fallback, Source::Text, Source::Cjk]
            } else {
                [own, Source::Fallback, Source::Text, Source::FallbackMono, Source::Cjk]
            };
            for source in order {
                let glyph = self.font_of(source).glyph_index(c);
                if glyph != 0 {
                    return (source, glyph);
                }
            }
            return (own, 0);
        }
        // (The sans-serif font has fewer letters than the text font: the fallbacks have only what the text font
        // lacks, so the text font's come before them.)
        let order: &[Source] = if face == Face::Mono {
            &[Source::Mono, Source::FallbackMono, Source::Fallback, Source::Cjk]
        } else if face.sans() {
            &[Source::Sans, Source::Fallback, Source::Text, Source::FallbackMono, Source::Cjk]
        } else {
            &[Source::Text, Source::Fallback, Source::FallbackMono, Source::Cjk]
        };
        for &source in order {
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
            Source::Sans => match self.sans_bold_advances.get(glyph as usize) {
                Some(&a) if face.bold() => a,
                _ => self.sans.advance(glyph),
            },
            Source::Web(i) => match self.web[i as usize].bold_advances.get(glyph as usize) {
                Some(&a) if face.bold() => a,
                _ => self.web[i as usize].font.advance(glyph),
            },
            Source::Loaded(id) => {
                let f = loaded::get(id)?;
                let a = match f.bold_advances.get(glyph as usize) {
                    Some(&a) if face.bold() => a,
                    _ => f.font.advance(glyph),
                };
                return Some(Set { source, glyph, face, advance: self.units(source, a as i32) });
            }
            _ => self.font_of(source).advance(glyph),
        };
        Some(Set { source, glyph, face, advance: advance as i32 })
    }

    /// The kerning between two characters set side by side, in font units: the text font's own, between its glyphs in
    /// one face (as a browser shapes each run of one font and style by itself).
    fn kern(&self, a: Set, b: Set) -> i32 {
        if a.face != b.face || a.source != b.source {
            return 0;
        }
        match (a.source, a.face.bold()) {
            (Source::Text, true) => self.bold_kerning.pair(a.glyph, b.glyph),
            (Source::Text, false) => self.kerning.pair(a.glyph, b.glyph),
            (Source::Sans, true) => self.sans_bold_kerning.pair(a.glyph, b.glyph),
            (Source::Sans, false) => self.sans_kerning.pair(a.glyph, b.glyph),
            (Source::Web(i), bold) => {
                let w = &self.web[i as usize];
                if bold && w.bold.is_some() { w.bold_kerning.pair(a.glyph, b.glyph) } else { w.kerning.pair(a.glyph, b.glyph) }
            }
            (Source::Loaded(id), bold) => match loaded::get(id) {
                Some(f) => {
                    let k = if bold && f.bold.is_some() { f.bold_kerning.pair(a.glyph, b.glyph) } else { f.kerning.pair(a.glyph, b.glyph) };
                    self.units(a.source, k)
                }
                None => 0,
            },
            _ => 0,
        }
    }

    /// The mark positioning of the font a glyph is from, in `face` (only the text font has any: the fallbacks are made
    /// without it).
    fn marks_of(&self, source: Source, face: Face) -> Option<&Marks<'static>> {
        match source {
            Source::Text if face.bold() => Some(&self.bold_marks),
            Source::Text => Some(&self.marks),
            Source::Sans if face.bold() => Some(&self.sans_bold_marks),
            Source::Sans => Some(&self.sans_marks),
            Source::Web(i) => {
                let w = &self.web[i as usize];
                Some(if face.bold() && w.bold.is_some() { &w.bold_marks } else { &w.marks })
            }
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
            let ink = self.bounds_of(base.source, base.glyph);
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
                let bounds = self.bounds_of(mark.source, mark.glyph);
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
    #[cfg(test)]
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

    /// Where the baseline is in `face`, below the top of a line box in `style`, in 1/256 pixel. A web face's as Chrome
    /// puts it: its font's ascent and descent (rounded to whole pixels) in the middle of the line, the leading left
    /// over shared out above and below, the odd pixel below.
    pub fn baseline_in(&self, face: Face, style: Style) -> i32 {
        let (a, d, _) = match face {
            Face::Web(family, ..) => family.rounded(style.size as f32 / ONE as f32),
            Face::Loaded(id, family, ..) => {
                loaded::rounded(id, style.size as f32 / ONE as f32).unwrap_or_else(|| family.rounded(style.size as f32 / ONE as f32))
            }
            _ => return self.baseline(style),
        };
        let (a, d) = (a as i32 * ONE, d as i32 * ONE);
        // Half the leading above, in whole pixels rounded down (Chrome's floor, a negative leading's too).
        let leading = style.line_height - (a + d);
        leading.div_euclid(2 * ONE) * ONE + a
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

    /// The fonts' units to the em (all of them have the same).
    pub fn units_per_em(&self) -> i32 {
        self.font.metrics.units_per_em as i32
    }

    /// The text font's ascender and descender (both up from the baseline: the descender below 0), in font units.
    pub fn vertical(&self) -> (i32, i32) {
        (self.font.metrics.ascender as i32, self.font.metrics.descender as i32)
    }

    /// A line of text's glyphs as outlines in `face`, handed to `out` in 64ths of a font unit (`units_per_em` font
    /// units to the em, y up) from the line's start on its baseline: for drawing text at any size and through any
    /// transform (a page's canvas), filled or stroked. Its width, in font units.
    pub fn outline_line(&self, text: &str, face: Face, out: &mut dyn Outline) -> i32 {
        let chars: Vec<(char, Face)> = text.chars().map(|c| (c, face)).collect();
        let placed = self.shape(&chars);
        let width = placed.last().map_or(0, |p| p.pen + p.advance);
        for (units, up, s) in placed.into_iter().filter_map(|p| p.drawn.map(|(s, x, y)| (x, y, s))) {
            let mut shifted = Shift { to: out, dx: units * 64, dy: up * 64 };
            let draw = |o: &mut dyn Outline| self.draw_glyph(s.source, s.glyph, face, o);
            if face.slanted() {
                draw(&mut Slant { to: &mut shifted });
            } else {
                draw(&mut shifted);
            }
        }
        width
    }

    /// Draws one line of text. `top` is the fraction of a pixel (0..256) the line box starts below the
    /// pixel row it will be placed on.
    pub fn ink(&self, text: &str, style: Style, top: i32) -> Ink {
        self.ink_in(text, Face::Regular, style, top)
    }

    /// Draws one line of text in `face`. (All of them sit on the text font's baseline: the code font has the same
    /// ascender and descender. A web face's on its own: `baseline_in`.)
    pub fn ink_in(&self, text: &str, face: Face, style: Style, top: i32) -> Ink {
        self.ink_at(text, face, style, top, 0)
    }

    /// As `ink_in`, the line starting `left` (in 1/256 pixel, under a pixel) right of the ink's own left: its glyphs
    /// drawn where a fraction of a pixel puts them, as Chrome places text a quarter of a pixel at a time.
    pub fn ink_at(&self, text: &str, face: Face, style: Style, top: i32, left: i32) -> Ink {
        self.ink_spaced(text, face, style, top, left, Spacing::default())
    }

    /// The width of `text` in `face` at `size`, spaced out (CSS's letter-spacing and word-spacing).
    pub fn width_spaced(&self, text: &str, face: Face, size: i32, spacing: Spacing) -> i32 {
        let chars: Vec<(char, Face)> = text.chars().map(|c| (c, face)).collect();
        let placed = self.shape(&chars);
        let width = self.scale(placed.last().map_or(0, |p| p.pen + p.advance), size);
        width + spacing.added(&chars, &placed)
    }

    /// As `ink_at`, spaced out: `spacing.letter` more after each letter (with its marks) and `spacing.word` more after
    /// each space, as CSS's letter-spacing and word-spacing space it.
    pub fn ink_spaced(&self, text: &str, face: Face, style: Style, top: i32, left: i32, spacing: Spacing) -> Ink {
        self.ink_made(text, face, style, top, left, spacing, false)
    }

    /// As `ink_spaced`, its glyphs whole where they're taller than the line (a descender, an accent, a line height
    /// less than the font's), as a browser draws them: the coverage starts `above` the line box and goes on below it.
    pub fn ink_overflowing(&self, text: &str, face: Face, style: Style, top: i32, left: i32, spacing: Spacing) -> Ink {
        self.ink_made(text, face, style, top, left, spacing, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn ink_made(&self, text: &str, face: Face, style: Style, top: i32, left: i32, spacing: Spacing, overflow: bool) -> Ink {
        let m = self.font.metrics;
        let width = (self.width_spaced(text, face, style.size, spacing) + ONE - 1) / ONE;
        // (A slanted line's tops lean past its last letter: about a fifth of the size.)
        let lean = if face.slanted() { (style.size / 5 + ONE - 1) / ONE } else { 0 } as usize;
        let w = width as usize + 2 * PAD + lean;
        let h = ((top + style.line_height + ONE - 1) / ONE) as usize;
        // The font's own height (ascender to descender), centred in the line box, as CSS does.
        let content = self.scale(m.ascender as i32 - m.descender as i32, style.size);
        let baseline = top
            + match face {
                Face::Web(..) | Face::Loaded(..) => self.baseline_in(face, style),
                _ => (style.line_height - content) / 2 + self.scale(m.ascender as i32, style.size),
            };
        let chars: Vec<(char, Face)> = text.chars().map(|c| (c, face)).collect();
        let placed = self.shape(&chars);
        // (Rows above and below the line box for glyphs that reach past it: from their outlines' bounds, and a row to
        // spare for a bold instance's.)
        let (mut above, mut below) = (0usize, 0usize);
        if overflow {
            let (mut hi, mut lo) = (0i32, h as i32 * ONE);
            for p in &placed {
                if let Some((s, _, up)) = p.drawn
                    && let Some((_, y0, _, y1)) = self.bounds_of(s.source, s.glyph)
                {
                    hi = hi.min(baseline - self.scale(up + y1, style.size));
                    lo = lo.max(baseline - self.scale(up + y0, style.size));
                }
            }
            if hi < 0 {
                above = ((-hi + ONE - 1) / ONE) as usize + 1;
            }
            if lo > h as i32 * ONE {
                below = ((lo - h as i32 * ONE + ONE - 1) / ONE) as usize + 1;
            }
        }
        let baseline = baseline + above as i32 * ONE;
        let h = h + above + below;
        let mut raster = Raster::new(w, h);
        let mut pen = Pen::new(&mut raster, style.size, m.units_per_em);
        pen.y = baseline;
        let apart = made_up_bold(style.size);
        // (Each glyph moved on by the spacing of the letters and spaces before it; a mark's by its letter's.)
        let (mut letters, mut spaces) = (0i32, 0i32);
        for (i, p) in placed.iter().enumerate() {
            let own = if p.mark && letters > 0 { letters - 1 } else { letters };
            let spaced = spacing.letter * own + spacing.word * spaces;
            if !p.mark {
                letters += 1;
            }
            if matches!(chars[i].0, ' ' | '\u{a0}') {
                spaces += 1;
            }
            let Some((s, units, up)) = p.drawn else { continue };
            let draw = |out: &mut dyn Outline| self.draw_glyph(s.source, s.glyph, face, out);
            // (In bold, a fallback font's glyph drawn twice, a little apart, as browsers make a bold up: it has none.)
            let strikes: &[i32] = if self.made_bold(s.source, face) { &[0, apart] } else { &[0] };
            for &dx in strikes {
                pen.x = PAD as i32 * ONE + left + spaced + self.scale(units, style.size) + dx;
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
        Ink { width: w, height: h, coverage, left: PAD, above }
    }
}

#[cfg(test)]
mod tests;
