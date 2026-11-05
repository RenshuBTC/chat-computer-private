//! JPEG 2000's pictures (ITU-T T.800, ISO 15444-1), as PDF's JPXDecode filter has them (ISO 32000-1, 7.4.9): a
//! codestream, bare or in a JP2 file's boxes. What archives' and some scanners' pages are in.
//!
//! The codestream's tiles, each a tile-component per component, each in resolutions of subbands (its wavelet
//! decomposition), cut into precincts and code-blocks; the packets (tier 2: their headers' tag trees, passes and
//! lengths, in any of the five progression orders, POC's changes of order too, over as many tile-parts as it has),
//! then each code-block's bit-planes (tier 1, EBCOT: significance propagation, magnitude refinement and clean-up
//! passes, through the MQ coder jbig2.rs has, with each code-block style: arithmetic coding bypassed, contexts reset,
//! each pass terminated, vertically causal contexts, segmentation symbols); its coefficients dequantised (or not,
//! when reversible), turned back by the inverse wavelet transform (5/3's integer lifting, or 9/7's), the colour
//! transform undone (RCT or ICT) and the samples shifted to unsigned. Regions of interest (RGN's max-shift) too. A
//! JP2 file's palette is applied. Not read: packed packet headers (PPM, PPT).
//!
//! The result: each component's samples, 8 bits each (deeper ones scaled), at the image's size (subsampled ones
//! repeated).

use crate::jbig2::Mq;
use alloc::vec;
use alloc::vec::Vec;

/// The most samples a picture may have (its components together), and code-blocks' passes.
const MOST_SAMPLES: usize = 160 << 20;
const MOST_PASSES: usize = 164;
const MOST_PACKETS: u64 = 1 << 22;

fn be16(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// A decoded picture: its size and each component's 8-bit samples.
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub components: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Default)]
struct Component {
    precision: u32,
    signed: bool,
    dx: u32,
    dy: u32,
}

/// Coding style for a component (COD's or COC's).
#[derive(Clone, Default)]
struct Coding {
    levels: u32,
    cb_w: u32,
    cb_h: u32,
    style: u8,
    reversible: bool,
    /// Each resolution's precinct size's exponents (PPx, PPy).
    precincts: Vec<(u32, u32)>,
}

/// Quantization for a component (QCD's or QCC's): its style, guard bits and each subband's (exponent, mantissa).
#[derive(Clone, Default)]
struct Quant {
    style: u8,
    guard: u32,
    steps: Vec<(u32, u32)>,
}

#[derive(Clone, Default)]
struct Header {
    width: u32,
    height: u32,
    x0: u32,
    y0: u32,
    tile_w: u32,
    tile_h: u32,
    tile_x0: u32,
    tile_y0: u32,
    comps: Vec<Component>,
    sop: bool,
    eph: bool,
    order: u8,
    layers: u32,
    mct: bool,
    coding: Vec<Coding>,
    quant: Vec<Quant>,
    roi: Vec<u32>,
    /// Progression order changes: (resolution start, component start, layer end, resolution end, component end, order).
    poc: Vec<(u32, u32, u32, u32, u32, u8)>,
}

/// To the nearest whole number (halves away from zero).
fn round(v: f32) -> f32 {
    if v >= 0.0 { (v + 0.5) as i64 as f32 } else { -((0.5 - v) as i64 as f32) }
}

fn ceil_div(a: u32, b: u32) -> u32 {
    if b == 0 { 0 } else { a.div_ceil(b) }
}

/// A coding style's parameters (SPcod, SPcoc), from `at`; whether precincts are given (Scod's bit 0).
fn coding(b: &[u8], at: usize, precincts: bool) -> Option<Coding> {
    let levels = *b.get(at)? as u32;
    if levels > 32 {
        return None;
    }
    let cb_w = (*b.get(at + 1)? as u32 & 0xF) + 2;
    let cb_h = (*b.get(at + 2)? as u32 & 0xF) + 2;
    if cb_w + cb_h > 12 {
        return None;
    }
    let style = *b.get(at + 3)?;
    let reversible = *b.get(at + 4)? == 1;
    let sizes = (0..=levels as usize)
        .map(|r| if precincts { b.get(at + 5 + r).map(|&v| ((v & 0xF) as u32, (v >> 4) as u32)) } else { Some((15, 15)) })
        .collect::<Option<Vec<_>>>()?;
    Some(Coding { levels, cb_w, cb_h, style, reversible, precincts: sizes })
}

fn quant(b: &[u8], at: usize, end: usize) -> Option<Quant> {
    let s = *b.get(at)?;
    let style = s & 0x1F;
    let guard = (s >> 5) as u32;
    let mut steps = Vec::new();
    let mut i = at + 1;
    while i < end {
        if style == 0 {
            steps.push(((*b.get(i)? >> 3) as u32, 0));
            i += 1;
        } else {
            let v = be16(b, i)?;
            steps.push(((v >> 11) as u32, (v & 0x7FF) as u32));
            i += 2;
        }
    }
    if steps.is_empty() {
        return None;
    }
    Some(Quant { style, guard, steps })
}

