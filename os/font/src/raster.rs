//! Fills outlines with exact anti-aliasing: each pixel gets the fraction of its area that the shape
//! covers, computed from the area every edge sweeps in each pixel it crosses. Integer arithmetic only
//! (the kernel uses no floating point); coordinates are in 1/256 of a pixel.
//!
//! For every pixel, two numbers are gathered: `cover`, the total height of edge passing through it
//! (signed by direction), and `area`, twice the area those edges leave to their left inside it. A sweep
//! along each row then gives each pixel's coverage: everything to the left (the running sum of cover)
//! minus what this pixel's own edges leave uncovered. Overlapping shapes add up and are capped at full.

use alloc::vec;
use alloc::vec::Vec;

/// One pixel, in this module's fixed-point units.
pub const ONE: i32 = 256;

pub struct Raster {
    w: usize,
    h: usize,
    cover: Vec<i32>,
    area: Vec<i32>,
}

fn muldiv(a: i32, b: i32, c: i32) -> i32 {
    (a as i64 * b as i64 / c as i64) as i32
}

fn isqrt(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

impl Raster {
    pub fn new(w: usize, h: usize) -> Raster {
        Raster { w, h, cover: vec![0; w * h], area: vec![0; w * h] }
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    /// A straight edge from (x0, y0) to (x1, y1), y pointing down.
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        if y0 == y1 {
            return; // horizontal edges cover nothing
        }
        // Always work from the top end down, so an edge gives exactly the same result whichever way it
        // was drawn; only the sign (`dir`) differs.
        let (x0, y0, x1, y1, dir) = if y0 < y1 { (x0, y0, x1, y1, 1) } else { (x1, y1, x0, y0, -1) };
        let bottom = self.h as i32 * ONE;
        if y1 <= 0 || y0 >= bottom {
            return;
        }
        let first = (y0.max(0) / ONE) as usize;
        let last = ((y1.min(bottom) - 1) / ONE) as usize;
        let x_at = |y: i32| -> i32 {
            if y == y0 {
                x0
            } else if y == y1 {
                x1
            } else {
                x0 + muldiv(y - y0, x1 - x0, y1 - y0)
            }
        };
        for row in first..=last {
            let top = row as i32 * ONE;
            let (ya, yb) = (y0.max(top), y1.min(top + ONE));
            if ya < yb {
                self.row(row, x_at(ya), ya - top, x_at(yb), yb - top, dir);
            }
        }
    }

    /// A quadratic curve from (x0, y0) to (x2, y2) pulled towards (x1, y1), drawn as short lines
    /// that stay within 1/16 of a pixel of the true curve.
    pub fn quad(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, x2: i32, y2: i32) {
        // The curve strays from its chord by at most |p0 - 2p1 + p2| / 4; n pieces divide that by n².
        let ddx = (x0 as i64 - 2 * x1 as i64 + x2 as i64).unsigned_abs();
        let ddy = (y0 as i64 - 2 * y1 as i64 + y2 as i64).unsigned_abs();
        let tolerance = (ONE / 16) as u64;
        let need = ddx.max(ddy).div_ceil(4 * tolerance);
        let mut n = isqrt(need);
        if n * n < need {
            n += 1;
        }
        let n = n.clamp(1, 100) as i64;
        let nn = n * n;
        let (mut px, mut py) = (x0, y0);
        for i in 1..=n {
            let (t, u) = (i, n - i);
            let x = (u * u * x0 as i64 + 2 * t * u * x1 as i64 + t * t * x2 as i64).div_euclid(nn) as i32;
            let y = (u * u * y0 as i64 + 2 * t * u * y1 as i64 + t * t * y2 as i64).div_euclid(nn) as i32;
            self.line(px, py, x, y);
            px = x;
            py = y;
        }
    }

    /// An edge piece within one row, going down; `ya` and `yb` are measured from the row's top.
    fn row(&mut self, row: usize, xa: i32, ya: i32, xb: i32, yb: i32, dir: i32) {
        let right_edge = self.w as i32 * ONE;
        // Split where the piece crosses the bitmap's left or right side.
        let mut cuts = [0i32; 2];
        let mut n = 0;
        for side in [0, right_edge] {
            if (xa < side && side < xb) || (xb < side && side < xa) {
                cuts[n] = side;
                n += 1;
            }
        }
        if n == 2 && (xa < xb) != (cuts[0] < cuts[1]) {
            cuts.swap(0, 1);
        }
        let (mut px, mut py) = (xa, ya);
        for &side in &cuts[..n] {
            let sy = ya + muldiv(side - xa, yb - ya, xb - xa);
            self.span(row, px, py, side, sy, dir);
            px = side;
            py = sy;
        }
        self.span(row, px, py, xb, yb, dir);
    }

    /// An edge piece that is entirely left of, inside, or right of the bitmap.
    fn span(&mut self, row: usize, xa: i32, ya: i32, xb: i32, yb: i32, dir: i32) {
        let dy = yb - ya;
        if dy == 0 {
            return;
        }
        let right_edge = self.w as i32 * ONE;
        if xa.max(xb) <= 0 {
            self.add(row, 0, dy * dir, 0); // left of the bitmap: everything visible is to its right
            return;
        }
        if xa.min(xb) >= right_edge {
            return; // right of the bitmap: nothing visible is to its right
        }
        if xa == xb {
            let c = xa / ONE;
            self.add(row, c as usize, dy * dir, 2 * (xa - c * ONE));
            return;
        }
        // Walk the columns from xa to xb.
        let rightwards = xb > xa;
        let mut c = if rightwards { xa.div_euclid(ONE) } else { (xa - 1).div_euclid(ONE) };
        let (mut px, mut py) = (xa, ya);
        loop {
            let side = if rightwards { (c + 1) * ONE } else { c * ONE };
            let done = if rightwards { xb <= side } else { xb >= side };
            let (nx, ny) = if done { (xb, yb) } else { (side, ya + muldiv(side - xa, yb - ya, xb - xa)) };
            let left = c * ONE;
            self.add(row, c as usize, (ny - py) * dir, (px - left) + (nx - left));
            if done {
                break;
            }
            px = nx;
            py = ny;
            c += if rightwards { 1 } else { -1 };
        }
    }

    fn add(&mut self, row: usize, col: usize, dy: i32, x_sum: i32) {
        if col >= self.w {
            return;
        }
        let i = row * self.w + col;
        self.cover[i] += dy;
        self.area[i] += dy * x_sum;
    }

    /// Each pixel's coverage, 0 (empty) to 255 (full), row by row.
    pub fn coverage(&self, out: &mut [u8]) {
        for y in 0..self.h {
            let mut left = 0i32;
            for x in 0..self.w {
                let i = y * self.w + x;
                // In 1/65536 of a pixel's area.
                let c = (left + self.cover[i]) * ONE - self.area[i] / 2;
                left += self.cover[i];
                let a = c.unsigned_abs().min(65536);
                out[i] = ((a * 255 + 32768) >> 16) as u8;
            }
        }
    }
}

/// Draws a font's outlines into a [`Raster`]: scales font units to pixels, flips y to point down, and
/// places the glyph's origin (on the baseline) at (`x`, `y`), in 1/256 pixel.
pub struct Pen<'a> {
    pub raster: &'a mut Raster,
    pub x: i32,
    pub y: i32,
    /// The font size (pixels per em), in 1/256 pixel.
    pub size: i32,
    pub units_per_em: i32,
    start: (i32, i32),
    at: (i32, i32),
}

