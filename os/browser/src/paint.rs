//! A laid-out page drawn into pixels (0xRRGGBB, row after row): its rectangles (rounded, see-through), borders,
//! shadows, backgrounds' pictures and gradients (linear, radial and conic, tiled as CSS places them), pictures and
//! words (the text engine's ink, kept between drawings, so a page scrolls without its words being drawn again). And
//! its groups, as a browser's compositor puts its layers together: what's in one is drawn into a layer of its own
//! (premultiplied, see-through), then filtered, cut to its clip and clip-path, masked, and put on what's under it
//! made see-through, turned (its transform, the layer drawn at the scale it's shown and sampled back) and blended. A
//! group that only clips to a rounded rectangle takes no layer: its coverage is a mask what's in it is drawn through.

use crate::effects::{self, Filter, Gradient, GradientKind, RadialSize};
use crate::layout::{Item, Laid, MaskLayer, Paint, Rect, Tile};
use crate::num;
use crate::values::Color;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use text::{Face, Ink, Type};

/// Where a page is drawn: pixels `width` to a row, and the part of them drawing may touch.
pub struct Surface<'a> {
    pub pixels: &'a mut [u32],
    pub width: usize,
    pub height: usize,
    /// (x, y, w, h), within the pixels.
    pub clip: (i32, i32, i32, i32),
}

/// The page's pictures, decoded: the `n`th's pixels (RGB, row after row), width and height, once it has come; and
/// its alpha (a byte a pixel), if it has transparent parts (its colours then are as they are, not mixed with a
/// background).
pub trait Shown {
    fn picture(&self, n: usize) -> Option<(&[u8], u32, u32)>;
    fn alpha(&self, _n: usize) -> Option<&[u8]> {
        None
    }
    /// Which version of the picture it is (a canvas's changes as it's drawn on): a new one is made its drawn size
    /// again.
    fn version(&self, _n: usize) -> u64 {
        0
    }
}

/// No pictures (yet).
pub struct NoPictures;

impl Shown for NoPictures {
    fn picture(&self, _n: usize) -> Option<(&[u8], u32, u32)> {
        None
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct InkKey {
    text: String,
    face: u16,
    size: i32,
    line: i32,
}

/// Draws pages, keeping their words' ink, and their pictures made the size they're drawn.
pub struct Painter {
    /// How far the page being drawn is scrolled (device pixels): what's fixed to the window is moved back by it.
    scroll: f32,
    inks: BTreeMap<InkKey, (Ink, u64)>,
    /// How many drawings there have been (to let go of ink unused for a while).
    round: u64,
    bytes: usize,
    /// Pictures at the size they're drawn (0xAARRGGBB, alpha straight), by (picture, width, height), and when each
    /// was last used, and its version.
    scaled: BTreeMap<(usize, u32, u32, bool), (Vec<u32>, u64, u64)>,
    scaled_bytes: usize,
}

/// The most of words' ink kept, and of pictures made their drawn size.
const MOST_INK_BYTES: usize = 48 << 20;
const MOST_SCALED_BYTES: usize = 64 << 20;
/// The most pixels a group's layer has (past them, it's drawn at a smaller scale).
const MOST_LAYER_PIXELS: f32 = 16.0 * 1024.0 * 1024.0;

impl Default for Painter {
    fn default() -> Self {
        Painter::new()
    }
}

/// Page coordinates to a target's pixels: x k + ox, y k + oy.
#[derive(Clone, Copy, Debug)]
struct Map {
    k: f32,
    ox: f32,
    oy: f32,
}

impl Map {
    fn x(&self, x: f32) -> f32 {
        x * self.k + self.ox
    }
    fn y(&self, y: f32) -> f32 {
        y * self.k + self.oy
    }
    fn len(&self, l: f32) -> f32 {
        l * self.k
    }
    fn matrix(&self) -> [f32; 6] {
        [self.k, 0.0, 0.0, self.k, self.ox, self.oy]
    }
    fn area(&self, r: &Rect) -> Area {
        Area::from_f(self.x(r.x), self.y(r.y), self.len(r.w), self.len(r.h))
    }
}

/// Rounded clips' coverage over an area of a target's pixels (a byte a pixel; none outside it).
struct Mask {
    area: Area,
    cover: Vec<u8>,
}

impl Mask {
    #[inline]
    fn at(&self, x: i32, y: i32) -> u32 {
        if x < self.area.x || y < self.area.y || x >= self.area.r || y >= self.area.b {
            return 0;
        }
        self.cover[(y - self.area.y) as usize * (self.area.r - self.area.x) as usize + (x - self.area.x) as usize] as u32
    }
}

/// What drawing goes through: page coordinates to pixels, the part of the pixels it may touch, the mask of the
/// rounded clips open.
#[derive(Clone)]
struct State {
    map: Map,
    clip: Area,
    mask: Option<Rc<Mask>>,
}

/// A group's layer: its pixels (premultiplied), where it goes on what's under it (its top left there, or for a turned
/// group the matrix from its pixels to those under it), its group (the Begin's item), and how drawing went under it.
struct Layer {
    px: Vec<u32>,
    w: usize,
    h: usize,
    at: (i32, i32),
    to_under: Option<[f32; 6]>,
    map: Map,
    begin: usize,
    under: State,
    /// What of what's under it it may change (its Begin's clip there).
    clip_under: Area,
}

enum Frame {
    /// A rounded clip (no layer): the mask and clip it put aside.
    Clip { mask: Option<Rc<Mask>>, clip: Area },
    Layer(Layer),
    /// A group moved as the page scrolls (fixed, sticky): the map it put aside.
    Shift { map: Map },
}

/// Pixels being drawn on: the page's (opaque, 0xRRGGBB) or a layer's (premultiplied 0xAARRGGBB), with rounded clips'
/// mask, if any.
struct Canvas<'a> {
    px: &'a mut [u32],
    w: usize,
    layer: bool,
    mask: Option<&'a Mask>,
}

impl Canvas<'_> {
    /// A colour (0xAARRGGBB, alpha straight) put on at `cover` of 255.
    #[inline]
    fn put(&mut self, x: i32, y: i32, color: Color, cover: u32) {
        let cover = match self.mask {
            Some(m) => (cover * m.at(x, y) + 127) / 255,
            None => cover,
        };
        if cover == 0 {
            return;
        }
        let i = y as usize * self.w + x as usize;
        self.px[i] = if self.layer { over_pm(self.px[i], color, cover) } else { over(self.px[i], color, cover) };
    }

    /// A premultiplied colour put on, blended by `blend` (255: normal).
    #[inline]
    fn put_pm(&mut self, x: i32, y: i32, src: u32, blend: u8) {
        let src = match self.mask {
            Some(m) => scale_pm(src, m.at(x, y)),
            None => src,
        };
        if src >> 24 == 0 {
            return;
        }
        let i = y as usize * self.w + x as usize;
        self.px[i] = composite_pm(self.px[i], src, blend, self.layer);
    }
}

/// The canvas drawing goes into now: the top layer's, else the page's.
fn target<'a>(frames: &'a mut [Frame], surface: &'a mut Surface, mask: Option<&'a Mask>) -> Canvas<'a> {
    for f in frames.iter_mut().rev() {
        if let Frame::Layer(l) = f {
            return Canvas { px: &mut l.px, w: l.w, layer: true, mask };
        }
    }
    Canvas { px: surface.pixels, w: surface.width, layer: false, mask }
}

impl Painter {
    pub fn new() -> Painter {
        Painter { scroll: 0.0, inks: BTreeMap::new(), round: 0, bytes: 0, scaled: BTreeMap::new(), scaled_bytes: 0 }
    }