/// The main header's and a tile's header's markers (from `at` to SOD or the first SOT), into `h`; where they end.
fn markers(b: &[u8], mut at: usize, h: &mut Header, main: bool) -> Option<usize> {
    let wide = h.comps.len() > 256;
    let comp = |b: &[u8], i: usize| -> Option<(usize, usize)> { if wide { Some((be16(b, i)? as usize, 2)) } else { Some((*b.get(i)? as usize, 1)) } };
    loop {
        let m = be16(b, at)?;
        if m == 0xFF93 || (main && m == 0xFF90) {
            return Some(at);
        }
        let len = be16(b, at + 2)? as usize;
        let seg = at + 4;
        let end = at + 2 + len;
        if len < 2 || end > b.len() {
            return None;
        }
        match m {
            0xFF51 => {
                h.width = be32(b, seg + 2)?;
                h.height = be32(b, seg + 6)?;
                h.x0 = be32(b, seg + 10)?;
                h.y0 = be32(b, seg + 14)?;
                h.tile_w = be32(b, seg + 18)?;
                h.tile_h = be32(b, seg + 22)?;
                h.tile_x0 = be32(b, seg + 26)?;
                h.tile_y0 = be32(b, seg + 30)?;
                let n = be16(b, seg + 34)? as usize;
                // (The tiles must start at or before the image, the first one reaching into it.)
                if n == 0
                    || n > 16384
                    || h.width <= h.x0
                    || h.height <= h.y0
                    || h.tile_w == 0
                    || h.tile_h == 0
                    || h.tile_x0 > h.x0
                    || h.tile_y0 > h.y0
                    || h.tile_x0.saturating_add(h.tile_w) <= h.x0
                    || h.tile_y0.saturating_add(h.tile_h) <= h.y0
                {
                    return None;
                }
                h.comps = (0..n)
                    .map(|c| {
                        let s = *b.get(seg + 36 + 3 * c)?;
                        let (dx, dy) = (*b.get(seg + 37 + 3 * c)? as u32, *b.get(seg + 38 + 3 * c)? as u32);
                        if dx == 0 || dy == 0 || (s & 0x7F) > 37 {
                            return None;
                        }
                        Some(Component { precision: (s & 0x7F) as u32 + 1, signed: s & 0x80 != 0, dx, dy })
                    })
                    .collect::<Option<Vec<_>>>()?;
                h.coding = vec![Coding::default(); n];
                h.quant = vec![Quant::default(); n];
                h.roi = vec![0; n];
            }
            0xFF52 => {
                let s = *b.get(seg)?;
                h.sop = s & 2 != 0;
                h.eph = s & 4 != 0;
                h.order = *b.get(seg + 1)?;
                h.layers = be16(b, seg + 2)? as u32;
                h.mct = *b.get(seg + 4)? != 0;
                let c = coding(b, seg + 5, s & 1 != 0)?;
                for slot in h.coding.iter_mut() {
                    *slot = c.clone();
                }
            }
            0xFF53 => {
                let (i, n) = comp(b, seg)?;
                let s = *b.get(seg + n)?;
                let c = coding(b, seg + n + 1, s & 1 != 0)?;
                *h.coding.get_mut(i)? = c;
            }
            0xFF5C => {
                let q = quant(b, seg, end)?;
                for slot in h.quant.iter_mut() {
                    *slot = q.clone();
                }
            }
            0xFF5D => {
                let (i, n) = comp(b, seg)?;
                *h.quant.get_mut(i)? = quant(b, seg + n, end)?;
            }
            0xFF5E => {
                let (i, n) = comp(b, seg)?;
                *h.roi.get_mut(i)? = *b.get(seg + n + 1)? as u32;
            }
            0xFF5F => {
                let step = if wide { 9 } else { 7 };
                let mut i = seg;
                h.poc.clear();
                while i + step <= end {
                    let (cs, n) = comp(b, i + 1)?;
                    let layer_end = be16(b, i + 1 + n)? as u32;
                    let re = *b.get(i + 3 + n)? as u32;
                    let (ce, m) = comp(b, i + 4 + n)?;
                    let order = *b.get(i + 4 + n + m)?;
                    h.poc.push((*b.get(i)? as u32, cs as u32, layer_end, re, if ce == 0 { 16384 } else { ce as u32 }, order));
                    i += step;
                }
            }
            0xFF60 | 0xFF61 => return None,
            _ => {}
        }
        at = end;
    }
}

// --------------------------------------------------------------------------------------------- tag trees

/// A tag tree (B.10.2): a quadtree of minima, its values found as far as each threshold needs.
struct TagTree {
    w: usize,
    h: usize,
    /// Each level's (width, its nodes' (value, known lower bound, whether it's known)).
    levels: Vec<(usize, Vec<(u32, bool)>)>,
}

impl TagTree {
    fn new(w: usize, h: usize) -> TagTree {
        let mut levels = Vec::new();
        let (mut lw, mut lh) = (w.max(1), h.max(1));
        loop {
            levels.push((lw, vec![(0u32, false); lw * lh]));
            if lw == 1 && lh == 1 {
                break;
            }
            lw = lw.div_ceil(2);
            lh = lh.div_ceil(2);
        }
        TagTree { w, h, levels }
    }

    /// Whether leaf (x, y)'s value is below `threshold` (reading bits as needed); its value when it's known.
    fn decode(&mut self, bits: &mut Bits, x: usize, y: usize, threshold: u32) -> Option<(bool, u32)> {
        if x >= self.w || y >= self.h {
            return None;
        }
        // The path from the root down to the leaf.
        let n = self.levels.len();
        let mut path = Vec::with_capacity(n);
        for (k, (lw, _)) in self.levels.iter().enumerate() {
            path.push(((y >> k) * lw) + (x >> k));
        }
        let mut low = 0u32;
        for k in (0..n).rev() {
            let i = path[k];
            let node = &mut self.levels[k].1[i];
            if node.0 < low {
                node.0 = low;
            }
            while !node.1 && node.0 < threshold {
                if bits.bit()? == 1 {
                    node.1 = true;
                } else {
                    node.0 += 1;
                }
            }
            low = node.0;
            if !node.1 {
                return Some((false, node.0));
            }
        }
        Some((low < threshold, low))
    }
}

/// A packet header's bits (B.10.1: after a 0xFF byte, the next has only 7).
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    byte: u32,
    left: u32,
}

impl Bits<'_> {
    fn bit(&mut self) -> Option<u32> {
        if self.left == 0 {
            let stuffed = self.byte == 0xFF;
            self.byte = *self.data.get(self.at)? as u32;
            self.at += 1;
            self.left = if stuffed { 7 } else { 8 };
        }
        self.left -= 1;
        Some((self.byte >> self.left) & 1)
    }

    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Some(v)
    }

    /// Where the header ends (its last byte's stuffing finished).
    fn end(&mut self) -> usize {
        if self.byte == 0xFF && self.left == 0 {
            self.at += 1;
        }
        self.at
    }
}

// ------------------------------------------------------------------------------------------ the structure

/// A code-block: where it is in its subband, its data's segments as the packets give them, and what it decodes to.
struct Block {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    included: bool,
    zero_planes: u32,
    lblock: u32,
    passes: usize,
    /// Each segment: its bytes and how many passes it holds.
    segments: Vec<(Vec<u8>, usize)>,
}

struct Band {
    /// 0 LL, 1 HL, 2 LH, 3 HH.
    kind: u8,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    /// Its magnitude's bit-planes (Mb), and its step (for an irreversible transform).
    planes: u32,
    step: f32,
    /// Each precinct's code-blocks (indexes into `blocks`), with its grid's width and height and tag trees.
    precincts: Vec<Precinct>,
    blocks: Vec<Block>,
}

struct Precinct {
    blocks: Vec<usize>,
    gw: usize,
    inclusion: TagTree,
    zeros: TagTree,
}

struct Resolution {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    pw: u32,
    ph: u32,
    bands: Vec<Band>,
}

struct TileComp {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    resolutions: Vec<Resolution>,
}

/// The highest precinct exponent wanted, the code-block size within it, for resolution `r`.
fn block_size(c: &Coding, r: usize) -> (u32, u32) {
    let (ppx, ppy) = c.precincts.get(r).copied().unwrap_or((15, 15));
    if r == 0 { (c.cb_w.min(ppx), c.cb_h.min(ppy)) } else { (c.cb_w.min(ppx.saturating_sub(1)), c.cb_h.min(ppy.saturating_sub(1))) }
}

