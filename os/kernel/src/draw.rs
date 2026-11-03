//! Drawing in memory: a picture of the whole screen (`Canvas`), in Chat Computer's colours (theme.css), redrawn
//! where something changed and then copied to the screen, so nothing flickers. Colours are 0xRRGGBB. Everything
//! drawn is clipped to the canvas's clip rectangle, so any part of the screen can be redrawn on its own.

use text::Ink;
use alloc::vec;
use alloc::vec::Vec;

/// The background: the middle of Claude's app on a PC, in dark mode (theme.css --ground).
pub const GROUND: u32 = 0x191919;
/// The words: a white just off pure (theme.css --ink).
pub const INK: u32 = 0xF0F0F0;
/// Your messages' bubbles (theme.css --bubble).
pub const BUBBLE: u32 = 0x252525;
/// Words that matter less: what Chat Computer is doing while it thinks (theme.css --muted).
pub const MUTED: u32 = 0x9E9E9E;
/// Lines between things (theme.css --line).
pub const LINE: u32 = 0x303030;
/// The message box, which keeps its own look (theme.css --paper, and #box in chat.css): the box itself; the thin
/// line round it; its words and caret, and its send button once there's something to send; the arrow on that
/// button; and its hint, and the send button until there's something to send.
pub const PAPER: u32 = 0x303030;
pub const BOX_LINE: u32 = 0x3D3D3C;
pub const BOX_INK: u32 = 0xF9F9F7;
pub const BOX_GROUND: u32 = 0x151515;
pub const DIM: u32 = 0x96958D;
/// Shadows (theme.css --shade).
pub const SHADE: u32 = 0x000000;

/// A point, (x, y).
pub type Point = (i32, i32);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        let (r, b) = (self.right().min(o.right()), self.bottom().min(o.bottom()));
        if r <= x || b <= y { Rect::default() } else { Rect::new(x, y, r - x, b - y) }
    }

    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        Rect::new(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }
}

/// `rgb` laid over `under` at `alpha` (of 255).
fn mix(under: u32, rgb: u32, alpha: u32) -> u32 {
    if alpha >= 255 {
        return rgb;
    }
    let inv = 255 - alpha;
    let ch = |shift: u32| ((under >> shift & 0xFF) * inv + (rgb >> shift & 0xFF) * alpha + 127) / 255;
    ch(16) << 16 | ch(8) << 8 | ch(0)
}

/// erf(x), to 1.5e-7 (Abramowitz and Stegun 7.1.26).
fn erf(x: f64) -> f64 {
    let s = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x * s;
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let poly = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
    s * (1.0 - poly * exp(-x * x))
}

/// e^x for x <= 0 (all erf needs), by halving and squaring.
fn exp(x: f64) -> f64 {
    if x < -40.0 {
        return 0.0;
    }
    let mut y = x / 1024.0;
    // e^y for small y, by its series.
    let mut term = 1.0;
    let mut sum = 1.0;
    for n in 1..8 {
        term *= y / n as f64;
        sum += term;
    }
    y = sum;
    for _ in 0..10 {
        y *= y;
    }
    y
}

/// How much (0 to 255) of pixel (`dx`, `dy`) of a corner's square is inside its rounding (a circle of `radius`
/// whose centre is `radius` in from the corner), from 4 × 4 points in it.
fn corner_coverage(radius: i32, dx: i32, dy: i32) -> u32 {
    let rr = (radius * 8) as i64 * (radius * 8) as i64;
    let mut inside = 0;
    for sy in 0..4 {
        for sx in 0..4 {
            // The point, in eighths of a pixel, from the circle's centre.
            let px = (radius - dx) as i64 * 8 - (sx * 2 + 1);
            let py = (radius - dy) as i64 * 8 - (sy * 2 + 1);
            if px * px + py * py <= rr {
                inside += 1;
            }
        }
    }
    (inside * 255 + 8) / 16
}

pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pixels: Vec<u32>,
    clip: Rect,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Canvas {
        Canvas { width, height, pixels: vec![GROUND; width * height], clip: Rect::new(0, 0, width as i32, height as i32) }
    }

    pub fn whole(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Where drawing goes from now on (within the canvas).
    pub fn set_clip(&mut self, r: Rect) {
        self.clip = r.intersect(&self.whole());
    }

    pub fn clip(&self) -> Rect {
        self.clip
    }

    pub fn fill(&mut self, r: Rect, rgb: u32) {
        let r = r.intersect(&self.clip);
        for y in r.y..r.bottom() {
            let row = y as usize * self.width;
            self.pixels[row + r.x as usize..row + r.right() as usize].fill(rgb);
        }
    }

    /// One pixel of `rgb` at `alpha` over what's there (inside the clip only).
    fn blend(&mut self, x: i32, y: i32, rgb: u32, alpha: u32) {
        if alpha == 0 || x < self.clip.x || y < self.clip.y || x >= self.clip.right() || y >= self.clip.bottom() {
            return;
        }
        let p = &mut self.pixels[y as usize * self.width + x as usize];
        *p = mix(*p, rgb, alpha);
    }

    /// A line of text's ink, in `rgb`, its top left corner at (`x`, `y`).
    pub fn ink(&mut self, x: i32, y: i32, ink: &Ink, rgb: u32) {
        let area = Rect::new(x, y, ink.width as i32, ink.height as i32).intersect(&self.clip);
        for py in area.y..area.bottom() {
            let src = (py - y) as usize * ink.width;
            let row = py as usize * self.width;
            for px in area.x..area.right() {
                let a = ink.coverage[src + (px - x) as usize] as u32;
                if a != 0 {
                    let p = &mut self.pixels[row + px as usize];
                    *p = mix(*p, rgb, a);
                }
            }
        }
    }

    /// A picture's pixels (RGB, row by row), its top left corner at (`x`, `y`).
    pub fn picture(&mut self, x: i32, y: i32, width: usize, height: usize, rgb: &[u8]) {
        let area = Rect::new(x, y, width as i32, height as i32).intersect(&self.clip);
        for py in area.y..area.bottom() {
            let src = &rgb[((py - y) as usize * width + (area.x - x) as usize) * 3..];
            let row = py as usize * self.width;
            for (i, p) in self.pixels[row + area.x as usize..row + area.right() as usize].iter_mut().enumerate() {
                *p = (src[i * 3] as u32) << 16 | (src[i * 3 + 1] as u32) << 8 | src[i * 3 + 2] as u32;
            }
        }
    }

    /// What's drawn in `r` given corners rounded to `radius` pixels: outside them, `rgb` (the ground), edges smoothed.
    pub fn round_corners(&mut self, r: Rect, radius: i32, rgb: u32) {
        let radius = radius.min(r.w / 2).min(r.h / 2).max(0);
        for dy in 0..radius {
            for dx in 0..radius {
                let outside = 255 - corner_coverage(radius, dx, dy);
                if outside == 0 {
                    continue;
                }
                self.blend(r.x + dx, r.y + dy, rgb, outside);
                self.blend(r.right() - 1 - dx, r.y + dy, rgb, outside);
                self.blend(r.x + dx, r.bottom() - 1 - dy, rgb, outside);
                self.blend(r.right() - 1 - dx, r.bottom() - 1 - dy, rgb, outside);
            }
        }
    }

    /// A rectangle with corners rounded to `radius` pixels, edges smoothed.
    pub fn round_rect(&mut self, r: Rect, radius: i32, rgb: u32) {
        let radius = radius.min(r.w / 2).min(r.h / 2).max(0);
        // The middle, and the edges between the corners: solid.
        self.fill(Rect::new(r.x + radius, r.y, r.w - 2 * radius, r.h), rgb);
        self.fill(Rect::new(r.x, r.y + radius, radius, r.h - 2 * radius), rgb);
        self.fill(Rect::new(r.right() - radius, r.y + radius, radius, r.h - 2 * radius), rgb);
        // The corners: how much of each pixel is inside the circle.
        for dy in 0..radius {
            for dx in 0..radius {
                let a = corner_coverage(radius, dx, dy);
                if a == 0 {
                    continue;
                }
                self.blend(r.x + dx, r.y + dy, rgb, a);
                self.blend(r.right() - 1 - dx, r.y + dy, rgb, a);
                self.blend(r.x + dx, r.bottom() - 1 - dy, rgb, a);
                self.blend(r.right() - 1 - dx, r.bottom() - 1 - dy, rgb, a);
            }
        }
    }

    /// A soft shadow under `r` (CSS box-shadow 0 `offset` `blur` black at `strength` of 255): `r` moved down by
    /// `offset`, blurred (a Gaussian with sigma = blur / 2), drawn over what's there.
    pub fn shadow(&mut self, r: Rect, offset: i32, blur: i32, strength: u32) {
        let sigma = (blur as f64 / 2.0).max(0.5);
        let reach = (blur * 3 / 2).max(1);
        let area = Rect::new(r.x - reach, r.y + offset - reach, r.w + 2 * reach, r.h + 2 * reach).intersect(&self.clip);
        if area.is_empty() {
            return;
        }
        let k = 1.0 / (sigma * core::f64::consts::SQRT_2);
        // How much of the blurred rectangle each column and each row gets (0..=1024), separately: their product is
        // the blur of a rectangle.
        let profile = |from: i32, to: i32, at: i32| -> u32 {
            let c = at as f64 + 0.5;
            let v = 0.5 * (erf((to as f64 - c) * k) - erf((from as f64 - c) * k));
            (v.clamp(0.0, 1.0) * 1024.0 + 0.5) as u32
        };
        let columns: Vec<u32> = (area.x..area.right()).map(|x| profile(r.x, r.right(), x)).collect();
        for y in area.y..area.bottom() {
            let fy = profile(r.y + offset, r.bottom() + offset, y);
            if fy == 0 {
                continue;
            }
            for (i, &fx) in columns.iter().enumerate() {
                let a = (fx * fy * strength + (1 << 19)) >> 20;
                if a != 0 {
                    self.blend(area.x + i as i32, y, SHADE, a);
                }
            }
        }
    }

    /// A filled circle (centre and radius in 1/8 pixel), at `alpha` of 255, edges smoothed.
    pub fn circle(&mut self, cx8: i32, cy8: i32, r8: i32, rgb: u32, alpha: u32) {
        let (x0, x1) = ((cx8 - r8) >> 3, (cx8 + r8 + 7) >> 3);
        let (y0, y1) = ((cy8 - r8) >> 3, (cy8 + r8 + 7) >> 3);
        let rr = r8 as i64 * r8 as i64;
        for y in y0..y1 {
            for x in x0..x1 {
                let mut inside = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let px = (x * 8 + sx * 2 + 1 - cx8) as i64;
                        let py = (y * 8 + sy * 2 + 1 - cy8) as i64;
                        if px * px + py * py <= rr {
                            inside += 1;
                        }
                    }
                }
                self.blend(x, y, rgb, (inside * alpha + 8) / 16);
            }
        }
    }

    /// Lines with round ends (an icon's strokes): each from one point to another, in 1/8 pixel, `width8` wide.
    pub fn strokes(&mut self, segments: &[(Point, Point)], width8: i32, rgb: u32) {
        let half = width8 as f64 / 2.0;
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &((ax, ay), (bx, by)) in segments {
            x0 = x0.min(ax.min(bx));
            y0 = y0.min(ay.min(by));
            x1 = x1.max(ax.max(bx));
            y1 = y1.max(ay.max(by));
        }
        let pad = width8 / 2 + 8;
        for y in (y0 - pad) >> 3..(y1 + pad + 7) >> 3 {
            for x in (x0 - pad) >> 3..(x1 + pad + 7) >> 3 {
                let mut inside = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let (px, py) = ((x * 8 + sx * 2 + 1) as f64, (y * 8 + sy * 2 + 1) as f64);
                        let near = segments.iter().any(|&((ax, ay), (bx, by))| {
                            let (ax, ay, bx, by) = (ax as f64, ay as f64, bx as f64, by as f64);
                            let (dx, dy) = (bx - ax, by - ay);
                            let len2 = dx * dx + dy * dy;
                            let t = if len2 == 0.0 { 0.0 } else { (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0) };
                            let (qx, qy) = (ax + t * dx - px, ay + t * dy - py);
                            qx * qx + qy * qy <= half * half
                        });
                        if near {
                            inside += 1;
                        }
                    }
                }
                self.blend(x, y, rgb, (inside * 255 + 8) / 16);
            }
        }
    }
}
