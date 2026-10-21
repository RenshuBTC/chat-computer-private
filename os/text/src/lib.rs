//! Text in Atkinson Hyperlegible Next (its regular and bold, both from the one variable font file) and, for code,
//! Atkinson Hyperlegible Mono, set with Chat Computer's own font engine: measuring, wrapping and drawing lines.
//! Characters they don't have (arrows, check marks, maths, Greek and Cyrillic, emoji, and box drawing in code) come
//! from fallback fonts made from Google's Noto fonts (`os/fonts/`), as a browser would take them from the system's.
//! Letters side by side are kerned as the font says ("AV", "To" and "y." closer), at its regular weight and its bold,
//! as a browser does: within a run of one font and face, so not across a change to bold or to a fallback font.
//! Sizes are in 1/256 of a pixel.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;
use font::raster::{Pen, Raster, ONE};
use font::{Font, Instance, Kerning, Outline};

static ATKINSON: &[u8] = include_bytes!("../../../chatcomputer/ui/assets/fonts/AtkinsonHyperlegibleNext.ttf");
static MONO: &[u8] = include_bytes!("../../../chatcomputer/ui/assets/fonts/AtkinsonHyperlegibleMono.ttf");
static FALLBACK: &[u8] = include_bytes!("../../fonts/ChatComputerFallback.ttf");
static FALLBACK_MONO: &[u8] = include_bytes!("../../fonts/ChatComputerFallbackMono.ttf");

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
}