fn tile_comp(h: &Header, ci: usize, tx0: u32, ty0: u32, tx1: u32, ty1: u32) -> Option<TileComp> {
    let comp = h.comps[ci];
    let c = &h.coding[ci];
    let q = &h.quant[ci];
    let (x0, y0, x1, y1) = (ceil_div(tx0, comp.dx), ceil_div(ty0, comp.dy), ceil_div(tx1, comp.dx), ceil_div(ty1, comp.dy));
    let nl = c.levels;
    let mut resolutions = Vec::new();
    for r in 0..=nl {
        let scale = 1u64 << (nl - r);
        let rc = |v: u32| (v as u64).div_ceil(scale) as u32;
        let (rx0, ry0, rx1, ry1) = (rc(x0), rc(y0), rc(x1), rc(y1));
        let (ppx, ppy) = c.precincts.get(r as usize).copied().unwrap_or((15, 15));
        let pw = if rx1 > rx0 { rx1.div_ceil(1 << ppx) - (rx0 >> ppx) } else { 0 };
        let ph = if ry1 > ry0 { ry1.div_ceil(1 << ppy) - (ry0 >> ppy) } else { 0 };
        if (pw as u64) * (ph as u64) > 1 << 20 {
            return None;
        }
        let kinds: &[u8] = if r == 0 { &[0] } else { &[1, 2, 3] };
        let mut bands = Vec::new();
        for &kind in kinds {
            // The subband's place (B.5): its decomposition level, and which half it is each way.
            let nb = if r == 0 { nl } else { nl - r + 1 };
            let (xob, yob) = ((kind & 1) as u64, (kind >> 1) as u64);
            let s = 1u64 << nb;
            let half = if nb == 0 { 0 } else { 1u64 << (nb - 1) };
            let bc = |v: u32, o: u64| ((v as u64).saturating_sub(half * o)).div_ceil(s) as u32;
            let (bx0, by0, bx1, by1) = (bc(x0, xob), bc(y0, yob), bc(x1, xob), bc(y1, yob));
            // Its quantization (E.1): its exponent and mantissa, given for each subband or derived from the LL
            // band's (the exponent less one for each level above).
            let index = if r == 0 { 0 } else { 3 * (r as usize - 1) + kind as usize };
            let (exp, mant) = if q.style == 1 {
                let (e0, m0) = q.steps.first().copied().unwrap_or((0, 0));
                ((e0 + nb).saturating_sub(nl), m0)
            } else {
                q.steps.get(index).copied().or_else(|| q.steps.last().copied()).unwrap_or((0, 0))
            };
            let gain = [0, 1, 1, 2][kind as usize];
            let planes = (q.guard + exp).saturating_sub(1);
            let range = comp.precision + gain;
            let step = if c.reversible {
                1.0
            } else {
                let e = range as i32 - exp as i32;
                let p = if e >= 0 { (1u64 << e.min(62)) as f32 } else { 1.0 / (1u64 << (-e).min(62)) as f32 };
                p * (1.0 + mant as f32 / 2048.0)
            };
            // Its code-blocks, precinct by precinct.
            let (cw, chh) = block_size(c, r as usize);
            let mut precincts = Vec::new();
            let mut blocks = Vec::new();
            for py in 0..ph {
                for px in 0..pw {
                    // The precinct in this subband's coordinates.
                    let (ppx_b, ppy_b) = if r == 0 { (ppx, ppy) } else { (ppx.saturating_sub(1), ppy.saturating_sub(1)) };
                    let px0 = ((rx0 >> ppx) + px) << ppx_b;
                    let py0 = ((ry0 >> ppy) + py) << ppy_b;
                    let (px1, py1) = (px0 + (1 << ppx_b), py0 + (1 << ppy_b));
                    let (cx0, cy0, cx1, cy1) = (px0.max(bx0), py0.max(by0), px1.min(bx1), py1.min(by1));
                    let (gw, gh) = if cx1 > cx0 && cy1 > cy0 {
                        ((cx1.div_ceil(1 << cw) - (cx0 >> cw)) as usize, (cy1.div_ceil(1 << chh) - (cy0 >> chh)) as usize)
                    } else {
                        (0, 0)
                    };
                    let mut list = Vec::with_capacity(gw * gh);
                    for gy in 0..gh {
                        for gx in 0..gw {
                            let bx = ((cx0 >> cw) + gx as u32) << cw;
                            let by = ((cy0 >> chh) + gy as u32) << chh;
                            list.push(blocks.len());
                            blocks.push(Block {
                                x0: bx.max(cx0),
                                y0: by.max(cy0),
                                x1: (bx + (1 << cw)).min(cx1),
                                y1: (by + (1 << chh)).min(cy1),
                                included: false,
                                zero_planes: 0,
                                lblock: 3,
                                passes: 0,
                                segments: Vec::new(),
                            });
                        }
                    }
                    if blocks.len() > 1 << 22 {
                        return None;
                    }
                    precincts.push(Precinct { blocks: list, gw, inclusion: TagTree::new(gw, gh), zeros: TagTree::new(gw, gh) });
                }
            }
            bands.push(Band { kind, x0: bx0, y0: by0, x1: bx1, y1: by1, planes, step, precincts, blocks });
        }
        resolutions.push(Resolution { x0: rx0, y0: ry0, x1: rx1, y1: ry1, pw, ph, bands });
    }
    Some(TileComp { x0, y0, x1, y1, resolutions })
}

/// The most passes a segment of a code-block holds, by its style, starting at pass `first`: all of them; with each
/// pass terminated, one; arithmetic coding bypassed, the first ten (four bit-planes), then two (raw) and one (MQ) in
/// turn.
fn segment_passes(style: u8, first: usize) -> usize {
    if style & 4 != 0 {
        1
    } else if style & 1 != 0 {
        if first < 10 {
            10 - first
        } else if (first - 10) % 3 == 0 {
            2
        } else {
            1
        }
    } else {
        MOST_PASSES
    }
}

/// A packet (B.9, B.10): its header, read; its code-blocks' data taken. `at` moves past it.
fn packet(data: &[u8], at: &mut usize, layer: u32, res: &mut Resolution, precinct: usize, style: u8, sop: bool, eph: bool) -> Option<()> {
    if sop && be16(data, *at) == Some(0xFF91) {
        *at += 6;
    }
    let mut bits = Bits { data, at: *at, byte: 0, left: 0 };
    let mut taken: Vec<(usize, usize, Vec<(usize, usize)>)> = Vec::new();
    if bits.bit()? == 1 {
        for (bi, band) in res.bands.iter_mut().enumerate() {
            let Some(p) = band.precincts.get_mut(precinct) else { continue };
            for k in 0..p.blocks.len() {
                let (gx, gy) = (k % p.gw.max(1), k / p.gw.max(1));
                let block = &mut band.blocks[p.blocks[k]];
                let first = !block.included;
                let included = if first { p.inclusion.decode(&mut bits, gx, gy, layer + 1)?.0 } else { bits.bit()? == 1 };
                if !included {
                    continue;
                }
                if first {
                    // Its zero bit-planes: the tree's value.
                    let mut t = 1;
                    let zeros = loop {
                        let (below, v) = p.zeros.decode(&mut bits, gx, gy, t)?;
                        if below {
                            break v;
                        }
                        t += 1;
                        if t > 64 {
                            return None;
                        }
                    };
                    block.included = true;
                    block.zero_planes = zeros;
                }
                // How many passes.
                let n = if bits.bit()? == 0 {
                    1
                } else if bits.bit()? == 0 {
                    2
                } else {
                    let v = bits.bits(2)?;
                    if v < 3 {
                        3 + v as usize
                    } else {
                        let v = bits.bits(5)?;
                        if v < 31 { 6 + v as usize } else { 37 + bits.bits(7)? as usize }
                    }
                };
                while bits.bit()? == 1 {
                    block.lblock += 1;
                    if block.lblock > 32 {
                        return None;
                    }
                }
                // The lengths of the segments these passes are in (B.10.7): a segment still open takes what it has
                // room for; the rest open new ones.
                let mut lengths = Vec::new();
                let mut left = n;
                while left > 0 {
                    let pass = block.passes;
                    let open = match block.segments.last() {
                        Some((_, k)) if *k < segment_passes(style, pass - *k) => Some(*k),
                        _ => None,
                    };
                    let in_segment = open.unwrap_or(0);
                    let here = (segment_passes(style, pass - in_segment) - in_segment).min(left);
                    let len = bits.bits(block.lblock + (usize::BITS - 1 - here.leading_zeros()))? as usize;
                    if open.is_some() {
                        if let Some(last) = block.segments.last_mut() {
                            last.1 += here;
                        }
                    } else {
                        block.segments.push((Vec::new(), here));
                    }
                    lengths.push((block.segments.len() - 1, len));
                    block.passes += here;
                    left -= here;
                    if block.passes > MOST_PASSES {
                        return None;
                    }
                }
                taken.push((bi, p.blocks[k], lengths));
            }
        }
    }
    *at = bits.end();
    if eph && be16(data, *at) == Some(0xFF92) {
        *at += 2;
    }
    // The packet's body: each code-block's bytes, in the same order, into its segments.
    for (bi, k, lengths) in taken {
        let block = &mut res.bands[bi].blocks[k];
        for (segment, len) in lengths {
            let bytes = data.get(*at..at.checked_add(len)?)?;
            *at += len;
            if let Some(seg) = block.segments.get_mut(segment) {
                seg.0.extend_from_slice(bytes);
            }
        }
    }
    Some(())
}