impl<'a> Pen<'a> {
    pub fn new(raster: &'a mut Raster, size: i32, units_per_em: u16) -> Pen<'a> {
        Pen { raster, x: 0, y: 0, size, units_per_em: units_per_em as i32, start: (0, 0), at: (0, 0) }
    }

    fn place(&self, x: i32, y: i32) -> (i32, i32) {
        let scale = |v: i32| -> i32 {
            let n = v as i64 * self.size as i64;
            let d = self.units_per_em as i64 * 64;
            ((n + if n >= 0 { d / 2 } else { -d / 2 }) / d) as i32
        };
        (self.x + scale(x), self.y - scale(y))
    }
}

impl crate::Outline for Pen<'_> {
    fn move_to(&mut self, x: i32, y: i32) {
        self.start = self.place(x, y);
        self.at = self.start;
    }
    fn line_to(&mut self, x: i32, y: i32) {
        let p = self.place(x, y);
        self.raster.line(self.at.0, self.at.1, p.0, p.1);
        self.at = p;
    }
    fn quad_to(&mut self, cx: i32, cy: i32, x: i32, y: i32) {
        let c = self.place(cx, cy);
        let p = self.place(x, y);
        self.raster.quad(self.at.0, self.at.1, c.0, c.1, p.0, p.1);
        self.at = p;
    }
    fn close(&mut self) {
        if self.at != self.start {
            self.raster.line(self.at.0, self.at.1, self.start.0, self.start.1);
            self.at = self.start;
        }
    }
}
