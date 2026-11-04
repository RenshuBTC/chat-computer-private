//! Variable fonts: one font file that holds a family's weights (Atkinson Hyperlegible Next, from 200 to 800), as
//! OpenType's font variations define them. An instance is a point on the font's axes ('wght' 700 is bold): each
//! axis's value normalized to −1…1 around the font's default (fvar), then mapped by the font's avar table. A glyph
//! at an instance is its default outline moved by deltas (gvar): each of its "tuple variations" gives deltas for
//! some or all of its points, scaled by how far the instance is into the tuple's region; points a tuple leaves out
//! are moved like their neighbours ("interpolation of untouched points"). Four "phantom" points after the outline's
//! carry the glyph's horizontal and vertical metrics, so the advance width varies too.
//!
//! Coordinates are fixed-point integers throughout: normalized values in 2.14 (as the font stores them), scalars in
//! 16.16, deltas in 1/64 of a font unit.

use crate::{i16_at, u16_at, u32_at};
use alloc::vec;
use alloc::vec::Vec;

/// A variable font's instance: each axis's normalized coordinate (2.14 fixed point, −16384 to 16384; 0 is the
/// font's default), after the font's avar mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instance {
    pub(crate) coords: Vec<i32>,
}

impl Instance {
    /// Each axis's normalized coordinate (2.14 fixed point).
    pub fn coords(&self) -> &[i32] {
        &self.coords
    }
}

const ONE_16_16: i64 = 1 << 16;

/// `a / b` rounded to the nearest, halves away from zero.
fn div_round(a: i64, b: i64) -> i64 {
    if b == 0 {
        return 0;
    }
    let (a, b) = if b < 0 { (-a, -b) } else { (a, b) };
    if a >= 0 { (a + b / 2) / b } else { -((-a + b / 2) / b) }
}

/// `value` (in the axis's units) normalized to the axis (fvar's minimum, default and maximum, 16.16), in 2.14.
pub(crate) fn normalize(value: i64, min: i64, default: i64, max: i64) -> i32 {
    let v = value.clamp(min, max);
    let n = if v < default {
        -div_round((default - v) << 14, default - min)
    } else if v > default {
        div_round((v - default) << 14, max - default)
    } else {
        0
    };
    n.clamp(-16384, 16384) as i32
}

/// A normalized coordinate through an axis's avar segment map (pairs of "from" and "to" values, 2.14): piecewise
/// linear between the pairs.
pub(crate) fn avar_map(map: &[(i32, i32)], v: i32) -> i32 {
    let Some(&(first_from, first_to)) = map.first() else { return v };
    if v <= first_from {
        return first_to + (v - first_from);
    }
    for w in map.windows(2) {
        let ((f0, t0), (f1, t1)) = (w[0], w[1]);
        if v == f1 {
            return t1;
        }
        if v < f1 {
            if f1 == f0 {
                return t1;
            }
            return t0 + div_round((t1 - t0) as i64 * (v - f0) as i64, (f1 - f0) as i64) as i32;
        }
    }
    let &(last_from, last_to) = map.last().unwrap();
    last_to + (v - last_from)
}

/// How much of a tuple variation applies at `coords` (16.16; 0: none), from its peak and, for an intermediate
/// region, its start and end (each axis's coordinate in 2.14), as OpenType's "algorithm for interpolation" gives it.
fn scalar(coords: &[i32], peak: &[i32], region: Option<(&[i32], &[i32])>) -> i64 {
    let mut s = ONE_16_16;
    for (i, &p) in peak.iter().enumerate() {
        if p == 0 {
            continue;
        }
        let (lower, upper) = match region {
            Some((start, end)) => (start[i], end[i]),
            None => (p.min(0), p.max(0)),
        };
        // A region that doesn't make sense, or crosses the default, has no effect on this axis.
        if lower > p || p > upper || (lower < 0 && upper > 0) {
            continue;
        }
        let v = coords.get(i).copied().unwrap_or(0);
        if v == p {
            continue;
        }
        if v <= lower || v >= upper {
            return 0;
        }
        let f = if v < p {
            ((v - lower) as i64 * ONE_16_16) / (p - lower) as i64
        } else {
            ((upper - v) as i64 * ONE_16_16) / (upper - p) as i64
        };
        s = s * f / ONE_16_16;
    }
    s
}

