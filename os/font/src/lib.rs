//! ClaudeOS's own TrueType font reader: which glyph a character uses, how wide it is, and its outline.
//!
//! It reads 'glyf' outlines (made of straight lines and quadratic curves), including glyphs built from
//! other glyphs (é = e + ´). It ignores the hinting programs fonts carry: ClaudeOS never runs code that
//! comes inside a font. Everything is bounds-checked; a damaged font gives `None`, never a crash.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod raster;

use alloc::vec::Vec;

fn u8_at(d: &[u8], o: usize) -> Option<u8> {
    d.get(o).copied()
}
fn i8_at(d: &[u8], o: usize) -> Option<i8> {
    d.get(o).map(|b| *b as i8)
}
fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(o)?, *d.get(o + 1)?]))
}
fn i16_at(d: &[u8], o: usize) -> Option<i16> {
    u16_at(d, o).map(|v| v as i16)
}
fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*d.get(o)?, *d.get(o + 1)?, *d.get(o + 2)?, *d.get(o + 3)?]))
}

/// Receives an outline. Coordinates are in font units × 64 (so composite glyphs' scaling loses
/// nothing), y pointing up.
pub trait Outline {
    fn move_to(&mut self, x: i32, y: i32);
    fn line_to(&mut self, x: i32, y: i32);
    fn quad_to(&mut self, cx: i32, cy: i32, x: i32, y: i32);
    fn close(&mut self);
}

/// Vertical measures, in font units.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    pub line_gap: i16,
    pub x_height: i16,
    pub cap_height: i16,
}

#[derive(Clone, Copy)]
enum Cmap {
    Format4(usize),
    Format12(usize),
}

pub struct Font<'a> {
    data: &'a [u8],
    glyf: &'a [u8],
    loca: &'a [u8],
    long_loca: bool,
    hmtx: &'a [u8],
    h_metrics: u16,
    glyphs: u16,
    cmap: Option<Cmap>,
    pub metrics: Metrics,
}

/// A 2×2 matrix in 2.14 fixed point plus an offset in font units × 64.
#[derive(Clone, Copy)]
struct Transform {
    xx: i64,
    xy: i64,
    yx: i64,
    yy: i64,
    dx: i64,
    dy: i64,
}

const ONE_2_14: i64 = 1 << 14;

impl Transform {
    const IDENTITY: Transform = Transform { xx: ONE_2_14, xy: 0, yx: 0, yy: ONE_2_14, dx: 0, dy: 0 };

    fn apply(&self, x: i64, y: i64) -> (i32, i32) {
        (((self.xx * x + self.xy * y) >> 14) as i32 + self.dx as i32, ((self.yx * x + self.yy * y) >> 14) as i32 + self.dy as i32)
    }

    /// This transform after `inner` (inner first).
    fn after(&self, inner: &Transform) -> Transform {
        Transform {
            xx: (self.xx * inner.xx + self.xy * inner.yx) >> 14,
            xy: (self.xx * inner.xy + self.xy * inner.yy) >> 14,
            yx: (self.yx * inner.xx + self.yy * inner.yx) >> 14,
            yy: (self.yx * inner.xy + self.yy * inner.yy) >> 14,
            dx: ((self.xx * inner.dx + self.xy * inner.dy) >> 14) + self.dx,
            dy: ((self.yx * inner.dx + self.yy * inner.dy) >> 14) + self.dy,
        }
    }
}

