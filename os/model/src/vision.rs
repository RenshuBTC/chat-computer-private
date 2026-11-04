//! What the model sees: Qwen 3.5's vision part, which makes a picture into rows the language model reads in place of
//! tokens (net.rs). As Qwen's own processor and model do it (transformers' Qwen2VLImageProcessorFast and
//! Qwen3_5VisionModel):
//!
//! - the picture resized so each side is a whole number of 32-pixel squares, its area within the model's bounds
//!   (`size_for`: Qwen's smart_resize), by bicubic resampling as Pillow and torchvision do it (`resize`);
//! - its pixels scaled to -1..1 and cut into 16-pixel patches, each 2x2 group of patches together (they become one
//!   row for the language model), each patch twice over in time (the model reads videos as pairs of frames);
//! - each patch's pixels times a matrix (the patch embedding), plus a learned position for where it is (the model's
//!   48x48 grid of them, interpolated to the picture's own grid);
//! - blocks (12 in Qwen3.5-0.8B) of attention over every patch of the picture, with positions turning queries and keys
//!   by row and column, and a feed-forward part, each after a layer norm and added to what went in;
//! - each 2x2 group's four patches normalised, side by side, through two more matrices: a row as wide as the language
//!   model's.

use crate::file::{Error, File};
use crate::math;
use crate::net::{Matrix, Pool};
use crate::quant::{self, Prepared};
use alloc::vec;
use alloc::vec::Vec;

/// The vision part's settings.
#[derive(Clone, Debug)]
pub struct Settings {
    pub depth: usize,
    pub hidden: usize,
    pub heads: usize,
    pub intermediate: usize,
    /// A patch's side, in pixels; how many frames a patch spans; how many patches a side of a row's square has.
    pub patch: usize,
    pub temporal: usize,
    pub merge: usize,
    /// The learned positions: a square grid this many (its side squared).
    pub positions: usize,
    /// The language model's width (each row's numbers).
    pub out: usize,
    pub rope_theta: f64,
    /// The fewest pixels a picture is read at, and the most (the model's own bounds; callers may ask for fewer).
    pub min_pixels: usize,
    pub max_pixels: usize,
    /// Each colour's mean and spread (after scaling to 0..1).
    pub mean: [f32; 3],
    pub std: [f32; 3],
}

struct Block<'a> {
    norm1: (Vec<f32>, Vec<f32>),
    qkv: Matrix<'a>,
    qkv_bias: Vec<f32>,
    proj: Matrix<'a>,
    proj_bias: Vec<f32>,
    norm2: (Vec<f32>, Vec<f32>),
    fc1: Matrix<'a>,
    fc1_bias: Vec<f32>,
    fc2: Matrix<'a>,
    fc2_bias: Vec<f32>,
}

pub struct Vision<'a> {
    pub settings: Settings,
    patch: Matrix<'a>,
    patch_bias: Vec<f32>,
    pos: Matrix<'a>,
    blocks: Vec<Block<'a>>,
    merger_norm: (Vec<f32>, Vec<f32>),
    merger_fc1: Matrix<'a>,
    merger_fc1_bias: Vec<f32>,
    merger_fc2: Matrix<'a>,
    merger_fc2_bias: Vec<f32>,
    /// The rotary frequencies (one per pair turned, for rows and columns alike).
    inv_freq: Vec<f64>,
}

/// A picture being made into rows, a step at a time (`Vision::begin`, `advance`): its patches' numbers so far,
/// their places' turns, how many patches, and the next block.
pub struct Encoding {
    x: Vec<f32>,
    turns: Vec<(f32, f32)>,
    n: usize,
    block: usize,
}