/// Packed point numbers: how many (0: all the glyph's points), then runs of increments, bytes or words. The point
/// numbers (None: all), and the bytes read.
fn packed_points(d: &[u8]) -> Option<(Option<Vec<usize>>, usize)> {
    let first = *d.first()?;
    let (count, mut p) = if first & 0x80 != 0 {
        ((((first & 0x7F) as usize) << 8) | *d.get(1)? as usize, 2)
    } else {
        (first as usize, 1)
    };
    if count == 0 {
        return Some((None, p));
    }
    let mut points = Vec::with_capacity(count);
    let mut last = 0usize;
    while points.len() < count {
        let control = *d.get(p)?;
        p += 1;
        let run = (control & 0x7F) as usize + 1;
        for _ in 0..run {
            let step = if control & 0x80 != 0 {
                p += 2;
                u16_at(d, p - 2)? as usize
            } else {
                p += 1;
                *d.get(p - 1)? as usize
            };
            last += step;
            if points.len() < count {
                points.push(last);
            }
        }
    }
    Some((Some(points), p))
}

/// `count` packed deltas: runs of zeros, bytes, words (or, newer, double words). The deltas, and the bytes read.
fn packed_deltas(d: &[u8], count: usize) -> Option<(Vec<i32>, usize)> {
    let mut out = Vec::with_capacity(count);
    let mut p = 0;
    while out.len() < count {
        let control = *d.get(p)?;
        p += 1;
        for _ in 0..(control & 0x3F) as usize + 1 {
            let v = match control & 0xC0 {
                0x80 => 0,
                0x40 => {
                    p += 2;
                    i16_at(d, p - 2)? as i32
                }
                0xC0 => {
                    p += 4;
                    u32_at(d, p - 4)? as i32
                }
                _ => {
                    p += 1;
                    *d.get(p - 1)? as i8 as i32
                }
            };
            out.push(v);
        }
    }
    out.truncate(count);
    Some((out, p))
}

/// Deltas for the points of one contour that a tuple doesn't list, from the listed ones on either side (OpenType's
/// "inferred deltas for un-referenced point numbers"), for one coordinate: `c` the default coordinates, `d` the
/// deltas (1/64 unit), `known` which were listed.
fn infer(c: &[i32], d: &mut [i64], known: &[bool]) {
    let n = c.len();
    let listed: Vec<usize> = (0..n).filter(|&i| known[i]).collect();
    if listed.is_empty() || listed.len() == n {
        return; // (none listed: none move)
    }
    for (k, &a) in listed.iter().enumerate() {
        let b = listed[(k + 1) % listed.len()];
        let mut i = (a + 1) % n;
        while i != b {
            d[i] = between(c[a], d[a], c[b], d[b], c[i]);
            i = (i + 1) % n;
        }
    }
}

/// A delta for coordinate `x` from the points at `x1` and `x2` with deltas `d1` and `d2`: theirs beyond them,
/// proportional between them.
fn between(x1: i32, d1: i64, x2: i32, d2: i64, x: i32) -> i64 {
    if x1 == x2 {
        return if d1 == d2 { d1 } else { 0 };
    }
    let (x1, d1, x2, d2) = if x1 < x2 { (x1, d1, x2, d2) } else { (x2, d2, x1, d1) };
    if x <= x1 {
        d1
    } else if x >= x2 {
        d2
    } else {
        d1 + div_round((d2 - d1) * (x - x1) as i64, (x2 - x1) as i64)
    }
}