// ----------------------------------------------------------------------------------------------- tier 1

/// The contexts' states at a code-block's start (D.7): uniform's at 46, run-length's at 3, the first zero coding
/// context's at 4, the rest at 0.
fn fresh_contexts() -> [u8; 19] {
    let mut c = [0u8; 19];
    c[0] = 4 << 1;
    c[17] = 3 << 1;
    c[18] = 46 << 1;
    c
}

const SIGNIFICANT: u8 = 1;
const VISITED: u8 = 2;
const REFINED: u8 = 4;
const NEGATIVE: u8 = 8;

/// The raw bits of a bypassed pass (D.6), as the MQ coder's software does them.
struct Raw<'a> {
    data: &'a [u8],
    at: usize,
    c: u32,
    ct: u32,
}

impl Raw<'_> {
    fn bit(&mut self) -> u32 {
        if self.ct == 0 {
            if self.c == 0xFF {
                let next = *self.data.get(self.at).unwrap_or(&0xFF) as u32;
                if next > 0x8F {
                    self.c = 0xFF;
                    self.ct = 8;
                } else {
                    self.c = next;
                    self.at += 1;
                    self.ct = 7;
                }
            } else {
                self.c = *self.data.get(self.at).unwrap_or(&0xFF) as u32;
                self.at += 1;
                self.ct = 8;
            }
        }
        self.ct -= 1;
        (self.c >> self.ct) & 1
    }
}

