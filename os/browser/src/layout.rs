//! A page's boxes laid out (CSS's visual formatting model, the parts pages lean on): blocks one under another, their
//! margins collapsing as CSS's do; words on lines (broken at spaces, between Chinese, Japanese and Korean characters,
//! and inside a word longer than a line), aligned left, centred or right, with inline boxes' backgrounds and the
//! pictures and fields on them sitting on the baseline; flex containers (rows and columns, wrapping, growing and
//! shrinking, justified and aligned); grids (their columns' tracks: lengths, fractions, auto, minmax(), repeat(),
//! named areas); tables (columns as wide as their cells want, colspan); positioned boxes (relative, absolute and
//! fixed, the latter at the page's top), translate(); overflow clipped. Floats are laid out where they come, as
//! blocks. Everything in device pixels, from the page's top left.

use crate::boxes::{BoxId, Content, Control, LBox, Tree};
use crate::effects::{self, BgSize, BoxArea, ClipPath, Filter, Gradient, Image, Layers, ObjectFit, Pos, Repeat, Shape, ShapeRadius};
use crate::num;
use crate::style::{Align, BorderStyle, Display, FlexDirection, GridPlace, ListStyle, Position, Size, Style, TextAlign, Track, Transform, VAlign};
use crate::values::{self, Color, Len};
use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::sync::Arc;
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
    /// A border whose sides are of different colours (top, right, bottom, left), each side's colour up to the
    /// diagonals where they meet at the corners (as CSS's arrows made of borders have them).
    SidedBorder { x: f32, y: f32, w: f32, h: f32, widths: [f32; 4], colors: [Color; 4], radius: [f32; 4] },
    /// Words: where their line box's top left is, how wide they are, in which face and size (and line height), and
    /// colour.
    Text { x: f32, y: f32, w: f32, text: String, face: Face, size: f32, line: f32, color: Color },
    /// A picture (the page's `n`th) drawn at `at`'s place and size, cut to a box (its corners rounded).
    Picture { x: f32, y: f32, w: f32, h: f32, n: usize, at: Rect, radius: [f32; 4], pixelated: bool },
    /// A background's (or mask's) layer: its picture or gradient in tiles the size of `tile`, one at `tile`'s place
    /// and more every `step` across and down where it repeats, within a rectangle (its corners rounded).
    Tiles { x: f32, y: f32, w: f32, h: f32, radius: [f32; 4], tile: Rect, step: (f32, f32), repeat: (bool, bool), image: Tile, pixelated: bool },
    /// What's drawn after it, up to its End, is drawn as one, then put on what's under it through its effects.
    Begin(Box<Group>),
    End,
    /// A box's shadow (x, y, w, h, radius: the box's border box): moved by (dx, dy), grown by `spread`, blurred by
    /// `blur`, and drawn only outside the box. An inset one's box is the padding box: it's cast inside it, by the
    /// edges of that box moved and shrunk.
    Shadow { x: f32, y: f32, w: f32, h: f32, radius: [f32; 4], dx: f32, dy: f32, blur: f32, spread: f32, color: Color, inset: bool },
}

#[derive(Clone, Debug)]
pub struct Item {
    pub paint: Paint,
    /// What it's drawn within (an overflow: hidden box's, or the page's).
    pub clip: Rect,
    /// The element it's for, if any (for a click: a link, a field).
    pub node: Option<usize>,
}

impl Item {
    fn end() -> Item {
        Item { paint: Paint::End, clip: Rect::default(), node: None }
    }
}

/// What a layer of tiles shows: a picture (the page's `n`th), or a gradient (drawn the tile's size).
#[derive(Clone, Debug)]
pub enum Tile {
    Picture(usize),
    Gradient(Arc<Gradient>),
}

/// A group's effects: what's in it is drawn as one, then clipped, masked, filtered, made see-through, turned and
/// blended onto what's under it.
#[derive(Clone, Debug)]
pub struct Group {
    pub opacity: f32,
    /// Its transform: from the coordinates of what's in it to those of what it's drawn on.
    pub transform: Option<[f32; 6]>,
    /// A rounded rectangle what's in it is cut to (an overflow clip with rounded corners).
    pub clip: Option<(Rect, [f32; 4])>,
    /// The polygons it's cut to (clip-path), and whether they're filled even-odd.
    pub clip_path: Option<(Vec<Vec<(f32, f32)>>, bool)>,
    /// Its mask's layers, the bottom one first.
    pub mask: Option<Vec<MaskLayer>>,
    pub filters: Vec<Filter>,
    /// backdrop-filter: its filters, and the rectangle (rounded) what's behind it is filtered within.
    pub backdrop: Option<(Vec<Filter>, Rect, [f32; 4])>,
    /// mix-blend-mode: 255 for normal.
    pub blend: u8,
    /// Turned to show its back with backface-visibility: hidden (not drawn).
    pub hidden: bool,
    /// What's in it stays where it is in the window as the page scrolls (position: fixed), or sticks there within
    /// its containing block (sticky).
    pub scroll: Option<Scroll>,
}

/// How a group moves as the page scrolls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scroll {
    /// With the window: not scrolled at all.
    Fixed,
    /// As it is in the page until its top would go above the window's top less `top` (or its bottom below its bottom
    /// less `bottom`), then held there, within its containing block (`lo` to `hi` down the page). Its top `y` and
    /// height `h`, and the window's height.
    Sticky { top: Option<f32>, bottom: Option<f32>, y: f32, h: f32, lo: f32, hi: f32, view: f32 },
}

impl Scroll {
    /// How far down it's moved with the page scrolled `scroll` down.
    pub fn shift(&self, scroll: f32) -> f32 {
        match *self {
            Scroll::Fixed => scroll,
            Scroll::Sticky { top, bottom, y, h, lo, hi, view } => {
                let mut dy = 0.0f32;
                if let Some(t) = top {
                    dy = (scroll + t - y).min(hi - (y + h)).max(0.0);
                }
                if let Some(b) = bottom {
                    let over = scroll + view - b - (y + h);
                    if over < 0.0 {
                        dy = over.max(lo - y).min(0.0);
                    }
                }
                dy
            }
        }
    }
}

impl Group {
    pub fn none() -> Group {
        Group { opacity: 1.0, transform: None, clip: None, clip_path: None, mask: None, filters: Vec::new(), backdrop: None, blend: 255, hidden: false, scroll: None }
    }

    fn clip_only(r: Rect, radii: [f32; 4]) -> Group {
        Group { clip: Some((r, radii)), ..Group::none() }
    }

    /// Whether it only clips (to a rounded rectangle): drawn without a layer of its own.
    pub fn only_clips(&self) -> bool {
        self.opacity >= 1.0 && self.transform.is_none() && self.clip_path.is_none() && self.mask.is_none() && self.filters.is_empty() && self.backdrop.is_none() && self.blend == 255 && self.scroll.is_none()
    }
}

/// One of a mask's layers: what's drawn into it, whether its luminance shows (else its alpha), and how it's combined
/// with those under it (0 add, 1 subtract, 2 intersect, 3 exclude).
#[derive(Clone, Debug)]
pub struct MaskLayer {
    pub paints: Vec<Paint>,
    pub luminance: bool,
    pub composite: u8,
}

/// What's clipped by: a rectangle, and the rounded rectangles (boxes' padding boxes, their corners rounded) that
/// aren't open round where it's drawn already.
#[derive(Clone, Debug)]
struct Clips {
    rect: Rect,
    rounded: Rc<Vec<(Rect, [f32; 4])>>,
}

impl Clips {
    fn all() -> Clips {
        Clips { rect: Rect::new(-1.0e6, -1.0e6, 2.0e6, 2.0e9), rounded: Rc::new(Vec::new()) }
    }

    fn unrounded(&self) -> Clips {
        Clips { rect: self.rect, rounded: Rc::new(Vec::new()) }
    }
}

/// A box's clips, and those of the absolutely positioned and fixed boxes in it (the clips round their containing
/// blocks).
#[derive(Clone, Debug)]
struct Within {
    own: Clips,
    abs: Clips,
    fixed: Clips,
}

impl Within {
    fn all() -> Within {
        Within { own: Clips::all(), abs: Clips::all(), fixed: Clips::all() }
    }
}

/// A box put off to its stacking context's turn: its z-index, its place in the tree, and its clips.
struct Later {
    z: i32,
    order: usize,
    b: BoxId,
    within: Within,
}

/// The rounded clips a box drawn away from its place in the tree is within, opened round it (each a group); how
/// many.
fn open_rounded(c: &Clips, out: &mut Vec<Item>) -> usize {
    for (r, rad) in c.rounded.iter() {
        out.push(Item { paint: Paint::Begin(Box::new(Group::clip_only(*r, *rad))), clip: c.rect, node: None });
    }
    c.rounded.len()
}

/// A box's inner edge's radii: its outer corners' less the widths of the sides beside each (corners top left, top
/// right, bottom right, bottom left; sides top, right, bottom, left).
fn inner_radii(r: [f32; 4], w: [f32; 4]) -> [f32; 4] {
    [(r[0] - w[0].max(w[3])).max(0.0), (r[1] - w[0].max(w[1])).max(0.0), (r[2] - w[2].max(w[1])).max(0.0), (r[3] - w[2].max(w[3])).max(0.0)]
}

/// How many straight steps a quarter of a circle of radius `r` is drawn in (each within a tenth of a pixel).
fn arc_steps(r: f32) -> usize {
    (num::ceil(1.8 * num::sqrt(r.max(0.0))) as usize).clamp(2, 128)
}

/// A rounded rectangle's outline, as a polygon.
fn rounded_outline(r: Rect, rad: [f32; 4]) -> Vec<(f32, f32)> {
    use core::f32::consts::PI;
    let corners = [
        (r.x + rad[0], r.y + rad[0], rad[0], PI),
        (r.right() - rad[1], r.y + rad[1], rad[1], 1.5 * PI),
        (r.right() - rad[2], r.bottom() - rad[2], rad[2], 0.0),
        (r.x + rad[3], r.bottom() - rad[3], rad[3], 0.5 * PI),
    ];
    let mut pts = Vec::new();
    for (cx, cy, rr, a0) in corners {
        if rr <= 0.0 {
            pts.push((cx, cy));
            continue;
        }
        let n = arc_steps(rr);
        for k in 0..=n {
            let a = a0 + 0.5 * PI * k as f32 / n as f32;
            pts.push((cx + rr * num::cos(a), cy + rr * num::sin(a)));
        }
    }
    pts
}

