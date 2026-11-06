//! A page's boxes laid out (CSS's visual formatting model, the parts pages lean on): blocks one under another, their
//! margins collapsing as CSS's do; words on lines (broken at spaces, between Chinese, Japanese and Korean characters,
//! and inside a word longer than a line), aligned left, centred or right, with inline boxes' backgrounds and the
//! pictures and fields on them sitting on the baseline; flex containers (rows and columns, wrapping, growing and
//! shrinking, justified and aligned); grids (their columns' tracks: lengths, fractions, auto, minmax(), repeat(),
//! named areas); tables (columns as wide as their cells want, colspan); positioned boxes (relative, absolute and
//! fixed, the latter at the page's top), translate(); overflow clipped. Floats are laid out where they come, as
//! blocks. Everything in device pixels, from the page's top left.

use crate::boxes::{BoxId, Content, Control, LBox, Tree};
use crate::style::{Align, BorderStyle, Display, FlexDirection, Image, ListStyle, Position, Size, Style, TextAlign, Track, Transform, VAlign};
use crate::values::{self, Color, Len};
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use text::{Face, Type};

/// What's drawn, in order (the page's top left at 0, 0).
#[derive(Clone, Debug)]
pub enum Paint {
    /// A rectangle filled (its corners rounded: top left, top right, bottom right, bottom left).
    Rect { x: f32, y: f32, w: f32, h: f32, color: Color, radius: [f32; 4] },
    /// A border round a rectangle, each side its own width thick (top, right, bottom, left), its corners rounded (the
    /// inside left as it is).
    Border { x: f32, y: f32, w: f32, h: f32, widths: [f32; 4], color: Color, radius: [f32; 4] },
    /// A linear gradient filling a rectangle: its angle (degrees, 180 down) and colours with their stops (0..1).
    Gradient { x: f32, y: f32, w: f32, h: f32, angle: f32, stops: Vec<(Color, f32)>, radius: [f32; 4] },
    /// Words: where their line box's top left is, how wide they are, in which face and size (and line height), and
    /// colour.
    Text { x: f32, y: f32, w: f32, text: String, face: Face, size: f32, line: f32, color: Color },
    /// A picture (the page's `n`th), filling a rectangle (`cover`: cut to fill it, else squeezed into it).
    Picture { x: f32, y: f32, w: f32, h: f32, n: usize, cover: bool, radius: [f32; 4] },
    /// A box's shadow (x, y, w, h, radius: the box's border box): moved by (dx, dy), grown by `spread`, blurred by
    /// `blur`, and drawn only outside the box.
    Shadow { x: f32, y: f32, w: f32, h: f32, radius: [f32; 4], dx: f32, dy: f32, blur: f32, spread: f32, color: Color },
}

#[derive(Clone, Debug)]
pub struct Item {
    pub paint: Paint,
    /// What it's drawn within (an overflow: hidden box's, or the page's).
    pub clip: Rect,
    /// The element it's for, if any (for a click: a link, a field).
    pub node: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        if r <= x || b <= y { Rect::default() } else { Rect::new(x, y, r - x, b - y) }
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// A page laid out: what's drawn, how tall and wide it is, and its background.
pub struct Laid {
    pub items: Vec<Item>,
    pub width: f32,
    pub height: f32,
    pub background: Color,
    /// Each box's place (its border box, device pixels from the page's top left) and its node, for scripts.
    pub boxes: Vec<(usize, Rect)>,
}

/// What's known of the page's pictures: the `n`th's size in pixels, once it has come.
pub trait Sizes {
    fn size(&self, n: usize) -> Option<(u32, u32)>;
}

/// A piece of a line, laid out: words (from a text box), an atomic inline box, or an inline box's background.
#[derive(Clone, Debug)]
struct Frag {
    kind: FragKind,
    x: f32,
    /// The top of its line box (for words: theirs, in their own line height).
    y: f32,
    w: f32,
    h: f32,
}

#[derive(Clone, Debug)]
enum FragKind {
    Words { text: String, owner: BoxId },
    /// An inline box's part on a line: its background and borders (its left edge on the line it starts on, its right
    /// on the line it ends on).
    Back { owner: BoxId, first: bool, last: bool },
    Marker { text: String, owner: BoxId },
}

pub struct Layout<'a> {
    tree: &'a mut Tree,
    ty: &'a Type,
    sizes: &'a dyn Sizes,
    view_w: f32,
    view_h: f32,
    dpr: f32,
    /// Each block's lines' pieces (by the block's box).
    frags: Vec<Vec<Frag>>,
    /// Out-of-flow boxes waiting for their containing block: the box, and where it would have been (static place).
    pending: Vec<(BoxId, f32, f32)>,
    /// List items whose marker is placed.
    markers: BTreeSet<BoxId>,
    /// The floats placed in each block formatting context being laid out (the innermost last).
    scopes: Vec<Vec<Placed>>,
    /// Boxes' min-content and max-content widths (border box, margins left out), worked out once.
    intrinsic: Vec<Option<(f32, f32)>>,
    /// Widths of words already measured (by face and size), to measure each once.
    widths: alloc::collections::BTreeMap<(String, u8, u32), f32>,
    /// The content height a box's next layout is to have, given it by its container (a stretched flex item's): its
    /// height is then definite, so its insides' percentages of it mean something.
    definite_h: Option<(BoxId, f32)>,
}

/// The face a style's words are set in: its family's web font (Chrome's defaults' metric twins), bold and italic as
/// it says.
fn face_of(s: &Style) -> Face {
    Face::Web(s.family(), s.is_bold(), s.italic)
}

fn face_key(f: Face) -> u8 {
    f.key()
}