/// A coder for a pass: the MQ coder or raw bits.
enum Coder<'a> {
    Mq(Mq<'a>),
    Raw(Raw<'a>),
}

/// A code-block decoded (Annex D): its coefficients' magnitudes (with the bit-plane each is known down to), signed.
#[allow(clippy::too_many_arguments)]
fn tier1(block: &Block, kind: u8, planes: u32, style: u8, roi: u32) -> Option<(usize, usize, Vec<i32>, Vec<u8>)> {
    let (w, h) = ((block.x1 - block.x0) as usize, (block.y1 - block.y0) as usize);
    let mut mag = vec![0i32; w * h];
    let mut flags = vec![0u8; w * h];
    // (What's known of each coefficient's magnitude: down to which plane.)
    let mut low = vec![0u8; w * h];
    if w == 0 || h == 0 || block.passes == 0 {
        return Some((w, h, mag, low));
    }
    let top = (planes + roi).saturating_sub(block.zero_planes);
    if top == 0 || top > 31 {
        return Some((w, h, mag, low));
    }
    let causal = style & 8 != 0;
    let mut cx = fresh_contexts();
    let sig = |flags: &[u8], x: isize, y: isize, row0: usize| -> u32 {
        if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
            return 0;
        }
        // (Vertically causal: what's below this stripe counts as insignificant.)
        if causal && y as usize >= row0 + 4 {
            return 0;
        }
        (flags[y as usize * w + x as usize] & SIGNIFICANT) as u32
    };
    let zc = |flags: &[u8], x: usize, y: usize| -> usize {
        let (xi, yi) = (x as isize, y as isize);
        let row0 = y - y % 4;
        let hsum = sig(flags, xi - 1, yi, row0) + sig(flags, xi + 1, yi, row0);
        let vsum = sig(flags, xi, yi - 1, row0) + sig(flags, xi, yi + 1, row0);
        let dsum = sig(flags, xi - 1, yi - 1, row0) + sig(flags, xi + 1, yi - 1, row0) + sig(flags, xi - 1, yi + 1, row0) + sig(flags, xi + 1, yi + 1, row0);
        let (hh, vv) = if kind == 1 { (vsum, hsum) } else { (hsum, vsum) };
        if kind == 3 {
            let hv = hh + vv;
            match dsum {
                d if d >= 3 => 8,
                2 => if hv >= 1 { 7 } else { 6 },
                1 => match hv { 0 => 3, 1 => 4, _ => 5 },
                _ => match hv { 0 => 0, 1 => 1, _ => 2 },
            }
        } else {
            match (hh, vv, dsum) {
                (2, _, _) => 8,
                (1, v, _) if v >= 1 => 7,
                (1, 0, d) if d >= 1 => 6,
                (1, 0, 0) => 5,
                (0, 2, _) => 4,
                (0, 1, _) => 3,
                (0, 0, d) if d >= 2 => 2,
                (0, 0, 1) => 1,
                _ => 0,
            }
        }
    };
    // A coefficient's sign's context and XOR bit (D.3.2).
    let sc = |flags: &[u8], x: usize, y: usize| -> (usize, u32) {
        let (xi, yi) = (x as isize, y as isize);
        let row0 = y - y % 4;
        let contrib = |dx: isize, dy: isize| -> i32 {
            if sig(flags, xi + dx, yi + dy, row0) == 0 {
                0
            } else if flags[(yi + dy) as usize * w + (xi + dx) as usize] & NEGATIVE != 0 {
                -1
            } else {
                1
            }
        };
        let hc = (contrib(-1, 0) + contrib(1, 0)).clamp(-1, 1);
        let vc = (contrib(0, -1) + contrib(0, 1)).clamp(-1, 1);
        match (hc, vc) {
            (1, 1) => (13, 0),
            (1, 0) => (12, 0),
            (1, -1) => (11, 0),
            (0, 1) => (10, 0),
            (0, 0) => (9, 0),
            (0, -1) => (10, 1),
            (-1, 1) => (11, 1),
            (-1, 0) => (12, 1),
            _ => (13, 1),
        }
    };
    // The passes, segment by segment: each segment a fresh MQ decoder (or raw bits, for a bypassed pass).
    let mut pass = 0usize;
    let total = block.passes.min(3 * top as usize - 2);
    let mut plane = top - 1;
    // (Which kind of pass comes next: 0 significance propagation, 1 magnitude refinement, 2 clean-up; the first is a
    // clean-up.)
    let mut which = 2u8;
    for (data, count) in &block.segments {
        // (A segment's passes are all bypassed, raw, or all through the MQ coder, begun afresh.)
        let raw = style & 1 != 0 && pass >= 10 && which != 2;
        let mut coder = if raw { Coder::Raw(Raw { data, at: 0, c: 0, ct: 0 }) } else { Coder::Mq(Mq::new(data)) };
        for _ in 0..*count {
            if pass >= total {
                break;
            }
            let bypassed = raw;
            let coder = &mut coder;
            let one = 1i32 << plane;
            let mut bit = |cx: &mut [u8; 19], k: usize| -> u32 {
                match coder {
                    Coder::Mq(m) => m.bit(&mut cx[k]),
                    Coder::Raw(r) => r.bit(),
                }
            };
            match which {
                0 => {
                    for y0 in (0..h).step_by(4) {
                        for x in 0..w {
                            for y in y0..(y0 + 4).min(h) {
                                let i = y * w + x;
                                if flags[i] & SIGNIFICANT != 0 {
                                    continue;
                                }
                                let k = zc(&flags, x, y);
                                if k == 0 {
                                    continue;
                                }
                                flags[i] |= VISITED;
                                if bit(&mut cx, k) == 1 {
                                    let (s, xor) = sc(&flags, x, y);
                                    let neg = if bypassed { bit(&mut cx, s) } else { bit(&mut cx, s) ^ xor };
                                    flags[i] |= SIGNIFICANT | if neg == 1 { NEGATIVE } else { 0 };
                                    mag[i] = one;
                                    low[i] = plane as u8;
                                }
                            }
                        }
                    }
                }
                1 => {
                    for y0 in (0..h).step_by(4) {
                        for x in 0..w {
                            for y in y0..(y0 + 4).min(h) {
                                let i = y * w + x;
                                if flags[i] & (SIGNIFICANT | VISITED) != SIGNIFICANT {
                                    continue;
                                }
                                let k = if flags[i] & REFINED != 0 {
                                    16
                                } else {
                                    let row0 = y0;
                                    let (xi, yi) = (x as isize, y as isize);
                                    let any = (-1..=1).any(|dy| (-1..=1).any(|dx| (dx, dy) != (0, 0) && sig(&flags, xi + dx, yi + dy, row0) != 0));
                                    if any { 15 } else { 14 }
                                };
                                if bit(&mut cx, k) == 1 {
                                    mag[i] |= one;
                                }
                                flags[i] |= REFINED;
                                low[i] = plane as u8;
                            }
                        }
                    }
                }
                _ => {
                    for y0 in (0..h).step_by(4) {
                        for x in 0..w {
                            let mut y = y0;
                            // A run: four rows of nothing significant around, coded as one bit.
                            if y0 + 4 <= h
                                && (y0..y0 + 4).all(|yy| flags[yy * w + x] & (SIGNIFICANT | VISITED) == 0 && zc(&flags, x, yy) == 0)
                            {
                                if bit(&mut cx, 17) == 0 {
                                    continue;
                                }
                                let at = (bit(&mut cx, 18) << 1 | bit(&mut cx, 18)) as usize;
                                y = y0 + at;
                                let i = y * w + x;
                                let (s, xor) = sc(&flags, x, y);
                                let neg = bit(&mut cx, s) ^ xor;
                                flags[i] |= SIGNIFICANT | if neg == 1 { NEGATIVE } else { 0 };
                                mag[i] = one;
                                low[i] = plane as u8;
                                y += 1;
                            }
                            while y < (y0 + 4).min(h) {
                                let i = y * w + x;
                                if flags[i] & (SIGNIFICANT | VISITED) == 0 {
                                    let k = zc(&flags, x, y);
                                    if bit(&mut cx, k) == 1 {
                                        let (s, xor) = sc(&flags, x, y);
                                        let neg = bit(&mut cx, s) ^ xor;
                                        flags[i] |= SIGNIFICANT | if neg == 1 { NEGATIVE } else { 0 };
                                        mag[i] = one;
                                        low[i] = plane as u8;
                                    }
                                }
                                y += 1;
                            }
                        }
                    }
                    if style & 32 != 0 {
                        // (The segmentation symbol: 1010, passed over.)
                        for _ in 0..4 {
                            bit(&mut cx, 18);
                        }
                    }
                    for f in flags.iter_mut() {
                        *f &= !VISITED;
                    }
                }
            }
            if style & 2 != 0 {
                cx = fresh_contexts();
            }
            pass += 1;
            which = match which {
                2 => {
                    if plane == 0 {
                        break;
                    }
                    plane -= 1;
                    0
                }
                0 => 1,
                _ => 2,
            };
        }
        if pass >= total {
            break;
        }
    }
    // Signs, and what a region of interest shifted up shifted back down.
    let roi_top = if roi > 0 { 1i32 << roi } else { i32::MAX };
    let out: Vec<i32> = mag
        .iter()
        .zip(&flags)
        .map(|(&m, &f)| {
            let m = if roi > 0 && m < roi_top { m } else if roi > 0 { m >> roi } else { m };
            if f & NEGATIVE != 0 { -m } else { m }
        })
        .collect();
    Some((w, h, out, low))
}

// ------------------------------------------------------------------------------------------ the transform

/// One row (or column) inverse transformed (F.3.6-F.3.8): interleaved samples from position `i0`, by 5/3 or 9/7.
fn idwt_1d(x: &mut [f32], i0: u32, reversible: bool) {
    let n = x.len();
    if n == 0 {
        return;
    }
    if n == 1 {
        if i0 % 2 == 1 {
            x[0] = if reversible { (x[0] as i64 / 2) as f32 } else { x[0] / 2.0 };
        }
        return;
    }
    // (Samples beyond either end mirrored: periodic symmetric extension.)
    let ext = 4usize;
    let mut y = vec![0f32; n + 2 * ext];
    for k in 0..n + 2 * ext {
        let mut j = k as isize - ext as isize;
        let period = 2 * (n as isize - 1);
        j = j.rem_euclid(period);
        if j >= n as isize {
            j = period - j;
        }
        y[k] = x[j as usize];
    }
    let off = ext as isize;
    let odd = |k: isize| (k + i0 as isize).rem_euclid(2) == 1;
    let len = y.len() as isize;
    if reversible {
        let mut k = 1;
        while k < len - 1 {
            if !odd(k - off) {
                y[k as usize] -= ((y[(k - 1) as usize] as i64 + y[(k + 1) as usize] as i64 + 2).div_euclid(4)) as f32;
            }
            k += 1;
        }
        let mut k = 1;
        while k < len - 1 {
            if odd(k - off) {
                y[k as usize] += ((y[(k - 1) as usize] as i64 + y[(k + 1) as usize] as i64).div_euclid(2)) as f32;
            }
            k += 1;
        }
    } else {
        const ALPHA: f32 = -1.586_134_3;
        const BETA: f32 = -0.052_980_118;
        const GAMMA: f32 = 0.882_911_1;
        const DELTA: f32 = 0.443_506_85;
        const K: f32 = 1.230_174_1;
        for k in 0..len {
            if odd(k - off) {
                y[k as usize] /= K;
            } else {
                y[k as usize] *= K;
            }
        }
        for (coef, even) in [(DELTA, true), (GAMMA, false), (BETA, true), (ALPHA, false)] {
            let mut k = 1;
            while k < len - 1 {
                if odd(k - off) != even {
                    y[k as usize] -= coef * (y[(k - 1) as usize] + y[(k + 1) as usize]);
                }
                k += 1;
            }
        }
    }
    x.copy_from_slice(&y[ext..ext + n]);
}

