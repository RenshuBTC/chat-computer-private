//! A laid-out page drawn into pixels (0xRRGGBB, row after row): its rectangles (rounded, see-through), borders,
//! gradients, pictures (made to fill their boxes) and words (the text engine's ink, kept between drawings, so a page
//! scrolls without its words being drawn again).

use crate::layout::{Item, Laid, Paint};
use crate::num;
use crate::values::Color;
use alloc::collections::BTreeMap;
use alloc::string::String;
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

/// The page's pictures, decoded: the `n`th's pixels (RGB, row after row), width and height, once it has come.
pub trait Shown {
    fn picture(&self, n: usize) -> Option<(&[u8], u32, u32)>;
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
    face: u8,
    size: i32,
    line: i32,
}

/// Draws pages, keeping their words' ink.
pub struct Painter {
    inks: BTreeMap<InkKey, (Ink, u64)>,
    /// How many drawings there have been (to let go of ink unused for a while).
    round: u64,
    bytes: usize,
}

/// The most of words' ink kept.
const MOST_INK_BYTES: usize = 48 << 20;

impl Default for Painter {
    fn default() -> Self {
        Painter::new()
    }
}

impl Painter {
    pub fn new() -> Painter {
        Painter { inks: BTreeMap::new(), round: 0, bytes: 0 }
    }