impl<'a> Font<'a> {
    pub fn parse(data: &'a [u8]) -> Option<Font<'a>> {
        let version = u32_at(data, 0)?;
        if version != 0x0001_0000 && version != u32::from_be_bytes(*b"true") {
            return None; // not TrueType outlines (e.g. a CFF font)
        }
        let tables = u16_at(data, 4)? as usize;
        let find = |tag: &[u8; 4]| -> Option<&'a [u8]> {
            for i in 0..tables {
                let r = 12 + 16 * i;
                if data.get(r..r + 4)? == tag {
                    let off = u32_at(data, r + 8)? as usize;
                    let len = u32_at(data, r + 12)? as usize;
                    return data.get(off..off.checked_add(len)?);
                }
            }
            None
        };
        let head = find(b"head")?;
        let maxp = find(b"maxp")?;
        let hhea = find(b"hhea")?;
        let units_per_em = u16_at(head, 18)?;
        if units_per_em == 0 {
            return None;
        }
        let mut metrics = Metrics {
            units_per_em,
            ascender: i16_at(hhea, 4)?,
            descender: i16_at(hhea, 6)?,
            line_gap: i16_at(hhea, 8)?,
            x_height: 0,
            cap_height: 0,
        };
        if let Some(os2) = find(b"OS/2") {
            // "Use typographic metrics" (fsSelection bit 7) says which line measures the font means.
            if u16_at(os2, 62).is_some_and(|s| s & (1 << 7) != 0) {
                metrics.ascender = i16_at(os2, 68)?;
                metrics.descender = i16_at(os2, 70)?;
                metrics.line_gap = i16_at(os2, 72)?;
            }
            if u16_at(os2, 0)? >= 2 {
                metrics.x_height = i16_at(os2, 86).unwrap_or(0);
                metrics.cap_height = i16_at(os2, 88).unwrap_or(0);
            }
        }
        let mut font = Font {
            data,
            glyf: find(b"glyf")?,
            loca: find(b"loca")?,
            long_loca: i16_at(head, 50)? == 1,
            hmtx: find(b"hmtx")?,
            h_metrics: u16_at(hhea, 34)?,
            glyphs: u16_at(maxp, 4)?,
            cmap: None,
            metrics,
        };
        font.cmap = find(b"cmap").and_then(|c| font.pick_cmap(c));
        Some(font)
    }

    /// The best Unicode character map: full Unicode (format 12) if present, else the basic plane
    /// (format 4). Returns its offset into the font.
    fn pick_cmap(&self, cmap: &[u8]) -> Option<Cmap> {
        let base = cmap.as_ptr() as usize - self.data.as_ptr() as usize;
        let count = u16_at(cmap, 2)? as usize;
        let mut best: Option<(u8, Cmap)> = None;
        for i in 0..count {
            let r = 4 + 8 * i;
            let (platform, encoding) = (u16_at(cmap, r)?, u16_at(cmap, r + 2)?);
            let off = u32_at(cmap, r + 4)? as usize;
            let format = u16_at(cmap, off)?;
            let unicode = platform == 0 || (platform == 3 && (encoding == 1 || encoding == 10));
            if !unicode {
                continue;
            }
            let pick = match format {
                12 => (2, Cmap::Format12(base + off)),
                4 => (1, Cmap::Format4(base + off)),
                _ => continue,
            };
            if best.is_none_or(|(rank, _)| pick.0 > rank) {
                best = Some(pick);
            }
        }
        best.map(|(_, c)| c)
    }

    /// The glyph for a character (0, the "missing" glyph, if the font doesn't have it).
    pub fn glyph_index(&self, c: char) -> u16 {
        let cp = c as u32;
        let gid = match self.cmap {
            Some(Cmap::Format4(at)) => self.format4(at, cp),
            Some(Cmap::Format12(at)) => self.format12(at, cp),
            None => None,
        };
        gid.filter(|g| *g < self.glyphs).unwrap_or(0)
    }

    fn format4(&self, at: usize, cp: u32) -> Option<u16> {
        if cp > 0xFFFF {
            return None;
        }
        let d = self.data;
        let segs = u16_at(d, at + 6)? as usize / 2;
        let ends = at + 14;
        let starts = ends + 2 * segs + 2;
        let deltas = starts + 2 * segs;
        let ranges = deltas + 2 * segs;
        // The first segment whose end is at or after the character.
        let (mut lo, mut hi) = (0, segs);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if (u16_at(d, ends + 2 * mid)? as u32) < cp {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == segs {
            return None;
        }
        let start = u16_at(d, starts + 2 * lo)? as u32;
        if cp < start {
            return None;
        }
        let delta = u16_at(d, deltas + 2 * lo)?;
        let range_at = ranges + 2 * lo;
        let range = u16_at(d, range_at)? as usize;
        if range == 0 {
            return Some((cp as u16).wrapping_add(delta));
        }
        let g = u16_at(d, range_at + range + 2 * (cp - start) as usize)?;
        if g == 0 { None } else { Some(g.wrapping_add(delta)) }
    }

    fn format12(&self, at: usize, cp: u32) -> Option<u16> {
        let d = self.data;
        let groups = u32_at(d, at + 12)? as usize;
        let (mut lo, mut hi) = (0, groups);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let g = at + 16 + 12 * mid;
            let (first, last) = (u32_at(d, g)?, u32_at(d, g + 4)?);
            if cp < first {
                hi = mid;
            } else if cp > last {
                lo = mid + 1;
            } else {
                return u16::try_from(u32_at(d, g + 8)? + (cp - first)).ok();
            }
        }
        None
    }