/// The deltas (1/64 font unit) of glyph `glyph`'s `n` points at `coords`, from the gvar table: the outline's points
/// (`xs`, `ys`: their default coordinates, in font units; `ends`: each contour's last point), then the 4 phantom
/// points. Without `xs` and `ys`, the points a tuple doesn't list don't move (enough for the phantom points, which
/// never take their neighbours' deltas). None if the gvar data is damaged.
pub(crate) fn deltas(
    gvar: &[u8],
    glyph: u16,
    coords: &[i32],
    n: usize,
    outline: Option<(&[i32], &[i32], &[usize])>,
) -> Option<(Vec<i64>, Vec<i64>)> {
    let mut dx = vec![0i64; n];
    let mut dy = vec![0i64; n];
    let axes = u16_at(gvar, 4)? as usize;
    let shared_tuples = u16_at(gvar, 6)? as usize;
    let shared_at = u32_at(gvar, 8)? as usize;
    let glyphs = u16_at(gvar, 12)?;
    let long = u16_at(gvar, 14)? & 1 != 0;
    let data_at = u32_at(gvar, 16)? as usize;
    if glyph >= glyphs || coords.iter().all(|&c| c == 0) {
        return Some((dx, dy));
    }
    let g = glyph as usize;
    let (start, end) = if long {
        (u32_at(gvar, 20 + 4 * g)? as usize, u32_at(gvar, 24 + 4 * g)? as usize)
    } else {
        (u16_at(gvar, 20 + 2 * g)? as usize * 2, u16_at(gvar, 22 + 2 * g)? as usize * 2)
    };
    if end <= start {
        return Some((dx, dy)); // no variations
    }
    let d = gvar.get(data_at.checked_add(start)?..data_at.checked_add(end)?)?;
    let count = u16_at(d, 0)?;
    let mut serialized = u16_at(d, 2)? as usize;
    let shared_points = if count & 0x8000 != 0 {
        let (points, len) = packed_points(d.get(serialized..)?)?;
        serialized += len;
        points
    } else {
        None
    };
    let tuple = |at: usize| -> Option<Vec<i32>> { (0..axes).map(|i| i16_at(d, at + 2 * i).map(|v| v as i32)).collect() };
    let mut header = 4;
    for _ in 0..count & 0x0FFF {
        let size = u16_at(d, header)? as usize;
        let index = u16_at(d, header + 2)?;
        header += 4;
        let peak = if index & 0x8000 != 0 {
            header += 2 * axes;
            tuple(header - 2 * axes)?
        } else {
            let i = (index & 0x0FFF) as usize;
            if i >= shared_tuples {
                return None;
            }
            (0..axes).map(|k| i16_at(gvar, shared_at + 2 * (i * axes + k)).map(|v| v as i32)).collect::<Option<Vec<i32>>>()?
        };
        let region = if index & 0x4000 != 0 {
            header += 4 * axes;
            Some((tuple(header - 4 * axes)?, tuple(header - 2 * axes)?))
        } else {
            None
        };
        let data = d.get(serialized..serialized.checked_add(size)?)?;
        serialized += size;
        let s = scalar(coords, &peak, region.as_ref().map(|(a, b)| (a.as_slice(), b.as_slice())));
        if s == 0 {
            continue;
        }
        let (points, mut p) = if index & 0x2000 != 0 { packed_points(data)? } else { (shared_points.clone(), 0) };
        let listed = points.as_ref().map_or(n, |pts| pts.len());
        let (xs_delta, len) = packed_deltas(data.get(p..)?, listed)?;
        p += len;
        let (ys_delta, _) = packed_deltas(data.get(p..)?, listed)?;
        // This tuple's deltas for every point (1/64 unit), its unlisted points inferred.
        let mut tx = vec![0i64; n];
        let mut ty = vec![0i64; n];
        match &points {
            None => {
                for i in 0..n {
                    tx[i] = xs_delta[i] as i64 * 64;
                    ty[i] = ys_delta[i] as i64 * 64;
                }
            }
            Some(pts) => {
                let mut known = vec![false; n];
                for (k, &i) in pts.iter().enumerate() {
                    if i < n {
                        tx[i] = xs_delta[k] as i64 * 64;
                        ty[i] = ys_delta[k] as i64 * 64;
                        known[i] = true;
                    }
                }
                if let Some((xs, ys, ends)) = outline {
                    let mut first = 0;
                    for &last in ends {
                        if last < first || last >= n.saturating_sub(4) {
                            break;
                        }
                        let r = first..last + 1;
                        infer(&xs[r.clone()], &mut tx[r.clone()], &known[r.clone()]);
                        infer(&ys[r.clone()], &mut ty[r.clone()], &known[r.clone()]);
                        first = last + 1;
                    }
                }
            }
        }
        for i in 0..n {
            dx[i] += div_round(tx[i] * s, ONE_16_16);
            dy[i] += div_round(ty[i] * s, ONE_16_16);
        }
    }
    Some((dx, dy))
}
