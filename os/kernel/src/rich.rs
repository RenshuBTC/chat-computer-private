//! The model's answers laid out as the main screen shows them (theme.css's ".md" rules): the Markdown read by the
//! markdown crate (md.js's rules), then paragraphs, headings, lists, code, quotes, tables and rules placed in the
//! column, in pixels from the answer's top left. Each run of text in one face is placed on its own; lines and frames
//! are "marks", filled rectangles in the text's colour.

use crate::draw::Rect;
use text::{Face, Style, Type};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use font::raster::ONE;
use markdown::{Block, Item, Span, BOLD, CODE, ITALIC, LINK, STRUCK};

/// Words in one face, at (x, y): the top left of their line box.
pub struct Run {
    pub text: String,
    pub face: Face,
    pub style: Style,
    pub x: i32,
    pub y: i32,
}

pub struct Laid {
    pub runs: Vec<Run>,
    pub marks: Vec<Rect>,
    pub height: i32,
}

/// Lays out `md` in a column `width` pixels wide: `rem` is the text's size in pixels, `body` its style, `hair` one of
/// the theme's pixels on this screen (its thin lines).
pub fn lay_out(ty: &Type, md: &str, rem: f64, body: Style, width: i32, hair: i32) -> Laid {
    let blocks = markdown::parse(md);
    let mut l = Lay { ty, rem, body, hair: hair.max(1), runs: Vec::new(), marks: Vec::new(), y: 0 };
    l.blocks(&blocks, 0, width);
    Laid { runs: l.runs, marks: l.marks, height: l.y }
}

fn face(style: u8) -> Face {
    match style {
        s if s & CODE != 0 => Face::Mono,
        s if s & BOLD != 0 && s & ITALIC != 0 => Face::BoldItalic,
        s if s & BOLD != 0 => Face::Bold,
        s if s & ITALIC != 0 => Face::Italic,
        _ => Face::Regular,
    }
}

struct Lay<'a> {
    ty: &'a Type,
    rem: f64,
    body: Style,
    hair: i32,
    runs: Vec<Run>,
    marks: Vec<Rect>,
    /// Where the next block goes.
    y: i32,
}

/// A block's margins above and below, in rem (theme.css: ems of its own size).
fn margins(b: &Block) -> (f64, f64) {
    match b {
        Block::Heading(1, _) => (1.2 * 1.5, 0.5 * 1.5),
        Block::Heading(2, _) => (1.2 * 1.25, 0.5 * 1.25),
        Block::Heading(..) => (1.2, 0.5),
        Block::Rule => (1.2, 1.2),
        _ => (0.0, 1.0),
    }
}

