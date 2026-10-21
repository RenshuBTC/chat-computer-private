//! Text in Atkinson Hyperlegible Next, set with Chat Computer's own font engine: measuring, wrapping and
//! drawing lines. Sizes are in 1/256 of a pixel.

use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;
use font::raster::{Pen, Raster, ONE};
use font::Font;

static ATKINSON: &[u8] = include_bytes!("../../../chatcomputer/ui/assets/fonts/AtkinsonHyperlegibleNext.ttf");

/// Some letters reach a little past their own width (an italic f, an accent); the drawn line keeps
/// this much room on each side, in pixels.
const PAD: usize = 2;

#[derive(Clone, Copy)]
pub struct Style {
    pub size: i32,
    pub line_height: i32,
}

pub struct Type {
    font: Font<'static>,
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
        Type { font: Font::parse(ATKINSON).expect("the built-in font is damaged") }
    }

    fn scale(&self, units: i32, size: i32) -> i32 {
        (units as i64 * size as i64 / self.font.metrics.units_per_em as i64) as i32
    }

    fn advance_units(&self, c: char) -> i32 {
        self.font.advance(self.font.glyph_index(c)) as i32
    }

    /// The width of `text` at `size`.
    pub fn width(&self, text: &str, size: i32) -> i32 {
        self.scale(text.chars().map(|c| self.advance_units(c)).sum(), size)
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
        let m = self.font.metrics;
        let width = (self.width(text, style.size) + ONE - 1) / ONE;
        let w = width as usize + 2 * PAD;
        let h = ((top + style.line_height + ONE - 1) / ONE) as usize;
        // The font's own height (ascender to descender), centred in the line box, as CSS does.
        let content = self.scale(m.ascender as i32 - m.descender as i32, style.size);
        let baseline = top + (style.line_height - content) / 2 + self.scale(m.ascender as i32, style.size);
        let mut raster = Raster::new(w, h);
        let mut pen = Pen::new(&mut raster, style.size, m.units_per_em);
        pen.y = baseline;
        let mut units = 0;
        for c in text.chars() {
            let glyph = self.font.glyph_index(c);
            pen.x = PAD as i32 * ONE + self.scale(units, style.size);
            self.font.outline(glyph, &mut pen);
            units += self.font.advance(glyph) as i32;
        }
        let mut coverage = vec![0; w * h];
        raster.coverage(&mut coverage);
        Ink { width: w, height: h, coverage, left: PAD }
    }
}