/// Chinese, Japanese and Korean (a line can break between their characters).
fn cjk(c: char) -> bool {
    matches!(c as u32, 0x2E80..=0x2FFF | 0x3000..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

/// A box's corners' radii (top left, top right, bottom right, bottom left), `w` × `h`: a percentage of its smaller side,
/// and corners that would overlap made smaller together, by the side that needs it most (CSS's rule).
fn radii(s: &Style, w: f32, h: f32) -> [f32; 4] {
    let base = w.min(h).max(0.0);
    let mut r = s.radius.map(|l| (l.px + l.pct * base / 100.0).max(0.0));
    let f = [(r[0] + r[1], w), (r[3] + r[2], w), (r[0] + r[3], h), (r[1] + r[2], h)]
        .iter()
        .filter(|(sum, _)| *sum > 0.0)
        .map(|(sum, side)| side.max(0.0) / sum)
        .fold(1.0f32, f32::min);
    if f < 1.0 {
        for x in &mut r {
            *x *= f;
        }
    }
    r
}

/// A float placed: its margin box, and which side it's on.
#[derive(Clone, Copy, Debug)]
struct Placed {
    x: f32,
    y: f32,
    r: f32,
    b: f32,
    left: bool,
}

/// Margins collapsing: the largest positive and the most negative, together.
#[derive(Clone, Copy, Default, Debug)]
struct Collapse {
    pos: f32,
    neg: f32,
}

impl Collapse {
    fn with(self, m: f32) -> Collapse {
        Collapse { pos: self.pos.max(m), neg: self.neg.min(m) }
    }
    fn join(self, o: Collapse) -> Collapse {
        Collapse { pos: self.pos.max(o.pos), neg: self.neg.min(o.neg) }
    }
    fn value(self) -> f32 {
        self.pos + self.neg
    }
}

/// A piece of inline content, before it's put on a line.
#[derive(Clone, Debug)]
enum Piece {
    /// A word (and the box it's from), with whether a line may break after it, and the space after it (its width;
    /// 0 for none).
    Word { text: String, owner: BoxId, w: f32, space: f32, breaks: bool },
    /// A line break.
    Break,
    /// An atomic inline box (its margin box's width).
    Atomic { b: BoxId, w: f32 },
    /// An inline box starts (its left margin, border and padding) or ends (its right).
    Open { b: BoxId, w: f32 },
    Close { b: BoxId, w: f32 },
    /// A float among the words: placed where the line it comes on is.
    Float { b: BoxId },
}

impl<'a> Layout<'a> {
    pub fn new(tree: &'a mut Tree, ty: &'a Type, sizes: &'a dyn Sizes, view_w: f32, view_h: f32, dpr: f32) -> Layout<'a> {
        let n = tree.boxes.len();
        Layout {
            tree,
            ty,
            sizes,
            view_w,
            view_h,
            dpr,
            frags: vec![Vec::new(); n],
            pending: Vec::new(),
            markers: BTreeSet::new(),
            scopes: Vec::new(),
            intrinsic: vec![None; n],
            widths: alloc::collections::BTreeMap::new(),
            definite_h: None,
        }
    }

    /// Lays out the whole page in a window `view_w` wide (`view_h` high, for vh and fixed boxes).
    pub fn run(mut self) -> Laid {
        if self.tree.boxes.is_empty() {
            return Laid { items: Vec::new(), width: self.view_w, height: 0.0, background: values::WHITE, boxes: Vec::new() };
        }
        self.metrics(0, self.view_w);
        self.block(0, 0.0, 0.0, self.view_w, Some(self.view_h), None, false);
        // Out-of-flow boxes, as their containing blocks are laid out (each may find more).
        let mut k = 0;
        while k < self.pending.len() && k < 10_000 {
            let (b, sx, sy) = self.pending[k];
            self.absolute(b, sx, sy);
            k += 1;
        }
        self.offsets(0);
        let height = self.extent(0).max(self.view_h);
        let background = self.canvas_background();
        let items = self.paint();
        let boxes = self.tree.boxes.iter().filter_map(|b| b.node.map(|n| (n, Rect::new(b.x, b.y, b.w, b.h)))).collect();
        Laid { items, width: self.view_w, height, background, boxes }
    }

    /// The page's background: the root's, else the body's (as CSS has the canvas take it).
    fn canvas_background(&self) -> Color {
        let root = &self.tree.boxes[0];
        if values::alpha(root.style.background) > 0 {
            return root.style.background;
        }
        for &k in &root.children {
            let b = &self.tree.boxes[k];
            if b.node.is_some() && values::alpha(b.style.background) > 0 && matches!(b.content, Content::Children) {
                return b.style.background;
            }
        }
        values::WHITE
    }

    /// How far down anything in the page reaches.
    fn extent(&self, root: BoxId) -> f32 {
        let mut most = 0.0f32;
        let mut stack = vec![root];
        while let Some(b) = stack.pop() {
            let x = &self.tree.boxes[b];
            if x.style.position == Position::Fixed {
                continue;
            }
            most = most.max(x.y + x.h + x.margin[2]);
            if !x.style.clip {
                stack.extend(x.children.iter().copied());
            }
        }
        most
    }

    // ------------------------------------------------------------------------------------------------ sizes

    /// A box's margins (auto as 0), borders and paddings, against its containing block's width.
    fn metrics(&mut self, b: BoxId, cb_w: f32) {
        let s = self.tree.boxes[b].style.clone();
        let m = |x: Size| x.len().map_or(0.0, |l| l.resolve(cb_w));
        let border = |k: usize| if s.border_style[k] == BorderStyle::None { 0.0 } else { s.border_width[k] };
        let x = &mut self.tree.boxes[b];
        x.margin = [m(s.margin[0]), m(s.margin[1]), m(s.margin[2]), m(s.margin[3])];
        x.border = [border(0), border(1), border(2), border(3)];
        x.padding = [s.padding[0].resolve(cb_w), s.padding[1].resolve(cb_w), s.padding[2].resolve(cb_w), s.padding[3].resolve(cb_w)];
    }

    fn frame_w(&self, b: BoxId) -> f32 {
        let x = &self.tree.boxes[b];
        x.border[1] + x.border[3] + x.padding[1] + x.padding[3]
    }

    fn frame_h(&self, b: BoxId) -> f32 {
        let x = &self.tree.boxes[b];
        x.border[0] + x.border[2] + x.padding[0] + x.padding[2]
    }

    /// A length's pixels for a box's width (a size given: its content box's, box-sizing taken into account).
    fn content_from(&self, b: BoxId, v: Len, base: f32, horizontal: bool) -> f32 {
        let s = &self.tree.boxes[b].style;
        let raw = v.resolve(base);
        let frame = if horizontal { self.frame_w(b) } else { self.frame_h(b) };
        if s.border_box { (raw - frame).max(0.0) } else { raw.max(0.0) }
    }

    /// Its content box's width, clamped by min-width and max-width.
    fn clamp_w(&self, b: BoxId, w: f32, cb_w: f32) -> f32 {
        let s = self.tree.boxes[b].style.clone();
        let mut w = w;
        if let Size::Len(l) = s.max_width {
            w = w.min(self.content_from(b, l, cb_w, true));
        }
        if let Size::Len(l) = s.min_width {
            w = w.max(self.content_from(b, l, cb_w, true));
        }
        w.max(0.0)
    }

    fn clamp_h(&self, b: BoxId, h: f32, cb_h: Option<f32>) -> f32 {
        let s = self.tree.boxes[b].style.clone();
        let mut h = h;
        if let Size::Len(l) = s.max_height
            && (!l.has_pct() || cb_h.is_some())
        {
            h = h.min(self.content_from(b, l, cb_h.unwrap_or(0.0), false));
        }
        if let Size::Len(l) = s.min_height
            && (!l.has_pct() || cb_h.is_some())
        {
            h = h.max(self.content_from(b, l, cb_h.unwrap_or(0.0), false));
        }
        h.max(0.0)
    }

    /// A replaced box's content size (a picture's, a field's): from its width and height, else what it is.
    fn replaced_size(&mut self, b: BoxId, cb_w: f32, cb_h: Option<f32>) -> (f32, f32) {
        let s = self.tree.boxes[b].style.clone();
        let dpr = self.dpr;
        let content = self.tree.boxes[b].content.clone();
        let natural: Option<(f32, f32)> = match &content {
            Content::Image { image: Some(n), .. } => self.sizes.size(*n).map(|(w, h)| (w as f32 * dpr, h as f32 * dpr)),
            Content::Image { .. } => None,
            Content::Control(c) => Some(self.control_size(b, c.clone())),
            _ => None,
        };
        // (A picture keeps its shape; a field doesn't: its height is its line's, however wide it's made.)
        let shaped = !matches!(content, Content::Control(_));
        let ratio = s.aspect_ratio.or(natural.filter(|(w, h)| shaped && *w > 0.0 && *h > 0.0).map(|(w, h)| w / h));
        let w = s.width.len().filter(|l| !l.has_pct() || cb_w > 0.0).map(|l| self.content_from(b, l, cb_w, true));
        let h = s.height.len().filter(|l| !l.has_pct() || cb_h.is_some()).map(|l| self.content_from(b, l, cb_h.unwrap_or(0.0), false));
        let (w, h) = match (w, h, ratio) {
            (Some(w), Some(h), _) => (w, h),
            (Some(w), None, Some(r)) => (w, w / r),
            (None, Some(h), Some(r)) => (h * r, h),
            (Some(w), None, None) => (w, natural.map_or(150.0 * dpr, |n| n.1)),
            (None, Some(h), None) => (natural.map_or(300.0 * dpr, |n| n.0), h),
            (None, None, _) => match natural {
                Some(n) => n,
                // (A picture not here yet, nor sized: nothing, as a browser shows a picture that hasn't come, but
                // its words when it can't be shown at all.)
                None => match &content {
                    Content::Image { image: None, alt } if !alt.is_empty() => {
                        let face = face_of(&s);
                        let w = self.measure(alt, face, s.font_size);
                        (w, s.line_px())
                    }
                    Content::Image { image: Some(_), .. } => (0.0, 0.0),
                    _ => (0.0, 0.0),
                },
            },
        };
        let w = self.clamp_w(b, w, cb_w);
        let h = self.clamp_h(b, h, cb_h);
        (w, h)
    }

    /// A field's size (its content box), as Chrome makes one: a text field 20 characters wide (its size attribute's
    /// number: Chrome's average character, and room for its widest), a line of its line height high; a button its
    /// words' width.
    fn control_size(&mut self, b: BoxId, c: Control) -> (f32, f32) {
        let s = self.tree.boxes[b].style.clone();
        let line = s.line_px();
        match c {
            Control::Text { size, .. } => (s.font_size * (0.6 * size as f32 + 1.4), line),
            // (The widest option, and room for the arrow.)
            Control::Select(_, widest) => (self.measure(&widest, face_of(&s), s.font_size) + s.font_size * 1.8, line),
            Control::Button(words) => (self.measure(&words, face_of(&s), s.font_size), line),
            Control::Check { .. } => (13.0 * self.dpr, 13.0 * self.dpr),
            // (Its characters a monospace font's, and room for its scroll bar.)
            Control::Area { cols, rows, .. } => (s.font_size * 0.6 * cols as f32 + 16.0 * self.dpr, line * rows as f32),
        }
    }

    /// The width of a word in a face and size (device pixels).
    fn measure(&mut self, text: &str, face: Face, size: f32) -> f32 {
        let size256 = (size * 256.0) as i32;
        if size256 <= 0 || text.is_empty() {
            return 0.0;
        }
        let key = (String::from(text), face_key(face), size256 as u32);
        if let Some(w) = self.widths.get(&key) {
            return *w;
        }
        let w = self.ty.width_in(text, face, size256) as f32 / 256.0;
        if self.widths.len() < 200_000 {
            self.widths.insert(key, w);
        }
        w
    }

    /// Where the baseline sits in a line box of a style's line height (from its top), and the line height.
    fn baseline(&self, s: &Style) -> (f32, f32) {
        let line = s.line_px().max(0.0);
        let size = (s.font_size * 256.0) as i32;
        let b = self.ty.baseline_in(face_of(s), text::Style { size, line_height: (line * 256.0) as i32 }) as f32 / 256.0;
        (b, line)
    }

    // ------------------------------------------------------------------------------------------------ intrinsic widths

    /// A box's min-content and max-content widths (its border box's; its margins' pixels added by whoever uses them).
    fn intrinsic(&mut self, b: BoxId) -> (f32, f32) {
        if let Some(v) = self.intrinsic.get(b).copied().flatten() {
            return v;
        }
        let v = self.intrinsic_uncached(b, 0);
        if let Some(slot) = self.intrinsic.get_mut(b) {
            *slot = Some(v);
        }
        v
    }

    fn intrinsic_uncached(&mut self, b: BoxId, depth: u32) -> (f32, f32) {
        if depth > 200 {
            return (0.0, 0.0);
        }
        self.metrics(b, 0.0);
        let s = self.tree.boxes[b].style.clone();
        let frame = self.frame_w(b);
        // A width given in pixels is both.
        if let Size::Len(l) = s.width
            && !l.has_pct()
        {
            let w = self.content_from(b, l, 0.0, true) + frame;
            return (w, w);
        }
        let content = self.tree.boxes[b].content.clone();
        let (mn, mx) = match content {
            Content::Image { .. } | Content::Control(_) => {
                let (w, _) = self.replaced_size(b, 0.0, None);
                (w, w)
            }
            Content::Text(_) | Content::Break => (0.0, 0.0),
            Content::Children => {
                let kids = self.tree.boxes[b].children.clone();
                let inline = kids.iter().any(|&k| self.tree.boxes[k].inline_level() && !self.tree.is_blank(k));
                match s.display {
                    Display::Flex | Display::InlineFlex if matches!(s.flex_direction, FlexDirection::Row | FlexDirection::RowReverse) => {
                        let gap = s.column_gap.px;
                        let (mut mn, mut mx, mut n) = (0.0f32, 0.0f32, 0);
                        for &k in &kids {
                            if self.tree.boxes[k].style.out_of_flow() {
                                continue;
                            }
                            let (a, z) = self.intrinsic(k);
                            let m = self.margin_px(k);
                            if s.flex_wrap { mn = mn.max(a + m) } else { mn += a + m }
                            mx += z + m;
                            n += 1;
                        }
                        let gaps = gap * (n.max(1) - 1) as f32;
                        (mn + if s.flex_wrap { 0.0 } else { gaps }, mx + gaps)
                    }
                    Display::Grid | Display::InlineGrid => {
                        let cols = s.grid_columns.len().max(1);
                        let (mut mn, mut mx) = (0.0f32, 0.0f32);
                        let mut row_max = 0.0f32;
                        let flow: Vec<BoxId> = kids.iter().copied().filter(|&k| !self.tree.boxes[k].style.out_of_flow()).collect();
                        for (i, &k) in flow.iter().enumerate() {
                            let (a, z) = self.intrinsic(k);
                            mn = mn.max(a);
                            row_max += z;
                            if (i + 1) % cols == 0 {
                                mx = mx.max(row_max);
                                row_max = 0.0;
                            }
                        }
                        (mn, mx.max(row_max) + s.column_gap.px * (cols - 1) as f32)
                    }
                    Display::Table | Display::InlineTable => {
                        let (cols_min, cols_max) = self.table_columns(b);
                        let spacing = if s.border_collapse { 0.0 } else { s.border_spacing };
                        let n = cols_min.len() as f32;
                        (cols_min.iter().sum::<f32>() + spacing * (n + 1.0), cols_max.iter().sum::<f32>() + spacing * (n + 1.0))
                    }
                    _ if inline => self.inline_intrinsic(b),
                    _ => {
                        let (mut mn, mut mx) = (0.0f32, 0.0f32);
                        for &k in &kids {
                            if self.tree.boxes[k].style.out_of_flow() {
                                continue;
                            }
                            let (a, z) = self.intrinsic(k);
                            let m = self.margin_px(k);
                            mn = mn.max(a + m);
                            mx = mx.max(z + m);
                        }
                        (mn, mx)
                    }
                }
            }
        };
        // (Its content's widths, clamped by its min-width and max-width, then its borders and padding.)
        let mut mn = mn;
        let mut mx = mx;
        if let Size::Len(l) = s.max_width
            && !l.has_pct()
        {
            let cap = self.content_from(b, l, 0.0, true);
            mx = mx.min(cap);
            mn = mn.min(cap);
        }
        if let Size::Len(l) = s.min_width
            && !l.has_pct()
        {
            let at_least = self.content_from(b, l, 0.0, true);
            mx = mx.max(at_least);
            mn = mn.max(at_least);
        }
        let frame = if content_kind(&self.tree.boxes[b]) == 1 { frame } else { 0.0 };
        (mn + frame, mx.max(mn) + frame)
    }

    /// A box's margins' pixels, side to side (auto as none).
    fn margin_px(&self, b: BoxId) -> f32 {
        let s = &self.tree.boxes[b].style;
        s.margin[1].len().map_or(0.0, |l| l.px) + s.margin[3].len().map_or(0.0, |l| l.px)
    }

    /// An inline formatting context's min-content (its widest unbreakable piece) and max-content (its longest line
    /// unbroken) widths.
    fn inline_intrinsic(&mut self, b: BoxId) -> (f32, f32) {
        let pieces = self.pieces(b, None);
        let nowrap = !self.tree.boxes[b].style.white_space.wraps();
        let (mut mn, mut mx, mut line, mut word) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for p in &pieces {
            match p {
                Piece::Word { w, space, breaks, .. } => {
                    word += w;
                    line += w + space;
                    if *breaks && !nowrap {
                        mn = mn.max(word);
                        word = 0.0;
                    } else {
                        word += space;
                    }
                }
                Piece::Atomic { b: k, w } => {
                    // (As little as it can be: its own narrowest, with what's against it.)
                    let least = self.intrinsic(*k).0 + self.margin_px(*k);
                    word += least;
                    line += w;
                    if !nowrap {
                        mn = mn.max(word);
                        word = 0.0;
                    }
                }
                Piece::Open { w, .. } | Piece::Close { w, .. } => {
                    word += w;
                    line += w;
                }
                Piece::Break => {
                    mx = mx.max(line);
                    mn = mn.max(word);
                    line = 0.0;
                    word = 0.0;
                }
                Piece::Float { b: k } => {
                    let (a, z) = self.intrinsic(*k);
                    let m = self.margin_px(*k);
                    mn = mn.max(a + m);
                    line += z + m;
                }
            }
        }
        mx = mx.max(line);
        mn = mn.max(word);
        if nowrap {
            mn = mx;
        }
        // (A first line's indent.)
        let indent = self.tree.boxes[b].style.text_indent.px.max(0.0);
        (mn + indent, mx + indent)
    }

    // ------------------------------------------------------------------------------------------------ blocks

    /// Lays out a block-level box: its border box's top at `top`, in a containing block from `cb_x` `cb_w` wide
    /// (`cb_h` high, when that's known); `width`: a border-box width it must take (a flex item's, a cell's); `shrink`:
    /// its width, if auto, fits its content (floats, inline-blocks, absolutely positioned boxes).
    #[allow(clippy::too_many_arguments)]
    fn block(&mut self, b: BoxId, cb_x: f32, top: f32, cb_w: f32, cb_h: Option<f32>, width: Option<f32>, shrink: bool) {
        // (A width given by its container comes with its margins and padding worked out by the container.)
        if width.is_none() {
            self.metrics(b, cb_w);
        }
        let s = self.tree.boxes[b].style.clone();
        let frame_w = self.frame_w(b);
        let frame_h = self.frame_h(b);
        let replaced = matches!(self.tree.boxes[b].content, Content::Image { .. } | Content::Control(_));
        let (ml, mr) = (self.tree.boxes[b].margin[3], self.tree.boxes[b].margin[1]);
        let auto_l = s.margin[3] == Size::Auto;
        let auto_r = s.margin[1] == Size::Auto;
        // Its content box's width.
        let mut replaced_h = None;
        let content_w = if let Some(w) = width {
            (w - frame_w).max(0.0)
        } else if replaced {
            let (w, h) = self.replaced_size(b, cb_w, cb_h);
            replaced_h = Some(h);
            w
        } else {
            match s.width {
                Size::Len(l) => self.content_from(b, l, cb_w, true),
                Size::MinContent => (self.intrinsic(b).0 - frame_w).max(0.0),
                Size::MaxContent => (self.intrinsic(b).1 - frame_w).max(0.0),
                _ => {
                    let avail = (cb_w - ml - mr - frame_w).max(0.0);
                    if shrink || s.width == Size::FitContent || matches!(s.display, Display::Table | Display::InlineTable) {
                        // Shrink to fit: its widest unbroken, within the room, but never less than its narrowest.
                        let (mn, mx) = self.intrinsic(b);
                        let (mn, mx) = ((mn - frame_w).max(0.0), (mx - frame_w).max(0.0));
                        mx.min(avail.max(mn))
                    } else {
                        avail
                    }
                }
            }
        };
        let content_w = if width.is_some() { content_w } else { self.clamp_w(b, content_w, cb_w) };
        let border_w = content_w + frame_w;
        // Auto margins centre it (or push it right).
        let mut x = cb_x + ml;
        if width.is_none() && !shrink {
            let free = cb_w - border_w - ml - mr;
            if auto_l && auto_r && free > 0.0 {
                x = cb_x + free / 2.0;
                let half = free / 2.0;
                self.tree.boxes[b].margin[3] = half;
                self.tree.boxes[b].margin[1] = half;
            } else if auto_l && free > 0.0 {
                x = cb_x + free;
                self.tree.boxes[b].margin[3] = free;
            }
        }
        {
            let bx = &mut self.tree.boxes[b];
            bx.x = x;
            bx.y = top;
            bx.w = border_w;
            bx.done = true;
        }
        // Its own height, when it has one it can know now (or its container gave it one).
        let given = match self.definite_h {
            Some((d, h)) if d == b => {
                self.definite_h = None;
                Some(h)
            }
            _ => None,
        };
        let own_h: Option<f32> = match (given, s.height) {
            (Some(h), _) => Some(h),
            (None, Size::Len(l)) if !l.has_pct() || cb_h.is_some() => Some(self.content_from(b, l, cb_h.unwrap_or(0.0), false)),
            _ => s.aspect_ratio.filter(|_| !replaced).map(|r| content_w / r),
        };
        let content_x = x + self.tree.boxes[b].border[3] + self.tree.boxes[b].padding[3];
        let content_y = top + self.tree.boxes[b].border[0] + self.tree.boxes[b].padding[0];
        let inner_h = own_h.map(|h| self.clamp_h(b, h, cb_h));
        // A new block formatting context: the floats inside it stay inside it, and it's as tall as they reach.
        let context = !replaced && (b == 0 || s.new_context() || width.is_some() || shrink);
        if context {
            self.scopes.push(Vec::new());
        }
        let mut content_h = if replaced {
            replaced_h.unwrap_or(0.0)
        } else {
            match s.display {
                Display::Flex | Display::InlineFlex => self.flex(b, content_x, content_y, content_w, inner_h),
                Display::Grid | Display::InlineGrid => self.grid(b, content_x, content_y, content_w, inner_h),
                Display::Table | Display::InlineTable => self.table(b, content_x, content_y, content_w, inner_h),
                _ => self.flow(b, content_x, content_y, content_w, inner_h),
            }
        };
        if context && let Some(floats) = self.scopes.pop() {
            let lowest = floats.iter().map(|f| f.b).fold(content_y, f32::max);
            content_h = content_h.max(lowest - content_y);
        }
        let h = match inner_h {
            Some(h) => h,
            None => self.clamp_h(b, content_h, cb_h),
        };
        // (A button's insides in the middle of its height, as Chrome puts them: an icon in a 40px button.)
        if self.tree.boxes[b].centered && !replaced && !matches!(s.display, Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid) && content_h < h {
            self.shift_inside(b, crate::num::floor((h - content_h) / 2.0));
        }
        self.tree.boxes[b].h = h + frame_h;
    }

    /// Moves what's inside a box (its lines and its boxes), not the box itself.
    fn shift_inside(&mut self, b: BoxId, dy: f32) {
        if dy == 0.0 {
            return;
        }
        if let Some(f) = self.frags.get_mut(b) {
            for fr in f.iter_mut() {
                fr.y += dy;
            }
        }
        for k in self.tree.boxes[b].children.clone() {
            self.shift(k, 0.0, dy);
        }
    }

    /// A block container's insides: lines of inline content, or blocks one under another (margins collapsing).
    /// Returns their height.
    fn flow(&mut self, b: BoxId, x: f32, y: f32, w: f32, h: Option<f32>) -> f32 {
        let kids = self.tree.boxes[b].children.clone();
        // (Words and inline boxes: lines. Spaces alone between blocks are nothing.)
        if kids.iter().any(|&k| self.tree.boxes[k].inline_level() && !self.tree.is_blank(k)) {
            return self.lines(b, x, y, w);
        }
        let collapse_top = self.collapses_top(b);
        let collapse_bottom = self.collapses_bottom(b);
        let mut at = y;
        let mut pending = Collapse::default();
        let mut first = true;
        let mut last_baseline = None;
        for &k in &kids {
            if self.tree.is_blank(k) {
                continue;
            }
            let ks = self.tree.boxes[k].style.clone();
            if ks.out_of_flow() {
                self.pending.push((k, x, at + pending.value()));
                continue;
            }
            if ks.float != crate::style::Float::None {
                // Out of the flow, at its side, where the flow has got to.
                self.place_float(k, x, w, at + pending.value());
                continue;
            }
            self.metrics(k, w);
            let (top, bottom) = (self.margin_top(k, w), self.margin_bottom(k, w));
            if self.is_empty_block(k) {
                pending = pending.join(top).join(bottom);
                self.block(k, x, at + pending.value(), w, h, None, false);
                self.tree.boxes[k].h = 0.0;
                continue;
            }
            let gap = if first && collapse_top { 0.0 } else { pending.join(top).value() };
            at += gap;
            // Clear: below the floats on the side (or sides) it says.
            let clears = match (ks.clear_both, ks.clear) {
                (true, _) => Some(None),
                (false, crate::style::Float::Left) => Some(Some(true)),
                (false, crate::style::Float::Right) => Some(Some(false)),
                _ => None,
            };
            if let Some(side) = clears {
                let below = self.floats_bottom(side);
                if below > at {
                    at = below;
                }
            }
            // A new formatting context (overflow: hidden, flow-root, a table...) goes beside the floats, in the room
            // they leave (or below them, where it doesn't fit).
            if ks.new_context() && !matches!(ks.display, Display::Table | Display::InlineTable) && self.floats_here(at)
            {
                let (bx, bw) = self.beside_floats(k, x, w, &mut at);
                self.block(k, bx, at, bw, h, None, false);
            } else {
                self.block(k, x, at, w, h, None, false);
            }
            // (A box whose top margin collapsed through this one starts at the content's top.)
            at += self.tree.boxes[k].h;
            if let Some(bl) = self.tree.boxes[k].baseline {
                last_baseline = Some(self.tree.boxes[k].y + bl);
            }
            pending = bottom;
            first = false;
        }
        if !collapse_bottom {
            at += pending.value();
        }
        if let Some(bl) = last_baseline {
            self.tree.boxes[b].baseline = Some(bl - self.tree.boxes[b].y);
        }
        (at - y).max(0.0)
    }

    /// The floats of the formatting context being laid out.
    fn floats(&self) -> &[Placed] {
        self.scopes.last().map_or(&[], |v| v.as_slice())
    }

    /// Where the floats on a side (Some(true): left; None: both) end.
    fn floats_bottom(&self, side: Option<bool>) -> f32 {
        self.floats().iter().filter(|f| side.is_none_or(|left| f.left == left)).map(|f| f.b).fold(f32::MIN, f32::max)
    }

    /// Whether any float reaches down past `y`.
    fn floats_here(&self, y: f32) -> bool {
        self.floats().iter().any(|f| f.b > y)
    }

    /// The room between the floats for a band from `top` to `bottom`, within `x0` to `x1`.
    fn band(&self, top: f32, bottom: f32, x0: f32, x1: f32) -> (f32, f32) {
        let (mut l, mut r) = (x0, x1);
        for f in self.floats() {
            if f.y < bottom && f.b > top {
                if f.left {
                    l = l.max(f.r);
                } else {
                    r = r.min(f.x);
                }
            }
        }
        (l, r.max(l))
    }

    /// The next place down from `y` where a float ends (where the room beside them changes), if there is one.
    fn next_float_end(&self, y: f32) -> Option<f32> {
        self.floats().iter().filter(|f| f.b > y + 0.01).map(|f| f.b).fold(None, |m: Option<f32>, b| Some(m.map_or(b, |m| m.min(b))))
    }

    /// Lays out a float and places it: as high as it can go from `top` (not above the floats before it), at the
    /// left or right of the room the other floats leave, below them where it doesn't fit.
    fn place_float(&mut self, k: BoxId, cb_x: f32, cb_w: f32, top: f32) {
        self.metrics(k, cb_w);
        self.block(k, 0.0, 0.0, cb_w, None, None, true);
        let left = self.tree.boxes[k].style.float != crate::style::Float::Right;
        let m = self.tree.boxes[k].margin;
        let (mw, mh) = (self.tree.boxes[k].w + m[1] + m[3], self.tree.boxes[k].h + m[0] + m[2]);
        let mut y = self.floats().iter().map(|f| f.y).fold(top, f32::max);
        // (Clear on a float: below those it clears.)
        let s = self.tree.boxes[k].style.clone();
        if s.clear_both || s.clear != crate::style::Float::None {
            let side = if s.clear_both { None } else { Some(s.clear == crate::style::Float::Left) };
            y = y.max(self.floats_bottom(side));
        }
        for _ in 0..200 {
            let (l, r) = self.band(y, y + mh.max(0.01), cb_x, cb_x + cb_w);
            if r - l >= mw - 0.01 || (l <= cb_x && r >= cb_x + cb_w) {
                break;
            }
            match self.next_float_end(y) {
                Some(next) => y = next,
                None => break,
            }
        }
        let (l, r) = self.band(y, y + mh.max(0.01), cb_x, cb_x + cb_w);
        let fx = if left { l } else { (r - mw).max(cb_x) };
        let (ox, oy) = (self.tree.boxes[k].x, self.tree.boxes[k].y);
        self.shift(k, fx + m[3] - ox, y + m[0] - oy);
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(Placed { x: fx, y, r: fx + mw, b: y + mh, left });
        }
    }

    /// Where a new formatting context goes beside the floats: its left edge and the width it has, `at` moved down
    /// below floats where it doesn't fit beside them.
    fn beside_floats(&mut self, k: BoxId, x: f32, w: f32, at: &mut f32) -> (f32, f32) {
        let (least, _) = self.intrinsic(k);
        let margins = self.margin_px(k);
        let want = match self.tree.boxes[k].style.width {
            Size::Len(l) if !l.has_pct() => self.content_from(k, l, w, true) + self.frame_w(k) + margins,
            _ => least + margins,
        };
        for _ in 0..200 {
            let (l, r) = self.band(*at, *at + 1.0, x, x + w);
            if r - l >= want - 0.01 || !self.floats_here(*at) {
                return (l, r - l);
            }
            match self.next_float_end(*at) {
                Some(next) => *at = next,
                None => break,
            }
        }
        (x, w)
    }

    /// Whether a box's top margin collapses with its first child's (no border or padding between, nor a new
    /// formatting context).
    fn collapses_top(&self, b: BoxId) -> bool {
        let x = &self.tree.boxes[b];
        b != 0 && x.border[0] == 0.0 && x.padding[0] == 0.0 && !self.own_context(b) && matches!(x.content, Content::Children)
            && !matches!(x.style.display, Display::Flex | Display::Grid | Display::Table | Display::InlineBlock | Display::InlineFlex | Display::InlineGrid | Display::TableCell)
    }

    /// Whether a box's insides are a formatting context of their own (its children's margins kept inside it): as its
    /// style makes it, or as a flex or grid container's item is.
    fn own_context(&self, b: BoxId) -> bool {
        let x = &self.tree.boxes[b];
        x.style.new_context()
            || x.parent.is_some_and(|p| matches!(self.tree.boxes[p].style.display, Display::Flex | Display::Grid | Display::InlineFlex | Display::InlineGrid))
    }

    fn collapses_bottom(&self, b: BoxId) -> bool {
        self.collapses_top_bottom(b) && self.tree.boxes[b].style.height == Size::Auto && self.tree.boxes[b].style.min_height.len().is_none_or(|l| l.px == 0.0 && l.pct == 0.0)
    }

    fn collapses_top_bottom(&self, b: BoxId) -> bool {
        let x = &self.tree.boxes[b];
        b != 0 && x.border[2] == 0.0 && x.padding[2] == 0.0 && !self.own_context(b) && matches!(x.content, Content::Children)
            && !matches!(x.style.display, Display::Flex | Display::Grid | Display::Table | Display::InlineBlock | Display::InlineFlex | Display::InlineGrid | Display::TableCell)
    }

    /// A box's top margin with those it collapses with: its first in-flow child's, through it (and through empty
    /// blocks before that child, whose margins collapse through them; white space between blocks is nothing).
    fn margin_top(&mut self, b: BoxId, cb_w: f32) -> Collapse {
        let mut c = Collapse::default().with(self.tree.boxes[b].margin[0]);
        let mut at = b;
        'down: for _ in 0..64 {
            if !self.collapses_top(at) {
                break;
            }
            let kids: Vec<BoxId> = self.tree.boxes[at].children.iter().copied().filter(|&k| !self.tree.is_blank(k) && !self.tree.boxes[k].style.out_of_flow()).collect();
            for k in kids {
                if self.tree.boxes[k].inline_level() || self.tree.boxes[k].style.float != crate::style::Float::None {
                    break 'down;
                }
                self.metrics(k, cb_w);
                c = c.with(self.tree.boxes[k].margin[0]);
                if self.is_empty_block(k) {
                    c = c.with(self.tree.boxes[k].margin[2]);
                    continue;
                }
                at = k;
                continue 'down;
            }
            break;
        }
        c
    }

    fn margin_bottom(&mut self, b: BoxId, cb_w: f32) -> Collapse {
        let mut c = Collapse::default().with(self.tree.boxes[b].margin[2]);
        let mut at = b;
        'down: for _ in 0..64 {
            if !self.collapses_bottom(at) {
                break;
            }
            let kids: Vec<BoxId> = self.tree.boxes[at].children.iter().rev().copied().filter(|&k| !self.tree.is_blank(k) && !self.tree.boxes[k].style.out_of_flow()).collect();
            for k in kids {
                if self.tree.boxes[k].inline_level() || self.tree.boxes[k].style.float != crate::style::Float::None {
                    break 'down;
                }
                self.metrics(k, cb_w);
                c = c.with(self.tree.boxes[k].margin[2]);
                if self.is_empty_block(k) {
                    c = c.with(self.tree.boxes[k].margin[0]);
                    continue;
                }
                at = k;
                continue 'down;
            }
            break;
        }
        c
    }

    /// A block with nothing in it to take room (its margins collapse through it).
    fn is_empty_block(&self, b: BoxId) -> bool {
        let x = &self.tree.boxes[b];
        matches!(x.content, Content::Children)
            && x.style.height == Size::Auto
            && x.style.min_height.len().is_none_or(|l| l.px == 0.0 && l.pct == 0.0)
            && x.border[0] == 0.0
            && x.border[2] == 0.0
            && x.padding[0] == 0.0
            && x.padding[2] == 0.0
            && !matches!(x.style.display, Display::Flex | Display::Grid | Display::Table | Display::ListItem)
            && x.style.aspect_ratio.is_none()
            && x.children.iter().all(|&k| self.tree.is_blank(k) || (self.tree.boxes[k].style.out_of_flow()) || self.is_empty_block(k))
    }

    // ------------------------------------------------------------------------------------------------ lines

    /// The inline content of `b` as pieces: words (each with the space after it), atomic boxes, inline boxes' ends,
    /// line breaks; white space handled as each box's white-space says.
    fn pieces(&mut self, b: BoxId, out_of_flow_at: Option<(f32, f32)>) -> Vec<Piece> {
        let mut out: Vec<Piece> = Vec::new();
        let mut space_before = true; // (at the line's start, a collapsible space goes)
        let mut stack: Vec<(BoxId, bool)> = self.tree.boxes[b].children.iter().rev().map(|&k| (k, false)).collect();
        while let Some((k, closing)) = stack.pop() {
            if closing {
                let w = self.tree.boxes[k].margin[1] + self.tree.boxes[k].border[1] + self.tree.boxes[k].padding[1];
                out.push(Piece::Close { b: k, w });
                continue;
            }
            let s = self.tree.boxes[k].style.clone();
            if s.out_of_flow() {
                if let Some((x, y)) = out_of_flow_at {
                    self.pending.push((k, x, y));
                }
                continue;
            }
            match self.tree.boxes[k].content.clone() {
                Content::Text(t) => {
                    if s.hidden_text {
                        continue;
                    }
                    self.words(k, &t, &s, &mut out, &mut space_before);
                }
                Content::Break => {
                    out.push(Piece::Break);
                    space_before = true;
                }
                Content::Children if self.tree.boxes[k].is_inline_box() => {
                    self.metrics(k, 0.0);
                    let w = self.tree.boxes[k].margin[3] + self.tree.boxes[k].border[3] + self.tree.boxes[k].padding[3];
                    out.push(Piece::Open { b: k, w });
                    stack.push((k, true));
                    for &c in self.tree.boxes[k].children.clone().iter().rev() {
                        stack.push((c, false));
                    }
                }
                _ if s.float != crate::style::Float::None => out.push(Piece::Float { b: k }),
                _ => {
                    // An atomic box: laid out now, to know its width.
                    let (_, mx) = self.intrinsic(k);
                    self.metrics(k, 0.0);
                    let m = self.tree.boxes[k].margin[1] + self.tree.boxes[k].margin[3];
                    out.push(Piece::Atomic { b: k, w: mx + m });
                    space_before = false;
                }
            }
        }
        out
    }

    /// A text box's words, white space handled.
    fn words(&mut self, owner: BoxId, t: &str, s: &Style, out: &mut Vec<Piece>, space_before: &mut bool) {
        let face = face_of(s);
        let size = s.font_size;
        let space_w = self.measure(" ", face, size);
        let keep_spaces = s.white_space.keeps_spaces();
        let keep_breaks = s.white_space.keeps_breaks();
        let wraps = s.white_space.wraps();
        let text: String = match s.transform_text {
            Transform::Uppercase => t.to_uppercase(),
            Transform::Lowercase => t.to_lowercase(),
            Transform::Capitalize => {
                let mut o = String::with_capacity(t.len());
                let mut start = true;
                for c in t.chars() {
                    if start && c.is_alphabetic() {
                        o.extend(c.to_uppercase());
                        start = false;
                    } else {
                        if c.is_whitespace() {
                            start = true;
                        } else if c.is_alphanumeric() {
                            start = false;
                        }
                        o.push(c);
                    }
                }
                o
            }
            Transform::None => String::from(t),
        };
        let mut word = String::new();
        let flush = |me: &mut Self, word: &mut String, out: &mut Vec<Piece>, space: f32, breaks: bool| {
            if !word.is_empty() {
                let w = me.measure(word, face, size);
                out.push(Piece::Word { text: core::mem::take(word), owner, w, space, breaks });
            } else if space > 0.0 {
                // A space with no word before it (after an inline box's start, say): a word of nothing.
                out.push(Piece::Word { text: String::new(), owner, w: 0.0, space, breaks });
            }
        };
        for c in text.chars() {
            match c {
                '\n' if keep_breaks => {
                    flush(self, &mut word, out, 0.0, false);
                    out.push(Piece::Break);
                    *space_before = true;
                }
                ' ' | '\t' | '\n' | '\r' | '\u{c}' => {
                    if keep_spaces {
                        // Each space kept (a tab as four).
                        let n = if c == '\t' { 4.0 } else { 1.0 };
                        flush(self, &mut word, out, space_w * n, wraps);
                        *space_before = true;
                    } else if !*space_before {
                        flush(self, &mut word, out, space_w, wraps);
                        *space_before = true;
                    }
                }
                '\u{a0}' => {
                    word.push(' ');
                    *space_before = false;
                }
                c if cjk(c) && wraps => {
                    flush(self, &mut word, out, 0.0, true);
                    word.push(c);
                    flush(self, &mut word, out, 0.0, true);
                    *space_before = false;
                }
                '\u{ad}' | '\u{200b}' => {
                    // (A soft hyphen or zero-width space: a place to break, showing nothing.)
                    flush(self, &mut word, out, 0.0, wraps);
                }
                c => {
                    word.push(c);
                    *space_before = false;
                }
            }
        }
        flush(self, &mut word, out, 0.0, false);
    }

    /// Lines of `b`'s inline content, from `x`, `y`, `w` wide: their pieces placed, atomic boxes laid out and moved
    /// there, words and inline boxes' backgrounds noted for drawing. Returns their height.
    fn lines(&mut self, b: BoxId, x: f32, y: f32, w: f32) -> f32 {
        let s = self.tree.boxes[b].style.clone();
        let pieces = self.pieces(b, Some((x, y)));
        // Atomic boxes laid out first (their heights for the lines).
        for p in &pieces {
            if let Piece::Atomic { b: k, .. } = p {
                let k = *k;
                let display = self.tree.boxes[k].style.display;
                let shrink = !matches!(display, Display::Block);
                self.block(k, 0.0, 0.0, w, None, None, shrink);
            }
        }
        let pieces: Vec<Piece> = pieces
            .into_iter()
            .map(|p| match p {
                Piece::Atomic { b: k, .. } => {
                    let m = self.tree.boxes[k].margin[1] + self.tree.boxes[k].margin[3];
                    Piece::Atomic { b: k, w: self.tree.boxes[k].w + m }
                }
                p => p,
            })
            .collect();
        let wraps = s.white_space.wraps();
        let indent = s.text_indent.resolve(w);
        let (_, strut) = self.baseline(&s);
        // Each line's room: the block's width, less the floats beside it (where the line will be: the lines before
        // it taken as tall as their tallest piece).
        let mut line_y = y;
        let mut line_h = strut;
        let mut bands: Vec<(f32, f32, f32)> = Vec::new();
        let band_at = |me: &Self, at: f32, h: f32| -> (f32, f32) {
            let (l, r) = me.band(at, at + h, x, x + w);
            (l - x, r - l)
        };
        let mut here = band_at(self, line_y, line_h);
        // Break into lines: each a range of pieces (a word too long for a line split first).
        let mut lines: Vec<Vec<Piece>> = Vec::new();
        let mut line: Vec<Piece> = Vec::new();
        let mut used = 0.0f32;
        let mut room = here.1 - indent;
        let mut has_content = false;
        // Whether the line may break before the next piece (after a space, between Chinese characters, around an
        // atomic box; not between two words of different boxes with nothing between them).
        let mut breakable = false;
        let mut pending_open: Vec<Piece> = Vec::new();
        let finish = |lines: &mut Vec<Vec<Piece>>, line: &mut Vec<Piece>| {
            lines.push(core::mem::take(line));
        };
        // (The next line: below this one, its room worked out where it goes.)
        macro_rules! next_line {
            () => {{
                bands.push((here.0, here.1, line_y));
                line_y += line_h;
                line_h = strut;
                here = band_at(self, line_y, line_h);
                room = here.1;
            }};
        }
        let mut i = 0;
        let pieces_len = pieces.len();
        let mut queue: Vec<Piece> = pieces;
        queue.reverse();
        while let Some(p) = queue.pop() {
            i += 1;
            if i > pieces_len * 4 + 100_000 {
                break;
            }
            match p {
                Piece::Break => {
                    line.append(&mut pending_open);
                    line.push(Piece::Break);
                    finish(&mut lines, &mut line);
                    next_line!();
                    used = 0.0;
                    has_content = false;
                    breakable = false;
                }
                Piece::Float { b: k } => {
                    // At this line's top, where it fits beside what's on the line already (the line's room made
                    // less by it); else below the line.
                    let (_, most) = self.intrinsic(k);
                    let fw = most.min(w) + self.margin_px(k);
                    let on_this_line = !has_content || used + fw <= room + 0.01;
                    let top = if on_this_line { line_y } else { line_y + line_h };
                    self.place_float(k, x, w, top);
                    if on_this_line {
                        here = band_at(self, line_y, line_h);
                        room = here.1 - if lines.is_empty() { indent } else { 0.0 };
                    }
                }
                Piece::Open { .. } => pending_open.push(p),
                Piece::Close { w: cw, .. } => {
                    line.append(&mut pending_open);
                    used += cw;
                    line.push(p);
                }
                Piece::Word { ref text, owner, w: ww, space, breaks } => {
                    let open_w: f32 = pending_open.iter().map(|o| if let Piece::Open { w, .. } = o { *w } else { 0.0 }).sum();
                    let need = open_w + ww;
                    if wraps && has_content && breakable && used + need > room + 0.01 {
                        finish(&mut lines, &mut line);
                        next_line!();
                        used = 0.0;
                        has_content = false;
                        breakable = false;
                        // (A space at a line's start goes.)
                        if text.is_empty() {
                            continue;
                        }
                    }
                    if wraps && !has_content && need > room + 0.01 && here.1 < w - 0.01 && self.next_float_end(line_y).is_some() {
                        // Too wide for the room beside the floats: down to where they end.
                        if let Some(next) = self.next_float_end(line_y) {
                            line_h = next - line_y;
                            finish(&mut lines, &mut line);
                            next_line!();
                            queue.push(p);
                            continue;
                        }
                    }
                    if wraps && !has_content && need > room + 0.01 && text.chars().count() > 1 {
                        // Longer than a line: as much as fits, then the rest on the next line.
                        let (head, tail) = self.split_word(text, owner, room - open_w);
                        if !tail.is_empty() {
                            let hw = self.measure(&head, face_of(&self.tree.boxes[owner].style.clone()), self.tree.boxes[owner].style.font_size);
                            let tw = self.measure(&tail, face_of(&self.tree.boxes[owner].style.clone()), self.tree.boxes[owner].style.font_size);
                            queue.push(Piece::Word { text: tail, owner, w: tw, space, breaks });
                            queue.push(Piece::Word { text: head, owner, w: hw, space: 0.0, breaks: true });
                            continue;
                        }
                    }
                    line.append(&mut pending_open);
                    used += need + space;
                    if !text.is_empty() {
                        has_content = true;
                    }
                    breakable = breaks || space > 0.0;
                    line_h = line_h.max(self.tree.boxes[owner].style.line_px());
                    line.push(p);
                }
                Piece::Atomic { b: k, w: aw } => {
                    let open_w: f32 = pending_open.iter().map(|o| if let Piece::Open { w, .. } = o { *w } else { 0.0 }).sum();
                    if wraps && has_content && used + open_w + aw > room + 0.01 {
                        finish(&mut lines, &mut line);
                        next_line!();
                        used = 0.0;
                    }
                    line.append(&mut pending_open);
                    used += open_w + aw;
                    has_content = true;
                    breakable = true;
                    let kb = &self.tree.boxes[k];
                    line_h = line_h.max(kb.h + kb.margin[0] + kb.margin[2]);
                    line.push(p);
                }
            }
        }
        line.append(&mut pending_open);
        if !line.is_empty() {
            lines.push(line);
            bands.push((here.0, here.1, line_y));
        }
        // Place each line.
        let (strut_base, strut_line) = self.baseline(&s);
        let mut at = y;
        let mut frags: Vec<Frag> = Vec::new();
        let mut last_baseline = None;
        let mut first_line = true;
        // Inline boxes open across lines: where each started on this line.
        let mut open_now: Vec<BoxId> = Vec::new();
        for (n, line) in lines.iter().enumerate() {
            let (band_x, band_w, band_top) = bands.get(n).copied().unwrap_or((0.0, w, at));
            // (Not above where its room beside the floats was worked out: below a float it went down past.)
            if band_top > at {
                at = band_top;
            }
            let visible = line.iter().any(|p| match p {
                Piece::Word { text, .. } => !text.is_empty(),
                Piece::Atomic { .. } | Piece::Break => true,
                Piece::Open { w, .. } | Piece::Close { w, .. } => *w > 0.0,
                Piece::Float { .. } => false,
            });
            if !visible {
                continue;
            }
            // Its width, without the space after its last word.
            let mut content_w: f32 = 0.0;
            let mut last_space = 0.0;
            for p in line {
                match p {
                    Piece::Word { w: ww, space, .. } => {
                        content_w += ww + space;
                        last_space = *space;
                    }
                    Piece::Atomic { w: aw, .. } => {
                        content_w += aw;
                        last_space = 0.0;
                    }
                    Piece::Open { w: ow, .. } | Piece::Close { w: ow, .. } => content_w += ow,
                    Piece::Break | Piece::Float { .. } => {}
                }
            }
            content_w -= last_space;
            let start_x = x + band_x + if first_line { indent } else { 0.0 };
            let room = band_w - if first_line { indent } else { 0.0 };
            let free = (room - content_w).max(0.0);
            let shift = match s.text_align {
                TextAlign::Center => free / 2.0,
                TextAlign::Right => free,
                _ => 0.0,
            };
            // Heights: each piece's ascent and descent about the baseline (the strut's first).
            let (mut asc, mut desc) = (strut_base, strut_line - strut_base);
            let mut metrics: Vec<(f32, f32, f32)> = Vec::new(); // (ascent, descent, baseline shift) per piece
            for p in line {
                let m = match p {
                    Piece::Word { owner, .. } => {
                        let os = self.tree.boxes[*owner].style.clone();
                        let (bo, lh) = self.baseline(&os);
                        let shift = self.valign_shift(*owner, &s);
                        (bo, lh - bo, shift)
                    }
                    Piece::Atomic { b: k, .. } => {
                        let kb = &self.tree.boxes[*k];
                        let mh = kb.h + kb.margin[0] + kb.margin[2];
                        let field = matches!(kb.content, Content::Control(Control::Text { .. } | Control::Button(_) | Control::Select(..)));
                        let a = match (kb.style.vertical_align, kb.baseline) {
                            (VAlign::Middle, _) => mh / 2.0 + s.font_size * 0.25,
                            (VAlign::Top | VAlign::TextTop, _) => strut_base,
                            (VAlign::Bottom | VAlign::TextBottom, _) => mh - (strut_line - strut_base),
                            (_, Some(bl)) if matches!(kb.content, Content::Children) => kb.margin[0] + bl,
                            // A field's: its words' (centred in it, as they're drawn).
                            _ if field => {
                                let ks = kb.style.clone();
                                let (fb, fl) = self.baseline(&ks);
                                let ch = kb.h - kb.border[0] - kb.border[2] - kb.padding[0] - kb.padding[2];
                                let lh = ks.font_size * 1.2;
                                let _ = fl;
                                kb.margin[0] + kb.border[0] + kb.padding[0] + (ch - lh) / 2.0 + fb - (ks.line_px() - lh) / 2.0
                            }
                            _ => mh,
                        };
                        (a, mh - a, 0.0)
                    }
                    _ => (0.0, 0.0, 0.0),
                };
                asc = asc.max(m.0 - m.2);
                desc = desc.max(m.1 + m.2);
                metrics.push(m);
            }
            let baseline_y = at + asc;
            let line_h = asc + desc;
            // Pieces placed, words of one box merged into one run.
            let mut pen = start_x + shift;
            let mut run: Option<(BoxId, f32, String, f32)> = None; // (owner, x, words, shift)
            let flush_run = |me: &mut Self, frags: &mut Vec<Frag>, run: &mut Option<(BoxId, f32, String, f32)>, end: f32| {
                if let Some((owner, rx, words, vshift)) = run.take() {
                    let os = me.tree.boxes[owner].style.clone();
                    let (bo, lh) = me.baseline(&os);
                    let words = String::from(words.trim_end_matches(' '));
                    if !words.is_empty() {
                        frags.push(Frag { kind: FragKind::Words { text: words, owner }, x: rx, y: baseline_y + vshift - bo, w: end - rx, h: lh });
                    }
                }
            };
            for (k, p) in line.iter().enumerate() {
                let (_, _, vshift) = metrics.get(k).copied().unwrap_or((0.0, 0.0, 0.0));
                match p {
                    Piece::Word { text, owner, w: ww, space, .. } => {
                        let same = run.as_ref().is_some_and(|(o, _, _, sh)| *o == *owner && *sh == vshift);
                        if !same {
                            flush_run(self, &mut frags, &mut run, pen);
                            run = Some((*owner, pen, String::new(), vshift));
                        }
                        if let Some((_, _, words, _)) = run.as_mut() {
                            words.push_str(text);
                            if *space > 0.0 {
                                words.push(' ');
                            }
                        }
                        pen += ww + space;
                    }
                    Piece::Atomic { b: kb, w: aw } => {
                        flush_run(self, &mut frags, &mut run, pen);
                        let (a, _, _) = metrics[k];
                        let ml = self.tree.boxes[*kb].margin[3];
                        let mt = self.tree.boxes[*kb].margin[0];
                        let (ox, oy) = (self.tree.boxes[*kb].x, self.tree.boxes[*kb].y);
                        let (nx, ny) = (pen + ml, baseline_y - a + mt);
                        self.shift(*kb, nx - ox, ny - oy);
                        pen += aw;
                    }
                    Piece::Open { b: ob, w: ow } => {
                        flush_run(self, &mut frags, &mut run, pen);
                        open_now.push(*ob);
                        frags.push(Frag { kind: FragKind::Back { owner: *ob, first: true, last: false }, x: pen + self.tree.boxes[*ob].margin[3], y: 0.0, w: -1.0, h: 0.0 });
                        pen += ow;
                    }
                    Piece::Close { b: cb, w: cw } => {
                        flush_run(self, &mut frags, &mut run, pen);
                        // Its background on this line ends here.
                        let end = pen + cw - self.tree.boxes[*cb].margin[1];
                        self.close_back(&mut frags, *cb, end, baseline_y, true);
                        open_now.retain(|&o| o != *cb);
                        pen += cw;
                    }
                    Piece::Break | Piece::Float { .. } => {}
                }
            }
            flush_run(self, &mut frags, &mut run, pen);
            // Inline boxes still open at the line's end: their background to here, and on from the next line's start.
            for &o in &open_now.clone() {
                self.close_back(&mut frags, o, pen, baseline_y, false);
                frags.push(Frag { kind: FragKind::Back { owner: o, first: false, last: false }, x, y: 0.0, w: -1.0, h: 0.0 });
            }
            last_baseline = Some(baseline_y);
            at += line_h;
            first_line = false;
        }
        // (Backgrounds opened for a line that never came.)
        frags.retain(|f| !matches!(f.kind, FragKind::Back { .. }) || f.w >= 0.0);
        // A list item's marker, beside its first line.
        if let Some(item) = self.list_owner(b)
            && self.markers.insert(item)
        {
            self.marker(item, b, x, y, &frags);
        }
        if let Some(bl) = last_baseline {
            self.tree.boxes[b].baseline = Some(bl - self.tree.boxes[b].y);
        }
        if let Some(slot) = self.frags.get_mut(b) {
            slot.extend(frags);
        }
        (at - y).max(0.0)
    }

    /// The baseline's shift for a word's box (sub, super, a length), relative to its block's.
    fn valign_shift(&self, owner: BoxId, block: &Style) -> f32 {
        let mut shift = 0.0;
        let mut at = Some(owner);
        let mut steps = 0;
        while let Some(k) = at {
            let s = &self.tree.boxes[k].style;
            shift += match s.vertical_align {
                VAlign::Sub => block.font_size * 0.25,
                VAlign::Super => -block.font_size * 0.4,
                VAlign::Len(l) => -l,
                _ => 0.0,
            };
            at = self.tree.boxes[k].parent.filter(|&p| self.tree.boxes[p].is_inline_box());
            steps += 1;
            if steps > 32 {
                break;
            }
        }
        shift
    }

    /// Ends an inline box's background on a line at `end`: its top and height from its font, about the baseline.
    fn close_back(&mut self, frags: &mut [Frag], owner: BoxId, end: f32, baseline_y: f32, last: bool) {
        let s = self.tree.boxes[owner].style.clone();
        let (bo, lh) = self.baseline(&s);
        // (The content area: the font's height, a little less than its line's, centred on it as the line's is.)
        let content = s.font_size * 1.2;
        let top = baseline_y - bo + (lh - content) / 2.0 - self.tree.boxes[owner].padding[0] - self.tree.boxes[owner].border[0];
        let h = content + self.frame_h(owner);
        if let Some(f) = frags.iter_mut().rev().find(|f| matches!(f.kind, FragKind::Back { owner: o, .. } if o == owner) && f.w < 0.0) {
            f.w = (end - f.x).max(0.0);
            f.y = top;
            f.h = h;
            if let FragKind::Back { last: l, .. } = &mut f.kind {
                *l = last;
            }
        }
    }

    /// A word too long for a line: as much of it as fits in `room`, and the rest.
    fn split_word(&mut self, text: &str, owner: BoxId, room: f32) -> (String, String) {
        let s = self.tree.boxes[owner].style.clone();
        let face = face_of(&s);
        let chars: Vec<char> = text.chars().collect();
        // (Halving, then a character at a time.)
        let (mut lo, mut hi) = (1usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let head: String = chars[..mid].iter().collect();
            if self.measure(&head, face, s.font_size) <= room {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let n = lo.max(1).min(chars.len());
        (chars[..n].iter().collect(), chars[n..].iter().collect())
    }

    /// The list item whose first line a block's lines are: itself, or the item it's the first thing in (a few
    /// levels down: <li><p>...).
    fn list_owner(&self, b: BoxId) -> Option<BoxId> {
        let mut at = b;
        for _ in 0..4 {
            if self.tree.boxes[at].style.display == Display::ListItem {
                return Some(at);
            }
            let p = self.tree.boxes[at].parent?;
            let first = self.tree.boxes[p].children.iter().copied().find(|&k| !self.tree.boxes[k].style.out_of_flow() && !self.tree.is_blank(k))?;
            if first != at {
                return None;
            }
            at = p;
        }
        None
    }

    /// A list item's marker ("•", "1."), to the left of its first line (or inside it, for list-style-position inside).
    fn marker(&mut self, item: BoxId, block: BoxId, x: f32, y: f32, frags: &[Frag]) {
        let s = self.tree.boxes[item].style.clone();
        if s.list_style == ListStyle::None || s.list_image || s.display != Display::ListItem {
            return;
        }
        let n = self.list_number(item);
        let text = match s.list_style {
            ListStyle::Disc => String::from("\u{2022}"),
            ListStyle::Circle => String::from("\u{25E6}"),
            ListStyle::Square => String::from("\u{25AA}"),
            ListStyle::Decimal => alloc::format!("{n}."),
            ListStyle::LowerAlpha => alloc::format!("{}.", alpha(n, b'a')),
            ListStyle::UpperAlpha => alloc::format!("{}.", alpha(n, b'A')),
            ListStyle::LowerRoman => alloc::format!("{}.", roman(n).to_lowercase()),
            ListStyle::UpperRoman => alloc::format!("{}.", roman(n)),
            ListStyle::None => return,
        };
        let face = face_of(&s);
        let mw = self.measure(&text, face, s.font_size);
        let (_, lh) = self.baseline(&s);
        // The first line's place: its first words', else the block's top; the marker to the left of the item's
        // content (inside it, for list-style-position: inside: there's room made for it by the line's indent).
        let line_top = frags.iter().find(|f| matches!(f.kind, FragKind::Words { .. })).map_or(y, |f| f.y);
        let item_x = self.tree.boxes[item].x + self.tree.boxes[item].border[3] + self.tree.boxes[item].padding[3];
        let mx = if s.list_inside { x } else { item_x.min(x) - mw - s.font_size * 0.5 };
        if let Some(slot) = self.frags.get_mut(block) {
            slot.push(Frag { kind: FragKind::Marker { text, owner: item }, x: mx, y: line_top, w: mw, h: lh });
        }
    }

    /// A list item's number: its place among its list's items.
    fn list_number(&self, item: BoxId) -> i64 {
        let Some(p) = self.tree.boxes[item].parent else { return 1 };
        let mut n: i64 = 1;
        for &k in &self.tree.boxes[p].children {
            if k == item {
                return n;
            }
            if self.tree.boxes[k].style.display == Display::ListItem {
                n += 1;
            }
        }
        n
    }

    // ------------------------------------------------------------------------------------------------ flex

    /// A flex container's insides. Returns their height.
    fn flex(&mut self, b: BoxId, x: f32, y: f32, w: f32, h: Option<f32>) -> f32 {
        let s = self.tree.boxes[b].style.clone();
        let mut items: Vec<BoxId> = Vec::new();
        for &k in &self.tree.boxes[b].children.clone() {
            if self.tree.boxes[k].style.out_of_flow() {
                self.pending.push((k, x, y));
                continue;
            }
            if self.tree.is_blank(k) {
                continue;
            }
            items.push(k);
        }
        items.sort_by_key(|&k| self.tree.boxes[k].style.order);
        let row = matches!(s.flex_direction, FlexDirection::Row | FlexDirection::RowReverse);
        let reverse = matches!(s.flex_direction, FlexDirection::RowReverse | FlexDirection::ColumnReverse);
        if row {
            self.flex_row(b, &items, x, y, w, h, reverse)
        } else {
            self.flex_column(b, &items, x, y, w, h, reverse)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn flex_row(&mut self, b: BoxId, items: &[BoxId], x: f32, y: f32, w: f32, h: Option<f32>, reverse: bool) -> f32 {
        let s = self.tree.boxes[b].style.clone();
        let gap = s.column_gap.resolve(w);
        let row_gap = s.row_gap.resolve(h.unwrap_or(0.0));
        // Each item's base size (content box), its frame and margins, and its least.
        struct It {
            b: BoxId,
            base: f32,
            size: f32,
            least: f32,
            most: f32,
            outer: f32,
            grow: f32,
            shrink: f32,
            auto_l: bool,
            auto_r: bool,
        }
        let mut its: Vec<It> = Vec::new();
        for &k in items {
            self.metrics(k, w);
            let ks = self.tree.boxes[k].style.clone();
            let frame = self.frame_w(k);
            let margins = self.tree.boxes[k].margin[1] + self.tree.boxes[k].margin[3];
            let (mn, mx) = self.intrinsic(k);
            let replaced = matches!(self.tree.boxes[k].content, Content::Image { .. } | Content::Control(_));
            let base = match (ks.basis, ks.width) {
                (Size::Len(l), _) => self.content_from(k, l, w, true),
                (_, Size::Len(l)) => self.content_from(k, l, w, true),
                _ if replaced => self.replaced_size(k, w, h).0,
                _ => (mx - frame).max(0.0),
            };
            // (Its automatic least: its min-content, unless it's clipped or its least is given.)
            let least = match ks.min_width {
                Size::Len(l) => self.content_from(k, l, w, true),
                _ if ks.clip => 0.0,
                _ => (mn - frame).max(0.0).min(match ks.width {
                    Size::Len(l) => self.content_from(k, l, w, true),
                    _ => f32::MAX,
                }),
            };
            let most = match ks.max_width {
                Size::Len(l) => self.content_from(k, l, w, true),
                _ => f32::MAX,
            };
            let size = base.max(least).min(most);
            its.push(It {
                b: k,
                base,
                size,
                least,
                most,
                outer: size + frame + margins,
                grow: ks.grow,
                shrink: ks.shrink,
                auto_l: ks.margin[3] == Size::Auto,
                auto_r: ks.margin[1] == Size::Auto,
            });
        }
        // Lines.
        let mut lines: Vec<core::ops::Range<usize>> = Vec::new();
        let mut start = 0;
        let mut used = 0.0;
        for (i, it) in its.iter().enumerate() {
            let add = if i > start { gap } else { 0.0 } + it.outer;
            if s.flex_wrap && i > start && used + add > w + 0.01 {
                lines.push(start..i);
                start = i;
                used = it.outer;
            } else {
                used += add;
            }
        }
        if start < its.len() || its.is_empty() {
            lines.push(start..its.len());
        }
        let mut at = y;
        let single = lines.len() == 1;
        for (li, r) in lines.iter().enumerate() {
            if li > 0 {
                at += row_gap;
            }
            let n = r.len();
            let gaps = gap * (n.max(1) - 1) as f32;
            // Grow or shrink.
            let outer_sum: f32 = its[r.clone()].iter().map(|i| i.outer).sum();
            let free = w - outer_sum - gaps;
            if free > 0.0 {
                let grow: f32 = its[r.clone()].iter().map(|i| i.grow).sum();
                if grow > 0.0 {
                    for it in &mut its[r.clone()] {
                        let add = free * it.grow / grow;
                        let new = (it.size + add).min(it.most);
                        it.outer += new - it.size;
                        it.size = new;
                    }
                }
            } else if free < 0.0 {
                let weight: f32 = its[r.clone()].iter().map(|i| i.shrink * i.base.max(1.0)).sum();
                if weight > 0.0 {
                    for it in &mut its[r.clone()] {
                        let take = -free * it.shrink * it.base.max(1.0) / weight;
                        let new = (it.size - take).max(it.least);
                        it.outer += new - it.size;
                        it.size = new;
                    }
                }
            }
            let outer_sum: f32 = its[r.clone()].iter().map(|i| i.outer).sum();
            let free = (w - outer_sum - gaps).max(0.0);
            // Auto margins take what's free; else justify-content.
            let autos = its[r.clone()].iter().map(|i| i.auto_l as u32 + i.auto_r as u32).sum::<u32>();
            let (mut pen, between) = if autos > 0 {
                (0.0, 0.0)
            } else {
                match s.justify_content {
                    Align::End => (free, 0.0),
                    Align::Center => (free / 2.0, 0.0),
                    Align::SpaceBetween if n > 1 => (0.0, free / (n - 1) as f32),
                    Align::SpaceAround => (free / n as f32 / 2.0, free / n as f32),
                    Align::SpaceEvenly => (free / (n + 1) as f32, free / (n + 1) as f32),
                    _ => (0.0, 0.0),
                }
            };
            let per_auto = if autos > 0 { free / autos as f32 } else { 0.0 };
            // Lay each out at its size, in its place along the line. (A single line in a container whose height is
            // definite is that high, and what stretches across it is laid out that high, less its margins.)
            let definite_line = if single { h } else { None };
            let mut line_h = 0.0f32;
            let mut placed: Vec<(BoxId, f32)> = Vec::new();
            for it in &its[r.clone()] {
                let k = it.b;
                if it.auto_l {
                    pen += per_auto;
                }
                let (ml, mr) = (self.tree.boxes[k].margin[3], self.tree.boxes[k].margin[1]);
                let border_w = it.size + self.frame_w(k);
                if let Some(lh) = definite_line
                    && self.stretches(b, k)
                {
                    let (mt, mb) = (self.tree.boxes[k].margin[0], self.tree.boxes[k].margin[2]);
                    let inner = self.clamp_h(k, (lh - mt - mb - self.frame_h(k)).max(0.0), h);
                    self.definite_h = Some((k, inner));
                }
                self.block(k, x + pen, at, border_w + ml + mr, h, Some(border_w), false);
                let mh = self.tree.boxes[k].h + self.tree.boxes[k].margin[0] + self.tree.boxes[k].margin[2];
                line_h = line_h.max(mh);
                placed.push((k, mh));
                pen += it.outer;
                if it.auto_r {
                    pen += per_auto;
                }
                pen += gap + between;
            }
            if let Some(hh) = definite_line {
                line_h = hh;
            }
            // Aligned across the line (stretched where the item's height is auto).
            for (k, mh) in placed {
                let ks = self.tree.boxes[k].style.clone();
                let align = if ks.align_self != Align::Normal { ks.align_self } else { s.align_items };
                let mt = self.tree.boxes[k].margin[0];
                let top = at + mt;
                let oy = self.tree.boxes[k].y;
                match align {
                    Align::Center => self.shift(k, 0.0, top + (line_h - mh) / 2.0 - oy),
                    Align::End => self.shift(k, 0.0, top + line_h - mh - oy),
                    Align::Normal | Align::Stretch if self.stretches(b, k) => {
                        self.shift(k, 0.0, top - oy);
                        let mb = self.tree.boxes[k].margin[2];
                        let stretched = (line_h - mt - mb).max(self.tree.boxes[k].h);
                        self.tree.boxes[k].h = self.clamp_h(k, stretched - self.frame_h(k), h) + self.frame_h(k);
                    }
                    _ => self.shift(k, 0.0, top - oy),
                }
            }
            at += line_h;
        }
        if reverse {
            // (Mirrored along the line.)
            for it in &its {
                let k = it.b;
                let (bx, bw) = (self.tree.boxes[k].x, self.tree.boxes[k].w);
                let mirrored = x + w - (bx - x) - bw;
                self.shift(k, mirrored - bx, 0.0);
            }
        }
        // The container's baseline: its first item's.
        if let Some(first) = its.first()
            && let Some(bl) = self.tree.boxes[first.b].baseline
        {
            let by = self.tree.boxes[first.b].y + bl;
            self.tree.boxes[b].baseline = Some(by - self.tree.boxes[b].y);
        }
        (at - y).max(0.0)
    }

    /// Whether flex item `k` of container `b` stretches across its line: aligned so (stretch, or normal), its height
    /// auto, and neither of its margins across the line auto.
    fn stretches(&self, b: BoxId, k: BoxId) -> bool {
        let (s, ks) = (&self.tree.boxes[b].style, &self.tree.boxes[k].style);
        let align = if ks.align_self != Align::Normal { ks.align_self } else { s.align_items };
        matches!(align, Align::Normal | Align::Stretch)
            && ks.height == Size::Auto
            && ks.aspect_ratio.is_none()
            && ks.margin[0] != Size::Auto
            && ks.margin[2] != Size::Auto
    }

    #[allow(clippy::too_many_arguments)]
    fn flex_column(&mut self, b: BoxId, items: &[BoxId], x: f32, y: f32, w: f32, h: Option<f32>, reverse: bool) -> f32 {
        let s = self.tree.boxes[b].style.clone();
        let gap = s.row_gap.resolve(h.unwrap_or(0.0));
        let mut at = y;
        let mut placed: Vec<BoxId> = Vec::new();
        for (i, &k) in items.iter().enumerate() {
            if i > 0 {
                at += gap;
            }
            self.metrics(k, w);
            let ks = self.tree.boxes[k].style.clone();
            let align = if ks.align_self != Align::Normal { ks.align_self } else { s.align_items };
            let stretch = matches!(align, Align::Normal | Align::Stretch) && ks.width == Size::Auto;
            let mt = self.tree.boxes[k].margin[0];
            if stretch {
                self.block(k, x, at + mt, w, h, None, false);
            } else {
                self.block(k, x, at + mt, w, h, None, true);
                let (bw, ml, mr) = (self.tree.boxes[k].w, self.tree.boxes[k].margin[3], self.tree.boxes[k].margin[1]);
                let free = (w - bw - ml - mr).max(0.0);
                let dx = match align {
                    Align::Center => free / 2.0,
                    Align::End => free,
                    _ => 0.0,
                };
                self.shift(k, dx, 0.0);
            }
            // (A basis or growth along the column, where the column's height is known.)
            if let Size::Len(l) = ks.basis
                && (!l.has_pct() || h.is_some())
            {
                let want = self.content_from(k, l, h.unwrap_or(0.0), false) + self.frame_h(k);
                self.tree.boxes[k].h = self.tree.boxes[k].h.max(want);
            }
            at += self.tree.boxes[k].h + mt + self.tree.boxes[k].margin[2];
            placed.push(k);
        }
        let used = at - y;
        if let Some(hh) = h {
            let free = hh - used;
            if free > 0.0 {
                let grow: f32 = placed.iter().map(|&k| self.tree.boxes[k].style.grow).sum();
                if grow > 0.0 {
                    let mut push = 0.0;
                    for &k in &placed {
                        self.shift(k, 0.0, push);
                        let add = free * self.tree.boxes[k].style.grow / grow;
                        self.tree.boxes[k].h += add;
                        push += add;
                    }
                } else {
                    let n = placed.len().max(1);
                    let (start, between) = match s.justify_content {
                        Align::End => (free, 0.0),
                        Align::Center => (free / 2.0, 0.0),
                        Align::SpaceBetween if n > 1 => (0.0, free / (n - 1) as f32),
                        Align::SpaceAround => (free / n as f32 / 2.0, free / n as f32),
                        Align::SpaceEvenly => (free / (n + 1) as f32, free / (n + 1) as f32),
                        _ => (0.0, 0.0),
                    };
                    for (i, &k) in placed.iter().enumerate() {
                        self.shift(k, 0.0, start + between * i as f32);
                    }
                }
            }
        }
        if reverse {
            let total = h.unwrap_or(used);
            for &k in &placed {
                let (by, bh) = (self.tree.boxes[k].y, self.tree.boxes[k].h);
                let mirrored = y + total - (by - y) - bh;
                self.shift(k, 0.0, mirrored - by);
            }
        }
        if let Some(&first) = placed.first()
            && let Some(bl) = self.tree.boxes[first].baseline
        {
            let by = self.tree.boxes[first].y + bl;
            self.tree.boxes[b].baseline = Some(by - self.tree.boxes[b].y);
        }
        used.max(0.0)
    }

    // ------------------------------------------------------------------------------------------------ grid

    /// A grid container's insides. Returns their height.
    fn grid(&mut self, b: BoxId, x: f32, y: f32, w: f32, h: Option<f32>) -> f32 {
        let s = self.tree.boxes[b].style.clone();
        let gap = s.column_gap.resolve(w);
        let row_gap = s.row_gap.resolve(h.unwrap_or(0.0));
        let mut items: Vec<BoxId> = Vec::new();
        for &k in &self.tree.boxes[b].children.clone() {
            if self.tree.boxes[k].style.out_of_flow() {
                self.pending.push((k, x, y));
            } else if !self.tree.is_blank(k) {
                items.push(k);
            }
        }
        items.sort_by_key(|&k| self.tree.boxes[k].style.order);
        // The columns' tracks, repeat(auto-fill, ...) as many as fit.
        let mut tracks: Vec<Track> = Vec::new();
        for t in &s.grid_columns {
            match t {
                Track::AutoRepeat(inner) => {
                    let one: f32 = inner.iter().map(|t| track_least(t, w)).sum::<f32>().max(1.0);
                    let per = inner.len().max(1) as f32;
                    let n = (((w + gap) / (one + gap * per)).max(1.0)) as usize;
                    for _ in 0..n.min(64) {
                        tracks.extend(inner.iter().cloned());
                    }
                }
                t => tracks.push(t.clone()),
            }
        }
        // Named areas: the columns they make, when there are no tracks.
        let area_cols = s.grid_areas.iter().map(|r| r.len()).max().unwrap_or(0);
        if tracks.is_empty() {
            tracks = vec![Track::Fr(1.0); area_cols.max(1)];
        }
        let cols = tracks.len();
        // Where each item goes: (row, column, column span).
        let mut cells: Vec<(BoxId, usize, usize, usize)> = Vec::new();
        let mut taken: Vec<Vec<bool>> = Vec::new();
        let mut cursor = (0usize, 0usize);
        for &k in &items {
            let ks = self.tree.boxes[k].style.clone();
            // A named area.
            if let Some(name) = &ks.grid_area {
                let mut found: Option<(usize, usize, usize)> = None;
                for (r, row) in s.grid_areas.iter().enumerate() {
                    for (c, cell) in row.iter().enumerate() {
                        if cell == name {
                            found = Some(match found {
                                None => (r, c, 1),
                                Some((fr, fc, span)) if fr == r => (fr, fc, span.max(c + 1 - fc)),
                                Some(f) => f,
                            });
                        }
                    }
                }
                if let Some((r, c, span)) = found {
                    mark(&mut taken, r, c, span, cols);
                    cells.push((k, r, c, span.min(cols)));
                    continue;
                }
            }
            let place = ks.grid_column;
            let mut span = place.span.max(1) as usize;
            let start = if place.start > 0 {
                Some((place.start as usize - 1).min(cols - 1))
            } else if place.start < 0 {
                Some(cols.saturating_sub((-place.start) as usize - 1).min(cols - 1))
            } else {
                None
            };
            if place.end != 0 && place.start != 0 {
                // A start line and an end line ("1 / 3"; "1 / -1": to the last line).
                let end_line = if place.end < 0 { cols as i32 + 2 + place.end } else { place.end };
                let start_line = start.unwrap_or(0) as i32 + 1;
                span = (end_line - start_line).max(1) as usize;
            }
            let span = span.min(cols);
            let row_start = if ks.grid_row.start > 0 { Some(ks.grid_row.start as usize - 1) } else { None };
            let (r, c) = match (row_start, start) {
                (Some(r), Some(c)) => (r, c),
                (Some(r), None) => (r, first_free(&taken, r, 0, span, cols).unwrap_or(0)),
                (None, Some(c)) => {
                    let mut r = cursor.0;
                    while is_taken(&taken, r, c, span) {
                        r += 1;
                    }
                    (r, c)
                }
                (None, None) => {
                    let (mut r, mut c) = cursor;
                    loop {
                        if c + span > cols {
                            r += 1;
                            c = 0;
                            continue;
                        }
                        if !is_taken(&taken, r, c, span) {
                            break;
                        }
                        c += 1;
                        if r > 10_000 {
                            break;
                        }
                    }
                    (r, c)
                }
            };
            mark(&mut taken, r, c, span, cols);
            cells.push((k, r, c, span));
            if row_start.is_none() {
                cursor = (r, c + span);
            }
        }
        // Column widths: lengths as they are, auto to their items' widest, fractions share what's left.
        let mut widths = vec![0.0f32; cols];
        let mut fr = vec![0.0f32; cols];
        for (c, t) in tracks.iter().enumerate() {
            match t {
                Track::Len(l) => widths[c] = l.resolve(w),
                Track::Fr(f) => fr[c] = *f,
                Track::MinMax(mm) => {
                    widths[c] = track_least(&mm.0, w);
                    match &mm.1 {
                        Track::Fr(f) => fr[c] = *f,
                        Track::Len(l) => widths[c] = widths[c].max(l.resolve(w)),
                        _ => fr[c] = 1.0,
                    }
                }
                Track::Auto | Track::MinContent | Track::MaxContent | Track::AutoRepeat(_) => {
                    let mut most = 0.0f32;
                    for &(k, _, cc, span) in &cells {
                        if cc == c && span == 1 {
                            let (mn, mx) = self.intrinsic(k);
                            let m = self.margin_px(k);
                            most = most.max(if matches!(t, Track::MinContent) { mn + m } else { mx + m });
                        }
                    }
                    widths[c] = most;
                }
            }
        }
        let gaps = gap * (cols.max(1) - 1) as f32;
        let fixed: f32 = widths.iter().zip(&fr).filter(|(_, f)| **f == 0.0).map(|(w, _)| *w).sum();
        let fr_sum: f32 = fr.iter().sum();
        let left = (w - gaps - fixed).max(0.0);
        if fr_sum > 0.0 {
            for c in 0..cols {
                if fr[c] > 0.0 {
                    widths[c] = widths[c].max(left * fr[c] / fr_sum.max(1.0).max(fr_sum));
                }
            }
        } else {
            // (Auto columns share the room left over, as a stretch.)
            let autos: Vec<usize> = (0..cols).filter(|&c| matches!(tracks[c], Track::Auto)).collect();
            let total: f32 = widths.iter().sum::<f32>() + gaps;
            if !autos.is_empty() && total < w {
                let add = (w - total) / autos.len() as f32;
                for c in autos {
                    widths[c] += add;
                }
            } else if total > w && total > 0.0 {
                let scale = ((w - gaps) / (total - gaps)).max(0.0);
                for v in &mut widths {
                    *v *= scale;
                }
            }
        }
        let col_x: Vec<f32> = {
            let mut xs = Vec::with_capacity(cols);
            let mut at = 0.0;
            for c in 0..cols {
                xs.push(at);
                at += widths[c] + gap;
            }
            xs
        };
        // Lay out each item in its cell's width; rows as tall as their tallest.
        let rows = cells.iter().map(|c| c.1 + 1).max().unwrap_or(0);
        let mut row_h = vec![0.0f32; rows];
        for (r, t) in s.grid_rows.iter().enumerate().take(rows) {
            if let Track::Len(l) = t {
                row_h[r] = l.resolve(h.unwrap_or(0.0));
            }
        }
        for &(k, r, c, span) in &cells {
            let cw = widths[c..(c + span).min(cols)].iter().sum::<f32>() + gap * (span.max(1) - 1) as f32;
            self.metrics(k, cw);
            let ks = self.tree.boxes[k].style.clone();
            let justify = if ks.justify_self != Align::Normal { ks.justify_self } else { s.justify_items };
            let shrink = matches!(justify, Align::Start | Align::End | Align::Center);
            self.block(k, x + col_x[c], y, cw, None, None, shrink);
            if shrink {
                let (bw, ml, mr) = (self.tree.boxes[k].w, self.tree.boxes[k].margin[3], self.tree.boxes[k].margin[1]);
                let free = (cw - bw - ml - mr).max(0.0);
                let dx = match justify {
                    Align::Center => free / 2.0,
                    Align::End => free,
                    _ => 0.0,
                };
                self.shift(k, dx, 0.0);
            }
            let mh = self.tree.boxes[k].h + self.tree.boxes[k].margin[0] + self.tree.boxes[k].margin[2];
            if r < rows {
                row_h[r] = row_h[r].max(mh);
            }
        }
        let mut row_y = Vec::with_capacity(rows);
        let mut at = 0.0;
        for r in 0..rows {
            row_y.push(at);
            at += row_h[r] + if r + 1 < rows { row_gap } else { 0.0 };
        }
        for &(k, r, _, _) in &cells {
            let ks = self.tree.boxes[k].style.clone();
            let align = if ks.align_self != Align::Normal { ks.align_self } else { s.align_items };
            let mt = self.tree.boxes[k].margin[0];
            let mh = self.tree.boxes[k].h + mt + self.tree.boxes[k].margin[2];
            let top = y + row_y[r] + mt;
            let oy = self.tree.boxes[k].y;
            match align {
                Align::Center => self.shift(k, 0.0, top + (row_h[r] - mh) / 2.0 - oy),
                Align::End => self.shift(k, 0.0, top + row_h[r] - mh - oy),
                Align::Normal | Align::Stretch if ks.height == Size::Auto => {
                    self.shift(k, 0.0, top - oy);
                    let stretched = row_h[r] - mt - self.tree.boxes[k].margin[2];
                    self.tree.boxes[k].h = self.tree.boxes[k].h.max(stretched);
                }
                _ => self.shift(k, 0.0, top - oy),
            }
        }
        at.max(0.0)
    }

    // ------------------------------------------------------------------------------------------------ tables

    /// A table's rows (each: its row box, if it has one, and its cells), the header group's first and the footer's
    /// last, and its captions. Cells straight in a table or a row group (CSS's display: table-cell on <div>s) make
    /// a row of their own, as CSS's anonymous table boxes do.
    fn table_rows(&self, t: BoxId) -> (Vec<(Option<BoxId>, Vec<BoxId>)>, Vec<BoxId>) {
        let mut head = Vec::new();
        let mut body = Vec::new();
        let mut foot = Vec::new();
        let mut captions = Vec::new();
        let mut loose: Vec<BoxId> = Vec::new();
        let flush = |loose: &mut Vec<BoxId>, into: &mut Vec<(Option<BoxId>, Vec<BoxId>)>| {
            if !loose.is_empty() {
                into.push((None, core::mem::take(loose)));
            }
        };
        for &k in &self.tree.boxes[t].children {
            if self.tree.is_blank(k) || self.tree.boxes[k].style.out_of_flow() {
                continue;
            }
            match self.tree.boxes[k].style.display {
                Display::TableRow => {
                    flush(&mut loose, &mut body);
                    body.push((Some(k), self.cells(k)));
                }
                Display::TableHeaderGroup | Display::TableRowGroup | Display::TableFooterGroup => {
                    flush(&mut loose, &mut body);
                    let mut rows = Vec::new();
                    let mut cells_here: Vec<BoxId> = Vec::new();
                    for &r in &self.tree.boxes[k].children {
                        if self.tree.is_blank(r) || self.tree.boxes[r].style.out_of_flow() {
                            continue;
                        }
                        if self.tree.boxes[r].style.display == Display::TableRow {
                            flush(&mut cells_here, &mut rows);
                            rows.push((Some(r), self.cells(r)));
                        } else {
                            cells_here.push(r);
                        }
                    }
                    flush(&mut cells_here, &mut rows);
                    match self.tree.boxes[k].style.display {
                        Display::TableHeaderGroup => head.extend(rows),
                        Display::TableFooterGroup => foot.extend(rows),
                        _ => body.extend(rows),
                    }
                }
                Display::TableCaption => captions.push(k),
                Display::TableColumn | Display::TableColumnGroup => {}
                _ => loose.push(k),
            }
        }
        flush(&mut loose, &mut body);
        head.extend(body);
        head.extend(foot);
        (head, captions)
    }

    fn cells(&self, row: BoxId) -> Vec<BoxId> {
        self.tree.boxes[row].children.iter().copied().filter(|&c| !self.tree.is_blank(c) && !self.tree.boxes[c].style.out_of_flow()).collect()
    }

    /// Each column's min-content and max-content widths (from the cells spanning one column).
    fn table_columns(&mut self, t: BoxId) -> (Vec<f32>, Vec<f32>) {
        let (rows, _) = self.table_rows(t);
        let mut mins: Vec<f32> = Vec::new();
        let mut maxs: Vec<f32> = Vec::new();
        let mut spanning: Vec<(usize, usize, f32, f32)> = Vec::new();
        for (_, cells) in rows {
            let mut c = 0;
            for cell in cells {
                let span = self.span_of(cell);
                let (mn, mx) = self.intrinsic(cell);
                if mins.len() < c + span {
                    mins.resize(c + span, 0.0);
                    maxs.resize(c + span, 0.0);
                }
                if span == 1 {
                    mins[c] = mins[c].max(mn);
                    maxs[c] = maxs[c].max(mx);
                } else {
                    spanning.push((c, span, mn, mx));
                }
                c += span;
            }
        }
        // Cells across several columns: what they need more than those columns have, shared among them.
        for (c, span, mn, mx) in spanning {
            let end = (c + span).min(mins.len());
            let n = (end - c).max(1) as f32;
            let have_min: f32 = mins[c..end].iter().sum();
            let have_max: f32 = maxs[c..end].iter().sum();
            if mn > have_min {
                for v in &mut mins[c..end] {
                    *v += (mn - have_min) / n;
                }
            }
            if mx > have_max {
                for v in &mut maxs[c..end] {
                    *v += (mx - have_max) / n;
                }
            }
        }
        for k in 0..mins.len() {
            maxs[k] = maxs[k].max(mins[k]);
        }
        (mins, maxs)
    }

    /// A cell's colspan (Tree::build keeps it in its style's grid_column span).
    fn span_of(&self, cell: BoxId) -> usize {
        (self.tree.boxes[cell].style.grid_column.span as usize).clamp(1, 1000)
    }

    /// A table's insides. Returns their height.
    fn table(&mut self, b: BoxId, x: f32, y: f32, w: f32, _h: Option<f32>) -> f32 {
        let s = self.tree.boxes[b].style.clone();
        let spacing = if s.border_collapse { 0.0 } else { s.border_spacing };
        let (rows, captions) = self.table_rows(b);
        let mut at = y;
        // Its captions above it (those for below, after its rows).
        let (below, above): (Vec<BoxId>, Vec<BoxId>) = captions.iter().partition(|&&c| self.tree.boxes[c].style.caption_bottom);
        for &c in &above {
            self.block(c, x, at, w, None, None, false);
            at += self.tree.boxes[c].h;
        }
        let (mins, maxs) = self.table_columns(b);
        let n = mins.len();
        if n == 0 {
            for &c in &below {
                self.block(c, x, at, w, None, None, false);
                at += self.tree.boxes[c].h;
            }
            return at - y;
        }
        let avail = (w - spacing * (n + 1) as f32).max(0.0);
        let (min_sum, max_sum): (f32, f32) = (mins.iter().sum(), maxs.iter().sum());
        let widths: Vec<f32> = if max_sum <= avail {
            // Room for each to have its most: the rest shared in proportion, when the table's width is given.
            if s.width.len().is_some() && max_sum > 0.0 {
                maxs.iter().map(|m| m + (avail - max_sum) * m / max_sum).collect()
            } else if s.width.len().is_some() {
                vec![avail / n as f32; n]
            } else {
                maxs.clone()
            }
        } else if min_sum >= avail {
            mins.clone()
        } else {
            let spread = (max_sum - min_sum).max(0.001);
            mins.iter().zip(&maxs).map(|(mn, mx)| mn + (avail - min_sum) * (mx - mn) / spread).collect()
        };
        at += spacing;
        for (r, cells) in rows {
            if let Some(r) = r {
                self.metrics(r, w);
            }
            let mut c = 0;
            let mut row_h = 0.0f32;
            let mut placed: Vec<BoxId> = Vec::new();
            let mut cx = x + spacing;
            for cell in cells {
                let span = self.span_of(cell).min(n.saturating_sub(c)).max(1);
                if c >= n {
                    break;
                }
                let cw = widths[c..(c + span).min(n)].iter().sum::<f32>() + spacing * (span - 1) as f32;
                self.metrics(cell, cw);
                self.block(cell, cx, at, cw, None, Some(cw), false);
                row_h = row_h.max(self.tree.boxes[cell].h);
                placed.push(cell);
                cx += cw + spacing;
                c += span;
            }
            if let Some(r) = r
                && let Size::Len(l) = self.tree.boxes[r].style.height
            {
                row_h = row_h.max(l.px);
            }
            // Each cell as tall as its row, its content where its vertical-align puts it.
            for cell in placed {
                let content_h = self.tree.boxes[cell].h;
                let va = self.tree.boxes[cell].style.vertical_align;
                let dy = match va {
                    VAlign::Top | VAlign::Baseline | VAlign::TextTop => 0.0,
                    VAlign::Bottom | VAlign::TextBottom => row_h - content_h,
                    _ => (row_h - content_h) / 2.0,
                };
                if dy > 0.0 {
                    let kids = self.tree.boxes[cell].children.clone();
                    for k in kids {
                        self.shift(k, 0.0, dy);
                    }
                    if let Some(f) = self.frags.get_mut(cell) {
                        for fr in f.iter_mut() {
                            fr.y += dy;
                        }
                    }
                }
                self.tree.boxes[cell].h = row_h;
            }
            if let Some(r) = r {
                let rb = &mut self.tree.boxes[r];
                rb.x = x;
                rb.y = at;
                rb.w = w;
                rb.h = row_h;
                rb.done = true;
            }
            at += row_h + spacing;
        }
        for &c in &below {
            self.block(c, x, at, w, None, None, false);
            at += self.tree.boxes[c].h;
        }
        // Row groups as tall as their rows.
        for &k in &self.tree.boxes[b].children.clone() {
            if matches!(self.tree.boxes[k].style.display, Display::TableHeaderGroup | Display::TableRowGroup | Display::TableFooterGroup) {
                let rows: Vec<BoxId> = self.tree.boxes[k].children.iter().copied().filter(|&r| self.tree.boxes[r].done).collect();
                if let (Some(&first), Some(&last)) = (rows.first(), rows.last()) {
                    let (top, bottom) = (self.tree.boxes[first].y, self.tree.boxes[last].y + self.tree.boxes[last].h);
                    let g = &mut self.tree.boxes[k];
                    g.x = x;
                    g.y = top;
                    g.w = w;
                    g.h = bottom - top;
                    g.done = true;
                }
            }
        }
        (at - y).max(0.0)
    }

    // ------------------------------------------------------------------------------------------------ positioned

    /// An absolutely positioned (or fixed) box, laid out in its containing block: the nearest positioned box's
    /// padding box (the page's first screen for none, and for fixed ones).
    fn absolute(&mut self, b: BoxId, static_x: f32, static_y: f32) {
        let s = self.tree.boxes[b].style.clone();
        let (cx, cy, cw, ch) = if s.position == Position::Fixed {
            (0.0, 0.0, self.view_w, self.view_h)
        } else {
            let mut at = self.tree.boxes[b].parent;
            let mut found = None;
            while let Some(p) = at {
                let ps = &self.tree.boxes[p].style;
                if ps.is_positioned() || p == 0 || ps.translate != (Len::ZERO, Len::ZERO) {
                    found = Some(p);
                    break;
                }
                at = self.tree.boxes[p].parent;
            }
            match found {
                Some(p) if p != 0 => {
                    let pb = &self.tree.boxes[p];
                    (pb.x + pb.border[3], pb.y + pb.border[0], pb.w - pb.border[1] - pb.border[3], pb.h - pb.border[0] - pb.border[2])
                }
                _ => (0.0, 0.0, self.view_w, self.view_h),
            }
        };
        self.metrics(b, cw);
        let len = |v: Size, base: f32| v.len().map(|l| l.resolve(base));
        let (top, right, bottom, left) = (len(s.inset[0], ch), len(s.inset[1], cw), len(s.inset[2], ch), len(s.inset[3], cw));
        let (ml, mr, mt, mb) = (self.tree.boxes[b].margin[3], self.tree.boxes[b].margin[1], self.tree.boxes[b].margin[0], self.tree.boxes[b].margin[2]);
        // Its width: given, or between left and right, or fitting its content.
        let width = if s.width == Size::Auto
            && let (Some(l), Some(r)) = (left, right)
        {
            Some((cw - l - r - ml - mr).max(0.0))
        } else {
            None
        };
        let avail = cw - left.unwrap_or(0.0) - right.unwrap_or(0.0);
        self.block(b, 0.0, 0.0, avail.max(0.0), Some(ch), width, width.is_none());
        let (bw, bh) = (self.tree.boxes[b].w, self.tree.boxes[b].h);
        // Its height between top and bottom, when it has none of its own.
        if s.height == Size::Auto
            && let (Some(t), Some(bo)) = (top, bottom)
        {
            let h = (ch - t - bo - mt - mb).max(bh);
            self.tree.boxes[b].h = h;
        }
        let bh = self.tree.boxes[b].h;
        // Auto margins, with both sides' insets given and a size of its own (not stretched between them), share the
        // room left over: "inset: 0; margin: auto" centres it (a dialog, a popover, a site's own modal).
        let auto = |i: usize| s.margin[i] == Size::Auto;
        let nx = match (left, right) {
            (Some(l), Some(r)) if s.width != Size::Auto && (auto(3) || auto(1)) => {
                let free = cw - l - r - bw - if auto(3) { 0.0 } else { ml } - if auto(1) { 0.0 } else { mr };
                cx + l + match (auto(3), auto(1)) {
                    (true, true) => (free / 2.0).max(0.0),
                    (true, false) => free,
                    _ => ml,
                }
            }
            (Some(l), _) => cx + l + ml,
            (None, Some(r)) => cx + cw - r - mr - bw,
            _ => static_x + ml,
        };
        let ny = match (top, bottom) {
            (Some(t), Some(bo)) if s.height != Size::Auto && (auto(0) || auto(2)) => {
                let free = ch - t - bo - bh - if auto(0) { 0.0 } else { mt } - if auto(2) { 0.0 } else { mb };
                cy + t + match (auto(0), auto(2)) {
                    (true, true) => (free / 2.0).max(0.0),
                    (true, false) => free.max(0.0),
                    _ => mt,
                }
            }
            (Some(t), _) => cy + t + mt,
            (None, Some(bo)) => cy + ch - bo - mb - bh,
            _ => static_y + mt,
        };
        let (ox, oy) = (self.tree.boxes[b].x, self.tree.boxes[b].y);
        self.shift(b, nx - ox, ny - oy);
    }

    /// Relative positions and translate(), each box moved with all that's in it (outermost first).
    fn offsets(&mut self, root: BoxId) {
        let mut stack = vec![root];
        while let Some(b) = stack.pop() {
            let s = self.tree.boxes[b].style.clone();
            let (mut dx, mut dy) = (0.0, 0.0);
            if s.position == Position::Relative {
                let parent_w = self.tree.boxes[b].parent.map_or(self.view_w, |p| self.tree.boxes[p].w);
                let parent_h = self.tree.boxes[b].parent.map_or(self.view_h, |p| self.tree.boxes[p].h);
                if let Some(l) = s.inset[3].len() {
                    dx += l.resolve(parent_w);
                } else if let Some(r) = s.inset[1].len() {
                    dx -= r.resolve(parent_w);
                }
                if let Some(t) = s.inset[0].len() {
                    dy += t.resolve(parent_h);
                } else if let Some(bo) = s.inset[2].len() {
                    dy -= bo.resolve(parent_h);
                }
            }
            if s.translate != (Len::ZERO, Len::ZERO) {
                dx += s.translate.0.resolve(self.tree.boxes[b].w);
                dy += s.translate.1.resolve(self.tree.boxes[b].h);
            }
            if dx != 0.0 || dy != 0.0 {
                self.shift(b, dx, dy);
            }
            stack.extend(self.tree.boxes[b].children.iter().copied());
        }
    }

    /// Moves a box and everything in it (its lines too).
    fn shift(&mut self, b: BoxId, dx: f32, dy: f32) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        let mut stack = vec![b];
        while let Some(k) = stack.pop() {
            let x = &mut self.tree.boxes[k];
            x.x += dx;
            x.y += dy;
            if let Some(f) = self.frags.get_mut(k) {
                for fr in f.iter_mut() {
                    fr.x += dx;
                    fr.y += dy;
                }
            }
            stack.extend(self.tree.boxes[k].children.iter().copied());
        }
    }

    // ------------------------------------------------------------------------------------------------ painting

    /// What's drawn, in order: each box's background and borders, then what's in it; positioned boxes with a
    /// z-index, and absolutely positioned ones, after the rest in their z-index's order (those below 0 first of
    /// all).
    fn paint(&mut self) -> Vec<Item> {
        let mut items: Vec<Item> = Vec::new();
        let page = Rect::new(-1.0e6, -1.0e6, 2.0e6, 2.0e9);
        // (Boxes put off: their z-index, the order they came, the box, its clip and opacity.)
        let mut later: Vec<(i32, usize, BoxId, Rect, f32)> = Vec::new();
        let mut came = 0usize;
        self.paint_tree(0, page, 1.0, &mut items, &mut later, &mut came);
        let mut below: Vec<Item> = Vec::new();
        let mut rounds = 0;
        while !later.is_empty() && rounds < 100_000 {
            rounds += 1;
            // (The lowest z-index next; of those, the first found.)
            let at = (0..later.len()).min_by_key(|&i| (later[i].0, later[i].1)).unwrap_or(0);
            let (z, _, b, clip, opacity) = later.swap_remove(at);
            let mut more = Vec::new();
            if z < 0 {
                self.paint_tree(b, clip, opacity, &mut below, &mut more, &mut came);
            } else {
                self.paint_tree(b, clip, opacity, &mut items, &mut more, &mut came);
            }
            later.extend(more);
        }
        if !below.is_empty() {
            below.extend(items);
            return below;
        }
        items
    }

    /// Paints `root` and what's in it (without recursion), putting off positioned boxes it finds inside.
    fn paint_tree(&mut self, root: BoxId, clip: Rect, opacity: f32, items: &mut Vec<Item>, later: &mut Vec<(i32, usize, BoxId, Rect, f32)>, came: &mut usize) {
        let mut stack: Vec<(BoxId, Rect, f32)> = vec![(root, clip, opacity)];
        while let Some((b, clip, opacity)) = stack.pop() {
            let s = self.tree.boxes[b].style.clone();
            let laid = self.tree.boxes[b].done || b == 0 || matches!(self.tree.boxes[b].content, Content::Text(_) | Content::Break) || self.tree.boxes[b].is_inline_box();
            if !laid {
                continue;
            }
            if b != root && (s.out_of_flow() || (s.is_positioned() && s.z_index.is_some())) {
                *came += 1;
                later.push((s.z_index.unwrap_or(0), *came, b, clip, opacity));
                continue;
            }
            let opacity = opacity * s.opacity;
            if opacity <= 0.01 || s.hidden_text {
                continue;
            }
            if s.visible {
                self.paint_box(b, clip, opacity, items);
            }
            // What's in it: clipped to its padding box, if it clips.
            let x = &self.tree.boxes[b];
            let inner = if s.clip && b != 0 {
                clip.intersect(&Rect::new(x.x + x.border[3], x.y + x.border[0], x.w - x.border[1] - x.border[3], x.h - x.border[0] - x.border[2]))
            } else {
                clip
            };
            // Its lines.
            if let Some(frags) = self.frags.get(b).cloned() {
                for f in &frags {
                    self.paint_frag(f, inner, opacity, items);
                }
            }
            for &k in self.tree.boxes[b].children.iter().rev() {
                if matches!(self.tree.boxes[k].content, Content::Text(_) | Content::Break) {
                    continue;
                }
                stack.push((k, inner, opacity));
            }
        }
    }

    fn fade(c: Color, opacity: f32) -> Color {
        if opacity >= 1.0 {
            return c;
        }
        let a = ((c >> 24) as f32 * opacity) as u32;
        (a.min(255) << 24) | (c & 0x00FF_FFFF)
    }

    /// A box's own background, borders, picture or field.
    fn paint_box(&mut self, b: BoxId, clip: Rect, opacity: f32, items: &mut Vec<Item>) {
        let x = &self.tree.boxes[b];
        if x.is_inline_box() || matches!(x.content, Content::Text(_) | Content::Break) {
            return;
        }
        let s = x.style.clone();
        let node = x.node;
        let (bx, by, bw, bh) = (x.x, x.y, x.w, x.h);
        let radius = radii(&s, bw, bh);
        let border = x.border;
        let padding = x.padding;
        let mut out: Vec<Paint> = Vec::new();
        // (The root's background is the page's: the canvas, drawn by whoever draws the page. A masked one is an
        // icon's shape: not drawn.)
        if let Some(sh) = s.shadow.filter(|sh| values::alpha(sh.color) > 0)
            && b != 0
            && !s.masked
        {
            out.push(Paint::Shadow {
                x: bx,
                y: by,
                w: bw,
                h: bh,
                radius,
                dx: sh.x,
                dy: sh.y,
                blur: sh.blur,
                spread: sh.spread,
                color: Self::fade(sh.color, opacity),
            });
        }
        if b != 0 && values::alpha(s.background) > 0 && !s.masked {
            out.push(Paint::Rect { x: bx, y: by, w: bw, h: bh, color: Self::fade(s.background, opacity), radius });
        }
        if let Some(Image::Linear(angle, stops)) = &s.background_image
            && b != 0
            && !s.masked
        {
            out.push(Paint::Gradient { x: bx, y: by, w: bw, h: bh, angle: *angle, stops: stops.iter().map(|(c, p)| (Self::fade(*c, opacity), *p)).collect(), radius });
        }
        // Borders: one all round when its sides are the same (rounded, if the box is), else each side a rectangle
        // (inset and outset with their top and left, or bottom and right, darker).
        let sides: Vec<usize> = (0..4).filter(|&k| border[k] > 0.0 && s.border_style[k] != BorderStyle::None).collect();
        let shade = |k: usize| -> Color {
            let mut c = s.border_color[k];
            if matches!(s.border_style[k], BorderStyle::Inset | BorderStyle::Groove) && (k == 0 || k == 3) {
                c = values::mix(c, values::BLACK, 0.6);
            }
            if matches!(s.border_style[k], BorderStyle::Outset | BorderStyle::Ridge) && (k == 1 || k == 2) {
                c = values::mix(c, values::BLACK, 0.6);
            }
            Self::fade(c, opacity)
        };
        let patterned = |k: usize| matches!(s.border_style[k], BorderStyle::Dashed | BorderStyle::Dotted | BorderStyle::Double);
        let uniform = sides.len() == 4
            && border.iter().all(|&w| (w - border[0]).abs() < 0.01)
            && (0..4).all(|k| shade(k) == shade(0) && !patterned(k));
        // (Sides of one colour, solid, round a rounded box: one border, its corners rounded, whatever each side's
        // width, as YouTube's search box has it with no right side.)
        let one_colour = !sides.is_empty()
            && radius.iter().any(|&r| r > 0.0)
            && sides.iter().all(|&k| shade(k) == shade(sides[0]) && !patterned(k));
        if uniform || one_colour {
            let widths = core::array::from_fn(|k| if sides.contains(&k) { border[k] } else { 0.0 });
            out.push(Paint::Border { x: bx, y: by, w: bw, h: bh, widths, color: shade(sides[0]), radius });
        } else {
            for &k in &sides {
                let r = match k {
                    0 => Rect::new(bx, by, bw, border[0]),
                    1 => Rect::new(bx + bw - border[1], by, border[1], bh),
                    2 => Rect::new(bx, by + bh - border[2], bw, border[2]),
                    _ => Rect::new(bx, by, border[3], bh),
                };
                for (piece, round) in border_pieces(r, k, s.border_style[k]) {
                    out.push(Paint::Rect { x: piece.x, y: piece.y, w: piece.w, h: piece.h, color: shade(k), radius: [round; 4] });
                }
            }
        }
        for paint in out {
            items.push(Item { paint, clip, node });
        }
        let content = self.tree.boxes[b].content.clone();
        let (cx, cy) = (bx + border[3] + padding[3], by + border[0] + padding[0]);
        let (cw, ch) = ((bw - border[1] - border[3] - padding[1] - padding[3]).max(0.0), (bh - border[0] - border[2] - padding[0] - padding[2]).max(0.0));
        match content {
            Content::Image { image: Some(n), .. } if self.sizes.size(n).is_some() => {
                items.push(Item { paint: Paint::Picture { x: cx, y: cy, w: cw, h: ch, n, cover: s.object_cover, radius }, clip, node });
            }
            Content::Image { alt, .. } if !alt.is_empty() && cw > 0.0 => {
                let (_, lh) = self.baseline(&s);
                items.push(Item {
                    paint: Paint::Text { x: cx, y: cy, w: cw, text: alt, face: face_of(&s), size: s.font_size, line: lh, color: Self::fade(s.color, opacity) },
                    clip: clip.intersect(&Rect::new(cx, cy, cw, ch.max(lh))),
                    node,
                });
            }
            Content::Image { .. } if cw > 0.0 && ch > 0.0 => {
                // (A picture that hasn't come, or can't be shown: its place, for clicks on a link round it.)
                items.push(Item { paint: Paint::Rect { x: cx, y: cy, w: cw, h: ch, color: 0, radius: [0.0; 4] }, clip, node });
            }
            Content::Control(c) => self.paint_control(b, c, cx, cy, cw, ch, clip, opacity, items),
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_control(&mut self, b: BoxId, c: Control, cx: f32, cy: f32, cw: f32, ch: f32, clip: Rect, opacity: f32, items: &mut Vec<Item>) {
        let s = self.tree.boxes[b].style.clone();
        let node = self.tree.boxes[b].node;
        let (_, lh) = self.baseline(&s);
        let inside = clip.intersect(&Rect::new(cx, cy, cw, ch));
        let text_y = cy + (ch - lh) / 2.0;
        let words = |text: String, color: Color, items: &mut Vec<Item>, x: f32| {
            if !text.is_empty() {
                items.push(Item { paint: Paint::Text { x, y: text_y, w: cw, text, face: face_of(&s), size: s.font_size, line: lh, color: Self::fade(color, opacity) }, clip: inside, node });
            }
        };
        match c {
            Control::Text { value, placeholder, password, .. } => {
                if value.is_empty() {
                    words(placeholder, 0xFF75_7575, items, cx);
                } else if password {
                    words("\u{2022}".repeat(value.chars().count().min(200)), s.color, items, cx);
                } else {
                    words(value, s.color, items, cx);
                }
            }
            Control::Area { value, placeholder, .. } => {
                let shown = if value.is_empty() { placeholder } else { value };
                let color = if shown.is_empty() { s.color } else { s.color };
                items.push(Item { paint: Paint::Text { x: cx, y: cy, w: cw, text: shown.lines().next().unwrap_or("").into(), face: face_of(&s), size: s.font_size, line: lh, color: Self::fade(color, opacity) }, clip: inside, node });
            }
            Control::Button(text) => {
                let w = self.measure(&text, face_of(&s), s.font_size);
                let x = cx + ((cw - w) / 2.0).max(0.0);
                words(text, s.color, items, x);
            }
            Control::Select(text, _) => {
                words(text, s.color, items, cx);
                let arrow_x = cx + cw - s.font_size * 0.9;
                items.push(Item { paint: Paint::Text { x: arrow_x, y: text_y, w: s.font_size, text: String::from("\u{25BE}"), face: face_of(&s), size: s.font_size, line: lh, color: Self::fade(s.color, opacity) }, clip: inside, node });
            }
            Control::Check { checked, radio } => {
                let side = cw.min(ch);
                let r = if radio { side / 2.0 } else { 2.0 * self.dpr };
                items.push(Item { paint: Paint::Rect { x: cx, y: cy, w: side, h: side, color: Self::fade(0xFF76_7676, opacity), radius: [r; 4] }, clip, node });
                let bw = 1.0 * self.dpr;
                items.push(Item { paint: Paint::Rect { x: cx + bw, y: cy + bw, w: side - 2.0 * bw, h: side - 2.0 * bw, color: Self::fade(if checked { 0xFF00_75FF } else { values::WHITE }, opacity), radius: [(r - bw).max(0.0); 4] }, clip, node });
                if checked {
                    let dot = side * 0.4;
                    items.push(Item { paint: Paint::Rect { x: cx + (side - dot) / 2.0, y: cy + (side - dot) / 2.0, w: dot, h: dot, color: Self::fade(values::WHITE, opacity), radius: [if radio { dot / 2.0 } else { 0.0 }; 4] }, clip, node });
                }
            }
        }
    }

    /// A piece of a line: words (with their underline or line through), a marker, an inline box's background.
    fn paint_frag(&mut self, f: &Frag, clip: Rect, opacity: f32, items: &mut Vec<Item>) {
        match &f.kind {
            FragKind::Words { text, owner } => {
                let s = self.tree.boxes[*owner].style.clone();
                if !s.visible || s.hidden_text {
                    return;
                }
                let node = self.tree.boxes[*owner].node;
                let color = Self::fade(s.color, opacity);
                items.push(Item { paint: Paint::Text { x: f.x, y: f.y, w: f.w, text: text.clone(), face: face_of(&s), size: s.font_size, line: f.h, color }, clip, node });
                let (bo, _) = self.baseline(&s);
                let thick = (s.font_size / 16.0).max(1.0);
                let dc = Self::fade(s.decoration_color.unwrap_or(s.color), opacity);
                if s.underline {
                    items.push(Item { paint: Paint::Rect { x: f.x, y: f.y + bo + s.font_size * 0.12, w: f.w, h: thick, color: dc, radius: [0.0; 4] }, clip, node });
                }
                if s.line_through {
                    items.push(Item { paint: Paint::Rect { x: f.x, y: f.y + bo - s.font_size * 0.3, w: f.w, h: thick, color: dc, radius: [0.0; 4] }, clip, node });
                }
                if s.overline {
                    items.push(Item { paint: Paint::Rect { x: f.x, y: f.y + bo - s.font_size * 0.95, w: f.w, h: thick, color: dc, radius: [0.0; 4] }, clip, node });
                }
            }
            FragKind::Marker { text, owner } => {
                let s = self.tree.boxes[*owner].style.clone();
                items.push(Item { paint: Paint::Text { x: f.x, y: f.y, w: f.w, text: text.clone(), face: face_of(&s), size: s.font_size, line: f.h, color: Self::fade(s.color, opacity) }, clip, node: self.tree.boxes[*owner].node });
            }
            FragKind::Back { owner, first, last } => {
                let s = self.tree.boxes[*owner].style.clone();
                let node = self.tree.boxes[*owner].node;
                let x = &self.tree.boxes[*owner];
                let border = x.border;
                if values::alpha(s.background) > 0 {
                    items.push(Item { paint: Paint::Rect { x: f.x, y: f.y, w: f.w, h: f.h, color: Self::fade(s.background, opacity), radius: radii(&s, f.w, f.h) }, clip, node });
                } else {
                    // (No background: a box for clicks on the link, drawn as nothing.)
                    items.push(Item { paint: Paint::Rect { x: f.x, y: f.y, w: f.w, h: f.h, color: 0, radius: [0.0; 4] }, clip, node });
                }
                let c = |k: usize| Self::fade(s.border_color[k], opacity);
                if border[0] > 0.0 && s.border_style[0] != BorderStyle::None {
                    items.push(Item { paint: Paint::Rect { x: f.x, y: f.y, w: f.w, h: border[0], color: c(0), radius: [0.0; 4] }, clip, node });
                }
                if border[2] > 0.0 && s.border_style[2] != BorderStyle::None {
                    items.push(Item { paint: Paint::Rect { x: f.x, y: f.y + f.h - border[2], w: f.w, h: border[2], color: c(2), radius: [0.0; 4] }, clip, node });
                }
                if *first && border[3] > 0.0 && s.border_style[3] != BorderStyle::None {
                    items.push(Item { paint: Paint::Rect { x: f.x, y: f.y, w: border[3], h: f.h, color: c(3), radius: [0.0; 4] }, clip, node });
                }
                if *last && border[1] > 0.0 && s.border_style[1] != BorderStyle::None {
                    items.push(Item { paint: Paint::Rect { x: f.x + f.w - border[1], y: f.y, w: border[1], h: f.h, color: c(1), radius: [0.0; 4] }, clip, node });
                }
            }
        }
    }
}

/// 1 for boxes whose intrinsic widths are of their content box (the frame added after), 0 for the rest.
fn content_kind(b: &LBox) -> u8 {
    match b.content {
        Content::Children | Content::Image { .. } | Content::Control(_) => 1,
        _ => 0,
    }
}

/// The least a track takes: a length's, else nothing.
fn track_least(t: &Track, w: f32) -> f32 {
    match t {
        Track::Len(l) => l.resolve(w),
        Track::MinMax(mm) => track_least(&mm.0, w).max(match &mm.1 {
            Track::Len(l) => l.resolve(w).min(track_least(&mm.0, w).max(0.0)),
            _ => 0.0,
        }),
        _ => 0.0,
    }
}

fn is_taken(taken: &[Vec<bool>], r: usize, c: usize, span: usize) -> bool {
    taken.get(r).is_some_and(|row| (c..c + span).any(|k| row.get(k).copied().unwrap_or(false)))
}

fn first_free(taken: &[Vec<bool>], r: usize, from: usize, span: usize, cols: usize) -> Option<usize> {
    (from..cols).find(|&c| c + span <= cols && !is_taken(taken, r, c, span))
}

fn mark(taken: &mut Vec<Vec<bool>>, r: usize, c: usize, span: usize, cols: usize) {
    while taken.len() <= r {
        taken.push(vec![false; cols]);
    }
    for k in c..(c + span).min(cols) {
        taken[r][k] = true;
    }
}

/// A list's letters for a number: 1 a, 26 z, 27 aa.
fn alpha(n: i64, base: u8) -> String {
    let mut n = n.max(1);
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push((base + (n % 26) as u8) as char);
        n /= 26;
    }
    out.iter().rev().collect()
}

fn roman(n: i64) -> String {
    let mut n = n.clamp(1, 3999);
    let mut out = String::new();
    for (v, s) in [(1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"), (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")] {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}



/// A border's side (`r`, side k: 0 top, 1 right, 2 bottom, 3 left) as the pieces its style draws, each with its
/// corners' radius: solid, one piece; dashed, dashes three times as long as the border is thick, spaced evenly
/// with a whole dash at each end; dotted, dots as wide as the border (round once they're 3 pixels or more), the same;
/// double, two lines a third as thick, a third apart (solid, when it's thinner than 3 pixels).
fn border_pieces(r: Rect, k: usize, style: BorderStyle) -> Vec<(Rect, f32)> {
    let across = k == 0 || k == 2;
    let (length, thick) = if across { (r.w, r.h) } else { (r.h, r.w) };
    let piece = |at: f32, len: f32| if across { Rect::new(r.x + at, r.y, len, r.h) } else { Rect::new(r.x, r.y + at, r.w, len) };
    match style {
        BorderStyle::Dashed | BorderStyle::Dotted if thick > 0.0 && length > 0.0 => {
            let dotted = style == BorderStyle::Dotted;
            let unit = if dotted { thick } else { (3.0 * thick).max(3.0) };
            // (Truncated: the whole dashes that fit, with a gap between each.)
            let count = (((length + unit) / (2.0 * unit)) as usize).clamp(1, 4096);
            if count == 1 {
                return vec![(piece(0.0, length), 0.0)];
            }
            let gap = (length - count as f32 * unit) / (count - 1) as f32;
            let round = if dotted && thick >= 3.0 { thick / 2.0 } else { 0.0 };
            (0..count).map(|i| (piece(i as f32 * (unit + gap), unit), round)).collect()
        }
        BorderStyle::Double if thick >= 3.0 => {
            let line = thick / 3.0;
            let (outer, inner) = match k {
                0 => (Rect::new(r.x, r.y, r.w, line), Rect::new(r.x, r.y + 2.0 * line, r.w, line)),
                1 => (Rect::new(r.x + 2.0 * line, r.y, line, r.h), Rect::new(r.x, r.y, line, r.h)),
                2 => (Rect::new(r.x, r.y + 2.0 * line, r.w, line), Rect::new(r.x, r.y, r.w, line)),
                _ => (Rect::new(r.x, r.y, line, r.h), Rect::new(r.x + 2.0 * line, r.y, line, r.h)),
            };
            vec![(outer, 0.0), (inner, 0.0)]
        }
        _ => vec![(r, 0.0)],
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_page_named_by_page_is_laid_out() {
        let html = std::fs::read_to_string(std::env::var("PAGE").unwrap_or_default()).unwrap_or_default();
        if html.is_empty() {
            return;
        }
        let mut page = crate::page::Page::new(web::Url::parse("https://www.example.com/").unwrap(), &html);
        while let Some((i, _, _, _)) = page.next_wanted() {
            page.picture_came(i, 96, 64);
        }
        let ty = text::Type::builtin();
        let laid = page.lay_out(&ty, 600.0, 900.0, 1.0);
        for i in &laid.items {
            std::eprintln!("{:?}", i.paint);
        }
    }
}