impl Lay<'_> {
    fn px(&self, rem: f64) -> i32 {
        (rem * self.rem + 0.5) as i32
    }

    /// Lines' height in pixels.
    fn line(style: Style) -> i32 {
        (style.line_height + ONE / 2) / ONE
    }

    /// Blocks one after another, a margin between each two (the larger of the two blocks', as CSS's margins
    /// collapse), none above the first or below the last.
    fn blocks(&mut self, blocks: &[Block], x: i32, width: i32) {
        let mut below: Option<f64> = None;
        for b in blocks {
            let (top, bottom) = margins(b);
            if let Some(gap) = below {
                self.y += self.px(gap.max(top));
            }
            self.block(b, x, width);
            below = Some(bottom);
        }
    }

    fn block(&mut self, b: &Block, x: i32, width: i32) {
        match b {
            Block::Para(spans) => self.text(spans, 0, x, width, self.body),
            Block::Heading(level, spans) => {
                let scale = match level {
                    1 => 1.5,
                    2 => 1.25,
                    _ => 1.0,
                };
                let size = (self.body.size as f64 * scale) as i32;
                self.text(spans, BOLD, x, width, Style { size, line_height: (size as f64 * 1.3) as i32 });
            }
            Block::Code(code) => self.code(code, x, width),
            Block::Rule => {
                self.marks.push(Rect::new(x, self.y, width, self.hair));
                self.y += self.hair;
            }
            Block::Quote(inner) => {
                // A bar on the left (2 of the theme's pixels), then 1 em, then the quote.
                let (top, bar, pad) = (self.y, 2 * self.hair, self.px(1.0));
                self.blocks(inner, x + bar + pad, width - bar - pad);
                self.marks.push(Rect::new(x, top, bar, self.y - top));
            }
            Block::List { ordered, start, items } => self.list(*ordered, *start, items, x, width),
            Block::Table { head, rows } => self.table(head, rows, x, width),
        }
    }

    /// Styled words (`extra` added to each span's style), wrapped to `width`, from `x`.
    fn text(&mut self, spans: &[Span], extra: u8, x: i32, width: i32, style: Style) {
        let mut chars: Vec<(char, Face)> = Vec::new();
        let mut marks: Vec<u8> = Vec::new();
        for s in spans {
            let st = s.style | extra;
            for c in s.text.chars() {
                // (Tabs as spaces: code is the only place they come.)
                let n = if c == '\t' { 4 } else { 1 };
                for _ in 0..n {
                    chars.push((if c == '\t' { ' ' } else { c }, face(st)));
                    marks.push(st & (LINK | STRUCK));
                }
            }
        }
        let line = Self::line(style);
        let baseline = self.ty.baseline(style) / ONE;
        for r in self.ty.breaks_styled(&chars, style.size, width.max(1) * ONE) {
            let mut pen = 0i32; // 1/256 pixel
            let mut k = r.start;
            while k < r.end {
                if k > r.start {
                    // (A link or struck words in the same face are kerned with the words before them, as measured.)
                    pen += self.ty.kern_between(chars[k - 1], chars[k], style.size);
                }
                let (f, m) = (chars[k].1, marks[k]);
                let mut e = k;
                while e < r.end && chars[e].1 == f && marks[e] == m {
                    e += 1;
                }
                let text: String = chars[k..e].iter().map(|c| c.0).collect();
                let w = self.ty.width_in(&text, f, style.size);
                let (from, to) = (x + (pen + ONE / 2) / ONE, x + (pen + w + ONE / 2) / ONE);
                // A link's words underlined (nothing can be opened from here), struck-out words struck through.
                if m & LINK != 0 {
                    self.marks.push(Rect::new(from, self.y + baseline + self.px(0.12), to - from, self.hair));
                }
                if m & STRUCK != 0 {
                    self.marks.push(Rect::new(from, self.y + baseline - self.px(0.28), to - from, self.hair));
                }
                self.runs.push(Run { text, face: f, style, x: from, y: self.y });
                pen += w;
                k = e;
            }
            self.y += line;
        }
    }

    /// A block of code: the code font, wrapped, in a frame (theme.css: 0.75 em above and below it, 1 em each side).
    fn code(&mut self, code: &str, x: i32, width: i32) {
        let (top, pad_y, pad_x, hair) = (self.y, self.px(0.75), self.px(1.0), self.hair);
        self.y += hair + pad_y;
        let span = [Span { text: String::from(code), style: CODE }];
        self.text(&span, 0, x + hair + pad_x, width - 2 * (hair + pad_x), self.body);
        self.y += pad_y + hair;
        self.frame(Rect::new(x, top, width, self.y - top));
    }

    /// A rectangle's outline, one of the theme's pixels thick.
    fn frame(&mut self, r: Rect) {
        let h = self.hair;
        self.marks.extend([
            Rect::new(r.x, r.y, r.w, h),
            Rect::new(r.x, r.y + r.h - h, r.w, h),
            Rect::new(r.x, r.y, h, r.h),
            Rect::new(r.x + r.w - h, r.y, h, r.h),
        ]);
    }

    /// A list: 1.5 em in, its bullets or numbers (or tasks' boxes) in front of each item, 0.25 em between items.
    fn list(&mut self, ordered: bool, start: u32, items: &[Item], x: i32, width: i32) {
        let indent = self.px(1.5);
        let gap = self.px(0.4);
        let line = Self::line(self.body);
        for (n, item) in items.iter().enumerate() {
            if n > 0 {
                self.y += self.px(0.25);
            }
            match item.task {
                Some(done) => {
                    let side = self.px(0.75);
                    let r = Rect::new(x + indent - gap - side, self.y + (line - side) / 2, side, side);
                    self.frame(r);
                    if done {
                        let inset = 2 * self.hair + 1;
                        self.marks.push(Rect::new(r.x + inset, r.y + inset, r.w - 2 * inset, r.h - 2 * inset));
                    }
                }
                None => {
                    let marker = if ordered { alloc::format!("{}.", start as u64 + n as u64) } else { String::from("\u{2022}") };
                    let w = (self.ty.width_in(&marker, Face::Regular, self.body.size) + ONE - 1) / ONE;
                    self.runs.push(Run { text: marker, face: Face::Regular, style: self.body, x: x + indent - gap - w, y: self.y });
                }
            }
            // (A done task's words struck through, as the main screen shows them.)
            let extra = if item.task == Some(true) { STRUCK } else { 0 };
            self.text(&item.spans, extra, x + indent, width - indent, self.body);
        }
    }

    /// A table: each column as wide as its widest cell (narrower in proportion when they don't fit), cells 0.4 em
    /// above and below and 1 em to the right of their words, the header bold, a line under every row.
    fn table(&mut self, head: &[Vec<Span>], rows: &[Vec<Vec<Span>>], x: i32, width: i32) {
        let cols = rows.iter().map(|r| r.len()).chain([head.len()]).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let (pad_y, pad_r) = (self.px(0.4), self.px(1.0));
        let cell_width = |cell: &[Span], extra: u8| -> i32 {
            let w: i32 = cell.iter().map(|s| self.ty.width_in(&s.text, face(s.style | extra), self.body.size)).sum();
            (w + ONE - 1) / ONE + pad_r
        };
        let mut natural = vec![0i32; cols];
        for (c, cell) in head.iter().enumerate() {
            natural[c] = natural[c].max(cell_width(cell, BOLD));
        }
        for row in rows {
            for (c, cell) in row.iter().enumerate() {
                natural[c] = natural[c].max(cell_width(cell, 0));
            }
        }
        let total: i32 = natural.iter().sum();
        let widths: Vec<i32> = if total <= width {
            natural
        } else {
            // Every column keeps up to 4 em; what's left is shared in proportion to the rest of each one's width.
            let least: Vec<i32> = natural.iter().map(|&w| w.min(self.px(4.0))).collect();
            let (least_total, rest) = (least.iter().sum::<i32>(), total - least.iter().sum::<i32>());
            let room = (width - least_total).max(0);
            natural
                .iter()
                .zip(&least)
                .map(|(&w, &l)| l + if rest > 0 { ((w - l) as i64 * room as i64 / rest as i64) as i32 } else { 0 })
                .collect()
        };
        let table_w: i32 = widths.iter().sum();
        let line = Self::line(self.body);
        let row_at = |lay: &mut Self, cells: &[Vec<Span>], extra: u8| {
            let top = lay.y;
            let mut bottom = top + pad_y + line + pad_y;
            let mut cx = x;
            for (c, &w) in widths.iter().enumerate() {
                lay.y = top + pad_y;
                if let Some(cell) = cells.get(c) {
                    lay.text(cell, extra, cx, (w - pad_r).max(1), lay.body);
                }
                bottom = bottom.max(lay.y + pad_y);
                cx += w;
            }
            lay.y = bottom;
            lay.marks.push(Rect::new(x, lay.y, table_w, lay.hair));
            lay.y += lay.hair;
        };
        row_at(self, head, BOLD);
        for row in rows {
            row_at(self, row, 0);
        }
    }
}