    /// Draws `laid` with the page's point (0, `scroll`) at (`x`, `y`) of the surface: its background first (when
    /// `background`), then each item, in order, within its clip and the surface's.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, ty: &Type, laid: &Laid, x: i32, y: i32, scroll: f32, surface: &mut Surface, pictures: &dyn Shown, background: bool) {
        self.round += 1;
        let scroll = num::round(scroll) as i32;
        self.scroll = scroll as f32;
        let (cx, cy, cw, ch) = surface.clip;
        let clip = Area::new(cx, cy, cw, ch).within(surface.width as i32, surface.height as i32);
        if clip.empty() {
            return;
        }
        if background {
            let mut c = Canvas { px: surface.pixels, w: surface.width, layer: false, mask: None };
            fill(&mut c, clip, 0xFF00_0000 | (laid.background & 0x00FF_FFFF), 255);
        }
        let items = &laid.items;
        let (ends, bounds) = extents(items);
        let mut frames: Vec<Frame> = Vec::new();
        let mut st = State { map: Map { k: 1.0, ox: x as f32, oy: (y - scroll) as f32 }, clip, mask: None };
        let mut i = 0;
        while i < items.len() {
            let item = &items[i];
            match &item.paint {
                Paint::Begin(_) => {
                    if !self.begin(items, i, bounds[i], &mut st, &mut frames, surface) {
                        i = ends[i].max(i);
                    }
                }
                Paint::End => self.end(ty, items, &mut st, &mut frames, surface, pictures),
                paint => {
                    let ic = item_clip(item, &st);
                    if !ic.empty() && extent(paint).is_some_and(|r| !st.map.area(&r).intersect(&ic).empty()) {
                        let mut canvas = target(&mut frames, surface, st.mask.as_deref());
                        self.draw_item(ty, paint, ic, st.map, &mut canvas, pictures);
                    }
                }
            }
            i += 1;
        }
        // (Groups left open, as a list cut short leaves them: closed.)
        while !frames.is_empty() {
            self.end(ty, items, &mut st, &mut frames, surface, pictures);
        }
        // (Pictures not drawn for a while, let go when there are a lot.)
        if self.scaled_bytes > MOST_SCALED_BYTES {
            let round = self.round;
            let mut freed = 0;
            self.scaled.retain(|_, (px, used, _)| {
                let keep = round - *used < 2;
                if !keep {
                    freed += px.len() * 4;
                }
                keep
            });
            self.scaled_bytes -= freed;
        }
        // (Ink not used for many drawings, let go when there's a lot.)
        if self.bytes > MOST_INK_BYTES {
            let round = self.round;
            let mut freed = 0;
            self.inks.retain(|_, (ink, used)| {
                let keep = round - *used < 4;
                if !keep {
                    freed += ink.coverage.len();
                }
                keep
            });
            self.bytes -= freed;
        }
    }

    /// One item drawn on a canvas, through `m`, within `clip`.
    fn draw_item(&mut self, ty: &Type, paint: &Paint, clip: Area, m: Map, c: &mut Canvas, pictures: &dyn Shown) {
        match paint {
            Paint::Rect { x, y, w, h, color, radius } => {
                if alpha(*color) == 0 {
                    return;
                }
                let r = Shape::new(m.x(*x), m.y(*y), m.len(*w), m.len(*h), radius.map(|v| m.len(v)));
                shape(c, &r, None, clip, *color);
            }
            Paint::Border { x, y, w, h, widths, color, radius } => {
                let [t, r, b, l] = widths.map(|v| m.len(v));
                let rad = radius.map(|v| m.len(v));
                let outer = Shape::new(m.x(*x), m.y(*y), m.len(*w), m.len(*h), rad);
                // (Each inner corner rounded less by the wider of the two sides it's between.)
                let inner_radii = [(rad[0] - t.max(l)).max(0.0), (rad[1] - t.max(r)).max(0.0), (rad[2] - b.max(r)).max(0.0), (rad[3] - b.max(l)).max(0.0)];
                let inner = Shape::new(outer.x + l, outer.y + t, outer.w - l - r, outer.h - t - b, inner_radii);
                shape(c, &outer, Some(&inner), clip, *color);
            }
            Paint::SidedBorder { x, y, w, h, widths, colors, radius } => {
                let [t, r, b, l] = widths.map(|v| m.len(v));
                let rad = radius.map(|v| m.len(v));
                let outer = Shape::new(m.x(*x), m.y(*y), m.len(*w), m.len(*h), rad);
                let inner_radii = [(rad[0] - t.max(l)).max(0.0), (rad[1] - t.max(r)).max(0.0), (rad[2] - b.max(r)).max(0.0), (rad[3] - b.max(l)).max(0.0)];
                let inner = Shape::new(outer.x + l, outer.y + t, outer.w - l - r, outer.h - t - b, inner_radii);
                sided_border(c, &outer, &inner, [t, r, b, l], *colors, clip);
            }
            Paint::Shadow { x, y, w, h, radius, dx, dy, blur, spread, color, inset } => {
                let rad = radius.map(|v| m.len(v));
                let spread = m.len(*spread);
                let own = Shape::new(m.x(*x), m.y(*y), m.len(*w), m.len(*h), rad);
                if *inset {
                    // (The hole the shadow's cast round: the padding box moved, shrunk by the spread.)
                    let shrink = |r: f32| (r - spread).max(0.0);
                    let hole = Shape::new(own.x + m.len(*dx) + spread, own.y + m.len(*dy) + spread, own.w - 2.0 * spread, own.h - 2.0 * spread, rad.map(shrink));
                    inset_shadow(c, &hole, &own, m.len(*blur), *color, clip);
                } else {
                    let grow = |r: f32| if r > 0.0 || spread < 0.0 { (r + spread).max(0.0) } else { 0.0 };
                    let cast = Shape::new(own.x + m.len(*dx) - spread, own.y + m.len(*dy) - spread, own.w + 2.0 * spread, own.h + 2.0 * spread, rad.map(grow));
                    shadow(c, &cast, &own, m.len(*blur), *color, clip);
                }
            }
            Paint::Picture { x, y, w, h, n, at, radius, pixelated } => {
                let bx = Shape::new(m.x(*x), m.y(*y), m.len(*w), m.len(*h), radius.map(|v| m.len(v)));
                let (ax, ay) = (num::round(m.x(at.x)) as i32, num::round(m.y(at.y)) as i32);
                let (tw, th) = (num::round(m.len(at.w)).max(1.0) as u32, num::round(m.len(at.h)).max(1.0) as u32);
                if let Some(px) = self.scaled_picture(*n, tw, th, *pixelated, pictures) {
                    picture(c, &bx, px, ax, ay, tw, th, clip);
                }
            }
            Paint::Tiles { x, y, w, h, radius, tile, step, repeat, image, pixelated } => {
                let area = Shape::new(m.x(*x), m.y(*y), m.len(*w), m.len(*h), radius.map(|v| m.len(v)));
                let t = Rect::new(m.x(tile.x), m.y(tile.y), m.len(tile.w), m.len(tile.h));
                let st = (m.len(step.0), m.len(step.1));
                match image {
                    Tile::Picture(n) => {
                        // (The picture made the tile's size in whole pixels; each pixel of the tiles takes the one
                        // its place in its tile falls on.)
                        let (tw, th) = (num::round(t.w).max(1.0) as u32, num::round(t.h).max(1.0) as u32);
                        if let Some(px) = self.scaled_picture(*n, tw, th, *pixelated, pictures) {
                            picture_tiles(c, &area, px, t, (tw, th), st, *repeat, clip);
                        }
                    }
                    Tile::Gradient(g) => {
                        let s = Sampler::new(g, t.w, t.h, m.k);
                        gradient_tiles(c, &area, &s, t, st, *repeat, clip);
                    }
                }
            }
            Paint::Text { x, y, text, face, size, line, color, .. } => {
                let ink = self.ink(ty, text, *face, m.len(*size), m.len(*line));
                let (tx, ty_) = (num::round(m.x(*x)) as i32 - ink.left as i32, num::round(m.y(*y)) as i32);
                words(c, ink, tx, ty_, *color, clip);
            }
            Paint::Begin(_) | Paint::End => {}
        }
    }

    /// A picture made `tw` × `th` (kept while it's drawn at that size).
    fn scaled_picture(&mut self, n: usize, tw: u32, th: u32, pixelated: bool, pictures: &dyn Shown) -> Option<&Vec<u32>> {
        let (rgb, pw, ph) = pictures.picture(n)?;
        let key = (n, tw, th, pixelated);
        let round = self.round;
        let version = pictures.version(n);
        if self.scaled.get(&key).is_some_and(|(_, _, v)| *v != version)
            && let Some((old, _, _)) = self.scaled.remove(&key)
        {
            self.scaled_bytes -= old.len() * 4;
        }
        if !self.scaled.contains_key(&key) && tw as u64 * th as u64 <= 16 << 20 {
            let px = if pixelated { nearest(rgb, pictures.alpha(n), pw, ph, tw, th) } else { scaled(rgb, pictures.alpha(n), pw, ph, tw, th) };
            self.scaled_bytes += px.len() * 4;
            self.scaled.insert(key, (px, round, version));
        }
        let (px, used, _) = self.scaled.get_mut(&key)?;
        *used = round;
        Some(px)
    }

    fn ink(&mut self, ty: &Type, text: &str, face: Face, size: f32, line: f32) -> &Ink {
        let key = InkKey { text: String::from(text), face: face.key(), size: (size * 256.0) as i32, line: (line * 256.0) as i32 };
        let round = self.round;
        if !self.inks.contains_key(&key) {
            let ink = ty.ink_in(text, face, text::Style { size: key.size, line_height: key.line }, 0);
            self.bytes += ink.coverage.len();
            self.inks.insert(key.clone(), (ink, round));
        }
        let entry = self.inks.get_mut(&key).expect("put there above");
        entry.1 = round;
        &entry.0
    }

    /// A group opens: a rounded clip's mask, or a layer for what's in it. False if nothing of it can show (what's
    /// in it is then passed over).
    #[allow(clippy::too_many_arguments)]
    fn begin(&mut self, items: &[Item], i: usize, bounds: Option<Rect>, st: &mut State, frames: &mut Vec<Frame>, surface: &mut Surface) -> bool {
        let item = &items[i];
        let Paint::Begin(g) = &item.paint else { return false };
        if let Some(sc) = &g.scroll {
            // (Moved as the page scrolls: what's in it drawn that much further down.)
            frames.push(Frame::Shift { map: st.map });
            st.map.oy += sc.shift(self.scroll / st.map.k.max(1e-6)) * st.map.k;
            return true;
        }
        let outer = item_clip(item, st);
        if outer.empty() || g.hidden {
            return false;
        }
        if g.only_clips() {
            let Some((r, rad)) = g.clip else {
                frames.push(Frame::Clip { mask: st.mask.clone(), clip: st.clip });
                return true;
            };
            let shape = Shape::new(st.map.x(r.x), st.map.y(r.y), st.map.len(r.w), st.map.len(r.h), rad.map(|v| st.map.len(v)));
            let area = shape.area().intersect(&outer);
            if area.empty() {
                return false;
            }
            let w = (area.r - area.x) as usize;
            let mut cover = vec![0u8; w * (area.b - area.y) as usize];
            for y in area.y..area.b {
                for x in area.x..area.r {
                    let mut c = shape.cover(x, y);
                    if let Some(m) = &st.mask {
                        c = (c * m.at(x, y) + 127) / 255;
                    }
                    cover[(y - area.y) as usize * w + (x - area.x) as usize] = c as u8;
                }
            }
            frames.push(Frame::Clip { mask: st.mask.take(), clip: st.clip });
            st.mask = Some(Rc::new(Mask { area, cover }));
            st.clip = area;
            return true;
        }
        if g.opacity <= 0.0 {
            return false;
        }
        let Some(b) = bounds else { return false };
        // (Blurs reach past what's in it, and take from what's just outside what shows.)
        let reach = filter_reach(&g.filters);
        let b = Rect::new(b.x - reach, b.y - reach, b.w + 2.0 * reach, b.h + 2.0 * reach);
        if let Some((filters, r, rad)) = &g.backdrop {
            let height = frames_height(frames, surface);
            let mut canvas = target(frames, surface, st.mask.as_deref());
            backdrop(&mut canvas, filters, st.map, *r, *rad, outer, height);
        }
        match g.transform {
            None => {
                let grown = num::ceil(st.map.len(reach)) as i32;
                let area = st.map.area(&b).intersect(&Area { x: outer.x - grown, y: outer.y - grown, r: outer.r + grown, b: outer.b + grown });
                if area.empty() {
                    return false;
                }
                let (w, h) = ((area.r - area.x) as usize, (area.b - area.y) as usize);
                let map = Map { k: st.map.k, ox: st.map.ox - area.x as f32, oy: st.map.oy - area.y as f32 };
                frames.push(Frame::Layer(Layer { px: vec![0; w * h], w, h, at: (area.x, area.y), to_under: None, map, begin: i, under: st.clone(), clip_under: outer }));
                *st = State { map, clip: Area::new(0, 0, w as i32, h as i32), mask: None };
            }
            Some(t) => {
                let d = effects::mul(st.map.matrix(), t);
                let Some(dinv) = effects::invert(d) else { return false };
                // The part of it that can show: what it's drawn within under it, brought back into its own
                // coordinates.
                let corners = [(outer.x, outer.y), (outer.r, outer.y), (outer.r, outer.b), (outer.x, outer.b)].map(|(x, y)| effects::apply(dinv, x as f32, y as f32));
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for (x, y) in corners {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
                let local = b.intersect(&Rect::new(x0 - reach, y0 - reach, x1 - x0 + 2.0 * reach, y1 - y0 + 2.0 * reach));
                if local.w <= 0.0 || local.h <= 0.0 {
                    return false;
                }
                // Drawn at the scale it's shown, twice that where it's turned (to be sampled back smoothly).
                let stretch = num::sqrt((t[0] * t[0] + t[1] * t[1]).max(t[2] * t[2] + t[3] * t[3]));
                let turned = t[1].abs() > 1e-4 || t[2].abs() > 1e-4;
                let mut k = (st.map.k * stretch).clamp(0.05, 8.0);
                if turned {
                    k = (k * 2.0).min(8.0);
                }
                while local.w * k * local.h * k > MOST_LAYER_PIXELS && k > 0.01 {
                    k /= 2.0;
                }
                let (w, h) = (num::ceil(local.w * k).max(1.0) as usize, num::ceil(local.h * k).max(1.0) as usize);
                let map = Map { k, ox: -local.x * k, oy: -local.y * k };
                let to_under = effects::mul(d, [1.0 / k, 0.0, 0.0, 1.0 / k, local.x, local.y]);
                frames.push(Frame::Layer(Layer { px: vec![0; w * h], w, h, at: (0, 0), to_under: Some(to_under), map, begin: i, under: st.clone(), clip_under: outer }));
                *st = State { map, clip: Area::new(0, 0, w as i32, h as i32), mask: None };
            }
        }
        true
    }

    /// A group closes: its rounded clip put away, or its layer filtered, cut, masked and put on what's under it.
    fn end(&mut self, ty: &Type, items: &[Item], st: &mut State, frames: &mut Vec<Frame>, surface: &mut Surface, pictures: &dyn Shown) {
        let Some(f) = frames.pop() else { return };
        let mut l = match f {
            Frame::Clip { mask, clip } => {
                st.mask = mask;
                st.clip = clip;
                return;
            }
            Frame::Shift { map } => {
                st.map = map;
                return;
            }
            Frame::Layer(l) => l,
        };
        let Paint::Begin(g) = &items[l.begin].paint else { return };
        for f in &g.filters {
            filter(&mut l.px, l.w, l.h, f, l.map.k);
        }
        if let Some((r, rad)) = g.clip {
            let s = Shape::new(l.map.x(r.x), l.map.y(r.y), l.map.len(r.w), l.map.len(r.h), rad.map(|v| l.map.len(v)));
            for y in 0..l.h as i32 {
                for x in 0..l.w as i32 {
                    let p = &mut l.px[y as usize * l.w + x as usize];
                    if *p != 0 {
                        *p = scale_pm(*p, s.cover(x, y));
                    }
                }
            }
        }
        if let Some((polys, even_odd)) = &g.clip_path {
            let mapped: Vec<Vec<(f32, f32)>> = polys.iter().map(|p| p.iter().map(|&(x, y)| (l.map.x(x), l.map.y(y))).collect()).collect();
            let cover = coverage(&mapped, *even_odd, l.w, l.h);
            for (p, c) in l.px.iter_mut().zip(cover.iter()) {
                *p = scale_pm(*p, *c as u32);
            }
        }
        if let Some(layers) = &g.mask {
            let cover = self.mask_cover(ty, layers, &l, pictures);
            for (p, c) in l.px.iter_mut().zip(cover.iter()) {
                *p = scale_pm(*p, *c as u32);
            }
        }
        *st = l.under.clone();
        let opacity = (g.opacity.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        let blend = g.blend;
        let mut canvas = target(frames, surface, st.mask.as_deref());
        let clip = l.clip_under.intersect(&st.clip);
        match l.to_under {
            None => {
                let (ax, ay) = l.at;
                let a = Area::new(ax, ay, l.w as i32, l.h as i32).intersect(&clip);
                for y in a.y..a.b {
                    let row = (y - ay) as usize * l.w;
                    for x in a.x..a.r {
                        let p = l.px[row + (x - ax) as usize];
                        if p != 0 {
                            canvas.put_pm(x, y, scale_pm(p, opacity), blend);
                        }
                    }
                }
            }
            Some(m) => {
                let Some(back) = effects::invert(m) else { return };
                let corners = [(0.0, 0.0), (l.w as f32, 0.0), (l.w as f32, l.h as f32), (0.0, l.h as f32)].map(|(x, y)| effects::apply(m, x, y));
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for (x, y) in corners {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
                let a = Area::from_f(x0, y0, x1 - x0, y1 - y0).intersect(&clip);
                // (A layer drawn finer than it's shown: four samples a pixel.)
                let fine = num::sqrt((back[0] * back[0] + back[1] * back[1]).max(back[2] * back[2] + back[3] * back[3])) > 1.4;
                for y in a.y..a.b {
                    for x in a.x..a.r {
                        let p = if fine {
                            let mut sum = [0u32; 4];
                            for (sx, sy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                                let (u, v) = effects::apply(back, x as f32 + sx, y as f32 + sy);
                                let q = bilinear(&l.px, l.w, l.h, u - 0.5, v - 0.5);
                                for (k, s) in sum.iter_mut().enumerate() {
                                    *s += (q >> (24 - 8 * k)) & 0xFF;
                                }
                            }
                            ((sum[0] + 2) / 4) << 24 | ((sum[1] + 2) / 4) << 16 | ((sum[2] + 2) / 4) << 8 | (sum[3] + 2) / 4
                        } else {
                            let (u, v) = effects::apply(back, x as f32 + 0.5, y as f32 + 0.5);
                            bilinear(&l.px, l.w, l.h, u - 0.5, v - 0.5)
                        };
                        if p != 0 {
                            canvas.put_pm(x, y, scale_pm(p, opacity), blend);
                        }
                    }
                }
            }
        }
    }

    /// A mask's coverage over a layer's pixels: its layers drawn (each into a layer of its own), their alpha (or
    /// luminance) combined as each says.
    fn mask_cover(&mut self, ty: &Type, layers: &[MaskLayer], l: &Layer, pictures: &dyn Shown) -> Vec<u8> {
        let n = l.w * l.h;
        let mut acc = vec![0u8; n];
        for (k, ml) in layers.iter().enumerate() {
            let mut px = vec![0u32; n];
            {
                let mut c = Canvas { px: &mut px, w: l.w, layer: true, mask: None };
                let all = Area::new(0, 0, l.w as i32, l.h as i32);
                for p in &ml.paints {
                    self.draw_item(ty, p, all, l.map, &mut c, pictures);
                }
            }
            for (i, p) in px.iter().enumerate() {
                let a = p >> 24;
                let s = if ml.luminance && a > 0 {
                    // (Luminance of the colour, premultiplied: so times its alpha already.)
                    let (r, g, b) = (((p >> 16) & 0xFF) as f32, ((p >> 8) & 0xFF) as f32, (p & 0xFF) as f32);
                    (0.2126 * r + 0.7152 * g + 0.0722 * b + 0.5) as u32
                } else {
                    a
                };
                let d = acc[i] as u32;
                let out = if k == 0 {
                    s
                } else {
                    match ml.composite {
                        1 => s * (255 - d) / 255,
                        2 => s * d / 255,
                        3 => (s * (255 - d) + d * (255 - s)) / 255,
                        _ => s + d * (255 - s) / 255,
                    }
                };
                acc[i] = out.min(255) as u8;
            }
        }
        acc
    }
}

/// How high the drawing's pixels are (the page's surface, or the top layer).
fn frames_height(frames: &[Frame], surface: &Surface) -> usize {
    for f in frames.iter().rev() {
        if let Frame::Layer(l) = f {
            return l.h;
        }
    }
    surface.height
}

/// What an item's own clip comes to in a target's pixels (within what drawing may touch).
fn item_clip(item: &Item, st: &State) -> Area {
    let c = &item.clip;
    if c.w >= 1.0e6 { st.clip } else { st.map.area(c).intersect(&st.clip) }
}

/// The rectangle an item draws in (page coordinates), roughly: for leaving out what's off the surface, and for a
/// group's layer's size.
fn extent(p: &Paint) -> Option<Rect> {
    Some(match p {
        Paint::Rect { x, y, w, h, .. } | Paint::Border { x, y, w, h, .. } | Paint::SidedBorder { x, y, w, h, .. } | Paint::Picture { x, y, w, h, .. } | Paint::Tiles { x, y, w, h, .. } => {
            Rect::new(*x, *y, *w, *h)
        }
        // (Words reach a little past their box: slanted letters, accents.)
        Paint::Text { x, y, w, line, size, .. } => Rect::new(x - size * 0.5, y - size * 0.3, w + size, line + size * 0.6),
        // (A shadow reaches as far as it's moved, grown and blurred; an inset one, its box.)
        Paint::Shadow { x, y, w, h, inset: true, .. } => Rect::new(*x, *y, *w, *h),
        Paint::Shadow { x, y, w, h, dx, dy, blur, spread, .. } => {
            let reach = spread.max(0.0) + blur;
            let (x0, y0) = (x.min(x + dx - reach), y.min(y + dy - reach));
            let (x1, y1) = ((x + w).max(x + dx + w + reach), (y + h).max(y + dy + h + reach));
            Rect::new(x0, y0, x1 - x0, y1 - y0)
        }
        Paint::Begin(_) | Paint::End => return None,
    })
}

fn union(a: Option<Rect>, b: Rect) -> Option<Rect> {
    if b.w <= 0.0 || b.h <= 0.0 {
        return a;
    }
    Some(match a {
        None => b,
        Some(a) => {
            let (x, y) = (a.x.min(b.x), a.y.min(b.y));
            Rect::new(x, y, a.right().max(b.right()) - x, a.bottom().max(b.bottom()) - y)
        }
    })
}

/// How far a group's filters reach past what's drawn in it (page units).
fn filter_reach(filters: &[Filter]) -> f32 {
    let mut r = 0.0f32;
    for f in filters {
        r += match f {
            Filter::Blur(s) => 3.0 * s,
            Filter::DropShadow { x, y, blur, .. } => x.abs().max(y.abs()) + 3.0 * blur,
            Filter::Color(..) => 0.0,
        };
    }
    r
}

/// For each group's Begin: the index of its End, and what's drawn in it covers (its own coordinates; a group in it
/// through that one's transform, and within its clip).
fn extents(items: &[Item]) -> (Vec<usize>, Vec<Option<Rect>>) {
    let mut ends = vec![usize::MAX; items.len()];
    let mut bounds: Vec<Option<Rect>> = vec![None; items.len()];
    let mut stack: Vec<(usize, Option<Rect>)> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        match &item.paint {
            Paint::Begin(_) => stack.push((i, None)),
            Paint::End => {
                let Some((b, acc)) = stack.pop() else { continue };
                ends[b] = i;
                bounds[b] = acc;
                let Paint::Begin(g) = &items[b].paint else { continue };
                let Some(mut r) = acc else { continue };
                // (One moved as the page scrolls could be anywhere down it.)
                if g.scroll.is_some() {
                    r = Rect::new(r.x, -1.0e6, r.w, 2.0e9);
                }
                let reach = filter_reach(&g.filters);
                r = Rect::new(r.x - reach, r.y - reach, r.w + 2.0 * reach, r.h + 2.0 * reach);
                if let Some((c, _)) = g.clip {
                    r = r.intersect(&c);
                }
                if let Some(t) = g.transform {
                    let pts = [(r.x, r.y), (r.right(), r.y), (r.right(), r.bottom()), (r.x, r.bottom())].map(|(x, y)| effects::apply(t, x, y));
                    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                    for (x, y) in pts {
                        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                    }
                    r = Rect::new(x0, y0, x1 - x0, y1 - y0);
                }
                let c = &items[b].clip;
                if c.w < 1.0e6 {
                    r = r.intersect(c);
                }
                if let Some(top) = stack.last_mut() {
                    top.1 = union(top.1, r);
                }
            }
            p => {
                if let (Some(top), Some(mut r)) = (stack.last_mut(), extent(p)) {
                    let c = &item.clip;
                    if c.w < 1.0e6 {
                        r = r.intersect(c);
                    }
                    top.1 = union(top.1, r);
                }
            }
        }
    }
    // (A Begin with no End: to the end.)
    for (b, _) in stack {
        ends[b] = items.len() - 1;
    }
    (ends, bounds)
}

/// Where a point (the page's coordinates, the page scrolled `scroll` down) is in each item's own, through the
/// transforms of the groups it's in (and back by how far those fixed to the window are moved).
pub fn points_in_items(items: &[Item], x: f32, y: f32, scroll: f32) -> Vec<Option<(f32, f32)>> {
    let mut out = Vec::with_capacity(items.len());
    let mut stack: Vec<Option<(f32, f32)>> = Vec::new();
    let mut cur = Some((x, y));
    for i in items {
        match &i.paint {
            Paint::Begin(g) => {
                out.push(cur);
                stack.push(cur);
                if let Some(sc) = &g.scroll {
                    let dy = sc.shift(scroll);
                    cur = cur.map(|(px, py)| (px, py - dy));
                }
                if let Some(m) = g.transform {
                    cur = cur.and_then(|(px, py)| effects::invert(m).map(|inv| effects::apply(inv, px, py)));
                }
            }
            Paint::End => {
                cur = stack.pop().unwrap_or(Some((x, y)));
                out.push(cur);
            }
            _ => out.push(cur),
        }
    }
    out
}

fn alpha(c: Color) -> u32 {
    c >> 24
}

/// A rectangle of whole pixels.
#[derive(Clone, Copy, Debug)]
struct Area {
    x: i32,
    y: i32,
    r: i32,
    b: i32,
}

impl Area {
    fn new(x: i32, y: i32, w: i32, h: i32) -> Area {
        Area { x, y, r: x + w.max(0), b: y + h.max(0) }
    }
    fn from_f(x: f32, y: f32, w: f32, h: f32) -> Area {
        let clampi = |v: f32| v.clamp(-1.0e9, 1.0e9);
        Area { x: num::floor(clampi(x)) as i32, y: num::floor(clampi(y)) as i32, r: num::ceil(clampi(x + w)) as i32, b: num::ceil(clampi(y + h)) as i32 }
    }
    fn within(self, w: i32, h: i32) -> Area {
        self.intersect(&Area::new(0, 0, w, h))
    }
    fn intersect(&self, o: &Area) -> Area {
        Area { x: self.x.max(o.x), y: self.y.max(o.y), r: self.r.min(o.r), b: self.b.min(o.b) }
    }
    fn empty(&self) -> bool {
        self.r <= self.x || self.b <= self.y
    }
}

/// `src` (0xAARRGGBB) over `dst` (0xRRGGBB), at `cover` of 255 as well as its own alpha.
fn over(dst: u32, src: Color, cover: u32) -> u32 {
    let a = (alpha(src) * cover + 127) / 255;
    if a >= 255 {
        return src & 0x00FF_FFFF;
    }
    if a == 0 {
        return dst;
    }
    let mix = |shift: u32| {
        let d = (dst >> shift) & 0xFF;
        let s = (src >> shift) & 0xFF;
        ((s * a + d * (255 - a) + 127) / 255) << shift
    };
    mix(16) | mix(8) | mix(0)
}

/// `src` (0xAARRGGBB, alpha straight) over `dst` (premultiplied), at `cover` of 255.
fn over_pm(dst: u32, src: Color, cover: u32) -> u32 {
    let a = (alpha(src) * cover + 127) / 255;
    if a == 0 {
        return dst;
    }
    if a >= 255 {
        return src | 0xFF00_0000;
    }
    let inv = 255 - a;
    let mix = |shift: u32| {
        let d = (dst >> shift) & 0xFF;
        let s = (src >> shift) & 0xFF;
        ((s * a + d * inv + 127) / 255) << shift
    };
    let oa = a + ((dst >> 24) * inv + 127) / 255;
    (oa.min(255) << 24) | mix(16) | mix(8) | mix(0)
}

/// A premultiplied colour scaled by `k` of 255 (more see-through).
#[inline]
fn scale_pm(p: u32, k: u32) -> u32 {
    if k >= 255 {
        return p;
    }
    if k == 0 {
        return 0;
    }
    let ch = |shift: u32| ((((p >> shift) & 0xFF) * k + 127) / 255) << shift;
    ch(24) | ch(16) | ch(8) | ch(0)
}

/// A premultiplied colour put on `dst` (premultiplied if `layer`, else opaque 0xRRGGBB), blended by `blend` (255:
/// normal; else a blend mode, as the compositing spec mixes a source with its backdrop).
fn composite_pm(dst: u32, src: u32, blend: u8, layer: bool) -> u32 {
    let sa = src >> 24;
    if blend == 255 {
        let inv = 255 - sa;
        let ch = |shift: u32| (((src >> shift) & 0xFF) + (((dst >> shift) & 0xFF) * inv + 127) / 255).min(255) << shift;
        let rgb = ch(16) | ch(8) | ch(0);
        return if layer { ch(24) | rgb } else { rgb };
    }
    let fa = sa as f32 / 255.0;
    let ba = if layer { (dst >> 24) as f32 / 255.0 } else { 1.0 };
    let unpm = |c: u32, a: f32| if a > 0.0 { (c as f32 / 255.0 / a).min(1.0) } else { 0.0 };
    let cs = [unpm((src >> 16) & 0xFF, fa), unpm((src >> 8) & 0xFF, fa), unpm(src & 0xFF, fa)];
    let cb = [unpm((dst >> 16) & 0xFF, ba), unpm((dst >> 8) & 0xFF, ba), unpm(dst & 0xFF, ba)];
    let b = blend_colours(blend, cb, cs);
    let mut out = 0u32;
    for k in 0..3 {
        let mixed = (1.0 - ba) * cs[k] + ba * b[k];
        let co = fa * mixed + (1.0 - fa) * ba * cb[k];
        out |= (((co.clamp(0.0, 1.0) * 255.0) + 0.5) as u32) << (16 - 8 * k);
    }
    if layer {
        let oa = fa + ba * (1.0 - fa);
        out |= ((oa.clamp(0.0, 1.0) * 255.0 + 0.5) as u32) << 24;
    }
    out
}

/// A blend mode's mix of a backdrop's colour `b` and a source's `s` (0..1 each): multiply 0, screen 1, overlay 2,
/// darken 3, lighten 4, color-dodge 5, color-burn 6, hard-light 7, soft-light 8, difference 9, exclusion 10, and the
/// non-separable hue 11, saturation 12, color 13 and luminosity 14 (the compositing spec's formulas).
pub(crate) fn blend_colours(mode: u8, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    let sep = |f: fn(f32, f32) -> f32| [f(b[0], s[0]), f(b[1], s[1]), f(b[2], s[2])];
    fn screen(b: f32, s: f32) -> f32 {
        b + s - b * s
    }
    fn hard_light(b: f32, s: f32) -> f32 {
        if s <= 0.5 { b * 2.0 * s } else { screen(b, 2.0 * s - 1.0) }
    }
    let lum = |c: [f32; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
    let clip_color = |c: [f32; 3]| {
        let l = lum(c);
        let n = c[0].min(c[1]).min(c[2]);
        let x = c[0].max(c[1]).max(c[2]);
        let mut c = c;
        if n < 0.0 && l - n > 1e-6 {
            c = c.map(|v| l + (v - l) * l / (l - n));
        }
        if x > 1.0 && x - l > 1e-6 {
            c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
        }
        c
    };
    let set_lum = |c: [f32; 3], l: f32| {
        let d = l - lum(c);
        clip_color(c.map(|v| v + d))
    };
    let sat = |c: [f32; 3]| c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
    let set_sat = |c: [f32; 3], s: f32| {
        let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        if mx - mn <= 1e-6 {
            return [0.0; 3];
        }
        c.map(|v| (v - mn) * s / (mx - mn))
    };
    match mode {
        0 => sep(|b, s| b * s),
        1 => sep(screen),
        2 => sep(|b, s| hard_light(s, b)),
        3 => sep(f32::min),
        4 => sep(f32::max),
        5 => sep(|b, s| {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }),
        6 => sep(|b, s| {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }),
        7 => sep(hard_light),
        8 => sep(|b, s| {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { num::sqrt(b) };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }),
        9 => sep(|b, s| (b - s).abs()),
        10 => sep(|b, s| b + s - 2.0 * b * s),
        11 => set_lum(set_sat(s, sat(b)), lum(b)),
        12 => set_lum(set_sat(b, sat(s)), lum(b)),
        13 => set_lum(s, lum(b)),
        _ => set_lum(b, lum(s)),
    }
}

/// A premultiplied layer's colour at (u, v) (its pixels' centres at whole numbers), between the four nearest; none
/// outside it.
fn bilinear(px: &[u32], w: usize, h: usize, u: f32, v: f32) -> u32 {
    if u <= -1.0 || v <= -1.0 || u >= w as f32 || v >= h as f32 {
        return 0;
    }
    let (fu, fv) = (num::floor(u), num::floor(v));
    let (x0, y0) = (fu as i32, fv as i32);
    let (du, dv) = (u - fu, v - fv);
    let get = |x: i32, y: i32| if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 { 0 } else { px[y as usize * w + x as usize] };
    let (a, b, c, d) = (get(x0, y0), get(x0 + 1, y0), get(x0, y0 + 1), get(x0 + 1, y0 + 1));
    if a == b && a == c && a == d {
        return a;
    }
    let (wa, wb, wc, wd) = ((1.0 - du) * (1.0 - dv), du * (1.0 - dv), (1.0 - du) * dv, du * dv);
    let ch = |shift: u32| {
        let f = |p: u32| ((p >> shift) & 0xFF) as f32;
        (((f(a) * wa + f(b) * wb + f(c) * wc + f(d) * wd) + 0.5) as u32).min(255) << shift
    };
    ch(24) | ch(16) | ch(8) | ch(0)
}

fn fill(c: &mut Canvas, a: Area, color: Color, cover: u32) {
    for y in a.y..a.b {
        for x in a.x..a.r {
            c.put(x, y, color, cover);
        }
    }
}

/// A rectangle with rounded corners (top left, top right, bottom right, bottom left), in fractions of pixels.
struct Shape {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radii: [f32; 4],
    /// The largest of them: the rows and columns further in than it are clear of every corner.
    radius: f32,
}

impl Shape {
    fn new(x: f32, y: f32, w: f32, h: f32, radii: [f32; 4]) -> Shape {
        let w = w.max(0.0);
        let h = h.max(0.0);
        let mut radii = radii.map(|r| r.max(0.0));
        // (Corners that would overlap made smaller together, as CSS has it.)
        let f = [(radii[0] + radii[1], w), (radii[3] + radii[2], w), (radii[0] + radii[3], h), (radii[1] + radii[2], h)]
            .iter()
            .filter(|(sum, _)| *sum > 0.0)
            .map(|(sum, side)| side / sum)
            .fold(1.0f32, f32::min);
        if f < 1.0 {
            for r in &mut radii {
                *r *= f;
            }
        }
        let radius = radii.iter().copied().fold(0.0f32, f32::max);
        Shape { x, y, w, h, radii, radius }
    }

    fn area(&self) -> Area {
        Area::from_f(self.x, self.y, self.w, self.h)
    }

    /// How far a point is outside the shape (inside: negative), its corners' rounding counted.
    fn distance(&self, px: f32, py: f32) -> f32 {
        let (cx, cy) = (self.x + self.w / 2.0, self.y + self.h / 2.0);
        let (qx0, qy0) = (px - cx, py - cy);
        let r = match (qx0 < 0.0, qy0 < 0.0) {
            (true, true) => self.radii[0],
            (false, true) => self.radii[1],
            (false, false) => self.radii[2],
            (true, false) => self.radii[3],
        };
        let qx = qx0.abs() - self.w / 2.0 + r;
        let qy = qy0.abs() - self.h / 2.0 + r;
        let (ox, oy) = (qx.max(0.0), qy.max(0.0));
        let outside = num::sqrt(ox * ox + oy * oy);
        outside + qx.max(qy).min(0.0) - r
    }

    /// Whether a point is inside.
    fn inside(&self, px: f32, py: f32) -> bool {
        if px < self.x || py < self.y || px >= self.x + self.w || py >= self.y + self.h {
            return false;
        }
        if self.radius <= 0.0 {
            return true;
        }
        // (Within a corner's square: inside its circle.)
        let (x0, y0, x1, y1) = (self.x, self.y, self.x + self.w, self.y + self.h);
        let r = &self.radii;
        let (cx, cy, rad) = if px < x0 + r[0] && py < y0 + r[0] {
            (x0 + r[0], y0 + r[0], r[0])
        } else if px > x1 - r[1] && py < y0 + r[1] {
            (x1 - r[1], y0 + r[1], r[1])
        } else if px > x1 - r[2] && py > y1 - r[2] {
            (x1 - r[2], y1 - r[2], r[2])
        } else if px < x0 + r[3] && py > y1 - r[3] {
            (x0 + r[3], y1 - r[3], r[3])
        } else {
            return true;
        };
        let (dx, dy) = (px - cx, py - cy);
        dx * dx + dy * dy <= rad * rad
    }

    /// How much of the pixel at (x, y) is inside, of 255: whole pixels well inside or outside at once, the edges
    /// sampled 4 × 4.
    fn cover(&self, x: i32, y: i32) -> u32 {
        let (fx, fy) = (x as f32, y as f32);
        let r = self.radius;
        if fx + 1.0 <= self.x || fy + 1.0 <= self.y || fx >= self.x + self.w || fy >= self.y + self.h {
            return 0;
        }
        let near_edge = fx < self.x + r.max(1.0) || fx + 1.0 > self.x + self.w - r.max(1.0) || fy < self.y + r.max(1.0) || fy + 1.0 > self.y + self.h - r.max(1.0);
        if !near_edge {
            return 255;
        }
        let mut n = 0;
        for sy in 0..4 {
            for sx in 0..4 {
                if self.inside(fx + (sx as f32 + 0.5) / 4.0, fy + (sy as f32 + 0.5) / 4.0) {
                    n += 1;
                }
            }
        }
        (n * 255 + 8) / 16
    }
}

/// A shape filled (with `hole` left out: a border), in `color`, within `clip`.
fn shape(s: &mut Canvas, outer: &Shape, hole: Option<&Shape>, clip: Area, color: Color) {
    let a = outer.area().intersect(&clip);
    if a.empty() {
        return;
    }
    // (The rows clear of the corners, and the columns clear of the edges, are filled whole.)
    let solid = |x: i32, y: i32| -> u32 {
        let c = outer.cover(x, y);
        match hole {
            Some(h) if c > 0 => c.saturating_sub(h.cover(x, y)),
            _ => c,
        }
    };
    for y in a.y..a.b {
        let fy = y as f32;
        let corner_row = fy < outer.y + outer.radius.max(1.0) || fy + 1.0 > outer.y + outer.h - outer.radius.max(1.0);
        let mut x = a.x;
        while x < a.r {
            let fx = x as f32;
            // Inside the hole's straight part: skip to its far side.
            if let Some(h) = hole
                && fy >= h.y + h.radius + 1.0
                && fy + 1.0 <= h.y + h.h - h.radius - 1.0
                && fx >= h.x + 1.0
                && fx + 1.0 <= h.x + h.w - 1.0
            {
                x = (num::floor(h.x + h.w - 1.0) as i32).max(x + 1);
                continue;
            }
            let edge = corner_row || fx < outer.x + 1.0 || fx + 1.0 > outer.x + outer.w - 1.0 || hole.is_some();
            let cover = if edge { solid(x, y) } else { 255 };
            if cover > 0 {
                s.put(x, y, color, cover);
            }
            x += 1;
        }
    }
}

/// A border whose sides are of different colours: within `outer` and outside `inner`, each point the colour of the
/// side it's nearest to by that side's width (so the sides meet on the lines from the outer corners to the inner
/// ones), edges sampled 4 × 4.
fn sided_border(c: &mut Canvas, outer: &Shape, inner: &Shape, widths: [f32; 4], colors: [Color; 4], clip: Area) {
    let a = outer.area().intersect(&clip);
    if a.empty() {
        return;
    }
    let per = |w: f32| if w > 0.0 { 1.0 / w } else { f32::INFINITY };
    let k = [per(widths[0]), per(widths[1]), per(widths[2]), per(widths[3])];
    let (x1, y1) = (outer.x + outer.w, outer.y + outer.h);
    let side = |px: f32, py: f32| -> usize {
        let d = [(py - outer.y) * k[0], (x1 - px) * k[1], (y1 - py) * k[2], (px - outer.x) * k[3]];
        let mut best = 0;
        for i in 1..4 {
            if d[i] < d[best] {
                best = i;
            }
        }
        best
    };
    let r = inner.radius.max(1.0);
    for y in a.y..a.b {
        let fy = y as f32;
        let mut x = a.x;
        while x < a.r {
            let fx = x as f32;
            // Well inside the inner box's straight part: skip to its far side.
            if fy >= inner.y + r && fy + 1.0 <= inner.y + inner.h - r && fx >= inner.x + 1.0 && fx + 1.0 <= inner.x + inner.w - 1.0 {
                x = (num::floor(inner.x + inner.w - 1.0) as i32).max(x + 1);
                continue;
            }
            let mut counts = [0u32; 4];
            for sy in 0..4 {
                for sx in 0..4 {
                    let (px, py) = (fx + (sx as f32 + 0.5) / 4.0, fy + (sy as f32 + 0.5) / 4.0);
                    if outer.inside(px, py) && !inner.inside(px, py) {
                        counts[side(px, py)] += 1;
                    }
                }
            }
            for (i, &n) in counts.iter().enumerate() {
                if n > 0 && alpha(colors[i]) > 0 {
                    c.put(x, y, colors[i], (n * 255 + 8) / 16);
                }
            }
            x += 1;
        }
    }
}

/// erf(x), to 1.5e-7 (Abramowitz and Stegun 7.1.26).
fn erf(x: f32) -> f32 {
    let s = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x * s;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = 1.0 - (((((1.061_405_4 * t - 1.453_152_1) * t) + 1.421_413_8) * t - 0.284_496_74) * t + 0.254_829_6) * t * num::exp(-x * x);
    s * y
}

/// A box's shadow: `cast` (the box moved, grown) blurred by `blur` (CSS's blur radius: twice the Gaussian's standard
/// deviation), drawn only where the box itself (`own`) isn't, as browsers draw it.
fn shadow(s: &mut Canvas, cast: &Shape, own: &Shape, blur: f32, color: Color, clip: Area) {
    let reach = blur.max(0.0) * 1.5 + 1.0;
    let a = Area::from_f(cast.x - reach, cast.y - reach, cast.w + 2.0 * reach, cast.h + 2.0 * reach).intersect(&clip);
    if a.empty() {
        return;
    }
    let sigma = (blur / 2.0).max(0.0);
    for y in a.y..a.b {
        for x in a.x..a.r {
            let mine = own.cover(x, y);
            if mine >= 255 {
                continue;
            }
            let cover = if sigma < 0.5 {
                cast.cover(x, y)
            } else {
                // How far the pixel's middle is outside the cast shape (inside: negative), softened by the blur.
                let d = cast.distance(x as f32 + 0.5, y as f32 + 0.5);
                (0.5 * (1.0 - erf(d / (sigma * core::f32::consts::SQRT_2))) * 255.0 + 0.5) as u32
            };
            let cover = cover.min(255) * (255 - mine) / 255;
            if cover > 0 {
                s.put(x, y, color, cover);
            }
        }
    }
}

/// An inset shadow: inside a box's padding box (`own`), all but where `hole` is, the hole's edges blurred by `blur`.
fn inset_shadow(s: &mut Canvas, hole: &Shape, own: &Shape, blur: f32, color: Color, clip: Area) {
    let a = own.area().intersect(&clip);
    if a.empty() {
        return;
    }
    let sigma = (blur / 2.0).max(0.0);
    let empty = hole.w <= 0.0 || hole.h <= 0.0;
    for y in a.y..a.b {
        for x in a.x..a.r {
            let mine = own.cover(x, y);
            if mine == 0 {
                continue;
            }
            let in_hole = if empty {
                0
            } else if sigma < 0.5 {
                hole.cover(x, y)
            } else {
                let d = hole.distance(x as f32 + 0.5, y as f32 + 0.5);
                (0.5 * (1.0 - erf(d / (sigma * core::f32::consts::SQRT_2))) * 255.0 + 0.5) as u32
            };
            let cover = mine * (255 - in_hole.min(255)) / 255;
            if cover > 0 {
                s.put(x, y, color, cover);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------------- gradients

/// A gradient made ready to sample over a tile `w` × `h` (device pixels): where along it a point is, and its stops'
/// places along that (device pixels; degrees for a conic one), fixed up as CSS fixes them.
struct Sampler {
    kind: GradientKind,
    /// A linear one's direction (unit), and its line's length; a radial one's centre and radii; a conic one's centre.
    dir: (f32, f32),
    len: f32,
    centre: (f32, f32),
    radii: (f32, f32),
    from: f32,
    repeating: bool,
    /// Its stops' places and colours (premultiplied, as 0..255 floats: a, r, g, b), with hints' places.
    stops: Vec<(f32, [f32; 4], Option<f32>)>,
}

fn premultiplied(c: Color) -> [f32; 4] {
    let a = (c >> 24) as f32;
    let k = a / 255.0;
    [a, ((c >> 16) & 0xFF) as f32 * k, ((c >> 8) & 0xFF) as f32 * k, (c & 0xFF) as f32 * k]
}

impl Sampler {
    fn new(g: &Gradient, w: f32, h: f32, scale: f32) -> Sampler {
        let mut s = Sampler { kind: g.kind, dir: (0.0, 1.0), len: h, centre: (w / 2.0, h / 2.0), radii: (0.0, 0.0), from: 0.0, repeating: g.repeating, stops: Vec::new() };
        match g.kind {
            GradientKind::Linear { angle, corner } => {
                let (dx, dy) = match corner {
                    // (Towards a corner: square to the line between the two corners beside it.)
                    Some((cx, cy)) => {
                        let (vx, vy) = (h * cx as f32, w * cy as f32);
                        let l = num::sqrt(vx * vx + vy * vy).max(1e-6);
                        (vx / l, vy / l)
                    }
                    None => {
                        let r = angle * core::f32::consts::PI / 180.0;
                        (num::sin(r), -num::cos(r))
                    }
                };
                s.dir = (dx, dy);
                s.len = w * dx.abs() + h * dy.abs();
            }
            GradientKind::Radial { circle, size, at } => {
                let c = (at.0.resolve(w), at.1.resolve(h));
                s.centre = c;
                let (sx_near, sx_far) = (c.0.abs().min((w - c.0).abs()), c.0.abs().max((w - c.0).abs()));
                let (sy_near, sy_far) = (c.1.abs().min((h - c.1).abs()), c.1.abs().max((h - c.1).abs()));
                s.radii = match (size, circle) {
                    (RadialSize::Radii(rx, ry), _) => (rx.resolve(w).max(0.0), if circle { rx.resolve(w).max(0.0) } else { ry.resolve(h).max(0.0) }),
                    (RadialSize::ClosestSide, true) => {
                        let r = sx_near.min(sy_near);
                        (r, r)
                    }
                    (RadialSize::FarthestSide, true) => {
                        let r = sx_far.max(sy_far);
                        (r, r)
                    }
                    (RadialSize::ClosestCorner, true) => {
                        let r = num::sqrt(sx_near * sx_near + sy_near * sy_near);
                        (r, r)
                    }
                    (RadialSize::FarthestCorner, true) => {
                        let r = num::sqrt(sx_far * sx_far + sy_far * sy_far);
                        (r, r)
                    }
                    (RadialSize::ClosestSide, false) => (sx_near, sy_near),
                    (RadialSize::FarthestSide, false) => (sx_far, sy_far),
                    (RadialSize::ClosestCorner, false) => (sx_near * core::f32::consts::SQRT_2, sy_near * core::f32::consts::SQRT_2),
                    (RadialSize::FarthestCorner, false) => (sx_far * core::f32::consts::SQRT_2, sy_far * core::f32::consts::SQRT_2),
                };
                s.len = s.radii.0;
            }
            GradientKind::Conic { from, at } => {
                s.centre = (at.0.resolve(w), at.1.resolve(h));
                s.from = from;
                s.len = 360.0;
            }
        }
        let conic = matches!(g.kind, GradientKind::Conic { .. });
        // The stops' places: lengths as they are (device pixels; a conic one's degrees), percentages of the line.
        let place = |l: effects::Stop| l.at.map(|a| if conic { a.px + a.pct * 3.6 } else { a.px + a.pct * s.len / 100.0 });
        let _ = scale;
        let n = g.stops.len();
        let mut at: Vec<Option<f32>> = g.stops.iter().map(|x| place(*x)).collect();
        // (The first at 0 and the last at the end, if not given; none before one before it.)
        let colours: Vec<usize> = (0..n).filter(|&i| !g.stops[i].hint).collect();
        if let Some(&f) = colours.first()
            && at[f].is_none()
        {
            at[f] = Some(0.0);
        }
        if let Some(&l) = colours.last()
            && at[l].is_none()
        {
            at[l] = Some(s.len);
        }
        let mut most = f32::MIN;
        for a in at.iter_mut().flatten() {
            if *a < most {
                *a = most;
            }
            most = *a;
        }
        // (Those without a place spread evenly between those with one.)
        let mut i = 0;
        while i < n {
            if at[i].is_none() {
                let start = i - 1;
                let mut end = i;
                while at[end].is_none() {
                    end += 1;
                }
                let (a0, a1) = (at[start].unwrap_or(0.0), at[end].unwrap_or(s.len));
                for k in i..end {
                    at[k] = Some(a0 + (a1 - a0) * (k - start) as f32 / (end - start) as f32);
                }
                i = end;
            }
            i += 1;
        }
        for k in 0..n {
            let p = at[k].unwrap_or(0.0);
            if g.stops[k].hint {
                if let Some(last) = s.stops.last_mut() {
                    last.2 = Some(p);
                }
            } else {
                s.stops.push((p, premultiplied(g.stops[k].color), None));
            }
        }
        s
    }

    /// Where a point (relative to the tile's top left) is along the gradient (device pixels; degrees, conic).
    fn place(&self, x: f32, y: f32) -> f32 {
        match self.kind {
            GradientKind::Linear { .. } => {
                let (cx, cy) = (x - self.centre.0, y - self.centre.1);
                (cx * self.dir.0 + cy * self.dir.1) + self.len / 2.0
            }
            GradientKind::Radial { .. } => {
                let (dx, dy) = (x - self.centre.0, y - self.centre.1);
                let (rx, ry) = self.radii;
                if rx <= 0.0 || ry <= 0.0 {
                    return if dx == 0.0 && dy == 0.0 { 0.0 } else { f32::MAX };
                }
                let ny = dy * rx / ry;
                num::sqrt(dx * dx + ny * ny)
            }
            GradientKind::Conic { .. } => {
                let (dx, dy) = (x - self.centre.0, y - self.centre.1);
                let a = picture::draw::atan2(dx, -dy) * 180.0 / core::f32::consts::PI;
                let a = a - self.from;
                a - num::floor(a / 360.0) * 360.0
            }
        }
    }

    /// The colour at a place along it.
    fn colour(&self, t: f32) -> Color {
        let n = self.stops.len();
        if n == 0 {
            return 0;
        }
        let (first, last) = (self.stops[0].0, self.stops[n - 1].0);
        let mut t = t;
        if self.repeating && last > first {
            let span = last - first;
            t = first + (t - first) - num::floor((t - first) / span) * span;
        }
        let pack = |c: [f32; 4]| -> Color {
            let a = c[0];
            if a <= 0.0 {
                return 0;
            }
            let k = 255.0 / a;
            let ch = |v: f32| ((v * k) + 0.5).clamp(0.0, 255.0) as u32;
            ((a + 0.5).clamp(0.0, 255.0) as u32) << 24 | ch(c[1]) << 16 | ch(c[2]) << 8 | ch(c[3])
        };
        if t <= first {
            return pack(self.stops[0].1);
        }
        if t >= last {
            return pack(self.stops[n - 1].1);
        }
        let k = self.stops.iter().position(|s| s.0 > t).unwrap_or(n - 1).max(1);
        let (p0, c0, hint) = self.stops[k - 1];
        let (p1, c1, _) = self.stops[k];
        let mut f = if p1 > p0 { (t - p0) / (p1 - p0) } else { 1.0 };
        // (A hint: half and half there, the mix's curve bent to pass through it.)
        if let Some(h) = hint
            && p1 > p0
        {
            let hp = ((h - p0) / (p1 - p0)).clamp(0.0, 1.0);
            f = if hp <= 0.0 {
                1.0
            } else if hp >= 1.0 {
                0.0
            } else {
                num::pow(f, num::ln(0.5) / num::ln(hp))
            };
        }
        let mut c = [0.0; 4];
        for i in 0..4 {
            c[i] = c0[i] + (c1[i] - c0[i]) * f;
        }
        pack(c)
    }
}

/// A gradient's tiles over a shape: each pixel's place in its tile (every `step` across and down, where it repeats).
fn gradient_tiles(c: &mut Canvas, area: &Shape, s: &Sampler, t: Rect, step: (f32, f32), repeat: (bool, bool), clip: Area) {
    let a = area.area().intersect(&clip);
    if a.empty() || t.w <= 0.0 || t.h <= 0.0 {
        return;
    }
    for y in a.y..a.b {
        let mut v = y as f32 + 0.5 - t.y;
        if repeat.1 {
            v -= num::floor(v / step.1) * step.1;
        }
        if v < 0.0 || v >= t.h {
            continue;
        }
        for x in a.x..a.r {
            let mut u = x as f32 + 0.5 - t.x;
            if repeat.0 {
                u -= num::floor(u / step.0) * step.0;
            }
            if u < 0.0 || u >= t.w {
                continue;
            }
            let cover = area.cover(x, y);
            if cover == 0 {
                continue;
            }
            let color = s.colour(s.place(u, v));
            if alpha(color) > 0 {
                c.put(x, y, color, cover);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------------- pictures

/// A picture made `tw` × `th` pixels (0xAARRGGBB, alpha straight). Shrinking, each pixel is the average of those it
/// covers (up to 8 by 8 of them, spread over it); enlarging, it's between the four nearest (bilinear). Colours are
/// averaged by their alpha.
fn scaled(rgb: &[u8], alpha: Option<&[u8]>, pw: u32, ph: u32, tw: u32, th: u32) -> Vec<u32> {
    let (pw_, ph_) = (pw as usize, ph as usize);
    if pw == 0 || ph == 0 || tw == 0 || th == 0 || rgb.len() < pw_ * ph_ * 3 {
        return vec![0; tw as usize * th as usize];
    }
    let alpha = alpha.filter(|a| a.len() >= pw_ * ph_);
    if tw == pw && th == ph {
        // (Drawn the size it is, as a canvas mostly is: each pixel as it is.)
        let mut out = vec![0u32; tw as usize * th as usize];
        for (i, o) in out.iter_mut().enumerate() {
            let a = alpha.map_or(255, |al| al[i] as u32);
            if a > 0 {
                *o = a << 24 | (rgb[i * 3] as u32) << 16 | (rgb[i * 3 + 1] as u32) << 8 | rgb[i * 3 + 2] as u32;
            }
        }
        return out;
    }
    let (kx, ky) = (pw as f32 / tw as f32, ph as f32 / th as f32);
    // A source pixel, premultiplied: (r, g, b, a), 0 to 255.
    let get = |u: usize, v: usize| -> [u32; 4] {
        let i = v * pw_ + u;
        let a = alpha.map_or(255, |al| al[i] as u32);
        [rgb[i * 3] as u32 * a, rgb[i * 3 + 1] as u32 * a, rgb[i * 3 + 2] as u32 * a, a]
    };
    let pack = |sum: [f32; 4]| -> u32 {
        let a = sum[3];
        if a <= 0.0 {
            return 0;
        }
        let c = |v: f32| ((v / a) + 0.5).clamp(0.0, 255.0) as u32;
        ((a + 0.5).clamp(0.0, 255.0) as u32) << 24 | c(sum[0]) << 16 | c(sum[1]) << 8 | c(sum[2])
    };
    let mut out = vec![0u32; tw as usize * th as usize];
    let (nx, ny) = ((num::ceil(kx) as usize).clamp(1, 8), (num::ceil(ky) as usize).clamp(1, 8));
    for ty in 0..th as usize {
        for tx in 0..tw as usize {
            let mut sum = [0.0f32; 4];
            if kx >= 1.0 || ky >= 1.0 {
                // The footprint's samples, each weighted alike.
                for j in 0..ny {
                    let v = ((ty as f32 + (j as f32 + 0.5) / ny as f32) * ky) as usize;
                    let v = v.min(ph_ - 1);
                    for i in 0..nx {
                        let u = ((tx as f32 + (i as f32 + 0.5) / nx as f32) * kx) as usize;
                        let p = get(u.min(pw_ - 1), v);
                        for k in 0..4 {
                            sum[k] += p[k] as f32;
                        }
                    }
                }
                let n = (nx * ny) as f32;
                out[ty * tw as usize + tx] = pack([sum[0] / n, sum[1] / n, sum[2] / n, sum[3] / n]);
            } else {
                let fu = ((tx as f32 + 0.5) * kx - 0.5).clamp(0.0, pw as f32 - 1.0);
                let fv = ((ty as f32 + 0.5) * ky - 0.5).clamp(0.0, ph as f32 - 1.0);
                let (u0, v0) = (fu as usize, fv as usize);
                let (u1, v1) = ((u0 + 1).min(pw_ - 1), (v0 + 1).min(ph_ - 1));
                let (du, dv) = (fu - u0 as f32, fv - v0 as f32);
                for (u, v, w) in [(u0, v0, (1.0 - du) * (1.0 - dv)), (u1, v0, du * (1.0 - dv)), (u0, v1, (1.0 - du) * dv), (u1, v1, du * dv)] {
                    let p = get(u, v);
                    for k in 0..4 {
                        sum[k] += p[k] as f32 * w;
                    }
                }
                out[ty * tw as usize + tx] = pack(sum);
            }
        }
    }
    out
}

/// A picture made `tw` × `th` pixel by pixel (image-rendering: pixelated): each the picture's pixel its middle falls
/// on.
fn nearest(rgb: &[u8], alpha: Option<&[u8]>, pw: u32, ph: u32, tw: u32, th: u32) -> Vec<u32> {
    let (pw_, ph_) = (pw as usize, ph as usize);
    let mut out = vec![0u32; tw as usize * th as usize];
    if pw == 0 || ph == 0 || rgb.len() < pw_ * ph_ * 3 {
        return out;
    }
    let alpha = alpha.filter(|a| a.len() >= pw_ * ph_);
    for ty in 0..th as usize {
        let v = (((ty as f32 + 0.5) * ph as f32 / th as f32) as usize).min(ph_ - 1);
        for tx in 0..tw as usize {
            let u = (((tx as f32 + 0.5) * pw as f32 / tw as f32) as usize).min(pw_ - 1);
            let i = v * pw_ + u;
            let a = alpha.map_or(255, |al| al[i] as u32);
            if a > 0 {
                out[ty * tw as usize + tx] = a << 24 | (rgb[i * 3] as u32) << 16 | (rgb[i * 3 + 1] as u32) << 8 | rgb[i * 3 + 2] as u32;
            }
        }
    }
    out
}

/// A picture made its drawn size (`scaled`), its top left at (ax, ay), drawn within a box's shape (its rounded
/// corners left out).
#[allow(clippy::too_many_arguments)]
fn picture(s: &mut Canvas, bx: &Shape, px: &[u32], ax: i32, ay: i32, tw: u32, th: u32, clip: Area) {
    let a = bx.area().intersect(&clip).intersect(&Area::new(ax, ay, tw as i32, th as i32));
    if a.empty() || tw == 0 {
        return;
    }
    for y in a.y..a.b {
        let src = (y - ay) as usize * tw as usize;
        for x in a.x..a.r {
            let c = if bx.radius > 0.0 || bx.x != num::floor(bx.x) || bx.y != num::floor(bx.y) { bx.cover(x, y) } else { 255 };
            if c == 0 {
                continue;
            }
            let Some(&color) = px.get(src + (x - ax) as usize) else { continue };
            s.put(x, y, color, c);
        }
    }
}

/// A picture's tiles (each `t`'s size, the picture made `tw` × `th` for them; one at `t`'s place, more every `step`
/// where it repeats) within a shape.
#[allow(clippy::too_many_arguments)]
fn picture_tiles(s: &mut Canvas, area: &Shape, px: &[u32], t: Rect, (tw, th): (u32, u32), step: (f32, f32), repeat: (bool, bool), clip: Area) {
    let a = area.area().intersect(&clip);
    if a.empty() || t.w <= 0.0 || t.h <= 0.0 {
        return;
    }
    let (kx, ky) = (tw as f32 / t.w, th as f32 / t.h);
    for y in a.y..a.b {
        let mut v = y as f32 + 0.5 - t.y;
        if repeat.1 {
            v -= num::floor(v / step.1) * step.1;
        }
        if v < 0.0 || v >= t.h {
            continue;
        }
        let row = ((v * ky) as usize).min(th as usize - 1) * tw as usize;
        for x in a.x..a.r {
            let mut u = x as f32 + 0.5 - t.x;
            if repeat.0 {
                u -= num::floor(u / step.0) * step.0;
            }
            if u < 0.0 || u >= t.w {
                continue;
            }
            let color = px[row + ((u * kx) as usize).min(tw as usize - 1)];
            if alpha(color) == 0 {
                continue;
            }
            let cover = area.cover(x, y);
            if cover > 0 {
                s.put(x, y, color, cover);
            }
        }
    }
}

/// Words' ink at (x, y), in `color`.
fn words(s: &mut Canvas, ink: &Ink, x: i32, y: i32, color: Color, clip: Area) {
    let a = Area::new(x, y, ink.width as i32, ink.height as i32).intersect(&clip);
    if a.empty() || alpha(color) == 0 {
        return;
    }
    for py in a.y..a.b {
        let src = (py - y) as usize * ink.width;
        for px in a.x..a.r {
            let cov = ink.coverage[src + (px - x) as usize] as u32;
            if cov != 0 {
                s.put(px, py, color, cov);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------------- clip-path

/// Polygons' coverage over `w` × `h` pixels (a byte each), filled even-odd or non-zero.
fn coverage(polys: &[Vec<(f32, f32)>], even_odd: bool, w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h];
    let Some(cov) = picture::draw::coverage_of(polys, w as f32, h as f32) else { return out };
    let stride = cov.w + 2;
    for y in 0..cov.h {
        let mut acc = 0.0f32;
        let py = cov.y0 + y as i32;
        if py < 0 || py >= h as i32 {
            continue;
        }
        for x in 0..cov.w + 2 {
            acc += cov.a[y * stride + x];
            let px = cov.x0 + x as i32;
            if px < 0 || px >= w as i32 {
                continue;
            }
            let c = if even_odd {
                let a = acc.abs() - 2.0 * num::floor(acc.abs() / 2.0);
                if a > 1.0 { 2.0 - a } else { a }
            } else {
                acc.abs().min(1.0)
            };
            out[py as usize * w + px as usize] = (c * 255.0 + 0.5) as u8;
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------------- filters

/// One filter applied to a layer's pixels (premultiplied), its lengths scaled by `k` (page units to the layer's
/// pixels).
fn filter(px: &mut [u32], w: usize, h: usize, f: &Filter, k: f32) {
    filter_edges(px, w, h, f, k, false)
}

/// A filter, what's past the pixels' edges taken as the edges' own (`clamp`: a backdrop's, as browsers blur what's
/// behind a box) or as transparent (an element's).
fn filter_edges(px: &mut [u32], w: usize, h: usize, f: &Filter, k: f32, clamp: bool) {
    match *f {
        Filter::Blur(sigma) => blur(px, w, h, sigma * k, clamp),
        Filter::DropShadow { x, y, blur: sigma, color } => {
            // Its alpha, moved, blurred, coloured; under what's there.
            let (dx, dy) = (num::round(x * k) as i32, num::round(y * k) as i32);
            let mut sh = vec![0u32; w * h];
            let ca = color >> 24;
            for yy in 0..h as i32 {
                let sy = yy - dy;
                if sy < 0 || sy >= h as i32 {
                    continue;
                }
                for xx in 0..w as i32 {
                    let sx = xx - dx;
                    if sx < 0 || sx >= w as i32 {
                        continue;
                    }
                    let a = px[sy as usize * w + sx as usize] >> 24;
                    if a > 0 {
                        sh[yy as usize * w + xx as usize] = a << 24;
                    }
                }
            }
            blur(&mut sh, w, h, sigma * k, false);
            for (p, s) in px.iter_mut().zip(sh.iter()) {
                let a = (s >> 24) * ca / 255;
                if a == 0 {
                    continue;
                }
                let shadow = scale_pm(0xFF00_0000 | (color & 0x00FF_FFFF), a);
                *p = composite_pm(shadow, *p, 255, true);
            }
        }
        Filter::Color(code, amount) => {
            let m = colour_matrix(code, amount);
            for p in px.iter_mut() {
                let a = *p >> 24;
                if a == 0 && code != 5 {
                    continue;
                }
                let af = a as f32;
                let un = |shift: u32| if a > 0 { ((*p >> shift) & 0xFF) as f32 * 255.0 / af } else { 0.0 };
                let (r, g, b) = (un(16) / 255.0, un(8) / 255.0, un(0) / 255.0);
                let (r2, g2, b2, a2) = match m {
                    Some(mm) => (
                        mm[0] * r + mm[1] * g + mm[2] * b + mm[3],
                        mm[4] * r + mm[5] * g + mm[6] * b + mm[7],
                        mm[8] * r + mm[9] * g + mm[10] * b + mm[11],
                        af / 255.0,
                    ),
                    // opacity
                    None => (r, g, b, af / 255.0 * amount),
                };
                let a2 = a2.clamp(0.0, 1.0);
                let q = |v: f32| ((v.clamp(0.0, 1.0) * a2 * 255.0) + 0.5) as u32;
                *p = (((a2 * 255.0) + 0.5) as u32) << 24 | q(r2) << 16 | q(g2) << 8 | q(b2);
            }
        }
    }
}

/// A colour filter's matrix (rows of r, g, b: three weights and an offset), as Filter Effects defines each; None for
/// opacity (its alpha alone).
fn colour_matrix(code: u8, a: f32) -> Option<[f32; 12]> {
    Some(match code {
        // brightness
        0 => [a, 0.0, 0.0, 0.0, 0.0, a, 0.0, 0.0, 0.0, 0.0, a, 0.0],
        // contrast
        1 => {
            let o = 0.5 - 0.5 * a;
            [a, 0.0, 0.0, o, 0.0, a, 0.0, o, 0.0, 0.0, a, o]
        }
        // grayscale
        2 => {
            let s = 1.0 - a;
            [
                0.2126 + 0.7874 * s, 0.7152 - 0.7152 * s, 0.0722 - 0.0722 * s, 0.0,
                0.2126 - 0.2126 * s, 0.7152 + 0.2848 * s, 0.0722 - 0.0722 * s, 0.0,
                0.2126 - 0.2126 * s, 0.7152 - 0.7152 * s, 0.0722 + 0.9278 * s, 0.0,
            ]
        }
        // hue-rotate (degrees)
        3 => {
            let r = a * core::f32::consts::PI / 180.0;
            let (c, s) = (num::cos(r), num::sin(r));
            [
                0.213 + c * 0.787 - s * 0.213, 0.715 - c * 0.715 - s * 0.715, 0.072 - c * 0.072 + s * 0.928, 0.0,
                0.213 - c * 0.213 + s * 0.143, 0.715 + c * 0.285 + s * 0.140, 0.072 - c * 0.072 - s * 0.283, 0.0,
                0.213 - c * 0.213 - s * 0.787, 0.715 - c * 0.715 + s * 0.715, 0.072 + c * 0.928 + s * 0.072, 0.0,
            ]
        }
        // invert
        4 => {
            let k = 1.0 - 2.0 * a;
            [k, 0.0, 0.0, a, 0.0, k, 0.0, a, 0.0, 0.0, k, a]
        }
        5 => return None,
        // saturate
        6 => [
            0.213 + 0.787 * a, 0.715 - 0.715 * a, 0.072 - 0.072 * a, 0.0,
            0.213 - 0.213 * a, 0.715 + 0.285 * a, 0.072 - 0.072 * a, 0.0,
            0.213 - 0.213 * a, 0.715 - 0.715 * a, 0.072 + 0.928 * a, 0.0,
        ],
        // sepia
        _ => {
            let s = 1.0 - a;
            [
                0.393 + 0.607 * s, 0.769 - 0.769 * s, 0.189 - 0.189 * s, 0.0,
                0.349 - 0.349 * s, 0.686 + 0.314 * s, 0.168 - 0.168 * s, 0.0,
                0.272 - 0.272 * s, 0.534 - 0.534 * s, 0.131 + 0.869 * s, 0.0,
            ]
        }
    })
}

/// A Gaussian blur of standard deviation `sigma` (pixels), as three box blurs each way (what's outside counted
/// transparent, or with `clamp` as the nearest edge's pixels).
fn blur(px: &mut [u32], w: usize, h: usize, sigma: f32, clamp: bool) {
    if sigma < 0.3 || w == 0 || h == 0 {
        return;
    }
    // Three boxes' widths whose blur is near the Gaussian's (Kovesi's way).
    let ideal = num::sqrt(12.0 * sigma * sigma / 3.0 + 1.0);
    let mut lo = num::floor(ideal) as i32;
    if lo % 2 == 0 {
        lo -= 1;
    }
    let up = lo + 2;
    let m = num::round((12.0 * sigma * sigma - 3.0 * (lo * lo) as f32 - 12.0 * lo as f32 - 9.0) / (-4.0 * lo as f32 - 4.0)) as i32;
    let radii: Vec<usize> = (0..3).map(|i| (((if i < m { lo } else { up }) - 1) / 2).max(0) as usize).collect();
    // Each channel apart (premultiplied), as u32 sums.
    let mut planes: Vec<Vec<u32>> = (0..4).map(|c| px.iter().map(|p| (p >> (24 - 8 * c)) & 0xFF).collect()).collect();
    let mut line = vec![0u32; w.max(h)];
    let mut tmp = vec![0u32; w.max(h)];
    for plane in planes.iter_mut() {
        for &r in &radii {
            if r == 0 {
                continue;
            }
            for y in 0..h {
                for x in 0..w {
                    line[x] = plane[y * w + x];
                }
                box_line(&line[..w], &mut tmp[..w], r, clamp);
                for x in 0..w {
                    plane[y * w + x] = tmp[x];
                }
            }
            for x in 0..w {
                for y in 0..h {
                    line[y] = plane[y * w + x];
                }
                box_line(&line[..h], &mut tmp[..h], r, clamp);
                for y in 0..h {
                    plane[y * w + x] = tmp[y];
                }
            }
        }
    }
    for (i, p) in px.iter_mut().enumerate() {
        *p = planes[0][i].min(255) << 24 | planes[1][i].min(255) << 16 | planes[2][i].min(255) << 8 | planes[3][i].min(255);
    }
}

/// A line's values averaged over 2r + 1 (what's past its ends counted 0, or with `clamp` as its end's value).
fn box_line(v: &[u32], out: &mut [u32], r: usize, clamp: bool) {
    let n = v.len();
    if n == 0 {
        return;
    }
    let d = (2 * r + 1) as u32;
    let at = |i: isize| -> u32 {
        if i < 0 {
            if clamp { v[0] } else { 0 }
        } else if i as usize >= n {
            if clamp { v[n - 1] } else { 0 }
        } else {
            v[i as usize]
        }
    };
    let r = r as isize;
    let mut sum: u32 = (-r..r).map(at).sum();
    for i in 0..n as isize {
        sum += at(i + r);
        out[i as usize] = (sum + d / 2) / d;
        sum -= at(i - r);
    }
}

/// backdrop-filter: what's drawn under a box, within its rounded border box, filtered there.
fn backdrop(c: &mut Canvas, filters: &[Filter], m: Map, r: Rect, rad: [f32; 4], clip: Area, height: usize) {
    let s = Shape::new(m.x(r.x), m.y(r.y), m.len(r.w), m.len(r.h), rad.map(|v| m.len(v)));
    let reach = num::ceil(m.len(filter_reach(filters))) as i32;
    let a = s.area().intersect(&clip);
    if a.empty() {
        return;
    }
    // (What's around it too, for a blur to take from.)
    let g = Area { x: (a.x - reach).max(0), y: (a.y - reach).max(0), r: (a.r + reach).min(c.w as i32), b: (a.b + reach).min(height as i32) };
    let (w, h) = ((g.r - g.x) as usize, (g.b - g.y) as usize);
    if w == 0 || h == 0 {
        return;
    }
    let mut px = vec![0u32; w * h];
    for y in 0..h {
        for x in 0..w {
            let p = c.px[(g.y as usize + y) * c.w + g.x as usize + x];
            px[y * w + x] = if c.layer { p } else { 0xFF00_0000 | p };
        }
    }
    for f in filters {
        filter_edges(&mut px, w, h, f, m.k, true);
    }
    for y in a.y..a.b {
        for x in a.x..a.r {
            let cover = s.cover(x, y);
            if cover == 0 {
                continue;
            }
            let p = px[(y - g.y) as usize * w + (x - g.x) as usize];
            let i = y as usize * c.w + x as usize;
            if c.layer {
                // (Replaced, by its coverage.)
                let old = c.px[i];
                c.px[i] = composite_pm(scale_pm(old, 255 - cover), scale_pm(p, cover), 255, true);
            } else {
                let a = p >> 24;
                let un = if a > 0 && a < 255 { 0xFF00_0000 | (p & 0x00FF_FFFF) } else { p };
                c.px[i] = over(c.px[i], 0xFF00_0000 | (un & 0x00FF_FFFF), cover);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(px: &mut [u32], w: usize) -> Canvas<'_> {
        Canvas { px, w, layer: false, mask: None }
    }

    #[test]
    fn colours_mix_by_their_alpha_and_cover() {
        assert_eq!(over(0x000000, 0xFFFF_FFFF, 255), 0xFFFFFF);
        assert_eq!(over(0x000000, 0x80FF_FFFF, 255), 0x808080);
        assert_eq!(over(0x123456, 0x00FF_FFFF, 255), 0x123456);
        assert_eq!(over(0x000000, 0xFFFF_FFFF, 0), 0x000000);
    }

    #[test]
    fn a_rounded_rectangle_leaves_its_corners_out() {
        let mut px = vec![0u32; 20 * 20];
        shape(&mut canvas(&mut px, 20), &Shape::new(0.0, 0.0, 20.0, 20.0, [8.0; 4]), None, Area::new(0, 0, 20, 20), 0xFFFF_FFFF);
        assert_eq!(px[0], 0, "the corner itself stays");
        assert_eq!(px[10 * 20 + 10], 0xFFFFFF, "the middle is filled");
        assert_eq!(px[10 * 20], 0xFFFFFF, "an edge's middle is filled");
    }

    #[test]
    fn a_border_leaves_its_inside_as_it_was() {
        let mut px = vec![0x111111u32; 30 * 30];
        let outer = Shape::new(0.0, 0.0, 30.0, 30.0, [6.0; 4]);
        let inner = Shape::new(2.0, 2.0, 26.0, 26.0, [4.0; 4]);
        shape(&mut canvas(&mut px, 30), &outer, Some(&inner), Area::new(0, 0, 30, 30), 0xFFFF_0000);
        assert_eq!(px[15 * 30 + 15], 0x111111);
        assert_eq!(px[15 * 30], 0xFF0000);
        assert_eq!(px[15 * 30 + 29], 0xFF0000);
        assert_eq!(px[29 * 30 + 15], 0xFF0000);
    }

    #[test]
    fn a_gradient_goes_from_its_first_colour_to_its_last() {
        let g = Gradient {
            kind: GradientKind::Linear { angle: 180.0, corner: None },
            repeating: false,
            stops: vec![effects::Stop { color: 0xFF00_0000, at: None, hint: false }, effects::Stop { color: 0xFFFF_FFFF, at: None, hint: false }],
        };
        let s = Sampler::new(&g, 10.0, 100.0, 1.0);
        assert!(s.colour(s.place(5.0, 0.5)) & 0xFF < 0x10, "the top is black");
        assert!(s.colour(s.place(5.0, 99.5)) & 0xFF > 0xF0, "the bottom is white");
        // (Half way: half and half.)
        let mid = s.colour(s.place(5.0, 50.0)) & 0xFF;
        assert!((0x7C..=0x83).contains(&mid), "the middle is grey: {mid:x}");
    }

    #[test]
    fn a_see_through_colour_fades_without_going_grey() {
        // transparent to red: half way is red at half alpha (premultiplied mixing), not a darker red.
        let g = Gradient {
            kind: GradientKind::Linear { angle: 90.0, corner: None },
            repeating: false,
            stops: vec![effects::Stop { color: 0x0000_0000, at: None, hint: false }, effects::Stop { color: 0xFFFF_0000, at: None, hint: false }],
        };
        let s = Sampler::new(&g, 100.0, 10.0, 1.0);
        let c = s.colour(50.0);
        assert_eq!(c & 0x00FF_FFFF, 0xFF0000, "{c:08x}");
        assert!((0x7C..=0x83).contains(&(c >> 24)), "{c:08x}");
    }

    #[test]
    fn a_blur_spreads_a_dot_and_keeps_its_ink() {
        let (w, h) = (21, 21);
        let mut px = vec![0u32; w * h];
        px[10 * w + 10] = 0xFF00_0000;
        blur(&mut px, w, h, 2.0, false);
        let total: u32 = px.iter().map(|p| p >> 24).sum();
        assert!((230..=280).contains(&total), "{total}");
        assert!(px[10 * w + 10] >> 24 < 40 && px[10 * w + 12] >> 24 > 0);
    }
}