/// An ellipse's outline, as a polygon.
fn ellipse_outline(c: (f32, f32), rx: f32, ry: f32) -> Vec<(f32, f32)> {
    let n = 4 * arc_steps(rx.max(ry));
    (0..n)
        .map(|k| {
            let a = 2.0 * core::f32::consts::PI * k as f32 / n as f32;
            (c.0 + rx * num::cos(a), c.1 + ry * num::sin(a))
        })
        .collect()
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
    /// The picture fetched for an address a style sheet gives (a background's url()), if it's asked for.
    fn image(&self, _url: &str) -> Option<usize> {
        None
    }
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
    /// How many of those are placed (those after it wait still: moved with the boxes they're in).
    pending_next: usize,
    /// A flex container's absolutely positioned children: the room each would be placed in as its only item (width,
    /// height), and where in it (0 at the start, 1 at the end, each way).
    static_room: alloc::collections::BTreeMap<BoxId, (f32, f32, f32, f32)>,
    /// List items whose marker is placed.
    markers: BTreeSet<BoxId>,
    /// The floats placed in each block formatting context being laid out (the innermost last).
    scopes: Vec<Vec<Placed>>,
    /// Boxes' min-content and max-content widths (border box, margins left out), worked out once.
    intrinsic: Vec<Option<(f32, f32)>>,
    /// Widths of words already measured (by face and size), to measure each once.
    widths: alloc::collections::BTreeMap<(String, u16, u32), f32>,
    /// The content height a box's next layout is to have, given it by its container (a stretched flex item's): its
    /// height is then definite, so its insides' percentages of it mean something.
    definite_h: Option<(BoxId, f32)>,
    /// Each box's place in the tree's order (for painting).
    order: Vec<usize>,
    /// The box whose background the canvas takes (the root, or the body), if either has one.
    canvas_from: Option<BoxId>,
    /// The lines a clamped box (-webkit-line-clamp) being laid out has left to show.
    clamp: Option<usize>,
}

/// The face a style's words are set in: its family's web font (Chrome's defaults' metric twins), bold and italic as
/// it says.
fn face_of(s: &Style) -> Face {
    match s.web_face {
        // (A web font the page loaded: its family's web font for what it lacks.)
        Some((id, bold, lean)) => Face::Loaded(id, s.family(), bold, lean),
        None => Face::Web(s.family(), s.is_bold(), s.italic),
    }
}