/// A tile-component's samples, from its code-blocks' coefficients, through the inverse transform (F.3).
fn reconstruct(tc: &TileComp, coefs: &[Vec<Vec<f32>>], reversible: bool) -> Vec<f32> {
    // Resolution 0: its LL band as it is.
    let r0 = &tc.resolutions[0];
    let (mut w, mut h) = ((r0.x1 - r0.x0) as usize, (r0.y1 - r0.y0) as usize);
    let mut a = coefs[0][0].clone();
    for (r, res) in tc.resolutions.iter().enumerate().skip(1) {
        let (nw, nh) = ((res.x1 - res.x0) as usize, (res.y1 - res.y0) as usize);
        let mut out = vec![0f32; nw * nh];
        // Interleave (2D_INTERLEAVE): LL at even columns and rows (in the absolute coordinates), HL odd columns, LH
        // odd rows, HH both.
        let (u0, v0) = (res.x0 as usize, res.y0 as usize);
        let place = |out: &mut [f32], src: &[f32], sw: usize, sh: usize, xo: usize, yo: usize| {
            // (Each band's first sample goes where its parity first comes in the resolution.)
            let first_x = if (u0 % 2) == xo { 0 } else { 1 };
            let first_y = if (v0 % 2) == yo { 0 } else { 1 };
            for y in 0..sh {
                let oy = first_y + 2 * y;
                if oy >= nh {
                    break;
                }
                for x in 0..sw {
                    let ox = first_x + 2 * x;
                    if ox >= nw {
                        break;
                    }
                    out[oy * nw + ox] = src[y * sw + x];
                }
            }
        };
        place(&mut out, &a, w, h, 0, 0);
        for (b, band) in res.bands.iter().enumerate() {
            let (bw, bh) = ((band.x1 - band.x0) as usize, (band.y1 - band.y0) as usize);
            let (xo, yo) = ((band.kind & 1) as usize, (band.kind >> 1) as usize);
            place(&mut out, &coefs[r][b], bw, bh, xo, yo);
        }
        // Rows, then columns.
        let mut line = vec![0f32; nw.max(nh)];
        for y in 0..nh {
            let row = &mut out[y * nw..(y + 1) * nw];
            idwt_1d(row, u0 as u32, reversible);
        }
        for x in 0..nw {
            for y in 0..nh {
                line[y] = out[y * nw + x];
            }
            idwt_1d(&mut line[..nh], v0 as u32, reversible);
            for y in 0..nh {
                out[y * nw + x] = line[y];
            }
        }
        a = out;
        w = nw;
        h = nh;
    }
    a
}

// --------------------------------------------------------------------------------------------- decoding

/// The order of a tile's packets: (layer, resolution, component, precinct) by progression order `order`, keys
/// from where each precinct is on the reference grid.
#[allow(clippy::too_many_arguments)]
fn packet_order(h: &Header, tcs: &[TileComp], tx0: u32, ty0: u32, order: u8, layers: (u32, u32), res: (u32, u32), comps: (u32, u32)) -> Vec<(u32, u32, u32, u32)> {
    // (Held to a most: a stream can say it has more layers and precincts than there are bytes.)
    let precincts: u64 = tcs.iter().flat_map(|tc| tc.resolutions.iter().map(|r| r.pw as u64 * r.ph as u64)).sum();
    if precincts.saturating_mul(layers.1.saturating_sub(layers.0) as u64) > MOST_PACKETS {
        return Vec::new();
    }
    let mut list: Vec<([u64; 5], (u32, u32, u32, u32))> = Vec::new();
    for (c, tc) in tcs.iter().enumerate() {
        let c = c as u32;
        if c < comps.0 || c >= comps.1 {
            continue;
        }
        let comp = h.comps[c as usize];
        let coding = &h.coding[c as usize];
        for (r, rs) in tc.resolutions.iter().enumerate() {
            let r = r as u32;
            if r < res.0 || r >= res.1 {
                continue;
            }
            let (ppx, ppy) = coding.precincts.get(r as usize).copied().unwrap_or((15, 15));
            let scale = coding.levels - r;
            for p in 0..rs.pw * rs.ph {
                let (i, j) = (p % rs.pw.max(1), p / rs.pw.max(1));
                let px = if i == 0 { tx0 as u64 } else { ((((rs.x0 >> ppx) + i) as u64) << ppx << scale) * comp.dx as u64 };
                let py = if j == 0 { ty0 as u64 } else { ((((rs.y0 >> ppy) + j) as u64) << ppy << scale) * comp.dy as u64 };
                for l in layers.0..layers.1 {
                    let (l64, r64, c64, p64) = (l as u64, r as u64, c as u64, p as u64);
                    let key = match order {
                        0 => [l64, r64, c64, p64, 0],
                        1 => [r64, l64, c64, p64, 0],
                        2 => [r64, py, px, c64, l64],
                        3 => [py, px, c64, r64, l64],
                        _ => [c64, py, px, r64, l64],
                    };
                    list.push((key, (l, r, c, p)));
                }
            }
        }
    }
    list.sort_by(|a, b| a.0.cmp(&b.0));
    list.into_iter().map(|(_, p)| p).collect()
}