/// Characters that show nothing and take no room: joiners and zero-width spaces, variation selectors (the "as an
/// emoji" after ❤ in ❤️), the byte order mark, and the soft hyphen. (Nor do they come between letters that kern, as
/// HarfBuzz leaves them out.)
fn invisible(c: char) -> bool {
    matches!(c, '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}')
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
    /// The bold instance, and every glyph's advance in it (worked out once: they come from the glyphs' variations).
    bold: Option<Instance>,
    bold_advances: Vec<u16>,
    /// The text font's kerning, at its regular weight and its bold. (The code font has none, nor have the fallbacks.)
    kerning: Kerning<'static>,
    bold_kerning: Kerning<'static>,
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
    pub fn atkinson() -> Type {
        let font = Font::parse(ATKINSON).expect("the built-in font is damaged");
        let mono = Font::parse(MONO).expect("the built-in code font is damaged");
        let fallback = Font::parse(FALLBACK).expect("the built-in fallback font is damaged");
        let fallback_mono = Font::parse(FALLBACK_MONO).expect("the built-in fallback code font is damaged");
        // (Glyphs from every font are drawn to the same scale: all have 1000 units to the em.)
        let upem = font.metrics.units_per_em;
        assert!([&mono, &fallback, &fallback_mono].iter().all(|f| f.metrics.units_per_em == upem), "the fonts' sizes differ");
        let bold = font.instance(b"wght", 700);
        let bold_advances = match &bold {
            Some(at) => font.advances_at(at, 0..font.glyph_count()),
            None => Vec::new(),
        };
        let (kerning, bold_kerning) = (font.kerning(None), font.kerning(bold.as_ref()));
        Type { font, mono, fallback, fallback_mono, bold, bold_advances, kerning, bold_kerning }
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
        }
    }

    /// Where a character's glyph is for `face`: in the face's own font, else in a fallback font; else the face's own
    /// "missing" glyph (a box), for characters none has (Chinese, Japanese and Korean, for now).
    fn glyph(&self, c: char, face: Face) -> (Source, u16) {
        let order: [Source; 3] = if face == Face::Mono {
            [Source::Mono, Source::FallbackMono, Source::Fallback]
        } else {
            [Source::Text, Source::Fallback, Source::FallbackMono]
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

    /// Where each character of `text` in `face` is set along the line (font units from its start), with how it's set:
    /// its advance, and the kerning with the one before it, already in.
    fn line<'a>(&'a self, text: &'a str, face: Face) -> impl Iterator<Item = (i32, Set)> + 'a {
        let (mut x, mut before): (i32, Option<Set>) = (0, None);
        text.chars().filter_map(move |c| {
            let s = self.set(c, face)?;
            x += before.map_or(0, |b| self.kern(b, s));
            let at = x;
            x += s.advance;
            before = Some(s);
            Some((at, s))
        })
    }

    /// The width of `text` at `size`.
    pub fn width(&self, text: &str, size: i32) -> i32 {
        self.width_in(text, Face::Regular, size)
    }

    /// The width of `text` in `face` at `size`.
    pub fn width_in(&self, text: &str, face: Face, size: i32) -> i32 {
        self.scale(self.line(text, face).last().map_or(0, |(x, s)| x + s.advance), size)
    }

    /// Where the character at `at` (a byte in `text`, set in the regular face) starts along the line, at `size`:
    /// where the caret goes before it (after the kerning with the one before: it starts where it's drawn).
    pub fn x_at(&self, text: &str, at: usize, size: i32) -> i32 {
        let end = text.len().min(at);
        let before = self.line(&text[..end], Face::Regular).last().map_or(0, |(x, s)| x + s.advance);
        let next = self.line(&text[..end], Face::Regular).last().zip(text[end..].chars().find_map(|c| self.set(c, Face::Regular)));
        self.scale(before + next.map_or(0, |((_, a), b)| self.kern(a, b)), size)
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
        self.break_lines(chars.iter().enumerate().map(|(i, &(c, f))| (i, c, f)), chars.len(), size, max)
    }

    /// Where `text` breaks into lines no wider than `max` (at `size`): at spaces, inside a word only if it's
    /// longer than a line, and after every line break ('\n', not part of any line). One pass, however long.
    pub fn breaks(&self, text: &str, size: i32, max: i32) -> Vec<Range<usize>> {
        self.break_lines(text.char_indices().map(|(i, c)| (i, c, Face::Regular)), text.len(), size, max)
    }

    /// Lines no wider than `max` from characters (each where it is, as `end` counts, and its face): ranges of those
    /// places, without the line breaks and the spaces lines broke at. Measured as they're drawn, kerning and all.
    fn break_lines(&self, chars: impl Iterator<Item = (usize, char, Face)>, end: usize, size: i32, max: i32) -> Vec<Range<usize>> {
        let upem = self.font.metrics.units_per_em as i64;
        let max = (max.max(0) as i64 * upem / size.max(1) as i64) as i32; // in the font's units
        let mut lines = Vec::new();
        let mut start = 0;
        let mut width = 0; // of the line so far, in font units
        // The last space on this line: where it is, and the line's width up to just after it; and the kerning between
        // it and the character after it (gone if the line breaks there).
        let mut space: Option<(usize, i32)> = None;
        let mut after_space = 0;
        // The character before this one on the line, and whether it was a space.
        let (mut before, mut spaced): (Option<Set>, bool) = (None, false);
        for (i, c, face) in chars {
            if c == '\n' {
                lines.push(start..i);
                (start, width, space, before, spaced) = (i + 1, 0, None, None, false);
                continue;
            }
            let Some(s) = self.set(c, face) else { continue };
            let mut k = before.map_or(0, |b| self.kern(b, s));
            let first_after_space = spaced;
            (before, spaced) = (Some(s), c == ' ');
            if c == ' ' {
                width += k + s.advance;
                space = Some((i, width));
                continue;
            }
            if first_after_space {
                after_space = k;
            }
            if width + k + s.advance > max && i > start {
                match space {
                    Some((at, after)) if at > start => {
                        lines.push(start..at);
                        start = at + 1;
                        // (What's after the space is the line now, without its kerning with the space.)
                        width -= after + after_space;
                        if first_after_space {
                            (width, k) = (0, 0);
                        }
                    }
                    _ => {
                        lines.push(start..i);
                        start = i;
                        (width, k) = (0, 0);
                    }
                }
                space = None;
            }
            width += k + s.advance;
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
        for (units, s) in self.line(text, face) {
            pen.x = PAD as i32 * ONE + self.scale(units, style.size);
            let font = self.font_of(s.source);
            let draw = |out: &mut dyn Outline| match &self.bold {
                Some(at) if face.bold() && s.source == Source::Text => font.outline_at(s.glyph, at, out),
                _ => font.outline(s.glyph, out),
            };
            if face.slanted() {
                draw(&mut Slant { to: &mut pen });
            } else {
                draw(&mut pen);
            }
        }
        let mut coverage = vec![0; w * h];
        raster.coverage(&mut coverage);
        Ink { width: w, height: h, coverage, left: PAD }
    }
}

#[cfg(test)]
mod tests;