/// A picture, resized for the model: its pixels (RGB, row after row) and its grid of patches.
#[derive(Clone, Debug)]
pub struct Resized {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

impl<'a> Vision<'a> {
    /// The model file's vision part (None: the file has none, so the model can't see).
    pub fn load(file: &File<'a>) -> Result<Option<Vision<'a>>, Error> {
        let c = file.config()?;
        if c.str("vision.depth").is_none() {
            return Ok(None);
        }
        let three = |key: &str, default: f32| -> [f32; 3] {
            let mut v = [default; 3];
            if let Some(s) = c.str(key) {
                for (slot, part) in v.iter_mut().zip(s.split(',')) {
                    if let Ok(x) = part.trim().parse::<f32>() {
                        *slot = x;
                    }
                }
            }
            v
        };
        let s = Settings {
            depth: c.usize("vision.depth")?,
            hidden: c.usize("vision.hidden")?,
            heads: c.usize("vision.heads")?,
            intermediate: c.usize("vision.intermediate")?,
            patch: c.usize("vision.patch")?,
            temporal: c.usize("vision.temporal")?,
            merge: c.usize("vision.merge")?,
            positions: c.usize("vision.positions")?,
            out: c.usize("vision.out")?,
            rope_theta: c.f64("vision.rope_theta").unwrap_or(10000.0),
            min_pixels: c.usize("vision.min_pixels").unwrap_or(65536),
            max_pixels: c.usize("vision.max_pixels").unwrap_or(16_777_216),
            mean: three("vision.mean", 0.5),
            std: three("vision.std", 0.5),
        };
        let side = isqrt(s.positions);
        if s.heads == 0 || !s.hidden.is_multiple_of(s.heads) || !(s.hidden / s.heads).is_multiple_of(4) || side * side != s.positions
            || s.patch == 0 || s.temporal == 0 || s.merge == 0 || s.depth == 0
        {
            return Err(Error::Config);
        }
        let h = s.hidden;
        let pixels = 3 * s.temporal * s.patch * s.patch;
        let wide = h * s.merge * s.merge;
        let pair = |n: &str| -> Result<(Vec<f32>, Vec<f32>), Error> {
            let w = crate::net::vector(file, n)?;
            let b = crate::net::vector(file, &alloc::format!("{n}_bias"))?;
            if w.len() != b.len() {
                return Err(Error::Damaged);
            }
            Ok((w, b))
        };
        let sized = |v: Vec<f32>, n: usize| if v.len() == n { Ok(v) } else { Err(Error::Damaged) };
        let mut blocks = Vec::with_capacity(s.depth);
        for i in 0..s.depth {
            let p = |n: &str| alloc::format!("vision.{i}.{n}");
            blocks.push(Block {
                norm1: pair(&p("norm1"))?,
                qkv: crate::net::matrix(file, &p("qkv"), 3 * h, h)?,
                qkv_bias: sized(crate::net::vector(file, &p("qkv_bias"))?, 3 * h)?,
                proj: crate::net::matrix(file, &p("proj"), h, h)?,
                proj_bias: sized(crate::net::vector(file, &p("proj_bias"))?, h)?,
                norm2: pair(&p("norm2"))?,
                fc1: crate::net::matrix(file, &p("fc1"), s.intermediate, h)?,
                fc1_bias: sized(crate::net::vector(file, &p("fc1_bias"))?, s.intermediate)?,
                fc2: crate::net::matrix(file, &p("fc2"), h, s.intermediate)?,
                fc2_bias: sized(crate::net::vector(file, &p("fc2_bias"))?, h)?,
            });
            if blocks[i].norm1.0.len() != h || blocks[i].norm2.0.len() != h {
                return Err(Error::Damaged);
            }
        }
        let merger_norm = pair("vision.merger.norm")?;
        if merger_norm.0.len() != h {
            return Err(Error::Damaged);
        }
        let head_dim = h / s.heads;
        // (Rows and columns each turn a quarter of a head's pairs... as the reference has it: frequencies for half
        // the head's width, used once for rows and once for columns.)
        let spatial = head_dim / 2;
        let inv_freq = (0..spatial / 2).map(|i| 1.0 / math::powf64(s.rope_theta, (2 * i) as f64 / spatial as f64)).collect();
        Ok(Some(Vision {
            patch: crate::net::matrix(file, "vision.patch", h, pixels)?,
            patch_bias: sized(crate::net::vector(file, "vision.patch_bias")?, h)?,
            pos: crate::net::matrix(file, "vision.pos", s.positions, h)?,
            blocks,
            merger_norm,
            merger_fc1: crate::net::matrix(file, "vision.merger.fc1", wide, wide)?,
            merger_fc1_bias: sized(crate::net::vector(file, "vision.merger.fc1_bias")?, wide)?,
            merger_fc2: crate::net::matrix(file, "vision.merger.fc2", s.out, wide)?,
            merger_fc2_bias: sized(crate::net::vector(file, "vision.merger.fc2_bias")?, s.out)?,
            inv_freq,
            settings: s,
        }))
    }

    /// The size a picture `height` by `width` is read at: each side a whole number of a row's squares (32 pixels),
    /// its area within the model's bounds and at most `most_rows` rows for the language model (if that's fewer).
    pub fn size_for(&self, height: usize, width: usize, most_rows: usize) -> (usize, usize) {
        let s = &self.settings;
        let factor = s.patch * s.merge;
        let most = s.max_pixels.min(most_rows.saturating_mul(factor * factor)).max(factor * factor);
        smart_resize(height, width, factor, s.min_pixels.min(most), most)
    }

    /// How many rows a picture read at `height` by `width` (a size_for size) makes for the language model.
    pub fn rows_for(&self, height: usize, width: usize) -> (usize, usize) {
        let f = self.settings.patch * self.settings.merge;
        (height / f, width / f)
    }

    /// A picture's pixels (RGB, `width` by `height`) resized for the model, with at most `most_rows` rows.
    pub fn prepare(&self, rgb: &[u8], width: usize, height: usize, most_rows: usize) -> Resized {
        let (h, w) = self.size_for(height, width, most_rows);
        Resized { width: w, height: h, rgb: resize(rgb, width, height, w, h) }
    }

    /// The patches of a picture already at a size_for size, as the processor lays them out: each 2x2 group's
    /// patches together, each patch's numbers colour by colour, frame by frame (the same twice), row by row.
    pub fn patches(&self, p: &Resized) -> Vec<f32> {
        let s = &self.settings;
        let (ps, m, t) = (s.patch, s.merge, s.temporal);
        let (gh, gw) = (p.height / ps, p.width / ps);
        let per = 3 * t * ps * ps;
        let mut out = vec![0.0f32; gh * gw * per];
        let mut at = 0;
        for bh in 0..gh / m {
            for bw in 0..gw / m {
                for my in 0..m {
                    for mx in 0..m {
                        let (py0, px0) = ((bh * m + my) * ps, (bw * m + mx) * ps);
                        for c in 0..3 {
                            for _ in 0..t {
                                for y in 0..ps {
                                    for x in 0..ps {
                                        let v = p.rgb[((py0 + y) * p.width + px0 + x) * 3 + c] as f32 / 255.0;
                                        out[at] = (v - s.mean[c]) / s.std[c];
                                        at += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// The rows the language model reads for a picture: from its patches (`patches`, for a grid of `gh` by `gw`
    /// patches), one row (as wide as the language model) for each 2x2 group, in order.
    pub fn encode(&self, patches: &[f32], gh: usize, gw: usize, pool: &dyn Pool) -> Vec<f32> {
        let mut e = self.begin(patches, gh, gw, pool);
        loop {
            if let Some(rows) = self.advance(&mut e, pool) {
                return rows;
            }
        }
    }

    /// The same a step at a time, for a caller with other things to do between them: the patches embedded, with
    /// their places (begin), then a block a step, the last with the merger (advance, until it gives the rows).
    pub fn begin(&self, patches: &[f32], gh: usize, gw: usize, pool: &dyn Pool) -> Encoding {
        let s = &self.settings;
        let (h, n) = (s.hidden, gh * gw);
        let per = 3 * s.temporal * s.patch * s.patch;
        assert!(patches.len() == n * per && gh.is_multiple_of(s.merge) && gw.is_multiple_of(s.merge) && n > 0);
        // The patch embedding.
        let ready: Vec<Prepared> = patches.chunks_exact(per).map(Prepared::new).collect();
        let mut x = self.patch.times_all(&ready, pool);
        drop(ready);
        add_bias(&mut x, &self.patch_bias);
        // Each patch's row and column (in the processor's order), and its learned position, interpolated.
        let m = s.merge;
        let mut places = Vec::with_capacity(n);
        for bh in 0..gh / m {
            for bw in 0..gw / m {
                for my in 0..m {
                    for mx in 0..m {
                        places.push((bh * m + my, bw * m + mx));
                    }
                }
            }
        }
        let side = isqrt(s.positions);
        let mut row = vec![0.0f32; h];
        for (i, &(r, c)) in places.iter().enumerate() {
            let (rt, rw) = taps(r, gh, side);
            let (ct, cw) = taps(c, gw, side);
            let xi = &mut x[i * h..(i + 1) * h];
            for a in 0..2 {
                for b in 0..2 {
                    let wgt = rw[a] * cw[b];
                    if wgt == 0.0 {
                        continue;
                    }
                    self.pos.row(rt[a] * side + ct[b], &mut row);
                    quant::axpy(wgt, &row, xi);
                }
            }
        }
        // The turning for each patch's place: per head, a pair (i, i + half) turned by its row's angle for the first
        // quarter of the pairs and its column's for the next.
        let hd = h / s.heads;
        let half = hd / 2;
        let quarter = self.inv_freq.len();
        let mut turns = vec![(0.0f32, 0.0f32); n * half];
        for (i, &(r, c)) in places.iter().enumerate() {
            for j in 0..half {
                let (pos, f) = if j < quarter { (r, self.inv_freq[j]) } else { (c, self.inv_freq[(j - quarter) % quarter.max(1)]) };
                let (sn, cn) = math::sin_cos(pos as f64 * f);
                turns[i * half + j] = (cn as f32, sn as f32);
            }
        }
        Encoding { x, turns, n, block: 0 }
    }

    /// One more step of an encoding: a block (and after the last, the merger). The rows, once it's done.
    pub fn advance(&self, e: &mut Encoding, pool: &dyn Pool) -> Option<Vec<f32>> {
        let s = &self.settings;
        let (h, n, m) = (s.hidden, e.n, s.merge);
        let mut normed = vec![0.0f32; n * h];
        if let Some(b) = self.blocks.get(e.block) {
            let x = &mut e.x;
            let mut out = vec![0.0f32; n * h];
            layer_norm(x, &b.norm1.0, &b.norm1.1, h, &mut normed);
            self.attention(b, &normed, &e.turns, n, &mut out, pool);
            for (a, o) in x.iter_mut().zip(&out) {
                *a += o;
            }
            layer_norm(x, &b.norm2.0, &b.norm2.1, h, &mut normed);
            let ready: Vec<Prepared> = normed.chunks_exact(h).map(Prepared::new).collect();
            let mut mid = b.fc1.times_all(&ready, pool);
            drop(ready);
            add_bias(&mut mid, &b.fc1_bias);
            for v in mid.iter_mut() {
                *v = gelu_tanh(*v);
            }
            let ready: Vec<Prepared> = mid.chunks_exact(s.intermediate).map(Prepared::new).collect();
            drop(mid);
            b.fc2.times_into(&ready, &mut out, pool);
            drop(ready);
            add_bias(&mut out, &b.fc2_bias);
            for (a, o) in x.iter_mut().zip(&out) {
                *a += o;
            }
            e.block += 1;
            return None;
        }
        // The merger: each patch normalised, each group's four side by side, two matrices.
        layer_norm(&e.x, &self.merger_norm.0, &self.merger_norm.1, h, &mut normed);
        let wide = h * m * m;
        let ready: Vec<Prepared> = normed.chunks_exact(wide).map(Prepared::new).collect();
        let mut mid = self.merger_fc1.times_all(&ready, pool);
        drop(ready);
        add_bias(&mut mid, &self.merger_fc1_bias);
        for v in mid.iter_mut() {
            *v = gelu(*v);
        }
        let ready: Vec<Prepared> = mid.chunks_exact(wide).map(Prepared::new).collect();
        let mut rows = self.merger_fc2.times_all(&ready, pool);
        add_bias(&mut rows, &self.merger_fc2_bias);
        Some(rows)
    }

    /// How many steps an encoding takes after it's begun (a block each, the last one the merger's too).
    pub fn steps(&self) -> usize {
        self.blocks.len() + 1
    }

    /// Attention over all the picture's patches (each sees every other): queries, keys and values with their
    /// biases, queries and keys turned by place, then the output projection (with its bias), into `out`.
    fn attention(&self, b: &Block, normed: &[f32], turns: &[(f32, f32)], n: usize, out: &mut [f32], pool: &dyn Pool) {
        let s = &self.settings;
        let h = s.hidden;
        let hd = h / s.heads;
        let half = hd / 2;
        let ready: Vec<Prepared> = normed.chunks_exact(h).map(Prepared::new).collect();
        let mut qkv = b.qkv.times_all(&ready, pool);
        drop(ready);
        add_bias(&mut qkv, &b.qkv_bias);
        // Queries and keys turned (each patch's, every head's).
        for t in 0..n {
            let tw = &turns[t * half..(t + 1) * half];
            for part in 0..2 {
                for hh in 0..s.heads {
                    let at = t * 3 * h + part * h + hh * hd;
                    let head = &mut qkv[at..at + hd];
                    for i in 0..half {
                        let (cn, sn) = tw[i];
                        let (a, c) = (head[i], head[i + half]);
                        head[i] = a * cn - c * sn;
                        head[i + half] = c * cn + a * sn;
                    }
                }
            }
        }
        let scale = 1.0 / math::sqrt(hd as f32);
        let mut att = vec![0.0f32; n * h];
        let att_ptr = SyncPtr(att.as_mut_ptr());
        let qkv = &qkv;
        // A job per head and a few queries: each sees every patch.
        const QUERIES: usize = 8;
        let per_head = n.div_ceil(QUERIES);
        pool.run(s.heads * per_head, &|j| {
            let (hh, block) = (j % s.heads, j / s.heads);
            let mut scores = vec![0.0f32; n];
            for t in block * QUERIES..((block + 1) * QUERIES).min(n) {
                let q = &qkv[t * 3 * h + hh * hd..t * 3 * h + (hh + 1) * hd];
                let mut max = f32::NEG_INFINITY;
                for (i, sc) in scores.iter_mut().enumerate() {
                    let k = &qkv[i * 3 * h + h + hh * hd..i * 3 * h + h + (hh + 1) * hd];
                    *sc = quant::dot(q, k) * scale;
                    max = max.max(*sc);
                }
                let mut sum = 0.0f32;
                for sc in scores.iter_mut() {
                    *sc = math::exp(*sc - max);
                    sum += *sc;
                }
                // (Each job writes its own queries' rows of its own head.)
                let o = unsafe { core::slice::from_raw_parts_mut(att_ptr.get().add(t * h + hh * hd), hd) };
                o.fill(0.0);
                for (i, sc) in scores.iter().enumerate() {
                    let v = &qkv[i * 3 * h + 2 * h + hh * hd..i * 3 * h + 2 * h + (hh + 1) * hd];
                    quant::axpy(sc / sum, v, o);
                }
            }
        });
        let ready: Vec<Prepared> = att.chunks_exact(h).map(Prepared::new).collect();
        b.proj.times_into(&ready, out, pool);
        add_bias(out, &b.proj_bias);
    }
}

#[derive(Clone, Copy)]
struct SyncPtr(*mut f32);
unsafe impl Sync for SyncPtr {}
unsafe impl Send for SyncPtr {}
impl SyncPtr {
    fn get(self) -> *mut f32 {
        self.0
    }
}

fn add_bias(x: &mut [f32], bias: &[f32]) {
    for row in x.chunks_exact_mut(bias.len()) {
        for (v, b) in row.iter_mut().zip(bias) {
            *v += b;
        }
    }
}

/// Layer normalisation of each row of `x` (rows of `w`'s length): to mean 0 and variance 1, times w, plus b.
fn layer_norm(x: &[f32], w: &[f32], b: &[f32], width: usize, out: &mut [f32]) {
    for (row, o) in x.chunks_exact(width).zip(out.chunks_exact_mut(width)) {
        let mean = row.iter().sum::<f32>() / width as f32;
        let var = row.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / width as f32;
        let r = 1.0 / math::sqrt(var + 1e-6);
        for (((o, &v), &g), &c) in o.iter_mut().zip(row).zip(w).zip(b) {
            *o = (v - mean) * r * g + c;
        }
    }
}

/// GELU with tanh's approximation (PyTorch's "gelu_pytorch_tanh").
pub fn gelu_tanh(x: f32) -> f32 {
    const K: f32 = 0.797_884_6; // √(2/π)
    0.5 * x * (1.0 + tanh(K * (x + 0.044715 * x * x * x)))
}

/// GELU exactly: x · Φ(x), with erf.
pub fn gelu(x: f32) -> f32 {
    0.5 * x * (1.0 + erf(x * core::f32::consts::FRAC_1_SQRT_2))
}

/// tanh x.
pub fn tanh(x: f32) -> f32 {
    if x > 10.0 {
        return 1.0;
    }
    if x < -10.0 {
        return -1.0;
    }
    if x.abs() < 0.004 {
        return x - x * x * x / 3.0;
    }
    let e = math::exp(2.0 * x);
    (e - 1.0) / (e + 1.0)
}

/// erf x, to within about 1e-7 (W. J. Cody's rational approximations would be closer; this is Abramowitz and
/// Stegun's 7.1.26 refined: computed in double precision, its error 1.5e-7).
pub fn erf(x: f32) -> f32 {
    let x = x as f64;
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let a = x.abs();
    if a > 6.0 {
        return sign as f32;
    }
    // (In double precision: e^-x² by the engine's own exp, in f32, is close enough for the tail's few digits.)
    let t = 1.0 / (1.0 + 0.327_591_1 * a);
    let poly = t * (0.254_829_592 + t * (-0.284_496_736 + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let e = math::exp(-(a * a) as f32) as f64;
    (sign * (1.0 - poly * e)) as f32
}

fn isqrt(n: usize) -> usize {
    let mut r = math::sqrt64(n as f64) as usize;
    while r * r > n {
        r -= 1;
    }
    while (r + 1) * (r + 1) <= n {
        r += 1;
    }
    r
}

/// The two taps (and their weights) into a side of `side` learned positions for place `i` of `size`: bilinear,
/// corners aligned (the first place on the first, the last on the last).
fn taps(i: usize, size: usize, side: usize) -> ([usize; 2], [f32; 2]) {
    let src = if size > 1 { i as f32 * (side - 1) as f32 / (size - 1) as f32 } else { 0.0 };
    let floor = floor(src as f64) as f32;
    let f0 = (floor as usize).min(side - 1);
    let f1 = (f0 + 1).min(side - 1);
    let d = src - floor;
    ([f0, f1], [1.0 - d, d])
}

/// The whole number at or below x, and at or above (x not negative, below 2^53: core has no floor or ceil).
fn floor(x: f64) -> f64 {
    (x as u64) as f64
}

fn ceil(x: f64) -> f64 {
    let f = floor(x);
    if f < x { f + 1.0 } else { f }
}

/// Python's round() of a float: to the nearest whole number, halves to the even one.
fn round_even(x: f64) -> f64 {
    let r = floor(x);
    let d = x - r;
    if d > 0.5 || (d == 0.5 && (r as i64) % 2 != 0) { r + 1.0 } else { r }
}

/// Qwen's smart_resize: the height and width a picture is read at, each a whole number of `factor`s, its area in
/// [min_pixels, max_pixels], its shape kept as nearly as that allows.
pub fn smart_resize(height: usize, width: usize, factor: usize, min_pixels: usize, max_pixels: usize) -> (usize, usize) {
    let (hf, wf, ff) = (height.max(1) as f64, width.max(1) as f64, factor as f64);
    let mut h = (round_even(hf / ff) as usize) * factor;
    let mut w = (round_even(wf / ff) as usize) * factor;
    if h * w > max_pixels {
        let beta = math::sqrt64(hf * wf / max_pixels as f64);
        h = factor.max((floor(hf / beta / ff) as usize) * factor);
        w = factor.max((floor(wf / beta / ff) as usize) * factor);
    } else if h * w < min_pixels {
        let beta = math::sqrt64(min_pixels as f64 / (hf * wf));
        h = (ceil(hf * beta / ff) as usize) * factor;
        w = (ceil(wf * beta / ff) as usize) * factor;
    }
    (h.max(factor), w.max(factor))
}

/// Pixels resized by bicubic resampling with antialiasing, as Pillow does it (and torchvision for 8-bit pictures):
/// across, then down, each pass's weights (a = -0.5, spread out as much as it shrinks) in whole numbers of 2^-22 and
/// its result rounded to bytes. RGB, row after row.
pub fn resize(rgb: &[u8], width: usize, height: usize, to_w: usize, to_h: usize) -> Vec<u8> {
    let across = if to_w == width { rgb.to_vec() } else { pass(rgb, width, height, to_w, true) };
    if to_h == height { across } else { pass(&across, to_w, height, to_h, false) }
}

/// One pass of `resize`: across (each row from `size` to `to` pixels) or down (each column).
fn pass(src: &[u8], width: usize, height: usize, to: usize, across: bool) -> Vec<u8> {
    const BITS: u32 = 22;
    let size = if across { width } else { height };
    let scale = size as f64 / to as f64;
    let filter_scale = scale.max(1.0);
    let support = 2.0 * filter_scale;
    let cubic = |x: f64| -> f64 {
        let a = -0.5;
        let x = x.abs();
        if x < 1.0 {
            ((a + 2.0) * x - (a + 3.0)) * x * x + 1.0
        } else if x < 2.0 {
            (((x - 5.0) * x + 8.0) * x - 4.0) * a
        } else {
            0.0
        }
    };
    // Each output pixel's first source pixel and its whole-number weights.
    let mut taps: Vec<(usize, Vec<i32>)> = Vec::with_capacity(to);
    for o in 0..to {
        let center = (o as f64 + 0.5) * scale;
        let lo = ((center - support + 0.5) as isize).max(0) as usize;
        let hi = (((center + support + 0.5) as isize).max(0) as usize).min(size);
        let mut ws: Vec<f64> = (lo..hi).map(|i| cubic((i as f64 - center + 0.5) / filter_scale)).collect();
        let total: f64 = ws.iter().sum();
        if total != 0.0 {
            for w in ws.iter_mut() {
                *w /= total;
            }
        }
        let ws: Vec<i32> = ws.iter().map(|&w| (w * (1u32 << BITS) as f64 + if w < 0.0 { -0.5 } else { 0.5 }) as i32).collect();
        taps.push((lo, ws));
    }
    let (out_w, out_h) = if across { (to, height) } else { (width, to) };
    let mut out = vec![0u8; out_w * out_h * 3];
    let round = 1i64 << (BITS - 1);
    for y in 0..out_h {
        for x in 0..out_w {
            let (o, from_fixed) = if across { (x, y) } else { (y, x) };
            let (lo, ws) = &taps[o];
            for c in 0..3 {
                let mut sum = round;
                for (k, &w) in ws.iter().enumerate() {
                    let i = lo + k;
                    let at = if across { (from_fixed * width + i) * 3 + c } else { (i * width + from_fixed) * 3 + c };
                    sum += src[at] as i64 * w as i64;
                }
                out[(y * out_w + x) * 3 + c] = (sum >> BITS).clamp(0, 255) as u8;
            }
        }
    }
    out
}