/// A tile: its tile-parts' data, through its packets and code-blocks, into each component's samples on `planes`.
fn tile(h: &Header, t: usize, data: &[u8], tile_h: &Header, planes: &mut [Vec<f32>], budget: &mut usize) -> Option<()> {
    let tiles_x = (h.width - h.tile_x0).div_ceil(h.tile_w);
    let (p, q) = ((t as u32) % tiles_x, (t as u32) / tiles_x);
    let tx0 = (h.tile_x0 + p * h.tile_w).max(h.x0);
    let ty0 = (h.tile_y0 + q * h.tile_h).max(h.y0);
    let tx1 = (h.tile_x0 + (p + 1) * h.tile_w).min(h.width);
    let ty1 = (h.tile_y0 + (q + 1) * h.tile_h).min(h.height);
    if tx1 <= tx0 || ty1 <= ty0 {
        return None;
    }
    let mut tcs: Vec<TileComp> = (0..h.comps.len()).map(|c| tile_comp(tile_h, c, tx0, ty0, tx1, ty1)).collect::<Option<_>>()?;
    let max_res = tcs.iter().map(|tc| tc.resolutions.len() as u32).max().unwrap_or(0);
    let mut order = Vec::new();
    if tile_h.poc.is_empty() {
        order = packet_order(tile_h, &tcs, tx0, ty0, tile_h.order, (0, tile_h.layers), (0, max_res), (0, h.comps.len() as u32));
    } else {
        let mut seen = alloc::collections::BTreeSet::new();
        for &(rs, cs, le, re, ce, o) in &tile_h.poc {
            for pk in packet_order(tile_h, &tcs, tx0, ty0, o, (0, le.min(tile_h.layers)), (rs, re.min(max_res)), (cs, ce.min(h.comps.len() as u32))) {
                if seen.insert(pk) {
                    order.push(pk);
                }
            }
        }
    }
    let mut at = 0;
    for (l, r, c, pr) in order {
        if at >= data.len() {
            break;
        }
        let style = tile_h.coding[c as usize].style;
        let res = &mut tcs[c as usize].resolutions[r as usize];
        if packet(data, &mut at, l, res, pr as usize, style, tile_h.sop, tile_h.eph).is_none() {
            break;
        }
    }
    // Each component: its code-blocks decoded, dequantised, put in their bands; then transformed back.
    for (c, tc) in tcs.iter().enumerate() {
        let coding = &tile_h.coding[c];
        let roi = tile_h.roi[c];
        let mut coefs: Vec<Vec<Vec<f32>>> = Vec::new();
        for res in &tc.resolutions {
            let mut bands = Vec::new();
            for band in &res.bands {
                let (bw, bh) = ((band.x1 - band.x0) as usize, (band.y1 - band.y0) as usize);
                *budget = budget.checked_sub(bw * bh)?;
                let mut v = vec![0f32; bw * bh];
                for block in &band.blocks {
                    let (w, hh, mags, low) = tier1(block, band.kind, band.planes, coding.style, roi)?;
                    for y in 0..hh {
                        for x in 0..w {
                            let m = mags[y * w + x];
                            let value = if coding.reversible {
                                m as f32
                            } else if m == 0 {
                                0.0
                            } else {
                                // (Midway between what the bits decoded say it could be.)
                                let half = (1i64 << low[y * w + x]) as f32 / 2.0;
                                (m.unsigned_abs() as f32 + half).copysign(m as f32) * band.step
                            };
                            let (bx, by) = ((block.x0 - band.x0) as usize + x, (block.y0 - band.y0) as usize + y);
                            v[by * bw + bx] = value;
                        }
                    }
                }
                bands.push(v);
            }
            coefs.push(bands);
        }
        let samples = reconstruct(tc, &coefs, coding.reversible);
        // Into the component's plane (its own grid: the image's, subsampled).
        let comp = h.comps[c];
        let (cw0, ch0) = (ceil_div(h.x0, comp.dx), ceil_div(h.y0, comp.dy));
        let cw = (ceil_div(h.width, comp.dx) - cw0) as usize;
        let (tw, th) = ((tc.x1 - tc.x0) as usize, (tc.y1 - tc.y0) as usize);
        for y in 0..th {
            for x in 0..tw {
                let (gx, gy) = ((tc.x0 - cw0) as usize + x, (tc.y0 - ch0) as usize + y);
                if let Some(s) = planes[c].get_mut(gy * cw + gx) {
                    *s = samples[y * tw + x];
                }
            }
        }
    }
    Some(())
}

/// A JP2 file's boxes: its codestream, whether its colour is YCC, and its palette (entries for each component made,
/// and which of the codestream's components it takes).
struct Jp2<'a> {
    codestream: &'a [u8],
    ycc: bool,
    palette: Option<(Vec<Vec<u32>>, Vec<u8>, Vec<(usize, u8, usize)>)>,
}

fn boxes(b: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= b.len() {
        let Some(len) = be32(b, at) else { break };
        let kind = &b[at + 4..at + 8];
        let (start, end) = match len {
            0 => (at + 8, b.len()),
            1 => {
                let Some(hi) = be32(b, at + 8) else { break };
                let Some(lo) = be32(b, at + 12) else { break };
                let l = ((hi as u64) << 32 | lo as u64) as usize;
                (at + 16, at.saturating_add(l).min(b.len()))
            }
            l => (at + 8, at.saturating_add(l as usize).min(b.len())),
        };
        if end < start {
            break;
        }
        out.push((kind, &b[start..end]));
        at = end;
        if len == 0 {
            break;
        }
    }
    out
}

fn jp2(b: &[u8]) -> Option<Jp2<'_>> {
    if !b.starts_with(&[0, 0, 0, 0x0C, b'j', b'P', b' ', b' ']) {
        return Some(Jp2 { codestream: b, ycc: false, palette: None });
    }
    let mut out = Jp2 { codestream: &[], ycc: false, palette: None };
    let mut pclr: Option<(Vec<Vec<u32>>, Vec<u8>)> = None;
    let mut cmap: Vec<(usize, u8, usize)> = Vec::new();
    for (kind, body) in boxes(b) {
        match kind {
            b"jp2h" => {
                for (k, inner) in boxes(body) {
                    match k {
                        b"colr" if inner.first() == Some(&1) => out.ycc = matches!(be32(inner, 3), Some(18) | Some(22)),
                        b"pclr" => {
                            let entries = be16(inner, 0)? as usize;
                            let n = *inner.get(2)? as usize;
                            let depths: Vec<u8> = inner.get(3..3 + n)?.to_vec();
                            let mut cols = vec![Vec::with_capacity(entries); n];
                            let mut i = 3 + n;
                            for _ in 0..entries {
                                for (c, &d) in depths.iter().enumerate() {
                                    let bytes = ((d & 0x7F) as usize + 8) / 8;
                                    let mut v = 0u32;
                                    for k in 0..bytes {
                                        v = v << 8 | *inner.get(i + k)? as u32;
                                    }
                                    i += bytes;
                                    cols[c].push(v);
                                }
                            }
                            pclr = Some((cols, depths));
                        }
                        b"cmap" => {
                            for e in inner.chunks_exact(4) {
                                cmap.push((u16::from_be_bytes([e[0], e[1]]) as usize, e[2], e[3] as usize));
                            }
                        }
                        _ => {}
                    }
                }
            }
            b"jp2c" => out.codestream = body,
            _ => {}
        }
    }
    if let Some((cols, depths)) = pclr {
        out.palette = Some((cols, depths, cmap));
    }
    (!out.codestream.is_empty()).then_some(out)
}

