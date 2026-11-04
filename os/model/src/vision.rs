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

/// A picture being made into rows, a step at a time (`Vision::begin`, then `advance` until it gives them), each step
/// a part of one of these, for as many patches as the caller says (so a step takes as long as the caller wants):
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// The patches embedded, with their learned positions.
    Embed,
    /// In a block: each patch normalised, its query, key and value made, the query and key turned by its place;
    Qkv,
    /// each patch's attention over all of them (all their keys and values made by now);
    Attend,
    /// the attention's output projected and added, then the feed-forward part's, each patch on its own.
    Out,
    /// Each 2x2 group's patches through the merger: a row for the language model.
    Merge,
}

pub struct Encoding {
    /// The patches' pixels, until they're embedded.
    patches: Vec<f32>,
    /// Each patch's numbers so far, its row and column, and its place's turns.
    x: Vec<f32>,
    places: Vec<(usize, usize)>,
    turns: Vec<(f32, f32)>,
    gh: usize,
    gw: usize,
    n: usize,
    /// Where it is: the block, the part of it, and how many patches (or groups, in the merger) that part has done.
    block: usize,
    phase: Phase,
    at: usize,
    /// The block's queries, keys and values, and its attention's output, as they're made.
    qkv: Vec<f32>,
    att: Vec<f32>,
    /// The rows made so far.
    rows: Vec<f32>,
    /// Patches' parts done (a patch embedded, a block's part for a patch, a group merged: one each).
    done: usize,
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
        let mut e = self.begin(patches.to_vec(), gh, gw);
        loop {
            if let Some(rows) = self.advance(&mut e, usize::MAX, pool) {
                return rows;
            }
        }
    }

    /// The same a step at a time, for a caller with other things to do between them: begun (each patch's place
    /// found), then advanced (`advance`) until it gives the rows.
    pub fn begin(&self, patches: Vec<f32>, gh: usize, gw: usize) -> Encoding {
        let s = &self.settings;
        let (h, n) = (s.hidden, gh * gw);
        let per = 3 * s.temporal * s.patch * s.patch;
        assert!(patches.len() == n * per && gh.is_multiple_of(s.merge) && gw.is_multiple_of(s.merge) && n > 0);
        // Each patch's row and column, in the processor's order.
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
        Encoding {
            patches,
            x: vec![0.0f32; n * h],
            places,
            turns,
            gh,
            gw,
            n,
            block: 0,
            phase: Phase::Embed,
            at: 0,
            qkv: Vec::new(),
            att: Vec::new(),
            rows: Vec::new(),
            done: 0,
        }
    }

    /// One more step of an encoding: the part it's at, for up to `most` more patches (at least one; the merger's
    /// part by groups of patches, at least one). The rows, once they're all made. However many patches a step takes,
    /// the rows are the same.
    pub fn advance(&self, e: &mut Encoding, most: usize, pool: &dyn Pool) -> Option<Vec<f32>> {
        let s = &self.settings;
        let (h, n, m) = (s.hidden, e.n, s.merge);
        let most = most.max(1);
        let from = e.at;
        let block = self.blocks.get(e.block);
        match (e.phase, block) {
            (Phase::Embed, _) => {
                // The patch embedding, plus each patch's learned position, interpolated.
                let end = from.saturating_add(most).min(n);
                let per = 3 * s.temporal * s.patch * s.patch;
                let ready: Vec<Prepared> = e.patches[from * per..end * per].chunks_exact(per).map(Prepared::new).collect();
                let x = &mut e.x[from * h..end * h];
                self.patch.times_into(&ready, x, pool);
                drop(ready);
                add_bias(x, &self.patch_bias);
                let side = isqrt(s.positions);
                let mut row = vec![0.0f32; h];
                for (i, &(r, c)) in e.places[from..end].iter().enumerate() {
                    let (rt, rw) = taps(r, e.gh, side);
                    let (ct, cw) = taps(c, e.gw, side);
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
                e.done += end - from;
                e.at = end;
                if end == n {
                    e.patches = Vec::new();
                    (e.phase, e.at) = (Phase::Qkv, 0);
                }
            }
            (Phase::Qkv, Some(b)) => {
                let end = from.saturating_add(most).min(n);
                if e.qkv.len() != n * 3 * h {
                    e.qkv = vec![0.0f32; n * 3 * h];
                }
                let mut normed = vec![0.0f32; (end - from) * h];
                layer_norm(&e.x[from * h..end * h], &b.norm1.0, &b.norm1.1, h, &mut normed);
                let ready: Vec<Prepared> = normed.chunks_exact(h).map(Prepared::new).collect();
                let qkv = &mut e.qkv[from * 3 * h..end * 3 * h];
                b.qkv.times_into(&ready, qkv, pool);
                drop(ready);
                add_bias(qkv, &b.qkv_bias);
                // Queries and keys turned (each patch's, every head's).
                let hd = h / s.heads;
                let half = hd / 2;
                for t in from..end {
                    let tw = &e.turns[t * half..(t + 1) * half];
                    for part in 0..2 {
                        for hh in 0..s.heads {
                            let at = (t - from) * 3 * h + part * h + hh * hd;
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
                e.done += end - from;
                e.at = end;
                if end == n {
                    (e.phase, e.at) = (Phase::Attend, 0);
                }
            }
            (Phase::Attend, Some(_)) => {
                let end = from.saturating_add(most).min(n);
                if e.att.len() != n * h {
                    e.att = vec![0.0f32; n * h];
                }
                self.attention(&e.qkv, n, from, end, &mut e.att[from * h..end * h], pool);
                e.done += end - from;
                e.at = end;
                if end == n {
                    (e.phase, e.at) = (Phase::Out, 0);
                }
            }
            (Phase::Out, Some(b)) => {
                let end = from.saturating_add(most).min(n);
                let rows = end - from;
                // The attention's output projected (with its bias) and added.
                let ready: Vec<Prepared> = e.att[from * h..end * h].chunks_exact(h).map(Prepared::new).collect();
                let mut out = vec![0.0f32; rows * h];
                b.proj.times_into(&ready, &mut out, pool);
                drop(ready);
                add_bias(&mut out, &b.proj_bias);
                let x = &mut e.x[from * h..end * h];
                for (a, o) in x.iter_mut().zip(&out) {
                    *a += o;
                }
                // The feed-forward part's, added.
                let mut normed = vec![0.0f32; rows * h];
                layer_norm(x, &b.norm2.0, &b.norm2.1, h, &mut normed);
                let ready: Vec<Prepared> = normed.chunks_exact(h).map(Prepared::new).collect();
                drop(normed);
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
                e.done += rows;
                e.at = end;
                if end == n {
                    e.block += 1;
                    (e.phase, e.at) = (if e.block < self.blocks.len() { Phase::Qkv } else { Phase::Merge }, 0);
                    if e.phase == Phase::Merge {
                        e.qkv = Vec::new();
                        e.att = Vec::new();
                    }
                }
            }
            (Phase::Merge, _) | (_, None) => {
                // The merger: each patch normalised, each group's four side by side, two matrices.
                let groups = n / (m * m);
                let end = from.saturating_add(most.div_ceil(m * m)).min(groups);
                let wide = h * m * m;
                let mut normed = vec![0.0f32; (end - from) * wide];
                layer_norm(&e.x[from * wide..end * wide], &self.merger_norm.0, &self.merger_norm.1, h, &mut normed);
                let ready: Vec<Prepared> = normed.chunks_exact(wide).map(Prepared::new).collect();
                drop(normed);
                let mut mid = self.merger_fc1.times_all(&ready, pool);
                drop(ready);
                add_bias(&mut mid, &self.merger_fc1_bias);
                for v in mid.iter_mut() {
                    *v = gelu(*v);
                }
                let ready: Vec<Prepared> = mid.chunks_exact(wide).map(Prepared::new).collect();
                drop(mid);
                let rows = self.merger_fc2.times_all(&ready, pool);
                drop(ready);
                let at = e.rows.len();
                e.rows.extend_from_slice(&rows);
                add_bias(&mut e.rows[at..], &self.merger_fc2_bias);
                e.done += end - from;
                e.at = end;
                if end == groups {
                    e.x = Vec::new();
                    return Some(core::mem::take(&mut e.rows));
                }
            }
        }
        None
    }

    /// How far an encoding has got: its parts done, of how many (a patch embedded, a block's part for a patch, and a
    /// group merged, each one).
    pub fn progress(&self, e: &Encoding) -> (usize, usize) {
        let m = self.settings.merge;
        (e.done, e.n * (1 + 3 * self.blocks.len()) + e.n / (m * m))
    }

    /// Attention for queries `from` to `end` over all the picture's patches (each sees every other), from their
    /// queries, keys and values (`qkv`, each patch's turned by its place): into `att` (those queries' rows).
    fn attention(&self, qkv: &[f32], n: usize, from: usize, end: usize, att: &mut [f32], pool: &dyn Pool) {
        let s = &self.settings;
        let h = s.hidden;
        let hd = h / s.heads;
        let scale = 1.0 / math::sqrt(hd as f32);
        let att_ptr = SyncPtr(att.as_mut_ptr());
        // A job per head and a few queries: each sees every patch.
        const QUERIES: usize = 8;
        let per_head = (end - from).div_ceil(QUERIES);
        pool.run(s.heads * per_head, &|j| {
            let (hh, block) = (j % s.heads, j / s.heads);
            let mut scores = vec![0.0f32; n];
            for t in from + block * QUERIES..(from + (block + 1) * QUERIES).min(end) {
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
                let o = unsafe { core::slice::from_raw_parts_mut(att_ptr.get().add((t - from) * h + hh * hd), hd) };
                o.fill(0.0);
                for (i, sc) in scores.iter().enumerate() {
                    let v = &qkv[i * 3 * h + 2 * h + hh * hd..i * 3 * h + 2 * h + (hh + 1) * hd];
                    quant::axpy(sc / sum, v, o);
                }
            }
        });
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

/// Pixels resized by bicubic resampling with antialiasing, as Pillow and torchvision do it (the picture crate's).
pub use picture::resample as resize;