    /// How far the pen moves after this glyph, in font units.
    pub fn advance(&self, glyph: u16) -> u16 {
        let i = glyph.min(self.h_metrics.saturating_sub(1)) as usize;
        u16_at(self.hmtx, 4 * i).unwrap_or(0)
    }

    fn glyph_data(&self, glyph: u16) -> Option<&'a [u8]> {
        let g = glyph as usize;
        let (start, end) = if self.long_loca {
            (u32_at(self.loca, 4 * g)? as usize, u32_at(self.loca, 4 * g + 4)? as usize)
        } else {
            (u16_at(self.loca, 2 * g)? as usize * 2, u16_at(self.loca, 2 * g + 2)? as usize * 2)
        };
        if end < start {
            return None;
        }
        self.glyf.get(start..end)
    }

    /// Sends the glyph's outline to `out`. False if the glyph data is damaged.
    pub fn outline(&self, glyph: u16, out: &mut dyn Outline) -> bool {
        self.outline_with(glyph, &Transform::IDENTITY, 0, out).is_some()
    }

    fn outline_with(&self, glyph: u16, t: &Transform, depth: u32, out: &mut dyn Outline) -> Option<()> {
        if depth > 8 {
            return None; // a glyph made of itself, or nested too deep
        }
        let g = self.glyph_data(glyph)?;
        if g.is_empty() {
            return Some(()); // e.g. the space
        }
        let contours = i16_at(g, 0)?;
        if contours >= 0 {
            self.simple(g, contours as usize, t, out)
        } else {
            self.composite(g, t, depth, out)
        }
    }

    fn simple(&self, g: &[u8], contours: usize, t: &Transform, out: &mut dyn Outline) -> Option<()> {
        if contours == 0 {
            return Some(());
        }
        let points = u16_at(g, 10 + 2 * (contours - 1))? as usize + 1;
        let instructions = u16_at(g, 10 + 2 * contours)? as usize;
        let mut p = 10 + 2 * contours + 2 + instructions;

        let mut flags: Vec<u8> = Vec::with_capacity(points);
        while flags.len() < points {
            let f = u8_at(g, p)?;
            p += 1;
            flags.push(f);
            if f & 0x08 != 0 {
                let repeat = u8_at(g, p)?;
                p += 1;
                for _ in 0..repeat {
                    flags.push(f);
                }
            }
        }
        flags.truncate(points);

        let mut xs: Vec<i32> = Vec::with_capacity(points);
        let mut v = 0i32;
        for &f in &flags {
            if f & 0x02 != 0 {
                let d = u8_at(g, p)? as i32;
                p += 1;
                v += if f & 0x10 != 0 { d } else { -d };
            } else if f & 0x10 == 0 {
                v += i16_at(g, p)? as i32;
                p += 2;
            }
            xs.push(v);
        }
        let mut ys: Vec<i32> = Vec::with_capacity(points);
        v = 0;
        for &f in &flags {
            if f & 0x04 != 0 {
                let d = u8_at(g, p)? as i32;
                p += 1;
                v += if f & 0x20 != 0 { d } else { -d };
            } else if f & 0x20 == 0 {
                v += i16_at(g, p)? as i32;
                p += 2;
            }
            ys.push(v);
        }

        let mut first = 0;
        for c in 0..contours {
            let last = u16_at(g, 10 + 2 * c)? as usize;
            if last < first || last >= points {
                return None;
            }
            let point = |i: usize| -> (i32, i32, bool) {
                let (x, y) = t.apply(xs[i] as i64 * 64, ys[i] as i64 * 64);
                (x, y, flags[i] & 0x01 != 0)
            };
            contour(first, last, &point, out);
            first = last + 1;
        }
        Some(())
    }

    fn composite(&self, g: &[u8], t: &Transform, depth: u32, out: &mut dyn Outline) -> Option<()> {
        const WORDS: u16 = 0x0001;
        const XY_VALUES: u16 = 0x0002;
        const SCALE: u16 = 0x0008;
        const MORE: u16 = 0x0020;
        const XY_SCALE: u16 = 0x0040;
        const TWO_BY_TWO: u16 = 0x0080;
        const SCALED_OFFSET: u16 = 0x0800;
        let mut p = 10;
        loop {
            let flags = u16_at(g, p)?;
            let glyph = u16_at(g, p + 2)?;
            p += 4;
            let (a, b) = if flags & WORDS != 0 {
                p += 4;
                (i16_at(g, p - 4)? as i64, i16_at(g, p - 2)? as i64)
            } else {
                p += 2;
                if flags & XY_VALUES != 0 {
                    (i8_at(g, p - 2)? as i64, i8_at(g, p - 1)? as i64)
                } else {
                    (u8_at(g, p - 2)? as i64, u8_at(g, p - 1)? as i64)
                }
            };
            let mut m = Transform::IDENTITY;
            if flags & SCALE != 0 {
                m.xx = i16_at(g, p)? as i64;
                m.yy = m.xx;
                p += 2;
            } else if flags & XY_SCALE != 0 {
                m.xx = i16_at(g, p)? as i64;
                m.yy = i16_at(g, p + 2)? as i64;
                p += 4;
            } else if flags & TWO_BY_TWO != 0 {
                m.xx = i16_at(g, p)? as i64;
                m.yx = i16_at(g, p + 2)? as i64;
                m.xy = i16_at(g, p + 4)? as i64;
                m.yy = i16_at(g, p + 6)? as i64;
                p += 8;
            }
            // Offsets given as point numbers (matching points) are rare and not supported: the part is
            // placed at the origin instead.
            let (dx, dy) = if flags & XY_VALUES != 0 { (a, b) } else { (0, 0) };
            let (dx, dy) = if flags & SCALED_OFFSET != 0 {
                ((m.xx * dx + m.xy * dy) >> 14, (m.yx * dx + m.yy * dy) >> 14)
            } else {
                (dx, dy)
            };
            m.dx = dx * 64;
            m.dy = dy * 64;
            self.outline_with(glyph, &t.after(&m), depth + 1, out)?;
            if flags & MORE == 0 {
                return Some(());
            }
        }
    }
}