/// A JPXDecode stream's picture: each component's 8-bit samples. None if it can't be read.
pub fn decode(data: &[u8], memory: usize) -> Option<Image> {
    let file = jp2(data)?;
    let b = file.codestream;
    if be16(b, 0)? != 0xFF4F {
        return None;
    }
    let mut h = Header::default();
    let mut at = markers(b, 2, &mut h, true)?;
    if h.comps.is_empty() || h.layers == 0 || h.coding.iter().any(|c| c.cb_w == 0) {
        return None;
    }
    let (w, ht) = ((h.width - h.x0) as usize, (h.height - h.y0) as usize);
    let mut budget = memory.min(MOST_SAMPLES);
    let mut planes: Vec<Vec<f32>> = Vec::new();
    for comp in &h.comps {
        let n = ((ceil_div(h.width, comp.dx) - ceil_div(h.x0, comp.dx)) as usize).checked_mul((ceil_div(h.height, comp.dy) - ceil_div(h.y0, comp.dy)) as usize)?;
        budget = budget.checked_sub(n.checked_mul(2)?)?;
        planes.push(vec![0f32; n]);
    }
    // The tile-parts: each tile's data gathered (and its header's markers, which change its coding).
    let tiles = ((h.width - h.tile_x0).div_ceil(h.tile_w) as usize).checked_mul((h.height - h.tile_y0).div_ceil(h.tile_h) as usize)?;
    if tiles > 1 << 16 {
        return None;
    }
    let mut parts: Vec<(Vec<u8>, Option<Header>)> = (0..tiles).map(|_| (Vec::new(), None)).collect();
    while be16(b, at) == Some(0xFF90) {
        let index = be16(b, at + 4)? as usize;
        let len = be32(b, at + 6)? as usize;
        let end = if len == 0 { b.len().saturating_sub(2) } else { at + len };
        let part = parts.get_mut(index)?;
        let mut th = part.1.clone().unwrap_or_else(|| h.clone());
        let sod = markers(b, at + 12, &mut th, false)?;
        part.1 = Some(th);
        part.0.extend_from_slice(b.get(sod + 2..end.min(b.len()))?);
        if end <= at {
            break;
        }
        at = end;
    }
    for (t, (data, th)) in parts.iter().enumerate() {
        let Some(th) = th else { continue };
        let _ = tile(&h, t, data, th, &mut planes, &mut budget);
    }
    // The colour transform undone (G.2, G.3), then each component shifted and made 8 bits.
    let reversible = h.coding.first().is_some_and(|c| c.reversible);
    if h.mct && planes.len() >= 3 && h.comps[..3].iter().all(|c| (c.dx, c.dy) == (h.comps[0].dx, h.comps[0].dy)) {
        let n = planes[0].len().min(planes[1].len()).min(planes[2].len());
        for i in 0..n {
            let (y0, y1, y2) = (planes[0][i], planes[1][i], planes[2][i]);
            let (r, g, bl) = if reversible {
                let g = y0 - ((y1 as i64 + y2 as i64).div_euclid(4)) as f32;
                (y2 + g, g, y1 + g)
            } else {
                (y0 + 1.402 * y2, y0 - 0.344_13 * y1 - 0.714_14 * y2, y0 + 1.772 * y1)
            };
            planes[0][i] = r;
            planes[1][i] = g;
            planes[2][i] = bl;
        }
    }
    let mut components = Vec::new();
    for (c, comp) in h.comps.iter().enumerate() {
        let shift = if comp.signed { 0.0 } else { (1u64 << (comp.precision - 1)) as f32 };
        let top = ((1u64 << comp.precision) - 1) as f32;
        let (cx0, cy0) = (ceil_div(h.x0, comp.dx), ceil_div(h.y0, comp.dy));
        let cw = (ceil_div(h.width, comp.dx) - cx0) as usize;
        let mut out = vec![0u8; w * ht];
        for y in 0..ht {
            let sy = ((h.y0 as usize + y) / comp.dy as usize).saturating_sub(cy0 as usize);
            for x in 0..w {
                let sx = ((h.x0 as usize + x) / comp.dx as usize).saturating_sub(cx0 as usize);
                let v = planes[c].get(sy * cw + sx).copied().unwrap_or(0.0);
                let v = round(v + shift);
                let v = if comp.signed { v + (1u64 << (comp.precision - 1)) as f32 } else { v };
                let v = v.clamp(0.0, top);
                out[y * w + x] = if comp.precision == 8 { v as u8 } else { round(v * 255.0 / top) as u8 };
            }
        }
        components.push(out);
    }
    // sYCC (as the JP2 file says, or as its chroma's being subsampled says, OpenJPEG's way of telling): made RGB, as
    // OpenJPEG makes it.
    let subsampled = h.comps.len() == 3 && (h.comps[0].dx, h.comps[0].dy) == (1, 1) && h.comps[1..].iter().any(|c| (c.dx, c.dy) != (1, 1));
    let ycc = file.ycc || subsampled;
    if ycc && components.len() >= 3 && !h.mct {
        let n = w * ht;
        for i in 0..n {
            let y = components[0][i] as i32;
            let (cb, cr) = (components[1][i] as i32 - 128, components[2][i] as i32 - 128);
            let r = y + (1.402 * cr as f32) as i32;
            let g = y - (0.344 * cb as f32 + 0.714 * cr as f32) as i32;
            let b = y + (1.772 * cb as f32) as i32;
            components[0][i] = r.clamp(0, 255) as u8;
            components[1][i] = g.clamp(0, 255) as u8;
            components[2][i] = b.clamp(0, 255) as u8;
        }
    }
    // A palette: its colours for each sample of the component it maps.
    if let Some((cols, depths, cmap)) = &file.palette {
        let mut made = Vec::new();
        let map: Vec<(usize, u8, usize)> = if cmap.is_empty() { (0..cols.len()).map(|k| (0, 1, k)).collect() } else { cmap.clone() };
        for (comp, kind, column) in map {
            let src = components.get(comp)?;
            if kind == 0 {
                made.push(src.clone());
                continue;
            }
            let col = cols.get(column)?;
            let depth = (depths.get(column).copied().unwrap_or(7) & 0x7F) as u32 + 1;
            let top = ((1u64 << depth) - 1) as f32;
            let prec = h.comps.get(comp).map_or(8, |c| c.precision);
            made.push(
                src.iter()
                    .map(|&s| {
                        // (The 8-bit sample back to the component's own value, to index the palette.)
                        let index = if prec == 8 { s as usize } else { round(s as f32 * ((1u64 << prec) - 1) as f32 / 255.0) as usize };
                        let v = col.get(index).copied().unwrap_or(0) as f32;
                        if depth == 8 { v as u8 } else { round(v * 255.0 / top) as u8 }
                    })
                    .collect(),
            );
        }
        components = made;
    }
    Some(Image { width: w, height: ht, components })
}


#[cfg(test)]
mod fuzz {
    extern crate std;
    use crate::file::File;
    use crate::lex::Obj;

    /// Every byte of six pages' codestreams changed in turn, and each cut short: none crashes it (minutes long, so
    /// only when asked: cargo test -p pdf -- --ignored).
    #[test]
    #[ignore]
    fn every_byte_changed() {
        let data = std::fs::read(std::format!("{}/tests/scan-jpx.pdf", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let f = File::open(&data);
        let mut n = 0;
        // Pages 1, 9 (every code-block style, SOP and EPH), 10 (tiles), 15 (POC), 19 (12 bits), 20 (subsampled).
        for page in [1u32, 9, 10, 15, 19, 20] {
            let Some((_, bytes, _)) = f.image_stream(&Obj::Ref(3 * page + 2, 0)) else { panic!("page {page}") };
            for i in 0..bytes.len() {
                for flip in [0x01u8, 0xFF] {
                    let mut d = bytes.clone();
                    d[i] ^= flip;
                    let _ = super::decode(&d, 1 << 26);
                    n += 1;
                }
            }
            for end in (0..bytes.len()).step_by(7) {
                let _ = super::decode(&bytes[..end], 1 << 26);
                n += 1;
            }
        }
        std::println!("{n} decodes, no crash");
    }
}
