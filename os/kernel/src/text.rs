//! Text in Atkinson Hyperlegible Next (its regular and bold, both from the one variable font file) and, for code,
//! Atkinson Hyperlegible Mono, set with Chat Computer's own font engine: measuring, wrapping and drawing lines.
//! Sizes are in 1/256 of a pixel.

use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;
use font::raster::{Pen, Raster, ONE};
use font::{Font, Instance, Outline};

static ATKINSON: &[u8] = include_bytes!("../../../chatcomputer/ui/assets/fonts/AtkinsonHyperlegibleNext.ttf");
static MONO: &[u8] = include_bytes!("../../../chatcomputer/ui/assets/fonts/AtkinsonHyperlegibleMono.ttf");

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

pub struct Type {
    font: Font<'static>,
    mono: Font<'static>,
    /// The bold instance, and every glyph's advance in it (worked out once: they come from the glyphs' variations).
    bold: Option<Instance>,
    bold_advances: Vec<u16>,
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
        let bold = font.instance(b"wght", 700);
        let bold_advances = match &bold {
            Some(at) => (0..font.glyph_count()).map(|g| font.advance_at(g, at)).collect(),
            None => Vec::new(),
        };
        Type { font, mono, bold, bold_advances }
    }

    fn scale(&self, units: i32, size: i32) -> i32 {
        (units as i64 * size as i64 / self.font.metrics.units_per_em as i64) as i32
    }

    fn advance_units(&self, c: char) -> i32 {
        self.font.advance(self.font.glyph_index(c)) as i32
    }

    /// A character's advance in `face`, in its font's units (both fonts have 1000 to the em).
    fn advance_in(&self, c: char, face: Face) -> i32 {
        if face == Face::Mono {
            return self.mono.advance(self.mono.glyph_index(c)) as i32;
        }
        let glyph = self.font.glyph_index(c);
        match self.bold_advances.get(glyph as usize) {
            Some(&a) if face.bold() => a as i32,
            _ => self.font.advance(glyph) as i32,
        }
    }

    /// The width of `text` at `size`.
    pub fn width(&self, text: &str, size: i32) -> i32 {
        self.scale(text.chars().map(|c| self.advance_units(c)).sum(), size)
    }

    /// The width of `text` in `face` at `size`.
    pub fn width_in(&self, text: &str, face: Face, size: i32) -> i32 {
        self.scale(text.chars().map(|c| self.advance_in(c, face)).sum(), size)
    }

    /// Where styled text (each character with its face) breaks into lines no wider than `max` at `size`, as
    /// `breaks` does: ranges of `chars`, without the line breaks and the spaces lines broke at.
    pub fn breaks_styled(&self, chars: &[(char, Face)], size: i32, max: i32) -> Vec<Range<usize>> {
        let upem = self.font.metrics.units_per_em as i64;
        let max = (max.max(0) as i64 * upem / size.max(1) as i64) as i32;
        let mut lines = Vec::new();
        let (mut start, mut width) = (0, 0);
        let mut space: Option<(usize, i32)> = None;
        for (i, &(c, face)) in chars.iter().enumerate() {
            if c == '\n' {
                lines.push(start..i);
                (start, width, space) = (i + 1, 0, None);
                continue;
            }
            let w = self.advance_in(c, face);
            if c == ' ' {
                width += w;
                space = Some((i, width));
                continue;
            }
            if width + w > max && i > start {
                match space {
                    Some((at, after)) if at > start => {
                        lines.push(start..at);
                        start = at + 1;
                        width -= after;
                    }
                    _ => {
                        lines.push(start..i);
                        start = i;
                        width = 0;
                    }
                }
                space = None;
            }
            width += w;
        }
        lines.push(start..chars.len());
        lines
    }

    /// Where `text` breaks into lines no wider than `max` (at `size`): at spaces, inside a word only if it's
    /// longer than a line, and after every line break ('\n', not part of any line). One pass, however long.
    pub fn breaks(&self, text: &str, size: i32, max: i32) -> Vec<Range<usize>> {
        let upem = self.font.metrics.units_per_em as i64;
        let max = (max.max(0) as i64 * upem / size.max(1) as i64) as i32; // in the font's units
        let mut lines = Vec::new();
        let mut start = 0;
        let mut width = 0; // of text[start..here], in font units
        // The last space on this line: where it is, and the width up to just after it.
        let mut space: Option<(usize, i32)> = None;
        for (i, c) in text.char_indices() {
            if c == '\n' {
                lines.push(start..i);
                start = i + 1;
                width = 0;
                space = None;
                continue;
            }
            let w = self.advance_units(c);
            if c == ' ' {
                width += w;
                space = Some((i, width));
                continue;
            }
            if width + w > max && i > start {
                match space {
                    Some((at, after)) if at > start => {
                        lines.push(start..at);
                        start = at + 1;
                        width -= after;
                    }
                    _ => {
                        lines.push(start..i);
                        start = i;
                        width = 0;
                    }
                }
                space = None;
            }
            width += w;
        }
        lines.push(start..text.len());
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
        let mut units = 0;
        for c in text.chars() {
            pen.x = PAD as i32 * ONE + self.scale(units, style.size);
            let font = if face == Face::Mono { &self.mono } else { &self.font };
            let glyph = font.glyph_index(c);
            let draw = |out: &mut dyn Outline| match &self.bold {
                Some(at) if face.bold() => font.outline_at(glyph, at, out),
                _ => font.outline(glyph, out),
            };
            if face.slanted() {
                draw(&mut Slant { to: &mut pen });
            } else {
                draw(&mut pen);
            }
            units += self.advance_in(c, face);
        }
        let mut coverage = vec![0; w * h];
        raster.coverage(&mut coverage);
        Ink { width: w, height: h, coverage, left: PAD }
    }
}