    /// Draws `laid` with the page's point (0, `scroll`) at (`x`, `y`) of the surface: its background first (when
    /// `background`), then each item, in order, within its clip and the surface's.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, ty: &Type, laid: &Laid, x: i32, y: i32, scroll: f32, surface: &mut Surface, pictures: &dyn Shown, background: bool) {
        self.round += 1;
        let scroll = num::round(scroll) as i32;
        let (cx, cy, cw, ch) = surface.clip;
        let clip = Area::new(cx, cy, cw, ch).within(surface.width as i32, surface.height as i32);
        if clip.empty() {
            return;
        }
        if background {
            fill(surface, clip, 0xFF00_0000 | (laid.background & 0x00FF_FFFF), 255);
        }
        // The page's top left on the surface.
        let (ox, oy) = (x, y - scroll);
        for item in &laid.items {
            let shown = self.place(item, ox, oy).intersect(&clip);
            if shown.empty() {
                continue;
            }
            let c = &item.clip;
            let item_clip = if c.w >= 1.0e6 { clip } else { Area::from_f(c.x + ox as f32, c.y + oy as f32, c.w, c.h).intersect(&clip) };
            if item_clip.empty() {
                continue;
            }
            match &item.paint {
                Paint::Rect { x, y, w, h, color, radius } => {
                    if alpha(*color) == 0 {
                        continue;
                    }
                    let r = Shape::new(x + ox as f32, y + oy as f32, *w, *h, *radius);
                    shape(surface, &r, None, item_clip, *color);
                }
                Paint::Border { x, y, w, h, width, color, radius } => {
                    let outer = Shape::new(x + ox as f32, y + oy as f32, *w, *h, *radius);
                    let inner = Shape::new(x + ox as f32 + width, y + oy as f32 + width, w - 2.0 * width, h - 2.0 * width, (radius - width).max(0.0));
                    shape(surface, &outer, Some(&inner), item_clip, *color);
                }
                Paint::Gradient { x, y, w, h, angle, stops, radius } => {
                    let r = Shape::new(x + ox as f32, y + oy as f32, *w, *h, *radius);
                    gradient(surface, &r, *angle, stops, item_clip);
                }
                Paint::Picture { x, y, w, h, n, cover, radius } => {
                    if let Some((rgb, pw, ph)) = pictures.picture(*n) {
                        let r = Shape::new(x + ox as f32, y + oy as f32, *w, *h, *radius);
                        picture(surface, &r, rgb, pw, ph, *cover, item_clip);
                    }
                }
                Paint::Text { x, y, text, face, size, line, color, .. } => {
                    let ink = self.ink(ty, text, *face, *size, *line);
                    let (tx, ty_) = (num::round(x + ox as f32) as i32 - ink.left as i32, num::round(y + oy as f32) as i32);
                    words(surface, ink, tx, ty_, *color, item_clip);
                }
            }
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

    /// An item's place on the surface (roughly: its rectangle, for leaving out what's off the surface).
    fn place(&self, item: &Item, ox: i32, oy: i32) -> Area {
        let (x, y, w, h) = match &item.paint {
            Paint::Rect { x, y, w, h, .. } | Paint::Border { x, y, w, h, .. } | Paint::Gradient { x, y, w, h, .. } | Paint::Picture { x, y, w, h, .. } => {
                (*x, *y, *w, *h)
            }
            // (Words reach a little past their box: slanted letters, accents.)
            Paint::Text { x, y, w, line, size, .. } => (*x - size * 0.5, *y - size * 0.3, *w + *size, *line + size * 0.6),
        };
        Area::from_f(x + ox as f32, y + oy as f32, w, h)
    }

    fn ink(&mut self, ty: &Type, text: &str, face: Face, size: f32, line: f32) -> &Ink {
        let key = InkKey { text: String::from(text), face: face as u8, size: (size * 256.0) as i32, line: (line * 256.0) as i32 };
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

fn fill(s: &mut Surface, a: Area, color: Color, cover: u32) {
    for y in a.y..a.b {
        let row = y as usize * s.width;
        for p in &mut s.pixels[row + a.x as usize..row + a.r as usize] {
            *p = over(*p, color, cover);
        }
    }
}

/// A rectangle with rounded corners, in fractions of pixels.
struct Shape {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
}

impl Shape {
    fn new(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Shape {
        let w = w.max(0.0);
        let h = h.max(0.0);
        Shape { x, y, w, h, radius: radius.min(w / 2.0).min(h / 2.0).max(0.0) }
    }

    fn area(&self) -> Area {
        Area::from_f(self.x, self.y, self.w, self.h)
    }

    /// Whether a point is inside.
    fn inside(&self, px: f32, py: f32) -> bool {
        if px < self.x || py < self.y || px >= self.x + self.w || py >= self.y + self.h {
            return false;
        }
        let r = self.radius;
        if r <= 0.0 {
            return true;
        }
        // (Within a corner's square: inside its circle.)
        let cx = if px < self.x + r { self.x + r } else if px > self.x + self.w - r { self.x + self.w - r } else { return true };
        let cy = if py < self.y + r { self.y + r } else if py > self.y + self.h - r { self.y + self.h - r } else { return true };
        let (dx, dy) = (px - cx, py - cy);
        dx * dx + dy * dy <= r * r
    }

    /// How much of the pixel at (x, y) is inside, of 255: whole pixels well inside or outside at once, the edges
    /// sampled 4 × 4.
    fn cover(&self, x: i32, y: i32) -> u32 {
        let (fx, fy) = (x as f32, y as f32);
        let r = self.radius;
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
fn shape(s: &mut Surface, outer: &Shape, hole: Option<&Shape>, clip: Area, color: Color) {
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
        let row = y as usize * s.width;
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
                let p = &mut s.pixels[row + x as usize];
                *p = over(*p, color, cover);
            }
            x += 1;
        }
    }
}

/// A linear gradient over a shape: `angle` degrees (0 up, 90 right, 180 down), its colours at their stops (0..1;
/// below 0 for one not given, spread evenly).
fn gradient(s: &mut Surface, r: &Shape, angle: f32, stops: &[(Color, f32)], clip: Area) {
    if stops.is_empty() {
        return;
    }
    let a = r.area().intersect(&clip);
    if a.empty() {
        return;
    }
    // The stops' places, the ones not given spread between those that are.
    let mut at: Vec<f32> = stops.iter().map(|s| s.1).collect();
    let n = at.len();
    if at[0] < 0.0 {
        at[0] = 0.0;
    }
    if at[n - 1] < 0.0 {
        at[n - 1] = 1.0;
    }
    let mut i = 1;
    while i < n {
        if at[i] < 0.0 {
            let start = i - 1;
            let mut end = i;
            while at[end] < 0.0 {
                end += 1;
            }
            for k in i..end {
                at[k] = at[start] + (at[end] - at[start]) * (k - start) as f32 / (end - start) as f32;
            }
            i = end;
        }
        i += 1;
    }
    for k in 1..n {
        at[k] = at[k].max(at[k - 1]);
    }
    // The gradient line, through the centre, long enough to reach the corners (as CSS makes it).
    let rad = angle * core::f32::consts::PI / 180.0;
    let (dx, dy) = (num::sin(rad), -num::cos(rad));
    let half = (r.w * dx.abs() + r.h * dy.abs()) / 2.0;
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    for y in a.y..a.b {
        let row = y as usize * s.width;
        for x in a.x..a.r {
            let cover = r.cover(x, y);
            if cover == 0 {
                continue;
            }
            let t = if half > 0.0 { (((x as f32 + 0.5 - cx) * dx + (y as f32 + 0.5 - cy) * dy) / (2.0 * half) + 0.5).clamp(0.0, 1.0) } else { 0.0 };
            let k = at.iter().position(|&p| p >= t).unwrap_or(n - 1);
            let color = if k == 0 {
                stops[0].0
            } else {
                let (p0, p1) = (at[k - 1], at[k]);
                let f = if p1 > p0 { (t - p0) / (p1 - p0) } else { 1.0 };
                lerp(stops[k - 1].0, stops[k].0, f)
            };
            let p = &mut s.pixels[row + x as usize];
            *p = over(*p, color, cover);
        }
    }
}

fn lerp(a: Color, b: Color, f: f32) -> Color {
    let f = (f.clamp(0.0, 1.0) * 256.0) as u32;
    let ch = |shift: u32| {
        let (x, y) = ((a >> shift) & 0xFF, (b >> shift) & 0xFF);
        ((x * (256 - f) + y * f) >> 8) << shift
    };
    ch(24) | ch(16) | ch(8) | ch(0)
}

/// A picture made to fill a shape: stretched to it, or (`cover`) as big as it takes to cover it, its middle shown.
fn picture(s: &mut Surface, r: &Shape, rgb: &[u8], pw: u32, ph: u32, cover: bool, clip: Area) {
    if pw == 0 || ph == 0 || r.w <= 0.0 || r.h <= 0.0 || rgb.len() < pw as usize * ph as usize * 3 {
        return;
    }
    let a = r.area().intersect(&clip);
    if a.empty() {
        return;
    }
    // Which part of the picture shows: all of it, or (cover) the middle that has the shape's proportions.
    let (mut sx, mut sy, mut sw, mut sh) = (0.0f32, 0.0f32, pw as f32, ph as f32);
    if cover {
        let want = r.w / r.h;
        let have = pw as f32 / ph as f32;
        if have > want {
            sw = ph as f32 * want;
            sx = (pw as f32 - sw) / 2.0;
        } else {
            sh = pw as f32 / want;
            sy = (ph as f32 - sh) / 2.0;
        }
    }
    let (kx, ky) = (sw / r.w, sh / r.h);
    for y in a.y..a.b {
        let v = ((sy + (y as f32 + 0.5 - r.y) * ky) as i64).clamp(0, ph as i64 - 1) as usize;
        let row = y as usize * s.width;
        let src = &rgb[v * pw as usize * 3..];
        for x in a.x..a.r {
            let c = if r.radius > 0.0 { r.cover(x, y) } else { 255 };
            if c == 0 {
                continue;
            }
            let u = ((sx + (x as f32 + 0.5 - r.x) * kx) as i64).clamp(0, pw as i64 - 1) as usize;
            let color = 0xFF00_0000 | (src[u * 3] as u32) << 16 | (src[u * 3 + 1] as u32) << 8 | src[u * 3 + 2] as u32;
            let p = &mut s.pixels[row + x as usize];
            *p = over(*p, color, c);
        }
    }
}

/// Words' ink at (x, y), in `color`.
fn words(s: &mut Surface, ink: &Ink, x: i32, y: i32, color: Color, clip: Area) {
    let a = Area::new(x, y, ink.width as i32, ink.height as i32).intersect(&clip);
    if a.empty() || alpha(color) == 0 {
        return;
    }
    for py in a.y..a.b {
        let src = (py - y) as usize * ink.width;
        let row = py as usize * s.width;
        for px in a.x..a.r {
            let cov = ink.coverage[src + (px - x) as usize] as u32;
            if cov != 0 {
                let p = &mut s.pixels[row + px as usize];
                *p = over(*p, color, cov);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_mix_by_their_alpha_and_cover() {
        assert_eq!(over(0x000000, 0xFFFF_FFFF, 255), 0xFFFFFF);
        assert_eq!(over(0x000000, 0x80FF_FFFF, 255), 0x808080);
        assert_eq!(over(0x123456, 0x00FF_FFFF, 255), 0x123456);
        assert_eq!(over(0x000000, 0xFFFF_FFFF, 0), 0x000000);
    }

    #[test]
    fn a_rounded_rectangle_leaves_its_corners_out() {
        let mut px = alloc::vec![0u32; 20 * 20];
        let mut s = Surface { pixels: &mut px, width: 20, height: 20, clip: (0, 0, 20, 20) };
        shape(&mut s, &Shape::new(0.0, 0.0, 20.0, 20.0, 8.0), None, Area::new(0, 0, 20, 20), 0xFFFF_FFFF);
        assert_eq!(px[0], 0, "the corner itself stays");
        assert_eq!(px[10 * 20 + 10], 0xFFFFFF, "the middle is filled");
        assert_eq!(px[10 * 20], 0xFFFFFF, "an edge's middle is filled");
    }

    #[test]
    fn a_border_leaves_its_inside_as_it_was() {
        let mut px = alloc::vec![0x111111u32; 30 * 30];
        let mut s = Surface { pixels: &mut px, width: 30, height: 30, clip: (0, 0, 30, 30) };
        let outer = Shape::new(0.0, 0.0, 30.0, 30.0, 6.0);
        let inner = Shape::new(2.0, 2.0, 26.0, 26.0, 4.0);
        shape(&mut s, &outer, Some(&inner), Area::new(0, 0, 30, 30), 0xFFFF_0000);
        assert_eq!(px[15 * 30 + 15], 0x111111);
        assert_eq!(px[15 * 30], 0xFF0000);
        assert_eq!(px[15 * 30 + 29], 0xFF0000);
        assert_eq!(px[29 * 30 + 15], 0xFF0000);
    }

    #[test]
    fn a_gradient_goes_from_its_first_colour_to_its_last() {
        let mut px = alloc::vec![0u32; 10 * 100];
        let mut s = Surface { pixels: &mut px, width: 10, height: 100, clip: (0, 0, 10, 100) };
        gradient(&mut s, &Shape::new(0.0, 0.0, 10.0, 100.0, 0.0), 180.0, &[(0xFF00_0000, -1.0), (0xFFFF_FFFF, -1.0)], Area::new(0, 0, 10, 100));
        assert!(px[5] & 0xFF < 0x10, "the top is black: {:06x}", px[5]);
        assert!(px[99 * 10 + 5] & 0xFF > 0xF0, "the bottom is white: {:06x}", px[99 * 10 + 5]);
    }
}