fn face_key(f: Face) -> u16 {
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
    /// An absolutely positioned box among them: where it would have been is where it comes on its line.
    Out { b: BoxId },
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
            pending_next: 0,
            static_room: alloc::collections::BTreeMap::new(),
            markers: BTreeSet::new(),
            scopes: Vec::new(),
            intrinsic: vec![None; n],
            widths: alloc::collections::BTreeMap::new(),
            definite_h: None,
            order: Vec::new(),
            canvas_from: None,
            clamp: None,
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
            self.pending_next = k + 1;
            self.absolute(b, sx, sy);
            k += 1;
        }
        self.offsets(0);
        let height = self.extent(0).max(self.view_h);
        let background = self.canvas_background();
        let items = self.paint(height);
        let boxes = self.tree.boxes.iter().filter_map(|b| b.node.map(|n| (n, Rect::new(b.x, b.y, b.w, b.h)))).collect();
        Laid { items, width: self.view_w, height, background, boxes }
    }

    /// The page's background: the root's, else the body's (as CSS has the canvas take it).
    fn canvas_background(&self) -> Color {
        // (A root with effects has its background drawn in their group: the page under it is white.)
        if self.group_of(0).is_some() {
            return values::WHITE;
        }
        match self.canvas_source() {
            Some(b) if values::alpha(self.tree.boxes[b].style.background) > 0 => self.tree.boxes[b].style.background,
            _ => values::WHITE,
        }
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
        // (A collapsible space at a line's end isn't part of it: the space after the last word.)
        let mut trailing = 0.0f32;
        for p in &pieces {
            match p {
                Piece::Word { w, space, breaks, owner, .. } => {
                    trailing = if self.tree.boxes[*owner].style.white_space.keeps_spaces() { 0.0 } else { *space };
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
                    trailing = 0.0;
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
                    mx = mx.max(line - trailing);
                    mn = mn.max(word);
                    line = 0.0;
                    word = 0.0;
                    trailing = 0.0;
                }
                Piece::Float { b: k } => {
                    let (a, z) = self.intrinsic(*k);
                    let m = self.margin_px(*k);
                    mn = mn.max(a + m);
                    line += z + m;
                }
                Piece::Out { .. } => {}
            }
        }
        mx = mx.max(line - trailing);
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
        // (A clamped box: the lines in it counted, those past the clamp left out.)
        let clamp = self.tree.boxes[b].style.line_clamp;
        if clamp > 0 {
            let outer = self.clamp.replace(clamp as usize);
            self.block_in(b, cb_x, top, cb_w, cb_h, width, shrink);
            self.clamp = outer;
        } else {
            self.block_in(b, cb_x, top, cb_w, cb_h, width, shrink);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn block_in(&mut self, b: BoxId, cb_x: f32, top: f32, cb_w: f32, cb_h: Option<f32>, width: Option<f32>, shrink: bool) {
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
                if out_of_flow_at.is_some() {
                    out.push(Piece::Out { b: k });
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
        // (Laid out again, as a grid item stretched to its area is: the lines it had go, its list marker with them.)
        if let Some(f) = self.frags.get_mut(b)
            && !f.is_empty()
        {
            f.clear();
            if let Some(item) = self.list_owner(b) {
                self.markers.remove(&item);
            }
        }
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
                Piece::Out { .. } => line.push(p),
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
        let shows = |line: &Vec<Piece>| {
            line.iter().any(|p| match p {
                Piece::Word { text, .. } => !text.is_empty(),
                Piece::Atomic { .. } | Piece::Break => true,
                Piece::Open { w, .. } | Piece::Close { w, .. } => *w > 0.0,
                Piece::Float { .. } | Piece::Out { .. } => false,
            })
        };
        for (n, line) in lines.iter().enumerate() {
            // (A clamped box's lines past its clamp: left out.)
            if self.clamp == Some(0) && shows(line) {
                break;
            }
            let (band_x, band_w, band_top) = bands.get(n).copied().unwrap_or((0.0, w, at));
            // (Not above where its room beside the floats was worked out: below a float it went down past.)
            if band_top > at {
                at = band_top;
            }
            let visible = line.iter().any(|p| match p {
                Piece::Word { text, .. } => !text.is_empty(),
                Piece::Atomic { .. } | Piece::Break => true,
                Piece::Open { w, .. } | Piece::Close { w, .. } => *w > 0.0,
                Piece::Float { .. } | Piece::Out { .. } => false,
            });
            if !visible {
                // (What's absolutely positioned on it: at its start.)
                for p in line {
                    if let Piece::Out { b: ob } = p {
                        self.pending.push((*ob, x + band_x, at));
                    }
                }
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
                    Piece::Break | Piece::Float { .. } | Piece::Out { .. } => {}
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
            let line_start = frags.len();
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
                    Piece::Out { b: ob } => {
                        // Where it would have been: here on the line (one that was inline), or under the line if
                        // anything's before it on it (one that was a block), as Chrome places them.
                        let before = line[..k].iter().any(|q| match q {
                            Piece::Word { text, .. } => !text.is_empty(),
                            Piece::Atomic { .. } => true,
                            _ => false,
                        });
                        let (sx, sy) = if self.tree.boxes[*ob].style.inline_origin {
                            (pen, at)
                        } else {
                            (x, if before { at + line_h } else { at })
                        };
                        self.pending.push((*ob, sx, sy));
                    }
                    Piece::Break | Piece::Float { .. } => {}
                }
            }
            flush_run(self, &mut frags, &mut run, pen);
            // The clamp's last line, with more after it, ends in "…"; as does a line too long for a box that clips it
            // (text-overflow: ellipsis).
            let mut cut = false;
            if let Some(left) = self.clamp {
                self.clamp = Some(left.saturating_sub(1));
                cut = left == 1 && lines[n + 1..].iter().any(shows);
            }
            let edge = x + band_x + band_w;
            if cut || (s.ellipsis && s.clip && pen > edge + 0.5) {
                self.ellipsis(b, &mut frags, line_start, edge);
            }
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

    /// A line's words (`frags` from `from` on) cut to end in "…" before `edge`, in the block's own font, as
    /// text-overflow: ellipsis and a line clamp end one.
    fn ellipsis(&mut self, b: BoxId, frags: &mut Vec<Frag>, from: usize, edge: f32) {
        let s = self.tree.boxes[b].style.clone();
        let face = face_of(&s);
        let ell = "\u{2026}";
        let ew = self.measure(ell, face, s.font_size);
        let limit = edge - ew;
        // The last words that reach past the limit: as many of their characters as fit; those after them go.
        let mut end_x = None;
        let mut line_y = None;
        let mut k = frags.len();
        while k > from {
            k -= 1;
            let f = frags[k].clone();
            let FragKind::Words { text, owner } = &f.kind else { continue };
            line_y = line_y.or(Some((f.y, f.h)));
            let os = self.tree.boxes[*owner].style.clone();
            let ofa = face_of(&os);
            // (Its words' own width: a run's box reaches over the space after its last word.)
            let own_w = self.measure(text, ofa, os.font_size).min(f.w);
            if f.x + own_w <= limit + 0.01 {
                end_x = Some(f.x + own_w);
                break;
            }
            let chars: Vec<char> = text.chars().collect();
            let mut keep = chars.len();
            let mut kept_w = f.w;
            while keep > 0 {
                keep -= 1;
                let head: String = chars[..keep].iter().collect::<String>().trim_end().into();
                kept_w = self.measure(&head, ofa, os.font_size);
                if f.x + kept_w <= limit + 0.01 {
                    break;
                }
            }
            let head: String = chars[..keep].iter().collect::<String>().trim_end().into();
            if head.is_empty() {
                frags.remove(k);
                continue;
            }
            frags[k] = Frag { kind: FragKind::Words { text: head, owner: *owner }, x: f.x, y: f.y, w: kept_w, h: f.h };
            frags.truncate(k + 1);
            end_x = Some(f.x + kept_w);
            break;
        }
        // (Words after the cut, and pieces of inline boxes past it, go.)
        let at = end_x.unwrap_or(limit.min(edge));
        let mut tail = frags.split_off(from.min(frags.len()));
        tail.retain(|f| !matches!(f.kind, FragKind::Words { .. }) || f.x < at + 0.01);
        frags.extend(tail);
        let (y, h) = line_y.unwrap_or((0.0, s.line_px()));
        frags.push(Frag { kind: FragKind::Words { text: String::from(ell), owner: b }, x: at, y, w: ew, h });
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
        let mut outside: Vec<BoxId> = Vec::new();
        for &k in &self.tree.boxes[b].children.clone() {
            if self.tree.boxes[k].style.out_of_flow() {
                outside.push(k);
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
        let inner_h = if row {
            self.flex_row(b, &items, x, y, w, h, reverse)
        } else {
            self.flex_column(b, &items, x, y, w, h, reverse)
        };
        // Absolutely positioned children: where each would be as the container's only item (its justify-content
        // along it, align-self across it), as CSS places them when they've no insets of their own.
        for k in outside {
            let ks = &self.tree.boxes[k].style;
            let along = match s.justify_content {
                Align::Center | Align::SpaceAround | Align::SpaceEvenly => 0.5,
                Align::End => 1.0,
                _ => 0.0,
            };
            let along = if reverse { 1.0 - along } else { along };
            let across = match if ks.align_self != Align::Normal { ks.align_self } else { s.align_items } {
                Align::Center => 0.5,
                Align::End => 1.0,
                _ => 0.0,
            };
            let (fx, fy) = if row { (along, across) } else { (across, along) };
            self.pending.push((k, x, y));
            if fx != 0.0 || fy != 0.0 {
                self.static_room.insert(k, (w, h.unwrap_or(inner_h), fx, fy));
            }
        }
        inner_h
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
            // (Less than none, where the items are wider than the line: centred and end-aligned ones then reach out
            // past both ends, or past the start.)
            let over = w - outer_sum - gaps;
            let free = over.max(0.0);
            // Auto margins take what's free; else justify-content.
            let autos = its[r.clone()].iter().map(|i| i.auto_l as u32 + i.auto_r as u32).sum::<u32>();
            let (mut pen, between) = if autos > 0 {
                (0.0, 0.0)
            } else {
                match s.justify_content {
                    Align::End => (over, 0.0),
                    Align::Center => (over / 2.0, 0.0),
                    Align::SpaceBetween if n > 1 => (0.0, free / (n - 1) as f32),
                    Align::SpaceAround | Align::SpaceEvenly if over < 0.0 => (over / 2.0, 0.0),
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
            } else if single {
                // (A single line in a container of no height of its own: as high as its min-height and max-height let
                // it be, its items aligned in that, as a button's icon is centred in its min-height.)
                if let Size::Len(l) = s.max_height
                    && !l.has_pct()
                {
                    line_h = line_h.min(self.content_from(b, l, 0.0, false));
                }
                if let Size::Len(l) = s.min_height
                    && !l.has_pct()
                {
                    line_h = line_h.max(self.content_from(b, l, 0.0, false));
                }
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
        // The container's baseline: its first item's (one that has none: its border box's bottom, as CSS makes one up).
        if let Some(first) = its.first() {
            let fb = &self.tree.boxes[first.b];
            let by = fb.y + fb.baseline.unwrap_or(fb.h);
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
                // (One wider than the column, centred, reaches out past both sides.)
                let free = w - bw - ml - mr;
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
        // (Its first item's baseline, or one made up from its border box's bottom.)
        if let Some(&first) = placed.first() {
            let fb = &self.tree.boxes[first];
            let by = fb.y + fb.baseline.unwrap_or(fb.h);
            self.tree.boxes[b].baseline = Some(by - self.tree.boxes[b].y);
        }
        used.max(0.0)
    }

    // ------------------------------------------------------------------------------------------------ grid

    /// A grid container's insides (CSS Grid): its items placed in their areas, by name, by line or flowing into the
    /// cells free, and its tracks sized to them as CSS's track sizing algorithm sizes them. Returns their height.
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
        let col_list = repeat_tracks(&s.grid_columns, Some(w), gap);
        let row_list = repeat_tracks(&s.grid_rows, h, row_gap);
        let explicit_cols = col_list.len().max(s.grid_areas.iter().map(|r| r.len()).max().unwrap_or(0));
        let explicit_rows = row_list.len().max(s.grid_areas.len());
        // Where each item goes: rows r0..r1, columns c0..c1 (grid-auto-flow: column fills the columns first).
        let by_column = s.grid_auto_columns;
        let specs: Vec<GridSpec> = items
            .iter()
            .map(|&k| {
                let ks = &self.tree.boxes[k].style;
                let area = ks.grid_area.as_ref().and_then(|name| named_area(&s.grid_areas, name));
                let (row, col) = (placement(ks.grid_row, explicit_rows), placement(ks.grid_column, explicit_cols));
                if by_column { (area.map(|[r0, r1, c0, c1]| [c0, c1, r0, r1]), col, row) } else { (area, row, col) }
            })
            .collect();
        let placed = place_grid(&specs, if by_column { explicit_rows } else { explicit_cols });
        let cells: Vec<(BoxId, usize, usize, usize, usize)> =
            items.iter().zip(placed).map(|(&k, p)| if by_column { (k, p[2], p[3], p[0], p[1]) } else { (k, p[0], p[1], p[2], p[3]) }).collect();
        let cols = cells.iter().map(|c| c.4).max().unwrap_or(0).max(explicit_cols).max(1);
        let rows = cells.iter().map(|c| c.2).max().unwrap_or(0).max(explicit_rows);
        // The columns, sized to what's in them.
        let mut col_items = Vec::with_capacity(cells.len());
        for &(k, _, _, c0, c1) in &cells {
            self.metrics(k, w);
            let (mn, mx) = self.intrinsic(k);
            let m = self.margin_px(k);
            let ks = self.tree.boxes[k].style.clone();
            let minimum = match ks.min_width {
                Size::Len(l) if !l.has_pct() => self.content_from(k, l, w, true) + self.frame_w(k) + m,
                // (One that clips what's in it can be as narrow as it likes.)
                _ if ks.clip => m,
                _ => mn + m,
            };
            col_items.push(Contrib { from: c0, to: c1, min_c: mn + m, max_c: mx + m, minimum });
        }
        let mut col_tr: Vec<Tr> = (0..cols).map(|c| Tr::of(col_list.get(c), Some(w))).collect();
        size_tracks(&mut col_tr, &col_items, Some(w), gap, matches!(s.justify_content, Align::Normal | Align::Stretch));
        let widths: Vec<f32> = col_tr.iter().map(|t| t.base).collect();
        let col_x = track_starts(&widths, gap, w, s.justify_content);
        let span_w = |c0: usize, c1: usize| widths[c0..c1].iter().sum::<f32>() + gap * (c1 - c0).saturating_sub(1) as f32;
        // Each item laid out in its columns, its height what it gives its rows.
        let mut row_items = Vec::with_capacity(cells.len());
        for &(k, r0, r1, c0, c1) in &cells {
            let area_w = span_w(c0, c1);
            self.metrics(k, area_w);
            let ks = self.tree.boxes[k].style.clone();
            let justify = if ks.justify_self != Align::Normal { ks.justify_self } else { s.justify_items };
            let shrink = matches!(justify, Align::Start | Align::End | Align::Center);
            self.block(k, x + col_x[c0], y, area_w, None, None, shrink);
            let (mt, mb) = (self.tree.boxes[k].margin[0], self.tree.boxes[k].margin[2]);
            let mh = self.tree.boxes[k].h + mt + mb;
            row_items.push(Contrib { from: r0, to: r1, min_c: mh, max_c: mh, minimum: if ks.clip { mt + mb } else { mh } });
        }
        let mut row_tr: Vec<Tr> = (0..rows).map(|r| Tr::of(row_list.get(r), h)).collect();
        size_tracks(&mut row_tr, &row_items, h, row_gap, matches!(s.align_content, Align::Normal | Align::Stretch));
        let heights: Vec<f32> = row_tr.iter().map(|t| t.base).collect();
        let row_y = track_starts(&heights, row_gap, h.unwrap_or(0.0), if h.is_some() { s.align_content } else { Align::Normal });
        let span_h = |r0: usize, r1: usize| heights[r0..r1].iter().sum::<f32>() + row_gap * (r1 - r0).saturating_sub(1) as f32;
        // Each in its area: aligned in it, or stretched to fill it (laid out again at that height, if it's taller).
        for &(k, r0, r1, c0, c1) in &cells {
            let (area_x, area_w) = (x + col_x[c0], span_w(c0, c1));
            let (area_y, area_h) = (y + row_y.get(r0).copied().unwrap_or(0.0), span_h(r0, r1.min(rows)));
            let ks = self.tree.boxes[k].style.clone();
            let replaced = matches!(self.tree.boxes[k].content, Content::Image { .. } | Content::Control(_));
            let justify = if ks.justify_self != Align::Normal { ks.justify_self } else { s.justify_items };
            let shrink = matches!(justify, Align::Start | Align::End | Align::Center);
            let align = if ks.align_self != Align::Normal { ks.align_self } else { s.align_items };
            let (mt, mb) = (self.tree.boxes[k].margin[0], self.tree.boxes[k].margin[2]);
            let stretch = match align {
                Align::Stretch => ks.height == Size::Auto,
                Align::Normal => ks.height == Size::Auto && !replaced,
                _ => false,
            };
            let target = (area_h - mt - mb).max(0.0);
            if stretch && target > self.tree.boxes[k].h + 0.5 {
                let inner = self.clamp_h(k, (target - self.frame_h(k)).max(0.0), Some(area_h));
                self.definite_h = Some((k, inner));
                self.block(k, area_x, area_y + mt, area_w, Some(area_h), None, shrink);
            } else {
                let mh = self.tree.boxes[k].h + mt + mb;
                let dy = match align {
                    Align::Center => (area_h - mh) / 2.0,
                    Align::End => area_h - mh,
                    _ => 0.0,
                };
                let oy = self.tree.boxes[k].y;
                self.shift(k, 0.0, area_y + mt + dy - oy);
            }
            if shrink {
                let (bw, ml, mr) = (self.tree.boxes[k].w, self.tree.boxes[k].margin[3], self.tree.boxes[k].margin[1]);
                let free = (area_w - bw - ml - mr).max(0.0);
                let dx = match justify {
                    Align::Center => free / 2.0,
                    Align::End => free,
                    _ => 0.0,
                };
                let ox = self.tree.boxes[k].x;
                self.shift(k, area_x + ml + dx - ox, 0.0);
            }
        }
        // The container's baseline: its first item's in its first row (or one made up from that item's border box).
        if let Some(&(first, ..)) = cells.iter().filter(|c| c.1 == 0).min_by_key(|c| c.3) {
            let fb = &self.tree.boxes[first];
            let by = fb.y + fb.baseline.unwrap_or(fb.h);
            self.tree.boxes[b].baseline = Some(by - self.tree.boxes[b].y);
        }
        let total = heights.iter().sum::<f32>() + row_gap * rows.saturating_sub(1) as f32;
        match h {
            Some(hh) => hh.max(0.0),
            None => total.max(0.0),
        }
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
        let (cx, cy, cw, ch) = {
            // (A fixed box's is the window, unless a box it's in has a transform, a filter or paint containment.)
            let fixed = s.position == Position::Fixed;
            let mut at = self.tree.boxes[b].parent;
            let mut found = None;
            while let Some(p) = at {
                let ps = &self.tree.boxes[p].style;
                let holds = ps.has_transform() || ps.filter.is_some() || ps.backdrop.is_some() || ps.contain_paint || (!fixed && (ps.is_positioned() || p == 0));
                if holds {
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
            _ => static_x + ml + self.static_room.get(&b).map_or(0.0, |&(rw, _, fx, _)| (rw - bw - ml - mr) * fx),
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
            _ => static_y + mt + self.static_room.get(&b).map_or(0.0, |&(_, rh, _, fy)| (rh - bh - mt - mb) * fy),
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
            // (One still waiting to be placed: where it would have been moves with what it's in.)
            if self.tree.boxes[k].style.out_of_flow() {
                let next = self.pending_next.min(self.pending.len());
                for p in self.pending[next..].iter_mut().filter(|p| p.0 == k) {
                    p.1 += dx;
                    p.2 += dy;
                }
            }
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

    /// What's drawn, in order, as CSS's painting order has it (CSS 2's appendix E): the canvas's background pictures,
    /// then the root's stacking context, each stacking context's positioned descendants and the stacking contexts in
    /// it by z-index.
    fn paint(&mut self, height: f32) -> Vec<Item> {
        // Each box's place in the tree's order (positioned boxes of one z-index are drawn in it).
        self.order = vec![0; self.tree.boxes.len()];
        let mut n = 0;
        let mut stack = vec![0];
        while let Some(b) = stack.pop() {
            self.order[b] = n;
            n += 1;
            stack.extend(self.tree.boxes[b].children.iter().rev().copied());
        }
        self.canvas_from = self.canvas_source();
        let mut items = Vec::new();
        // (The root's effects, opacity and the like, are on all of it: its background too, the canvas's.)
        let root_group = self.group_of(0);
        if let Some(g) = &root_group {
            items.push(Item { paint: Paint::Begin(Box::new(g.clone())), clip: Clips::all().rect, node: None });
            if self.canvas_from.is_some() {
                let page = Rect::new(0.0, 0.0, self.view_w.max(self.tree.boxes[0].w), height.max(self.view_h));
                let c = self.tree.boxes[self.canvas_from.unwrap_or(0)].style.background;
                items.push(Item { paint: Paint::Rect { x: page.x, y: page.y, w: page.w, h: page.h, color: c, radius: [0.0; 4] }, clip: page, node: None });
            }
        }
        self.paint_canvas(height, &mut items);
        self.paint_context(0, &Within::all(), &mut items);
        if root_group.is_some() {
            items.push(Item::end());
        }
        items
    }

    /// Whether a box is a stacking context: the root; positioned with a z-index, or a flex or grid item with one; or
    /// with what makes one of itself (opacity, a transform, a filter, a mask, clip-path, blending...).
    fn makes_context(&self, b: BoxId) -> bool {
        let x = &self.tree.boxes[b];
        let s = &x.style;
        b == 0 || s.effects_context() || (s.z_index.is_some() && self.z_applies(b))
    }

    /// Whether a box's z-index counts: it's positioned, or a flex or grid item.
    fn z_applies(&self, b: BoxId) -> bool {
        let x = &self.tree.boxes[b];
        x.style.is_positioned() || x.parent.is_some_and(|p| matches!(self.tree.boxes[p].style.display, Display::Flex | Display::Grid | Display::InlineFlex | Display::InlineGrid))
    }

    /// Whether a box is drawn at its stacking context's turn for z-indexes rather than where it is in the tree:
    /// positioned, or a stacking context.
    fn put_off(&self, b: BoxId) -> bool {
        self.tree.boxes[b].style.position != Position::Static || self.makes_context(b)
    }

    /// A stacking context: its root's background and borders, its descendants with a negative z-index, what's in it
    /// in flow, then its positioned descendants and the stacking contexts in it by z-index (0 and auto in the tree's
    /// order), all inside its group when it has effects (opacity, a transform, filters, a mask, clip-path,
    /// blending).
    fn paint_context(&mut self, root: BoxId, within: &Within, items: &mut Vec<Item>) {
        let node = self.tree.boxes[root].node;
        if self.tree.boxes[root].style.hidden_text {
            return;
        }
        let group = if root == 0 { None } else { self.group_of(root) };
        if group.as_ref().is_some_and(|g| g.hidden) {
            return;
        }
        // Inside a turned one, its own coordinates: the clips round it are its group's. Inside the others, the same
        // rectangles; the rounded clips round it are open round all of it already.
        let inside = if group.as_ref().is_some_and(|g| g.transform.is_some()) {
            Within::all()
        } else {
            Within { own: within.own.unrounded(), abs: within.abs.unrounded(), fixed: within.fixed.unrounded() }
        };
        let (mut own, mut neg, mut flow, mut pos) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut later: Vec<Later> = Vec::new();
        if self.tree.boxes[root].style.visible {
            self.paint_box(root, &inside.own, &mut own);
        }
        self.paint_tree(root, &inside, &mut flow, &mut later, true);
        let mut rounds = 0;
        while !later.is_empty() && rounds < 100_000 {
            rounds += 1;
            let at = (0..later.len()).min_by_key(|&i| (later[i].z, later[i].order)).unwrap_or(0);
            let l = later.swap_remove(at);
            let out = if l.z < 0 { &mut neg } else { &mut pos };
            // (Drawn away from its place in the tree: the rounded clips round it there are opened round it again.)
            let opened = open_rounded(&l.within.own, out);
            if self.makes_context(l.b) {
                self.paint_context(l.b, &l.within, out);
            } else {
                self.paint_tree(l.b, &l.within, out, &mut later, false);
            }
            for _ in 0..opened {
                out.push(Item::end());
            }
        }
        let grouped = group.is_some();
        // (Fixed to the window, or sticky: moved as the page scrolls, round all its effects.)
        let scroll = self.scroll_of(root);
        if let Some(sc) = scroll {
            items.push(Item { paint: Paint::Begin(Box::new(Group { scroll: Some(sc), ..Group::none() })), clip: Clips::all().rect, node: None });
        }
        if let Some(g) = group {
            items.push(Item { paint: Paint::Begin(Box::new(g)), clip: within.own.rect, node });
        }
        items.extend(own);
        items.extend(neg);
        items.extend(flow);
        items.extend(pos);
        if grouped {
            items.push(Item::end());
        }
        if scroll.is_some() {
            items.push(Item::end());
        }
    }

    /// Paints `root` and what's in it in flow (without recursion), putting off what's drawn at its stacking
    /// context's turn (positioned boxes and stacking contexts). `root_drawn`: its own background and borders are drawn
    /// already.
    fn paint_tree(&mut self, root: BoxId, within: &Within, items: &mut Vec<Item>, later: &mut Vec<Later>, root_drawn: bool) {
        enum Step {
            Box(BoxId, Within),
            Close,
        }
        let mut stack = vec![Step::Box(root, within.clone())];
        while let Some(step) = stack.pop() {
            let (b, w) = match step {
                Step::Close => {
                    items.push(Item::end());
                    continue;
                }
                Step::Box(b, w) => (b, w),
            };
            let laid = self.tree.boxes[b].done || b == 0 || matches!(self.tree.boxes[b].content, Content::Text(_) | Content::Break) || self.tree.boxes[b].is_inline_box();
            if !laid {
                continue;
            }
            if b != root && self.put_off(b) {
                // (z-index is for positioned boxes, and flex and grid items: on others it's nothing.)
                let z = if self.z_applies(b) { self.tree.boxes[b].style.z_index.unwrap_or(0) } else { 0 };
                later.push(Later { z, order: self.order[b], b, within: w });
                continue;
            }
            let x = &self.tree.boxes[b];
            let s = &x.style;
            if s.hidden_text {
                continue;
            }
            let (visible, clips, contain) = (s.visible, s.clip, s.contain_paint);
            let abs_home = s.is_positioned() || s.has_transform() || b == 0;
            let fixed_home = s.has_transform() || s.filter.is_some() || s.backdrop.is_some() || contain;
            let pad = Rect::new(x.x + x.border[3], x.y + x.border[0], x.w - x.border[1] - x.border[3], x.h - x.border[0] - x.border[2]);
            let inner = inner_radii(radii(s, x.w, x.h), x.border);
            if visible && !(b == root && root_drawn) {
                self.paint_box(b, &w.own, items);
            }
            // What's in it: clipped to its padding box (its rounded corners too), if it clips.
            let mut content = w.own.clone();
            if (clips || contain) && b != 0 {
                content.rect = content.rect.intersect(&pad);
                if inner.iter().any(|&r| r > 0.0) {
                    items.push(Item { paint: Paint::Begin(Box::new(Group::clip_only(pad, inner))), clip: content.rect, node: None });
                    let mut list = (*content.rounded).clone();
                    list.push((pad, inner));
                    content.rounded = Rc::new(list);
                    stack.push(Step::Close);
                }
            }
            let for_abs = if abs_home { content.clone() } else { w.abs.clone() };
            let for_fixed = if fixed_home { content.clone() } else { w.fixed.clone() };
            // Its lines.
            if let Some(frags) = self.frags.get(b).cloned() {
                for (k, f) in frags.iter().enumerate() {
                    // (An inline box's pieces' backgrounds are placed as if its pieces were one, side by side.)
                    let slice = match f.kind {
                        FragKind::Back { owner, .. } => {
                            let same = |g: &Frag| matches!(g.kind, FragKind::Back { owner: o, .. } if o == owner);
                            let before: f32 = frags[..k].iter().filter(|g| same(g)).map(|g| g.w).sum();
                            let total: f32 = frags.iter().filter(|g| same(g)).map(|g| g.w).sum();
                            (before, total)
                        }
                        _ => (0.0, f.w),
                    };
                    self.paint_frag(f, content.rect, items, slice);
                }
            }
            for &k in self.tree.boxes[b].children.iter().rev() {
                if matches!(self.tree.boxes[k].content, Content::Text(_) | Content::Break) {
                    continue;
                }
                let own = match self.tree.boxes[k].style.position {
                    Position::Absolute => for_abs.clone(),
                    Position::Fixed => for_fixed.clone(),
                    _ => content.clone(),
                };
                stack.push(Step::Box(k, Within { own, abs: for_abs.clone(), fixed: for_fixed.clone() }));
            }
        }
    }

    /// How a box moves as the page scrolls: a fixed box with the window (unless a box it's in has a transform, a
    /// filter or paint containment: it's placed in that box, and scrolls with it); a sticky one within its containing
    /// block (unless a box it's in clips what's in it: that box would scroll it, and doesn't).
    fn scroll_of(&self, b: BoxId) -> Option<Scroll> {
        let x = &self.tree.boxes[b];
        let s = &x.style;
        // (The root's and the body's overflow is the window's.)
        let body = self.tree.boxes[0].children.iter().copied().find(|&k| self.tree.boxes[k].node.is_some() && matches!(self.tree.boxes[k].content, Content::Children));
        match s.position {
            Position::Fixed => {
                let mut p = x.parent;
                while let Some(k) = p {
                    let ps = &self.tree.boxes[k].style;
                    if ps.has_transform() || ps.filter.is_some() || ps.backdrop.is_some() || ps.contain_paint {
                        return None;
                    }
                    p = self.tree.boxes[k].parent;
                }
                Some(Scroll::Fixed)
            }
            Position::Sticky => {
                let parent = x.parent?;
                let mut p = Some(parent);
                while let Some(k) = p {
                    if k != 0 && Some(k) != body && self.tree.boxes[k].style.clip {
                        return None;
                    }
                    p = self.tree.boxes[k].parent;
                }
                let top = s.inset[0].len().map(|l| l.resolve(self.view_h));
                let bottom = s.inset[2].len().map(|l| l.resolve(self.view_h));
                if top.is_none() && bottom.is_none() {
                    return None;
                }
                let pb = &self.tree.boxes[parent];
                let (lo, hi) = (pb.y + pb.border[0] + pb.padding[0], pb.y + pb.h - pb.border[2] - pb.padding[2]);
                let (mt, mb) = (x.margin[0], x.margin[2]);
                Some(Scroll::Sticky { top, bottom, y: x.y - mt, h: x.h + mt + mb, lo, hi, view: self.view_h })
            }
            _ => None,
        }
    }

    /// The box whose background is the canvas's: the root's, else (when the root has none) the body's.
    fn canvas_source(&self) -> Option<BoxId> {
        let has = |b: &LBox| values::alpha(b.style.background) > 0 || b.style.bg.as_ref().is_some_and(|l| l.any());
        let root = &self.tree.boxes[0];
        if has(root) {
            return Some(0);
        }
        root.children.iter().copied().find(|&k| {
            let b = &self.tree.boxes[k];
            b.node.is_some() && matches!(b.content, Content::Children) && has(b)
        })
    }

    /// The canvas's background pictures and gradients (the root's, or the body's), over all of it, placed as the
    /// root's would be.
    fn paint_canvas(&mut self, height: f32, items: &mut Vec<Item>) {
        let Some(from) = self.canvas_from else { return };
        let Some(l) = self.tree.boxes[from].style.bg.clone().filter(|l| l.any()) else { return };
        let page = Rect::new(0.0, 0.0, self.view_w.max(self.tree.boxes[0].w), height.max(self.view_h));
        for i in (0..l.images.len()).rev() {
            if let Some(Some(img)) = l.images.get(i)
                && let Some(t) = self.tiles(0, &l, i, img, false, Some((page, [0.0; 4])))
            {
                items.push(Item { paint: t, clip: page, node: None });
            }
        }
    }

    /// A box's border box, padding box or content box, and its corners' radii there (each a border's or padding's
    /// width less than the outer edge's).
    fn area(&self, b: BoxId, which: BoxArea) -> (Rect, [f32; 4]) {
        let x = &self.tree.boxes[b];
        let r = radii(&x.style, x.w, x.h);
        let (t, rt, bo, l) = (x.border[0], x.border[1], x.border[2], x.border[3]);
        match which {
            BoxArea::Border | BoxArea::Text => (Rect::new(x.x, x.y, x.w, x.h), r),
            BoxArea::Padding => (Rect::new(x.x + l, x.y + t, (x.w - l - rt).max(0.0), (x.h - t - bo).max(0.0)), inner_radii(r, x.border)),
            BoxArea::Content => {
                let p = x.padding;
                let e = [t + p[0], rt + p[1], bo + p[2], l + p[3]];
                (Rect::new(x.x + e[3], x.y + e[0], (x.w - e[1] - e[3]).max(0.0), (x.h - e[0] - e[2]).max(0.0)), inner_radii(r, e))
            }
        }
    }

    /// One background (or mask) layer's tiles: the `i`th layer's picture or gradient, sized, placed and repeated as
    /// its background-size, -position and -repeat say, within its clip box (or `painting`, the canvas's). None if
    /// there's nothing to draw (a picture that hasn't come, a size of 0).
    fn tiles(&self, b: BoxId, l: &Layers, i: usize, img: &Image, mask: bool, painting: Option<(Rect, [f32; 4])>) -> Option<Paint> {
        self.tiles_in(b, l, i, img, mask, None, painting)
    }

    /// `tiles`, placed in `positioning` when it's given (an inline box's pieces, as one).
    #[allow(clippy::too_many_arguments)]
    fn tiles_in(&self, b: BoxId, l: &Layers, i: usize, img: &Image, mask: bool, positioning: Option<Rect>, painting: Option<(Rect, [f32; 4])>) -> Option<Paint> {
        let origin = effects::nth(&l.origins, i, if mask { BoxArea::Border } else { BoxArea::Padding });
        let fixed = effects::nth(&l.fixed, i, false);
        let pa = match positioning {
            Some(r) => r,
            None if fixed => Rect::new(0.0, 0.0, self.view_w, self.view_h),
            None => self.area(b, origin).0,
        };
        let (ca, crad) = painting.unwrap_or_else(|| self.area(b, effects::nth(&l.clips, i, BoxArea::Border)));
        let (image, natural) = match img {
            Image::Url(u) => {
                let n = self.sizes.image(u)?;
                let (w, h) = self.sizes.size(n)?;
                (Tile::Picture(n), Some((w as f32 * self.dpr, h as f32 * self.dpr)))
            }
            Image::Gradient(g) => (Tile::Gradient(g.clone()), None),
        };
        let natural = natural.filter(|&(w, h)| w > 0.0 && h > 0.0);
        let size = effects::nth(&l.sizes, i, BgSize::Sized(None, None));
        let (auto_w, auto_h) = match size {
            BgSize::Sized(a, b) => (a.is_none(), b.is_none()),
            _ => (false, false),
        };
        let (mut w, mut h) = match size {
            BgSize::Cover | BgSize::Contain => match natural {
                Some((iw, ih)) => {
                    let k = if size == BgSize::Cover { (pa.w / iw).max(pa.h / ih) } else { (pa.w / iw).min(pa.h / ih) };
                    (iw * k, ih * k)
                }
                None => (pa.w, pa.h),
            },
            BgSize::Sized(wl, hl) => {
                let (wv, hv) = (wl.map(|x| x.resolve(pa.w)), hl.map(|x| x.resolve(pa.h)));
                match (wv, hv, natural) {
                    (Some(w), Some(h), _) => (w, h),
                    (Some(w), None, Some((iw, ih))) => (w, w * ih / iw),
                    (None, Some(h), Some((iw, ih))) => (h * iw / ih, h),
                    (None, None, Some((iw, ih))) => (iw, ih),
                    (Some(w), None, None) => (w, pa.h),
                    (None, Some(h), None) => (pa.w, h),
                    (None, None, None) => (pa.w, pa.h),
                }
            }
        };
        let (rx, ry) = effects::nth(&l.repeats, i, (Repeat::Repeat, Repeat::Repeat));
        // round: as many whole tiles as fit best, stretched to fill (the other side kept in proportion if it's auto).
        if rx == Repeat::Round && w > 0.0 && pa.w > 0.0 {
            let nw = pa.w / num::round(pa.w / w).max(1.0);
            if ry != Repeat::Round && auto_h {
                h *= nw / w;
            }
            w = nw;
        }
        if ry == Repeat::Round && h > 0.0 && pa.h > 0.0 {
            let nh = pa.h / num::round(pa.h / h).max(1.0);
            if rx != Repeat::Round && auto_w {
                w *= nh / h;
            }
            h = nh;
        }
        if w <= 0.0 || h <= 0.0 || ca.w <= 0.0 || ca.h <= 0.0 {
            return None;
        }
        let mut x = pa.x + effects::nth(&l.pos_x, i, Pos::START).resolve(pa.w - w);
        let mut y = pa.y + effects::nth(&l.pos_y, i, Pos::START).resolve(pa.h - h);
        let (mut sx, mut sy) = (w, h);
        let (mut rep_x, mut rep_y) = (rx != Repeat::NoRepeat, ry != Repeat::NoRepeat);
        // space: as many whole tiles as fit, spread out to touch the area's edges (one, placed, where two don't fit).
        if rx == Repeat::Space {
            let n = num::floor(pa.w / w);
            if n >= 2.0 {
                sx = w + (pa.w - n * w) / (n - 1.0);
                x = pa.x;
            } else {
                rep_x = false;
            }
        }
        if ry == Repeat::Space {
            let n = num::floor(pa.h / h);
            if n >= 2.0 {
                sy = h + (pa.h - n * h) / (n - 1.0);
                y = pa.y;
            } else {
                rep_y = false;
            }
        }
        let pixelated = self.tree.boxes[b].style.pixelated;
        Some(Paint::Tiles { x: ca.x, y: ca.y, w: ca.w, h: ca.h, radius: crad, tile: Rect::new(x, y, w, h), step: (sx, sy), repeat: (rep_x, rep_y), image, pixelated })
    }

    /// The words drawn in a box and what's in it (for a background clipped to them), as paints.
    fn text_paints(&self, b: BoxId) -> Vec<Paint> {
        let mut out = Vec::new();
        let mut stack = vec![b];
        while let Some(k) = stack.pop() {
            if let Some(frags) = self.frags.get(k) {
                for f in frags {
                    if let FragKind::Words { text, owner } = &f.kind {
                        let s = &self.tree.boxes[*owner].style;
                        out.push(Paint::Text { x: f.x, y: f.y, w: f.w, text: text.clone(), face: face_of(s), size: s.font_size, line: f.h, color: values::BLACK });
                    }
                }
            }
            stack.extend(self.tree.boxes[k].children.iter().copied());
        }
        out
    }

    /// A box's background: its colour (within its bottom layer's clip box), then its layers' pictures and gradients,
    /// the bottom one first; clipped to its words, if a layer says so.
    fn paint_background(&mut self, b: BoxId, clip: Rect, items: &mut Vec<Item>) {
        let x = &self.tree.boxes[b];
        let node = x.node;
        let background = x.style.background;
        let layers = x.style.bg.clone().filter(|l| l.any());
        let n = layers.as_ref().map_or(1, |l| l.images.len().max(1));
        let clip_of = |i: usize| layers.as_ref().map_or(BoxArea::Border, |l| effects::nth(&l.clips, i, BoxArea::Border));
        let to_text = (0..n).any(|i| clip_of(i) == BoxArea::Text);
        let mut out = Vec::new();
        if values::alpha(background) > 0 {
            let (r, rad) = self.area(b, clip_of(n - 1));
            out.push(Paint::Rect { x: r.x, y: r.y, w: r.w, h: r.h, color: background, radius: rad });
        }
        if let Some(l) = &layers {
            for i in (0..l.images.len()).rev() {
                if let Some(Some(img)) = l.images.get(i)
                    && let Some(t) = self.tiles(b, l, i, img, false, None)
                {
                    out.push(t);
                }
            }
        }
        if out.is_empty() {
            return;
        }
        if to_text {
            let words = self.text_paints(b);
            let g = Group { mask: Some(vec![MaskLayer { paints: words, luminance: false, composite: 0 }]), ..Group::none() };
            items.push(Item { paint: Paint::Begin(Box::new(g)), clip, node: None });
        }
        for paint in out {
            items.push(Item { paint, clip, node });
        }
        if to_text {
            items.push(Item::end());
        }
    }

    /// A stacking context's effects, as a group drawn as one (None when it has none: a positioned box with a
    /// z-index, say).
    fn group_of(&self, b: BoxId) -> Option<Group> {
        let x = &self.tree.boxes[b];
        let s = &x.style;
        let (bx, by, bw, bh) = (x.x, x.y, x.w, x.h);
        let mut g = Group::none();
        let mut any = false;
        if s.opacity < 1.0 {
            g.opacity = s.opacity;
            any = true;
        }
        if s.turned {
            let o = (bx + s.origin.0.resolve(bw), by + s.origin.1.resolve(bh));
            let m = effects::matrix_of(&s.transform_fns(), bw, bh, o);
            // (Turned to show its back, with backface-visibility: hidden: not drawn.)
            if s.backface_hidden && m[0] * m[3] - m[1] * m[2] < 0.0 {
                g.hidden = true;
            }
            g.transform = Some(m);
            any = true;
        }
        if let Some(f) = &s.filter {
            g.filters = (**f).clone();
            any = true;
        }
        if let Some(f) = &s.backdrop {
            g.backdrop = Some(((**f).clone(), Rect::new(bx, by, bw, bh), radii(s, bw, bh)));
            any = true;
        }
        if s.blend != 255 {
            g.blend = s.blend;
            any = true;
        }
        if let Some(m) = s.mask.as_ref().filter(|m| m.any()) {
            // (A mask that's an SVG <mask> in the page isn't read: it's left unmasked.)
            if !m.images.iter().flatten().any(|i| matches!(i, Image::Url(u) if u.starts_with('#') || u.contains(".svg#"))) {
                let mut layers = Vec::new();
                for i in (0..m.images.len()).rev() {
                    let paints = match m.images.get(i) {
                        Some(Some(img)) => self.tiles(b, m, i, img, true, None).into_iter().collect(),
                        _ => Vec::new(),
                    };
                    layers.push(MaskLayer { paints, luminance: effects::nth(&m.luminance, i, false), composite: effects::nth(&m.composite, i, 0) });
                }
                g.mask = Some(layers);
                any = true;
            }
        }
        if let Some(cp) = &s.clip_path {
            g.clip_path = Some(self.clip_shape(b, cp));
            any = true;
        }
        if any { Some(g) } else { None }
    }

    /// A clip-path's shape: its outline's polygons (page coordinates), and whether they're filled even-odd.
    fn clip_shape(&self, b: BoxId, cp: &ClipPath) -> (Vec<Vec<(f32, f32)>>, bool) {
        let (r, rad) = self.area(b, cp.area);
        let centre = |at: (Pos, Pos)| (r.x + at.0.resolve(r.w), r.y + at.1.resolve(r.h));
        let side = |rr: ShapeRadius, c: f32, lo: f32, hi: f32, base: f32| match rr {
            ShapeRadius::Len(l) => l.resolve(base),
            ShapeRadius::ClosestSide => (c - lo).abs().min((hi - c).abs()),
            ShapeRadius::FarthestSide => (c - lo).abs().max((hi - c).abs()),
        };
        match &cp.shape {
            Shape::Box => (vec![rounded_outline(r, rad)], false),
            Shape::Inset(ins, corners) => {
                let x0 = r.x + ins[3].resolve(r.w);
                let y0 = r.y + ins[0].resolve(r.h);
                let x1 = r.right() - ins[1].resolve(r.w);
                let y1 = r.bottom() - ins[2].resolve(r.h);
                if x1 <= x0 || y1 <= y0 {
                    return (Vec::new(), false);
                }
                let (w, h) = (x1 - x0, y1 - y0);
                let base = w.min(h);
                (vec![rounded_outline(Rect::new(x0, y0, w, h), corners.map(|c| c.resolve(base)))], false)
            }
            Shape::Circle(rr, at) => {
                let c = centre(*at);
                let rad = match rr {
                    ShapeRadius::Len(l) => l.resolve(num::sqrt(r.w * r.w + r.h * r.h) / core::f32::consts::SQRT_2),
                    ShapeRadius::ClosestSide => (c.0 - r.x).abs().min((r.right() - c.0).abs()).min((c.1 - r.y).abs()).min((r.bottom() - c.1).abs()),
                    ShapeRadius::FarthestSide => (c.0 - r.x).abs().max((r.right() - c.0).abs()).max((c.1 - r.y).abs()).max((r.bottom() - c.1).abs()),
                };
                (vec![ellipse_outline(c, rad, rad)], false)
            }
            Shape::Ellipse(rx, ry, at) => {
                let c = centre(*at);
                let a = side(*rx, c.0, r.x, r.right(), r.w);
                let bb = side(*ry, c.1, r.y, r.bottom(), r.h);
                (vec![ellipse_outline(c, a, bb)], false)
            }
            Shape::Polygon(even_odd, pts) => (vec![pts.iter().map(|(px, py)| (r.x + px.resolve(r.w), r.y + py.resolve(r.h))).collect()], *even_odd),
            Shape::Path(even_odd, d) => {
                let m = picture::draw::Matrix([self.dpr, 0.0, 0.0, self.dpr, r.x, r.y]);
                (picture::svg::path_polygons(d, m), *even_odd)
            }
        }
    }

    /// A box's own background, borders, picture or field.
    fn paint_box(&mut self, b: BoxId, clips: &Clips, items: &mut Vec<Item>) {
        let x = &self.tree.boxes[b];
        if x.is_inline_box() || matches!(x.content, Content::Text(_) | Content::Break) {
            return;
        }
        let clip = clips.rect;
        let s = x.style.clone();
        let node = x.node;
        let (bx, by, bw, bh) = (x.x, x.y, x.w, x.h);
        let radius = radii(&s, bw, bh);
        let border = x.border;
        let padding = x.padding;
        // (The root's background, and the body's when the canvas takes it, are the canvas's: drawn under all.)
        let canvas = b == 0 || self.canvas_from == Some(b);
        // Its shadows outside it (the first on top), its background, then its shadows inside it.
        let shadows = s.shadows.clone().filter(|_| b != 0);
        for sh in shadows.iter().flat_map(|v| v.iter().rev()).filter(|sh| !sh.inset && values::alpha(sh.color) > 0) {
            items.push(Item { paint: Paint::Shadow { x: bx, y: by, w: bw, h: bh, radius, dx: sh.x, dy: sh.y, blur: sh.blur, spread: sh.spread, color: sh.color, inset: false }, clip, node });
        }
        if !canvas {
            self.paint_background(b, clip, items);
        }
        if shadows.as_ref().is_some_and(|v| v.iter().any(|sh| sh.inset)) {
            let (p, prad) = self.area(b, BoxArea::Padding);
            for sh in shadows.iter().flat_map(|v| v.iter().rev()).filter(|sh| sh.inset && values::alpha(sh.color) > 0) {
                items.push(Item { paint: Paint::Shadow { x: p.x, y: p.y, w: p.w, h: p.h, radius: prad, dx: sh.x, dy: sh.y, blur: sh.blur, spread: sh.spread, color: sh.color, inset: true }, clip, node });
            }
        }
        let mut out: Vec<Paint> = Vec::new();
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
            c
        };
        let patterned = |k: usize| matches!(s.border_style[k], BorderStyle::Dashed | BorderStyle::Dotted | BorderStyle::Double);
        let uniform = sides.len() == 4 && border.iter().all(|&w| (w - border[0]).abs() < 0.01) && (0..4).all(|k| shade(k) == shade(0) && !patterned(k));
        // (Sides of one colour, solid, round a rounded box: one border, its corners rounded, whatever each side's
        // width, as YouTube's search box has it with no right side.)
        let one_colour = !sides.is_empty() && radius.iter().any(|&r| r > 0.0) && sides.iter().all(|&k| shade(k) == shade(sides[0]) && !patterned(k));
        if uniform || one_colour {
            let widths = core::array::from_fn(|k| if sides.contains(&k) { border[k] } else { 0.0 });
            out.push(Paint::Border { x: bx, y: by, w: bw, h: bh, widths, color: shade(sides[0]), radius });
        } else if !sides.is_empty() && sides.iter().all(|&k| !patterned(k)) {
            // Sides of different colours: each its own, meeting on the diagonals at the corners.
            let widths = core::array::from_fn(|k| if sides.contains(&k) { border[k] } else { 0.0 });
            out.push(Paint::SidedBorder { x: bx, y: by, w: bw, h: bh, widths, colors: core::array::from_fn(shade), radius });
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
                // object-fit and object-position: the picture's place in its box (cut to the box).
                let (iw, ih) = self.sizes.size(n).map(|(w, h)| (w as f32 * self.dpr, h as f32 * self.dpr)).unwrap_or((cw, ch));
                let (iw, ih) = if iw > 0.0 && ih > 0.0 { (iw, ih) } else { (cw, ch) };
                let (dw, dh) = match s.object_fit {
                    ObjectFit::Fill => (cw, ch),
                    ObjectFit::Contain => ((cw / iw).min(ch / ih) * iw, (cw / iw).min(ch / ih) * ih),
                    ObjectFit::Cover => ((cw / iw).max(ch / ih) * iw, (cw / iw).max(ch / ih) * ih),
                    ObjectFit::None => (iw, ih),
                    ObjectFit::ScaleDown => ((cw / iw).min(ch / ih).min(1.0) * iw, (cw / iw).min(ch / ih).min(1.0) * ih),
                };
                // (object-position places it in its box: its lengths move even one that fills it.)
                let at = Rect::new(cx + s.object_position.0.resolve(cw - dw), cy + s.object_position.1.resolve(ch - dh), dw, dh);
                let (_, rad) = self.area(b, BoxArea::Content);
                items.push(Item { paint: Paint::Picture { x: cx, y: cy, w: cw, h: ch, n, at, radius: rad, pixelated: s.pixelated }, clip, node });
            }
            Content::Image { alt, .. } if !alt.is_empty() && cw > 0.0 => {
                let (_, lh) = self.baseline(&s);
                items.push(Item { paint: Paint::Text { x: cx, y: cy, w: cw, text: alt, face: face_of(&s), size: s.font_size, line: lh, color: s.color }, clip: clip.intersect(&Rect::new(cx, cy, cw, ch.max(lh))), node });
            }
            Content::Image { .. } if cw > 0.0 && ch > 0.0 => {
                // (A picture that hasn't come, or can't be shown: its place, for clicks on a link round it.)
                items.push(Item { paint: Paint::Rect { x: cx, y: cy, w: cw, h: ch, color: 0, radius: [0.0; 4] }, clip, node });
            }
            Content::Control(c) => self.paint_control(b, c, cx, cy, cw, ch, clip, items),
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_control(&mut self, b: BoxId, c: Control, cx: f32, cy: f32, cw: f32, ch: f32, clip: Rect, items: &mut Vec<Item>) {
        let s = self.tree.boxes[b].style.clone();
        let node = self.tree.boxes[b].node;
        let (_, lh) = self.baseline(&s);
        let inside = clip.intersect(&Rect::new(cx, cy, cw, ch));
        let text_y = cy + (ch - lh) / 2.0;
        let words = |text: String, color: Color, items: &mut Vec<Item>, x: f32| {
            if !text.is_empty() {
                items.push(Item { paint: Paint::Text { x, y: text_y, w: cw, text, face: face_of(&s), size: s.font_size, line: lh, color: color }, clip: inside, node });
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
                items.push(Item { paint: Paint::Text { x: cx, y: cy, w: cw, text: shown.lines().next().unwrap_or("").into(), face: face_of(&s), size: s.font_size, line: lh, color: color }, clip: inside, node });
            }
            Control::Button(text) => {
                let w = self.measure(&text, face_of(&s), s.font_size);
                let x = cx + ((cw - w) / 2.0).max(0.0);
                words(text, s.color, items, x);
            }
            Control::Select(text, _) => {
                words(text, s.color, items, cx);
                let arrow_x = cx + cw - s.font_size * 0.9;
                items.push(Item { paint: Paint::Text { x: arrow_x, y: text_y, w: s.font_size, text: String::from("\u{25BE}"), face: face_of(&s), size: s.font_size, line: lh, color: s.color }, clip: inside, node });
            }
            Control::Check { checked, radio } => {
                let side = cw.min(ch);
                let r = if radio { side / 2.0 } else { 2.0 * self.dpr };
                items.push(Item { paint: Paint::Rect { x: cx, y: cy, w: side, h: side, color: 0xFF76_7676, radius: [r; 4] }, clip, node });
                let bw = 1.0 * self.dpr;
                items.push(Item { paint: Paint::Rect { x: cx + bw, y: cy + bw, w: side - 2.0 * bw, h: side - 2.0 * bw, color: if checked { 0xFF00_75FF } else { values::WHITE }, radius: [(r - bw).max(0.0); 4] }, clip, node });
                if checked {
                    let dot = side * 0.4;
                    items.push(Item { paint: Paint::Rect { x: cx + (side - dot) / 2.0, y: cy + (side - dot) / 2.0, w: dot, h: dot, color: values::WHITE, radius: [if radio { dot / 2.0 } else { 0.0 }; 4] }, clip, node });
                }
            }
        }
    }

    /// A piece of a line: words (with their underline or line through), a marker, an inline box's background.
    fn paint_frag(&mut self, f: &Frag, clip: Rect, items: &mut Vec<Item>, slice: (f32, f32)) {
        match &f.kind {
            FragKind::Words { text, owner } => {
                let s = self.tree.boxes[*owner].style.clone();
                if !s.visible || s.hidden_text {
                    return;
                }
                let node = self.tree.boxes[*owner].node;
                let color = s.color;
                items.push(Item { paint: Paint::Text { x: f.x, y: f.y, w: f.w, text: text.clone(), face: face_of(&s), size: s.font_size, line: f.h, color }, clip, node });
                let (bo, _) = self.baseline(&s);
                let thick = (s.font_size / 16.0).max(1.0);
                let dc = s.decoration_color.unwrap_or(s.color);
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
                items.push(Item { paint: Paint::Text { x: f.x, y: f.y, w: f.w, text: text.clone(), face: face_of(&s), size: s.font_size, line: f.h, color: s.color }, clip, node: self.tree.boxes[*owner].node });
            }
            FragKind::Back { owner, first, last } => {
                let s = self.tree.boxes[*owner].style.clone();
                let node = self.tree.boxes[*owner].node;
                let x = &self.tree.boxes[*owner];
                let border = x.border;
                if values::alpha(s.background) > 0 {
                    items.push(Item { paint: Paint::Rect { x: f.x, y: f.y, w: f.w, h: f.h, color: s.background, radius: radii(&s, f.w, f.h) }, clip, node });
                } else {
                    // (No background: a box for clicks on the link, drawn as nothing.)
                    items.push(Item { paint: Paint::Rect { x: f.x, y: f.y, w: f.w, h: f.h, color: 0, radius: [0.0; 4] }, clip, node });
                }
                if let Some(l) = s.bg.clone().filter(|l| l.any()) {
                    let piece = Rect::new(f.x, f.y, f.w, f.h);
                    let whole = Rect::new(f.x - slice.0, f.y, slice.1, f.h);
                    for i in (0..l.images.len()).rev() {
                        if let Some(Some(img)) = l.images.get(i)
                            && let Some(t) = self.tiles_in(*owner, &l, i, img, false, Some(whole), Some((piece, [0.0; 4])))
                        {
                            items.push(Item { paint: t, clip, node });
                        }
                    }
                }
                let c = |k: usize| s.border_color[k];
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
/// One grid item's placement as place_grid takes it: an area it's in already (major from, to, minor from, to), and
/// along each axis its tracks from its lines (Ok) or the span it takes, placed automatically (Err).
type GridSpec = (Option<[usize; 4]>, Result<(i64, i64), usize>, Result<(i64, i64), usize>);

/// A named grid area's rows and columns (from, to; to not included), as grid-template-areas draws it.
fn named_area(areas: &[Vec<String>], name: &str) -> Option<[usize; 4]> {
    let mut found: Option<[usize; 4]> = None;
    for (r, row) in areas.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if cell == name {
                found = Some(match found {
                    None => [r, r + 1, c, c + 1],
                    Some([r0, r1, c0, c1]) => [r0.min(r), r1.max(r + 1), c0.min(c), c1.max(c + 1)],
                });
            }
        }
    }
    found
}

/// The tracks grid-row or grid-column puts an item in (lines counted from 1, negative from the explicit grid's end),
/// or, with no line given, the span it's placed with automatically.
fn placement(p: GridPlace, n: usize) -> Result<(i64, i64), usize> {
    let line = |l: i32| -> i64 { if l > 0 { l as i64 - 1 } else { n as i64 + 1 + l as i64 } };
    let span = p.span.max(1) as i64;
    match (p.start, p.end) {
        (0, 0) => Err(span as usize),
        (a, 0) => {
            let a = line(a);
            Ok((a, a + span))
        }
        (0, e) => {
            let e = line(e);
            Ok((e - span, e))
        }
        (a, e) => {
            let (a, e) = (line(a), line(e));
            Ok(if e > a { (a, e) } else if e < a { (e, a) } else { (a, a + 1) })
        }
    }
}

/// Grid items placed, as CSS's auto-placement ("sparse") places them: those with an area or both lines where they
/// say, those locked to a major track (a row, when the flow is by row) at the first place along it they fit, and the
/// rest on from a cursor, along the minor axis (`n_minor` tracks: wider when an item's span is) and on to the next.
fn place_grid(items: &[GridSpec], n_minor: usize) -> Vec<[usize; 4]> {
    let mut out = vec![[0usize; 4]; items.len()];
    let mut done = vec![false; items.len()];
    let mut taken: Vec<Vec<bool>> = Vec::new();
    let fits = |taken: &Vec<Vec<bool>>, m0: usize, m1: usize, n0: usize, n1: usize| -> bool {
        (m0..m1).all(|m| taken.get(m).is_none_or(|row| (n0..n1).all(|n| !row.get(n).copied().unwrap_or(false))))
    };
    let occupy = |taken: &mut Vec<Vec<bool>>, p: [usize; 4]| {
        while taken.len() < p[1] {
            taken.push(Vec::new());
        }
        for row in &mut taken[p[0]..p[1]] {
            if row.len() < p[3] {
                row.resize(p[3], false);
            }
            for cell in &mut row[p[2]..p[3]] {
                *cell = true;
            }
        }
    };
    let range = |(a, b): (i64, i64)| -> (usize, usize) {
        let a0 = a.clamp(0, 10_000) as usize;
        (a0, (b.clamp(a0 as i64 + 1, 10_001)) as usize)
    };
    let mut minor_n = n_minor.max(1);
    for (_, _, minor) in items {
        minor_n = minor_n.max(match minor {
            Err(span) => (*span).min(10_000),
            Ok(r) => range(*r).1,
        });
    }
    for (i, (area, major, minor)) in items.iter().enumerate() {
        let p = match (area, major, minor) {
            (Some(a), _, _) => Some(*a),
            (None, Ok(ma), Ok(mi)) => {
                let ((m0, m1), (n0, n1)) = (range(*ma), range(*mi));
                Some([m0, m1, n0, n1])
            }
            _ => None,
        };
        if let Some(p) = p {
            out[i] = p;
            done[i] = true;
            occupy(&mut taken, p);
        }
    }
    for (i, (_, major, minor)) in items.iter().enumerate() {
        if let (false, Ok(ma), Err(span)) = (done[i], major, minor) {
            let (m0, m1) = range(*ma);
            let mut n0 = 0;
            while !fits(&taken, m0, m1, n0, n0 + span) && n0 < 10_000 {
                n0 += 1;
            }
            out[i] = [m0, m1, n0, n0 + span];
            done[i] = true;
            occupy(&mut taken, out[i]);
        }
    }
    let (mut cm, mut cn) = (0usize, 0usize);
    for (i, (_, major, minor)) in items.iter().enumerate() {
        if done[i] {
            continue;
        }
        let mspan = match major {
            Err(span) => (*span).min(10_000),
            Ok(r) => {
                let (a, b) = range(*r);
                b - a
            }
        };
        match minor {
            Ok(mi) => {
                let (n0, n1) = range(*mi);
                if n0 < cn {
                    cm += 1;
                }
                cn = n0;
                while !fits(&taken, cm, cm + mspan, n0, n1) && cm < 100_000 {
                    cm += 1;
                }
                out[i] = [cm, cm + mspan, n0, n1];
            }
            Err(span) => {
                let span = (*span).clamp(1, minor_n);
                loop {
                    if cn + span > minor_n {
                        cm += 1;
                        cn = 0;
                        continue;
                    }
                    if fits(&taken, cm, cm + mspan, cn, cn + span) || cm > 100_000 {
                        break;
                    }
                    cn += 1;
                }
                out[i] = [cm, cm + mspan, cn, cn + span];
                cn += span;
            }
        }
        occupy(&mut taken, out[i]);
    }
    out
}

/// A track sizing function (CSS Grid's): a length, min-content, max-content, auto, or a flexible fraction.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Sizing {
    Fixed(f32),
    MinContent,
    MaxContent,
    Auto,
    Flex(f32),
}

/// A grid track as it's sized: its min and max sizing functions, its base size and its growth limit.
struct Tr {
    min: Sizing,
    max: Sizing,
    base: f32,
    limit: f32,
}

impl Tr {
    /// A track from its definition (an implicit one's: auto), percentages of `room` when it's known.
    fn of(t: Option<&Track>, room: Option<f32>) -> Tr {
        let one = |t: &Track, as_min: bool| -> Sizing {
            match t {
                Track::Len(l) if l.has_pct() => room.map_or(Sizing::Auto, |r| Sizing::Fixed(l.resolve(r))),
                Track::Len(l) => Sizing::Fixed(l.resolve(0.0)),
                Track::Fr(f) if !as_min => Sizing::Flex(*f),
                Track::MinContent => Sizing::MinContent,
                Track::MaxContent => Sizing::MaxContent,
                _ => Sizing::Auto,
            }
        };
        let (min, max) = match t {
            None => (Sizing::Auto, Sizing::Auto),
            Some(Track::MinMax(mm)) => (one(&mm.0, true), one(&mm.1, false)),
            Some(t) => (one(t, true), one(t, false)),
        };
        Tr { min, max, base: 0.0, limit: 0.0 }
    }

    fn flex(&self) -> Option<f32> {
        if let Sizing::Flex(f) = self.max { Some(f) } else { None }
    }
}

/// A grid item's contributions along an axis: the tracks it spans (from, to), and its min-content, max-content and
/// minimum contributions (margin box).
struct Contrib {
    from: usize,
    to: usize,
    min_c: f32,
    max_c: f32,
    minimum: f32,
}

/// CSS Grid's track sizing algorithm, a little simplified: base sizes and growth limits from the tracks' sizing
/// functions and the items in them (one track first, then those spanning several, then those spanning flexible
/// ones), the free space given to the tracks up to their limits, flexible tracks their share of what's left by their
/// factors, and auto tracks stretched over the rest. `room`: the axis's size, if it's known.
fn size_tracks(tr: &mut [Tr], items: &[Contrib], room: Option<f32>, gap: f32, stretch: bool) {
    let n = tr.len();
    if n == 0 {
        return;
    }
    let intrinsic = |s: Sizing| matches!(s, Sizing::MinContent | Sizing::MaxContent | Sizing::Auto);
    for t in tr.iter_mut() {
        t.base = if let Sizing::Fixed(v) = t.min { v } else { 0.0 };
        t.limit = if let Sizing::Fixed(v) = t.max { v } else { f32::INFINITY };
        if t.limit < t.base {
            t.limit = t.base;
        }
    }
    // Items in one track (not a flexible one).
    for it in items.iter().filter(|it| it.to == it.from + 1 && it.from < n) {
        let t = &mut tr[it.from];
        if t.flex().is_some() {
            continue;
        }
        match t.min {
            Sizing::MinContent => t.base = t.base.max(it.min_c),
            Sizing::MaxContent => t.base = t.base.max(it.max_c),
            Sizing::Auto => t.base = t.base.max(it.minimum),
            _ => {}
        }
        let grow = match t.max {
            Sizing::MinContent => Some(it.min_c),
            Sizing::MaxContent | Sizing::Auto => Some(it.max_c),
            _ => None,
        };
        if let Some(g) = grow {
            t.limit = if t.limit.is_infinite() { g } else { t.limit.max(g) };
        }
        if t.limit < t.base {
            t.limit = t.base;
        }
    }
    // Items spanning several tracks, none flexible (the fewest first): what they need beyond those tracks' sizes,
    // shared among the ones sized by their content.
    let mut spanning: Vec<&Contrib> = items.iter().filter(|it| it.to > it.from + 1 && it.to <= n && !(it.from..it.to).any(|c| tr[c].flex().is_some())).collect();
    spanning.sort_by_key(|it| it.to - it.from);
    for it in spanning {
        let gaps = gap * (it.to - it.from - 1) as f32;
        let want = if (it.from..it.to).any(|c| tr[c].min == Sizing::MaxContent) { it.max_c } else { it.minimum };
        let have: f32 = (it.from..it.to).map(|c| tr[c].base).sum::<f32>() + gaps;
        let eligible: Vec<usize> = (it.from..it.to).filter(|&c| intrinsic(tr[c].min)).collect();
        if want > have && !eligible.is_empty() {
            let share = (want - have) / eligible.len() as f32;
            for c in eligible {
                tr[c].base += share;
                if tr[c].limit < tr[c].base {
                    tr[c].limit = tr[c].base;
                }
            }
        }
        let have: f32 = (it.from..it.to).map(|c| if tr[c].limit.is_infinite() { tr[c].base } else { tr[c].limit }).sum::<f32>() + gaps;
        let eligible: Vec<usize> = (it.from..it.to).filter(|&c| intrinsic(tr[c].max)).collect();
        if it.max_c > have && !eligible.is_empty() {
            let share = (it.max_c - have) / eligible.len() as f32;
            for c in eligible {
                let now = if tr[c].limit.is_infinite() { tr[c].base } else { tr[c].limit };
                tr[c].limit = now + share;
            }
        }
    }
    // Items spanning flexible tracks: their minimum shared among those flexible tracks sized by their content at least.
    let over_flex: Vec<&Contrib> = items.iter().filter(|it| it.to <= n && (it.from..it.to).any(|c| tr[c].flex().is_some())).collect();
    for it in over_flex {
        let gaps = gap * (it.to - it.from).saturating_sub(1) as f32;
        let have: f32 = (it.from..it.to).map(|c| tr[c].base).sum::<f32>() + gaps;
        let eligible: Vec<usize> = (it.from..it.to).filter(|&c| tr[c].flex().is_some() && intrinsic(tr[c].min)).collect();
        if it.minimum > have && !eligible.is_empty() {
            let share = (it.minimum - have) / eligible.len() as f32;
            for c in eligible {
                tr[c].base += share;
            }
        }
    }
    for t in tr.iter_mut() {
        if t.limit.is_infinite() || t.limit < t.base {
            t.limit = t.base;
        }
    }
    let gaps = gap * (n - 1) as f32;
    // The free space, to each track up to its limit (with no room known: each at its limit).
    match room {
        Some(r) => {
            let mut free = r - gaps - tr.iter().map(|t| t.base).sum::<f32>();
            for _ in 0..16 {
                let open: Vec<usize> = (0..n).filter(|&c| tr[c].base < tr[c].limit - 0.001).collect();
                if free <= 0.01 || open.is_empty() {
                    break;
                }
                let share = free / open.len() as f32;
                for c in open {
                    let add = share.min(tr[c].limit - tr[c].base);
                    tr[c].base += add;
                    free -= add;
                }
            }
        }
        None => {
            for t in tr.iter_mut() {
                t.base = t.limit;
            }
        }
    }
    // Flexible tracks: an fr's size, from the room left (or, with none known, from what their items need).
    if tr.iter().any(|t| t.flex().is_some()) {
        let fr = match room {
            Some(r) => {
                let mut fixed_flex = vec![false; n];
                let mut fr = 0.0;
                for _ in 0..8 {
                    let used: f32 = (0..n).filter(|&c| tr[c].flex().is_none() || fixed_flex[c]).map(|c| tr[c].base).sum::<f32>() + gaps;
                    let sum: f32 = (0..n).filter(|&c| !fixed_flex[c]).filter_map(|c| tr[c].flex()).sum();
                    fr = (r - used).max(0.0) / sum.max(1.0);
                    let mut changed = false;
                    for c in 0..n {
                        if let Some(f) = tr[c].flex()
                            && !fixed_flex[c]
                            && tr[c].base > f * fr
                        {
                            fixed_flex[c] = true;
                            changed = true;
                        }
                    }
                    if !changed {
                        break;
                    }
                }
                fr
            }
            None => {
                let mut fr = 0.0f32;
                for t in tr.iter() {
                    if let Some(f) = t.flex() {
                        fr = fr.max(if f > 1.0 { t.base / f } else { t.base });
                    }
                }
                for it in items.iter().filter(|it| it.to <= n && (it.from..it.to).any(|c| tr[c].flex().is_some())) {
                    let fixed: f32 = (it.from..it.to).filter(|&c| tr[c].flex().is_none()).map(|c| tr[c].base).sum::<f32>() + gap * (it.to - it.from).saturating_sub(1) as f32;
                    let sum: f32 = (it.from..it.to).filter_map(|c| tr[c].flex()).sum();
                    fr = fr.max((it.max_c - fixed).max(0.0) / sum.max(1.0));
                }
                fr
            }
        };
        for t in tr.iter_mut() {
            if let Some(f) = t.flex() {
                t.base = t.base.max(f * fr);
            }
        }
    }
    // Auto tracks stretched over what's left.
    if stretch && let Some(r) = room {
        let free = r - gaps - tr.iter().map(|t| t.base).sum::<f32>();
        let autos: Vec<usize> = (0..n).filter(|&c| tr[c].max == Sizing::Auto).collect();
        if free > 0.0 && !autos.is_empty() {
            let share = free / autos.len() as f32;
            for c in autos {
                tr[c].base += share;
            }
        }
    }
}

/// Where each track starts, with what's left of `room` placed as justify-content (or align-content) says.
fn track_starts(sizes: &[f32], gap: f32, room: f32, how: Align) -> Vec<f32> {
    let n = sizes.len();
    let used = sizes.iter().sum::<f32>() + gap * n.saturating_sub(1) as f32;
    let free = (room - used).max(0.0);
    let (start, between) = match how {
        Align::Center => (free / 2.0, 0.0),
        Align::End => (free, 0.0),
        Align::SpaceBetween if n > 1 => (0.0, free / (n - 1) as f32),
        Align::SpaceAround if n > 0 => (free / n as f32 / 2.0, free / n as f32),
        Align::SpaceEvenly => (free / (n + 1) as f32, free / (n + 1) as f32),
        _ => (0.0, 0.0),
    };
    let mut out = Vec::with_capacity(n);
    let mut at = start;
    for &size in sizes {
        out.push(at);
        at += size + gap + between;
    }
    out
}

/// A track list with its repeat(auto-fill, ...) repeated as many times as fit in `room` (once, when that's unknown).
fn repeat_tracks(list: &[Track], room: Option<f32>, gap: f32) -> Vec<Track> {
    let mut out = Vec::new();
    for t in list {
        match t {
            Track::AutoRepeat(inner) => {
                let times = match room {
                    Some(w) => {
                        let one: f32 = inner.iter().map(|t| track_least(t, w)).sum::<f32>().max(1.0);
                        let per = inner.len().max(1) as f32;
                        (((w + gap) / (one + gap * per)).max(1.0)) as usize
                    }
                    None => 1,
                };
                for _ in 0..times.min(64) {
                    out.extend(inner.iter().cloned());
                }
            }
            t => out.push(t.clone()),
        }
    }
    out
}

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