/// One closed contour of points on and off the curve. Two off-curve points in a row have an implied
/// on-curve point halfway between them.
fn contour(first: usize, last: usize, point: &dyn Fn(usize) -> (i32, i32, bool), out: &mut dyn Outline) {
    let n = last - first + 1;
    let at = |k: usize| point(first + k % n);
    let mid = |a: (i32, i32, bool), b: (i32, i32, bool)| ((a.0 + b.0) / 2, (a.1 + b.1) / 2);
    let p0 = at(0);
    let pl = at(n - 1);
    // Start on the curve: the first point if it is on it, else the last, else between them.
    let (start, from, count) = if p0.2 {
        ((p0.0, p0.1), 1, n - 1)
    } else if pl.2 {
        ((pl.0, pl.1), 0, n - 1)
    } else {
        (mid(pl, p0), 0, n)
    };
    out.move_to(start.0, start.1);
    let mut control: Option<(i32, i32, bool)> = None;
    for k in from..from + count {
        let q = at(k);
        match (q.2, control) {
            (true, Some(c)) => {
                out.quad_to(c.0, c.1, q.0, q.1);
                control = None;
            }
            (true, None) => out.line_to(q.0, q.1),
            (false, Some(c)) => {
                let m = mid(c, q);
                out.quad_to(c.0, c.1, m.0, m.1);
                control = Some(q);
            }
            (false, None) => control = Some(q),
        }
    }
    match control {
        Some(c) => out.quad_to(c.0, c.1, start.0, start.1),
        None => out.line_to(start.0, start.1),
    }
    out.close();
}

#[cfg(test)]
mod tests;
