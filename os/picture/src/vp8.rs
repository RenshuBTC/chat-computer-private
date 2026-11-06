//! WebP's lossy format: a VP8 key frame (RFC 6386). Its first partition gives the frame's header (segments, loop
//! filter, quantizers, coefficient probabilities) and each macroblock's prediction modes; the other partitions, its
//! coefficients (DCT tokens, by bool decoding). Each macroblock is predicted from the pixels above it and to its left
//! (as they were before the loop filter), its residue added (inverse DCT, and WHT for the DCs of a 16 × 16 prediction);
//! then the loop filter runs over the frame, and its YUV 4:2:0 is made RGB with the chroma upsampled as libwebp does
//! (its "fancy" upsampling, as Chrome shows a WebP). The same numbers, step by step, as libwebp.

use crate::webp_tables::{AC_TABLE, BANDS, BMODE_PROBS, CAT3, CAT4, CAT5, CAT6, COEFF_PROBS, COEFF_UPDATE_PROBS, DC_TABLE, ZIGZAG};
use crate::Error;
use alloc::vec;
use alloc::vec::Vec;

/// The bool decoder (RFC 6386 section 7): its value left-aligned in 64 bits, its top byte compared with the split.
struct Bool<'a> {
    data: &'a [u8],
    at: usize,
    value: u64,
    /// How many bits are loaded below the top byte (less than 0: the top byte isn't whole yet).
    count: i32,
    range: u32,
}

impl<'a> Bool<'a> {
    fn new(data: &'a [u8]) -> Bool<'a> {
        let mut b = Bool { data, at: 0, value: 0, count: -8, range: 255 };
        b.fill();
        b
    }

    fn fill(&mut self) {
        let mut shift = 48 - self.count;
        while shift >= 0 {
            // (Past the end, zeros.)
            let byte = if self.at < self.data.len() { self.data[self.at] } else { 0 };
            self.at += 1;
            self.value |= (byte as u64) << shift;
            self.count += 8;
            shift -= 8;
        }
    }

    fn bit(&mut self, prob: u8) -> bool {
        let split = 1 + (((self.range - 1) * prob as u32) >> 8);
        if self.count < 0 {
            self.fill();
        }
        let big = (split as u64) << 56;
        let bit = self.value >= big;
        if bit {
            self.range -= split;
            self.value -= big;
        } else {
            self.range = split;
        }
        let shift = self.range.leading_zeros() - 24;
        self.range <<= shift;
        self.value <<= shift;
        self.count -= shift as i32;
        bit
    }

    /// An unsigned number of `n` bits, most significant first.
    fn literal(&mut self, n: u32) -> i32 {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit(128) as i32;
        }
        v
    }

    /// A number of `n` bits and its sign after it.
    fn signed(&mut self, n: u32) -> i32 {
        let v = self.literal(n);
        if self.bit(128) { -v } else { v }
    }

    /// A number of `n` bits and its sign, if the flag before it says it's there (0 if not).
    fn maybe_signed(&mut self, n: u32) -> i32 {
        if self.bit(128) { self.signed(n) } else { 0 }
    }
}

// The prediction modes, numbered as libwebp numbers them (its tables are in this order).
const DC: u8 = 0;
const TM: u8 = 1;
const VE: u8 = 2;
const HE: u8 = 3;
const RD: u8 = 4;
const VR: u8 = 5;
const LD: u8 = 6;
const VL: u8 = 7;
const HD: u8 = 8;
const HU: u8 = 9;

/// The size a VP8 frame's header gives (a key frame's: the only kind a WebP has).
pub(crate) fn size(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 10 || data[0] & 1 != 0 || data[3..6] != [0x9D, 0x01, 0x2A] {
        return None;
    }
    let w = (u16::from_le_bytes([data[6], data[7]]) & 0x3FFF) as u32;
    let h = (u16::from_le_bytes([data[8], data[9]]) & 0x3FFF) as u32;
    if w == 0 || h == 0 { None } else { Some((w, h)) }
}

/// Dequantization factors: [DC, AC] for Y, Y2 and chroma.
#[derive(Clone, Copy, Default)]
struct Quant {
    y1: [i32; 2],
    y2: [i32; 2],
    uv: [i32; 2],
}

/// A macroblock's loop filter: its limit (0: none), interior limit, high edge variance threshold, and whether its
/// inner edges are filtered.
#[derive(Clone, Copy, Default)]
struct Filter {
    limit: i32,
    ilevel: i32,
    hev: i32,
    inner: bool,
}

/// The work area a macroblock is predicted and reconstructed in, as libwebp lays it out: 32 bytes a row; the pixels
/// above (and above to the right) and to the left of Y, U and V around them.
const BPS: usize = 32;
const Y_OFF: usize = BPS + 8;
const U_OFF: usize = Y_OFF + BPS * 16 + BPS;
const V_OFF: usize = U_OFF + 16;
const WORK: usize = BPS * 17 + BPS * 9;

/// A VP8 frame's pixels: its width, height and RGB.
pub(crate) fn decode(data: &[u8], most: u64) -> Result<(usize, usize, Vec<u8>), Error> {
    let (w, h) = size(data).ok_or(Error::Damaged)?;
    let (w, h) = (w as usize, h as usize);
    if w as u64 * h as u64 > most {
        return Err(Error::TooBig);
    }
    let tag = data[0] as usize | (data[1] as usize) << 8 | (data[2] as usize) << 16;
    if (tag >> 1) & 7 > 3 {
        return Err(Error::Unsupported("a VP8 profile past 3"));
    }
    if (tag >> 4) & 1 == 0 {
        return Err(Error::Unsupported("a VP8 frame not shown"));
    }
    let first_size = tag >> 5;
    let rest = &data[10..];
    if first_size > rest.len() {
        return Err(Error::Damaged);
    }
    let mut b = Bool::new(&rest[..first_size]);
    let parts = &rest[first_size..];
    let _colour_space = b.bit(128);
    let _clamping = b.bit(128);

    // Segments.
    let use_segment = b.bit(128);
    let mut update_map = false;
    let mut absolute = true;
    let mut seg_quant = [0i32; 4];
    let mut seg_filter = [0i32; 4];
    let mut seg_probs = [255u8; 3];
    if use_segment {
        update_map = b.bit(128);
        if b.bit(128) {
            absolute = b.bit(128);
            for q in seg_quant.iter_mut() {
                *q = b.maybe_signed(7);
            }
            for f in seg_filter.iter_mut() {
                *f = b.maybe_signed(6);
            }
        }
        if update_map {
            for p in seg_probs.iter_mut() {
                *p = if b.bit(128) { b.literal(8) as u8 } else { 255 };
            }
        }
    }

    // The loop filter.
    let simple = b.bit(128);
    let level = b.literal(6);
    let sharpness = b.literal(3);
    let mut ref_delta = [0i32; 4];
    let mut mode_delta = [0i32; 4];
    let use_delta = b.bit(128);
    if use_delta && b.bit(128) {
        for d in ref_delta.iter_mut() {
            if b.bit(128) {
                *d = b.signed(6);
            }
        }
        for d in mode_delta.iter_mut() {
            if b.bit(128) {
                *d = b.signed(6);
            }
        }
    }
    let filter_type = if level == 0 { 0 } else if simple { 1 } else { 2 };

    // The token partitions: each but the last's size (3 bytes), then them.
    let n_parts = 1usize << b.literal(2);
    if parts.len() < 3 * (n_parts - 1) {
        return Err(Error::Damaged);
    }
    let mut at = 3 * (n_parts - 1);
    let mut partitions = Vec::with_capacity(n_parts);
    for p in 0..n_parts {
        let size = if p + 1 < n_parts {
            let s = &parts[3 * p..];
            (s[0] as usize | (s[1] as usize) << 8 | (s[2] as usize) << 16).min(parts.len() - at)
        } else {
            parts.len() - at
        };
        partitions.push(Bool::new(&parts[at..at + size]));
        at += size;
    }

    // Quantizers.
    let base = b.literal(7);
    let dy1_dc = b.maybe_signed(4);
    let dy2_dc = b.maybe_signed(4);
    let dy2_ac = b.maybe_signed(4);
    let duv_dc = b.maybe_signed(4);
    let duv_ac = b.maybe_signed(4);
    let mut quant = [Quant::default(); 4];
    for (s, q) in quant.iter_mut().enumerate() {
        let qi = if use_segment { seg_quant[s] + if absolute { 0 } else { base } } else { base };
        let at = |d: i32, most: i32| (qi + d).clamp(0, most) as usize;
        q.y1 = [DC_TABLE[at(dy1_dc, 127)] as i32, AC_TABLE[at(0, 127)] as i32];
        q.y2 = [DC_TABLE[at(dy2_dc, 127)] as i32 * 2, ((AC_TABLE[at(dy2_ac, 127)] as i32 * 101581) >> 16).max(8)];
        q.uv = [DC_TABLE[at(duv_dc, 117)] as i32, AC_TABLE[at(duv_ac, 127)] as i32];
    }

    // (Whether to keep these probabilities for later frames: a WebP has no later frames.)
    let _refresh = b.bit(128);
    let mut probs = COEFF_PROBS;
    for (i, p) in probs.iter_mut().enumerate() {
        if b.bit(COEFF_UPDATE_PROBS[i]) {
            *p = b.literal(8) as u8;
        }
    }
    let use_skip = b.bit(128);
    let skip_prob = if use_skip { b.literal(8) as u8 } else { 0 };

    // Each segment's loop filter, for a macroblock predicted 16 × 16 and for one predicted 4 × 4.
    let mut filters = [[Filter::default(); 2]; 4];
    if filter_type > 0 {
        for s in 0..4 {
            let base_level = if use_segment { seg_filter[s] + if absolute { 0 } else { level } } else { level };
            for i4 in 0..2 {
                let mut l = base_level;
                if use_delta {
                    l += ref_delta[0];
                    if i4 == 1 {
                        l += mode_delta[0];
                    }
                }
                let l = l.clamp(0, 63);
                let f = &mut filters[s][i4];
                f.inner = i4 == 1;
                if l > 0 {
                    let mut il = l;
                    if sharpness > 0 {
                        il >>= if sharpness > 4 { 2 } else { 1 };
                        il = il.min(9 - sharpness);
                    }
                    let il = il.max(1);
                    f.ilevel = il;
                    f.limit = 2 * l + il;
                    f.hev = if l >= 40 { 2 } else if l >= 15 { 1 } else { 0 };
                }
            }
        }
    }

    let mb_w = w.div_ceil(16);
    let mb_h = h.div_ceil(16);
    let (ys, uvs) = (mb_w * 16, mb_w * 8);
    let mut yp = vec![0u8; ys * mb_h * 16];
    let mut up = vec![0u8; uvs * mb_h * 8];
    let mut vp = vec![0u8; uvs * mb_h * 8];
    let mut mb_filters = vec![Filter::default(); if filter_type > 0 { mb_w * mb_h } else { 0 }];

    // Contexts: the 4 × 4 modes of the blocks above (each column of blocks) and to the left; whether the blocks above
    // and to the left had coefficients (Y, U, V and Y2).
    let mut modes_top = vec![DC; mb_w * 4];
    let mut nz_top = vec![Nz::default(); mb_w];
    let mut work = [0u8; WORK];
    let mut coeffs = [0i16; 384];

    for mby in 0..mb_h {
        let mut modes_left = [DC; 4];
        let mut nz_left = Nz::default();
        let tokens = &mut partitions[mby & (n_parts - 1)];
        for mbx in 0..mb_w {
            // The macroblock's header (in the first partition).
            let segment = if update_map {
                if !b.bit(seg_probs[0]) { b.bit(seg_probs[1]) as usize } else { 2 + b.bit(seg_probs[2]) as usize }
            } else {
                0
            };
            let skip = use_skip && b.bit(skip_prob);
            let i4x4 = !b.bit(145);
            let mut modes = [DC; 16];
            if !i4x4 {
                let m = if b.bit(156) {
                    if b.bit(128) { TM } else { HE }
                } else if b.bit(163) {
                    VE
                } else {
                    DC
                };
                modes[0] = m;
                modes_top[mbx * 4..mbx * 4 + 4].fill(m);
                modes_left = [m; 4];
            } else {
                for y in 0..4 {
                    let mut m = modes_left[y];
                    for x in 0..4 {
                        let p = &BMODE_PROBS[(modes_top[mbx * 4 + x] as usize * 10 + m as usize) * 9..][..9];
                        m = sub_mode(&mut b, p);
                        modes_top[mbx * 4 + x] = m;
                        modes[y * 4 + x] = m;
                    }
                    modes_left[y] = m;
                }
            }
            let uv_mode = if !b.bit(142) {
                DC
            } else if !b.bit(114) {
                VE
            } else if b.bit(183) {
                TM
            } else {
                HE
            };

            // Its coefficients (in its row's token partition).
            coeffs.fill(0);
            let q = &quant[segment];
            let mut kinds = [0u8; 24];
            let has_coeffs = if skip {
                nz_left.clear(!i4x4);
                nz_top[mbx].clear(!i4x4);
                false
            } else {
                residuals(tokens, &probs, q, i4x4, &mut nz_top[mbx], &mut nz_left, &mut coeffs, &mut kinds)
            };
            if filter_type > 0 {
                let mut f = filters[segment][i4x4 as usize];
                f.inner |= has_coeffs;
                mb_filters[mby * mb_w + mbx] = f;
            }

            // Predicted, and its residue added, in the work area.
            edges(&mut work, &yp, &up, &vp, ys, uvs, mbx, mby, mb_w);
            if i4x4 {
                // (The pixels above to the right of the macroblock, for the blocks of its right column below the
                // first: those above the macroblock, as the encoder had them.)
                let tr = Y_OFF - BPS + 16;
                for k in 1..4 {
                    work.copy_within(tr..tr + 4, tr + 4 * k * BPS);
                }
                for n in 0..16 {
                    let at = Y_OFF + (n / 4) * 4 * BPS + (n % 4) * 4;
                    predict4(&mut work, at, modes[n]);
                    add_residue(&mut work, at, &coeffs[n * 16..n * 16 + 16], kinds[n]);
                }
            } else {
                predict16(&mut work, Y_OFF, dc_mode(modes[0], mbx, mby), 16);
                for n in 0..16 {
                    let at = Y_OFF + (n / 4) * 4 * BPS + (n % 4) * 4;
                    add_residue(&mut work, at, &coeffs[n * 16..n * 16 + 16], kinds[n]);
                }
            }
            let m = dc_mode(uv_mode, mbx, mby);
            predict16(&mut work, U_OFF, m, 8);
            predict16(&mut work, V_OFF, m, 8);
            for n in 0..4 {
                let off = (n / 2) * 4 * BPS + (n % 2) * 4;
                add_residue(&mut work, U_OFF + off, &coeffs[256 + n * 16..256 + n * 16 + 16], kinds[16 + n]);
                add_residue(&mut work, V_OFF + off, &coeffs[320 + n * 16..320 + n * 16 + 16], kinds[20 + n]);
            }
            for r in 0..16 {
                let o = (mby * 16 + r) * ys + mbx * 16;
                yp[o..o + 16].copy_from_slice(&work[Y_OFF + r * BPS..Y_OFF + r * BPS + 16]);
            }
            for r in 0..8 {
                let o = (mby * 8 + r) * uvs + mbx * 8;
                up[o..o + 8].copy_from_slice(&work[U_OFF + r * BPS..U_OFF + r * BPS + 8]);
                vp[o..o + 8].copy_from_slice(&work[V_OFF + r * BPS..V_OFF + r * BPS + 8]);
            }
        }
    }

    // The loop filter, macroblock by macroblock: its left edge, its inner vertical edges, its top edge, its inner
    // horizontal edges.
    if filter_type > 0 {
        for mby in 0..mb_h {
            for mbx in 0..mb_w {
                let f = mb_filters[mby * mb_w + mbx];
                if f.limit == 0 {
                    continue;
                }
                let yo = mby * 16 * ys + mbx * 16;
                let uo = mby * 8 * uvs + mbx * 8;
                if filter_type == 1 {
                    if mbx > 0 {
                        simple_edge(&mut yp, yo, 1, ys, f.limit + 4);
                    }
                    if f.inner {
                        for k in 1..4 {
                            simple_edge(&mut yp, yo + 4 * k, 1, ys, f.limit);
                        }
                    }
                    if mby > 0 {
                        simple_edge(&mut yp, yo, ys, 1, f.limit + 4);
                    }
                    if f.inner {
                        for k in 1..4 {
                            simple_edge(&mut yp, yo + 4 * k * ys, ys, 1, f.limit);
                        }
                    }
                } else {
                    let (lim, il, hev) = (f.limit, f.ilevel, f.hev);
                    if mbx > 0 {
                        edge(&mut yp, yo, 1, ys, 16, lim + 4, il, hev, true);
                        edge(&mut up, uo, 1, uvs, 8, lim + 4, il, hev, true);
                        edge(&mut vp, uo, 1, uvs, 8, lim + 4, il, hev, true);
                    }
                    if f.inner {
                        for k in 1..4 {
                            edge(&mut yp, yo + 4 * k, 1, ys, 16, lim, il, hev, false);
                        }
                        edge(&mut up, uo + 4, 1, uvs, 8, lim, il, hev, false);
                        edge(&mut vp, uo + 4, 1, uvs, 8, lim, il, hev, false);
                    }
                    if mby > 0 {
                        edge(&mut yp, yo, ys, 1, 16, lim + 4, il, hev, true);
                        edge(&mut up, uo, uvs, 1, 8, lim + 4, il, hev, true);
                        edge(&mut vp, uo, uvs, 1, 8, lim + 4, il, hev, true);
                    }
                    if f.inner {
                        for k in 1..4 {
                            edge(&mut yp, yo + 4 * k * ys, ys, 1, 16, lim, il, hev, false);
                        }
                        edge(&mut up, uo + 4 * uvs, uvs, 1, 8, lim, il, hev, false);
                        edge(&mut vp, uo + 4 * uvs, uvs, 1, 8, lim, il, hev, false);
                    }
                }
            }
        }
    }

    Ok((w, h, to_rgb(&yp, ys, &up, &vp, uvs, w, h)))
}

/// A 4 × 4 block's prediction mode, by the modes above it and to its left (`p`: the tree's probabilities).
fn sub_mode(b: &mut Bool, p: &[u8]) -> u8 {
    if !b.bit(p[0]) {
        DC
    } else if !b.bit(p[1]) {
        TM
    } else if !b.bit(p[2]) {
        VE
    } else if !b.bit(p[3]) {
        if !b.bit(p[4]) {
            HE
        } else if !b.bit(p[5]) {
            RD
        } else {
            VR
        }
    } else if !b.bit(p[6]) {
        LD
    } else if !b.bit(p[7]) {
        VL
    } else if !b.bit(p[8]) {
        HD
    } else {
        HU
    }
}

/// DC prediction at the frame's edges: from what's there (above, or to the left), or 128 at its top left.
const DC_NO_TOP: u8 = 10;
const DC_NO_LEFT: u8 = 11;
const DC_NONE: u8 = 12;

fn dc_mode(mode: u8, mbx: usize, mby: usize) -> u8 {
    if mode != DC {
        return mode;
    }
    match (mbx == 0, mby == 0) {
        (true, true) => DC_NONE,
        (true, false) => DC_NO_LEFT,
        (false, true) => DC_NO_TOP,
        (false, false) => DC,
    }
}

/// Whether each block above or to the left had coefficients: Y's four, U's two, V's two, and Y2.
#[derive(Clone, Copy, Default)]
struct Nz {
    y: [bool; 4],
    u: [bool; 2],
    v: [bool; 2],
    dc: bool,
}

impl Nz {
    /// For a macroblock with none (Y2's kept for one predicted 4 × 4, which has no Y2).
    fn clear(&mut self, with_dc: bool) {
        self.y = [false; 4];
        self.u = [false; 2];
        self.v = [false; 2];
        if with_dc {
            self.dc = false;
        }
    }
}

/// A macroblock's coefficients, dequantized, in `coeffs` (16 Y blocks, 4 U, 4 V; each 16 in raster order), and for
/// each block whether it has any (`kinds`: 0 none, 1 only its DC, 2 more). Whether any block has any.
#[allow(clippy::too_many_arguments)]
fn residuals(
    b: &mut Bool,
    probs: &[u8; 1056],
    q: &Quant,
    i4x4: bool,
    top: &mut Nz,
    left: &mut Nz,
    coeffs: &mut [i16; 384],
    kinds: &mut [u8; 24],
) -> bool {
    let (first, y_type) = if !i4x4 {
        // Y2: the 16 Y blocks' DCs, through the inverse WHT.
        let mut dc = [0i16; 16];
        let ctx = top.dc as usize + left.dc as usize;
        let nz = block(b, probs, 1, ctx, q.y2, 0, &mut dc);
        top.dc = nz > 0;
        left.dc = nz > 0;
        if nz > 1 {
            wht(&dc, coeffs);
        } else {
            let dc0 = ((dc[0] as i32 + 3) >> 3) as i16;
            for k in 0..16 {
                coeffs[k * 16] = dc0;
            }
        }
        (1, 0)
    } else {
        (0, 3)
    };
    let mut any = false;
    for y in 0..4 {
        for x in 0..4 {
            let n = y * 4 + x;
            let ctx = left.y[y] as usize + top.y[x] as usize;
            let out = &mut coeffs[n * 16..n * 16 + 16];
            let nz = block(b, probs, y_type, ctx, q.y1, first, out);
            let flag = nz > first;
            left.y[y] = flag;
            top.y[x] = flag;
            kinds[n] = kind(nz, out[0]);
            any |= kinds[n] != 0;
        }
    }
    for (plane, base) in [(0, 256), (1, 320)] {
        for y in 0..2 {
            for x in 0..2 {
                let n = y * 2 + x;
                let (l, t) = if plane == 0 { (&mut left.u[y], &mut top.u[x]) } else { (&mut left.v[y], &mut top.v[x]) };
                let ctx = *l as usize + *t as usize;
                let out = &mut coeffs[base + n * 16..base + n * 16 + 16];
                let nz = block(b, probs, 2, ctx, q.uv, 0, out);
                *l = nz > 0;
                *t = nz > 0;
                kinds[16 + plane * 4 + n] = kind(nz, out[0]);
                any |= kinds[16 + plane * 4 + n] != 0;
            }
        }
    }
    any
}

/// What a block has, by where its coefficients stopped and its DC: 0 none, 1 only its DC, 2 more.
fn kind(nz: usize, dc: i16) -> u8 {
    if nz > 1 {
        2
    } else {
        (dc != 0) as u8
    }
}

/// A block's coefficients (of type `t`: 0 Y after Y2, 1 Y2, 2 chroma, 3 Y with its DC), from `first`, dequantized
/// (`dq`: DC, AC) into `out` in raster order. Where they stopped: the position after the last token.
fn block(b: &mut Bool, probs: &[u8; 1056], t: usize, ctx: usize, dq: [i32; 2], first: usize, out: &mut [i16]) -> usize {
    let p_at = |n: usize, ctx: usize| ((t * 8 + BANDS[n] as usize) * 3 + ctx) * 11;
    let mut n = first;
    let mut p = p_at(n, ctx);
    while n < 16 {
        if !b.bit(probs[p]) {
            // (The end of the block.)
            return n;
        }
        while !b.bit(probs[p + 1]) {
            // (A zero.)
            n += 1;
            if n == 16 {
                return 16;
            }
            p = p_at(n, 0);
        }
        let (v, next) = if !b.bit(probs[p + 2]) { (1, 1) } else { (large(b, &probs[p..p + 11]), 2) };
        let v = if b.bit(128) { -v } else { v };
        // (Stored in 16 bits, as libwebp stores it.)
        out[ZIGZAG[n] as usize] = (v * dq[(n > 0) as usize]) as i16;
        n += 1;
        if n < 16 {
            p = p_at(n, next);
        }
    }
    16
}

/// A token past 1: 2 to 4, or a category's base and extra bits.
fn large(b: &mut Bool, p: &[u8]) -> i32 {
    if !b.bit(p[3]) {
        if !b.bit(p[4]) { 2 } else { 3 + b.bit(p[5]) as i32 }
    } else if !b.bit(p[6]) {
        if !b.bit(p[7]) {
            5 + b.bit(159) as i32
        } else {
            let v = 7 + 2 * b.bit(165) as i32;
            v + b.bit(145) as i32
        }
    } else {
        let bit1 = b.bit(p[8]) as usize;
        let bit0 = b.bit(p[9 + bit1]) as usize;
        let cat = 2 * bit1 + bit0;
        let tab: &[u8] = match cat {
            0 => &CAT3,
            1 => &CAT4,
            2 => &CAT5,
            _ => &CAT6,
        };
        let mut v = 0;
        for &pr in tab.iter().take_while(|&&p| p != 0) {
            v = v + v + b.bit(pr) as i32;
        }
        v + 3 + (8 << cat)
    }
}

/// The inverse Walsh-Hadamard transform: Y2's coefficients into each Y block's DC.
fn wht(input: &[i16; 16], coeffs: &mut [i16; 384]) {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let a0 = input[i] as i32 + input[12 + i] as i32;
        let a1 = input[4 + i] as i32 + input[8 + i] as i32;
        let a2 = input[4 + i] as i32 - input[8 + i] as i32;
        let a3 = input[i] as i32 - input[12 + i] as i32;
        tmp[i] = a0 + a1;
        tmp[8 + i] = a0 - a1;
        tmp[4 + i] = a3 + a2;
        tmp[12 + i] = a3 - a2;
    }
    for i in 0..4 {
        let dc = tmp[i * 4] + 3;
        let a0 = dc + tmp[3 + i * 4];
        let a1 = tmp[1 + i * 4] + tmp[2 + i * 4];
        let a2 = tmp[1 + i * 4] - tmp[2 + i * 4];
        let a3 = dc - tmp[3 + i * 4];
        let out = i * 64;
        coeffs[out] = ((a0 + a1) >> 3) as i16;
        coeffs[out + 16] = ((a3 + a2) >> 3) as i16;
        coeffs[out + 32] = ((a0 - a1) >> 3) as i16;
        coeffs[out + 48] = ((a3 - a2) >> 3) as i16;
    }
}

fn mul1(a: i32) -> i32 {
    ((a * 20091) >> 16) + a
}

fn mul2(a: i32) -> i32 {
    (a * 35468) >> 16
}

fn clip8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// A block's residue (the inverse DCT of its coefficients) added to the prediction at `at`.
fn add_residue(work: &mut [u8; WORK], at: usize, c: &[i16], kind: u8) {
    match kind {
        0 => {}
        1 => {
            let dc = (c[0] as i32 + 4) >> 3;
            for r in 0..4 {
                for x in 0..4 {
                    let p = &mut work[at + r * BPS + x];
                    *p = clip8(*p as i32 + dc);
                }
            }
        }
        _ => {
            let mut tmp = [0i32; 16];
            for i in 0..4 {
                let a = c[i] as i32 + c[8 + i] as i32;
                let b = c[i] as i32 - c[8 + i] as i32;
                let cc = mul2(c[4 + i] as i32) - mul1(c[12 + i] as i32);
                let d = mul1(c[4 + i] as i32) + mul2(c[12 + i] as i32);
                tmp[i * 4] = a + d;
                tmp[i * 4 + 1] = b + cc;
                tmp[i * 4 + 2] = b - cc;
                tmp[i * 4 + 3] = a - d;
            }
            for i in 0..4 {
                let dc = tmp[i] + 4;
                let a = dc + tmp[8 + i];
                let b = dc - tmp[8 + i];
                let cc = mul2(tmp[4 + i]) - mul1(tmp[12 + i]);
                let d = mul1(tmp[4 + i]) + mul2(tmp[12 + i]);
                let row = at + i * BPS;
                for (x, v) in [a + d, b + cc, b - cc, a - d].into_iter().enumerate() {
                    let p = &mut work[row + x];
                    *p = clip8(*p as i32 + (v >> 3));
                }
            }
        }
    }
}

/// The pixels around a macroblock, in the work area: those above (and four above to its right), to its left, and above
/// to its left, from the frame as decoded so far; at the frame's edges, 127 above and 129 to the left.
#[allow(clippy::too_many_arguments)]
fn edges(work: &mut [u8; WORK], yp: &[u8], up: &[u8], vp: &[u8], ys: usize, uvs: usize, mbx: usize, mby: usize, mb_w: usize) {
    for j in 0..16 {
        work[Y_OFF + j * BPS - 1] = if mbx == 0 { 129 } else { yp[(mby * 16 + j) * ys + mbx * 16 - 1] };
    }
    for j in 0..8 {
        let (u, v) = if mbx == 0 {
            (129, 129)
        } else {
            let o = (mby * 8 + j) * uvs + mbx * 8 - 1;
            (up[o], vp[o])
        };
        work[U_OFF + j * BPS - 1] = u;
        work[V_OFF + j * BPS - 1] = v;
    }
    if mby == 0 {
        work[Y_OFF - BPS - 1..Y_OFF - BPS + 20].fill(127);
        work[U_OFF - BPS - 1..U_OFF - BPS + 8].fill(127);
        work[V_OFF - BPS - 1..V_OFF - BPS + 8].fill(127);
        return;
    }
    let yrow = (mby * 16 - 1) * ys + mbx * 16;
    work[Y_OFF - BPS..Y_OFF - BPS + 16].copy_from_slice(&yp[yrow..yrow + 16]);
    if mbx + 1 < mb_w {
        work[Y_OFF - BPS + 16..Y_OFF - BPS + 20].copy_from_slice(&yp[yrow + 16..yrow + 20]);
    } else {
        let last = yp[yrow + 15];
        work[Y_OFF - BPS + 16..Y_OFF - BPS + 20].fill(last);
    }
    let uvrow = (mby * 8 - 1) * uvs + mbx * 8;
    work[U_OFF - BPS..U_OFF - BPS + 8].copy_from_slice(&up[uvrow..uvrow + 8]);
    work[V_OFF - BPS..V_OFF - BPS + 8].copy_from_slice(&vp[uvrow..uvrow + 8]);
    if mbx == 0 {
        work[Y_OFF - BPS - 1] = 129;
        work[U_OFF - BPS - 1] = 129;
        work[V_OFF - BPS - 1] = 129;
    } else {
        work[Y_OFF - BPS - 1] = yp[yrow - 1];
        work[U_OFF - BPS - 1] = up[uvrow - 1];
        work[V_OFF - BPS - 1] = vp[uvrow - 1];
    }
}

/// A 16 × 16 (luma) or 8 × 8 (chroma) prediction at `at`.
fn predict16(work: &mut [u8; WORK], at: usize, mode: u8, size: usize) {
    let shift = if size == 16 { 5 } else { 4 };
    let fill = |work: &mut [u8; WORK], v: u8| {
        for r in 0..size {
            work[at + r * BPS..at + r * BPS + size].fill(v);
        }
    };
    match mode {
        VE => {
            for r in 0..size {
                work.copy_within(at - BPS..at - BPS + size, at + r * BPS);
            }
        }
        HE => {
            for r in 0..size {
                let v = work[at + r * BPS - 1];
                work[at + r * BPS..at + r * BPS + size].fill(v);
            }
        }
        TM => true_motion(work, at, size),
        DC_NO_TOP => {
            let sum: u32 = (0..size).map(|r| work[at + r * BPS - 1] as u32).sum();
            fill(work, ((sum + (size as u32 >> 1)) >> (shift - 1)) as u8);
        }
        DC_NO_LEFT => {
            let sum: u32 = (0..size).map(|x| work[at - BPS + x] as u32).sum();
            fill(work, ((sum + (size as u32 >> 1)) >> (shift - 1)) as u8);
        }
        DC_NONE => fill(work, 128),
        _ => {
            let sum: u32 = (0..size).map(|k| work[at + k * BPS - 1] as u32 + work[at - BPS + k] as u32).sum();
            fill(work, ((sum + size as u32) >> shift) as u8);
        }
    }
}

fn true_motion(work: &mut [u8; WORK], at: usize, size: usize) {
    let tl = work[at - BPS - 1] as i32;
    for r in 0..size {
        let l = work[at + r * BPS - 1] as i32;
        for x in 0..size {
            let t = work[at - BPS + x] as i32;
            work[at + r * BPS + x] = clip8(t + l - tl);
        }
    }
}

fn avg3(a: u8, b: u8, c: u8) -> u8 {
    ((a as u32 + 2 * b as u32 + c as u32 + 2) >> 2) as u8
}

fn avg2(a: u8, b: u8) -> u8 {
    ((a as u32 + b as u32 + 1) >> 1) as u8
}

/// A 4 × 4 block's prediction at `at`, from the 13 pixels around it: 8 above (4 of them to the right), 4 to the
/// left and the one above to the left.
fn predict4(work: &mut [u8; WORK], at: usize, mode: u8) {
    let top = at - BPS;
    let (a, b, c, d) = (work[top], work[top + 1], work[top + 2], work[top + 3]);
    let (e, f, g, h) = (work[top + 4], work[top + 5], work[top + 6], work[top + 7]);
    let (i, j, k, l) = (work[at - 1], work[at + BPS - 1], work[at + 2 * BPS - 1], work[at + 3 * BPS - 1]);
    let x = work[top - 1];
    let mut out = [[0u8; 4]; 4]; // [row][column]
    match mode {
        VE => {
            let v = [avg3(x, a, b), avg3(a, b, c), avg3(b, c, d), avg3(c, d, e)];
            out = [v; 4];
        }
        HE => {
            out[0] = [avg3(x, i, j); 4];
            out[1] = [avg3(i, j, k); 4];
            out[2] = [avg3(j, k, l); 4];
            out[3] = [avg3(k, l, l); 4];
        }
        TM => {
            true_motion(work, at, 4);
            return;
        }
        RD => {
            out[3][0] = avg3(j, k, l);
            let v = avg3(i, j, k);
            out[3][1] = v;
            out[2][0] = v;
            let v = avg3(x, i, j);
            out[3][2] = v;
            out[2][1] = v;
            out[1][0] = v;
            let v = avg3(a, x, i);
            out[3][3] = v;
            out[2][2] = v;
            out[1][1] = v;
            out[0][0] = v;
            let v = avg3(b, a, x);
            out[2][3] = v;
            out[1][2] = v;
            out[0][1] = v;
            let v = avg3(c, b, a);
            out[1][3] = v;
            out[0][2] = v;
            out[0][3] = avg3(d, c, b);
        }
        LD => {
            out[0][0] = avg3(a, b, c);
            let v = avg3(b, c, d);
            out[0][1] = v;
            out[1][0] = v;
            let v = avg3(c, d, e);
            out[0][2] = v;
            out[1][1] = v;
            out[2][0] = v;
            let v = avg3(d, e, f);
            out[0][3] = v;
            out[1][2] = v;
            out[2][1] = v;
            out[3][0] = v;
            let v = avg3(e, f, g);
            out[1][3] = v;
            out[2][2] = v;
            out[3][1] = v;
            let v = avg3(f, g, h);
            out[2][3] = v;
            out[3][2] = v;
            out[3][3] = avg3(g, h, h);
        }
        VR => {
            let v = avg2(x, a);
            out[0][0] = v;
            out[2][1] = v;
            let v = avg2(a, b);
            out[0][1] = v;
            out[2][2] = v;
            let v = avg2(b, c);
            out[0][2] = v;
            out[2][3] = v;
            out[0][3] = avg2(c, d);
            out[3][0] = avg3(k, j, i);
            out[2][0] = avg3(j, i, x);
            let v = avg3(i, x, a);
            out[1][0] = v;
            out[3][1] = v;
            let v = avg3(x, a, b);
            out[1][1] = v;
            out[3][2] = v;
            let v = avg3(a, b, c);
            out[1][2] = v;
            out[3][3] = v;
            out[1][3] = avg3(b, c, d);
        }
        VL => {
            out[0][0] = avg2(a, b);
            let v = avg2(b, c);
            out[0][1] = v;
            out[2][0] = v;
            let v = avg2(c, d);
            out[0][2] = v;
            out[2][1] = v;
            let v = avg2(d, e);
            out[0][3] = v;
            out[2][2] = v;
            out[1][0] = avg3(a, b, c);
            let v = avg3(b, c, d);
            out[1][1] = v;
            out[3][0] = v;
            let v = avg3(c, d, e);
            out[1][2] = v;
            out[3][1] = v;
            let v = avg3(d, e, f);
            out[1][3] = v;
            out[3][2] = v;
            out[2][3] = avg3(e, f, g);
            out[3][3] = avg3(f, g, h);
        }
        HU => {
            out[0][0] = avg2(i, j);
            let v = avg2(j, k);
            out[0][2] = v;
            out[1][0] = v;
            let v = avg2(k, l);
            out[1][2] = v;
            out[2][0] = v;
            out[0][1] = avg3(i, j, k);
            let v = avg3(j, k, l);
            out[0][3] = v;
            out[1][1] = v;
            let v = avg3(k, l, l);
            out[1][3] = v;
            out[2][1] = v;
            out[2][3] = l;
            out[2][2] = l;
            out[3] = [l; 4];
        }
        HD => {
            let v = avg2(i, x);
            out[0][0] = v;
            out[1][2] = v;
            let v = avg2(j, i);
            out[1][0] = v;
            out[2][2] = v;
            let v = avg2(k, j);
            out[2][0] = v;
            out[3][2] = v;
            out[3][0] = avg2(l, k);
            out[0][3] = avg3(a, b, c);
            out[0][2] = avg3(x, a, b);
            let v = avg3(i, x, a);
            out[0][1] = v;
            out[1][3] = v;
            let v = avg3(j, i, x);
            out[1][1] = v;
            out[2][3] = v;
            let v = avg3(k, j, i);
            out[2][1] = v;
            out[3][3] = v;
            out[3][1] = avg3(l, k, j);
        }
        _ => {
            // DC (a block's DC needs nothing at the frame's edges: the pixels there are 127 and 129).
            let sum: u32 = [a, b, c, d, i, j, k, l].iter().map(|&v| v as u32).sum();
            out = [[((sum + 4) >> 3) as u8; 4]; 4];
        }
    }
    for (r, row) in out.iter().enumerate() {
        work[at + r * BPS..at + r * BPS + 4].copy_from_slice(row);
    }
}

// --------------------------------------------------------------------------------------------- the loop filter

/// The simple filter across an edge: 16 pixels along it (`along` apart), each with the pixels across it (`step`
/// apart), from `at` (the first pixel past the edge).
fn simple_edge(p: &mut [u8], at: usize, step: usize, along: usize, limit: i32) {
    let t = 2 * limit + 1;
    for k in 0..16 {
        let q = at + k * along;
        if needs_filter(p, q, step, t) {
            filter2(p, q, step);
        }
    }
}

/// The normal filter across an edge: a macroblock's edge (`mb`: 6 pixels changed) or an inner one (4).
#[allow(clippy::too_many_arguments)]
fn edge(p: &mut [u8], at: usize, step: usize, along: usize, size: usize, limit: i32, ilevel: i32, hev: i32, mb: bool) {
    let t = 2 * limit + 1;
    for k in 0..size {
        let q = at + k * along;
        if needs_filter2(p, q, step, t, ilevel) {
            if high_variance(p, q, step, hev) {
                filter2(p, q, step);
            } else if mb {
                filter6(p, q, step);
            } else {
                filter4(p, q, step);
            }
        }
    }
}

fn px(p: &[u8], q: usize, step: usize, k: isize) -> i32 {
    p[(q as isize + k * step as isize) as usize] as i32
}

fn needs_filter(p: &[u8], q: usize, step: usize, t: i32) -> bool {
    let (p1, p0, q0, q1) = (px(p, q, step, -2), px(p, q, step, -1), px(p, q, step, 0), px(p, q, step, 1));
    4 * (p0 - q0).abs() + (p1 - q1).abs() <= t
}

fn needs_filter2(p: &[u8], q: usize, step: usize, t: i32, it: i32) -> bool {
    let (p3, p2, p1, p0) = (px(p, q, step, -4), px(p, q, step, -3), px(p, q, step, -2), px(p, q, step, -1));
    let (q0, q1, q2, q3) = (px(p, q, step, 0), px(p, q, step, 1), px(p, q, step, 2), px(p, q, step, 3));
    if 4 * (p0 - q0).abs() + (p1 - q1).abs() > t {
        return false;
    }
    (p3 - p2).abs() <= it
        && (p2 - p1).abs() <= it
        && (p1 - p0).abs() <= it
        && (q3 - q2).abs() <= it
        && (q2 - q1).abs() <= it
        && (q1 - q0).abs() <= it
}

fn high_variance(p: &[u8], q: usize, step: usize, thresh: i32) -> bool {
    let (p1, p0, q0, q1) = (px(p, q, step, -2), px(p, q, step, -1), px(p, q, step, 0), px(p, q, step, 1));
    (p1 - p0).abs() > thresh || (q1 - q0).abs() > thresh
}

fn sclip1(v: i32) -> i32 {
    v.clamp(-128, 127)
}

fn sclip2(v: i32) -> i32 {
    v.clamp(-16, 15)
}

fn set(p: &mut [u8], q: usize, step: usize, k: isize, v: i32) {
    p[(q as isize + k * step as isize) as usize] = clip8(v);
}

fn filter2(p: &mut [u8], q: usize, step: usize) {
    let (p1, p0, q0, q1) = (px(p, q, step, -2), px(p, q, step, -1), px(p, q, step, 0), px(p, q, step, 1));
    let a = 3 * (q0 - p0) + sclip1(p1 - q1);
    let a1 = sclip2((a + 4) >> 3);
    let a2 = sclip2((a + 3) >> 3);
    set(p, q, step, -1, p0 + a2);
    set(p, q, step, 0, q0 - a1);
}

fn filter4(p: &mut [u8], q: usize, step: usize) {
    let (p1, p0, q0, q1) = (px(p, q, step, -2), px(p, q, step, -1), px(p, q, step, 0), px(p, q, step, 1));
    let a = 3 * (q0 - p0);
    let a1 = sclip2((a + 4) >> 3);
    let a2 = sclip2((a + 3) >> 3);
    let a3 = (a1 + 1) >> 1;
    set(p, q, step, -2, p1 + a3);
    set(p, q, step, -1, p0 + a2);
    set(p, q, step, 0, q0 - a1);
    set(p, q, step, 1, q1 - a3);
}

fn filter6(p: &mut [u8], q: usize, step: usize) {
    let (p2, p1, p0) = (px(p, q, step, -3), px(p, q, step, -2), px(p, q, step, -1));
    let (q0, q1, q2) = (px(p, q, step, 0), px(p, q, step, 1), px(p, q, step, 2));
    let a = sclip1(3 * (q0 - p0) + sclip1(p1 - q1));
    let a1 = (27 * a + 63) >> 7;
    let a2 = (18 * a + 63) >> 7;
    let a3 = (9 * a + 63) >> 7;
    set(p, q, step, -3, p2 + a3);
    set(p, q, step, -2, p1 + a2);
    set(p, q, step, -1, p0 + a1);
    set(p, q, step, 0, q0 - a1);
    set(p, q, step, 1, q1 - a2);
    set(p, q, step, 2, q2 - a3);
}

// --------------------------------------------------------------------------------------------- to RGB

fn mult_hi(v: i32, coeff: i32) -> i32 {
    (v * coeff) >> 8
}

fn clip_yuv(v: i32) -> u8 {
    if v & !((256 << 6) - 1) == 0 {
        (v >> 6) as u8
    } else if v < 0 {
        0
    } else {
        255
    }
}

fn yuv_to_rgb(y: u8, u: i32, v: i32, out: &mut [u8]) {
    let y = mult_hi(y as i32, 19077);
    out[0] = clip_yuv(y + mult_hi(v, 26149) - 14234);
    out[1] = clip_yuv(y - mult_hi(u, 6419) - mult_hi(v, 13320) + 8708);
    out[2] = clip_yuv(y + mult_hi(u, 33050) - 17685);
}

/// Two rows of pixels (`bottom` perhaps none) from their Y and the chroma rows nearest them (`top`'s, `cur`'s): each
/// pixel's U and V from the four chroma samples around it, weighted 9, 3, 3, 1, as libwebp's fancy upsampler has
/// it (its roundings too).
#[allow(clippy::too_many_arguments)]
fn upsample_pair(
    top_y: &[u8],
    bottom_y: Option<&[u8]>,
    top_uv: (&[u8], &[u8]),
    cur_uv: (&[u8], &[u8]),
    top_out: &mut [u8],
    bottom_out: Option<&mut [u8]>,
    len: usize,
) {
    let mut bottom = bottom_y.zip(bottom_out);
    let edge = |a: i32, b: i32| (3 * a + b + 2) >> 2;
    let sample = |row: (&[u8], &[u8]), x: usize| (row.0[x] as i32, row.1[x] as i32);
    let (mut tl, mut l) = (sample(top_uv, 0), sample(cur_uv, 0));
    yuv_to_rgb(top_y[0], edge(tl.0, l.0), edge(tl.1, l.1), &mut top_out[0..3]);
    if let Some((by, bo)) = bottom.as_mut() {
        yuv_to_rgb(by[0], edge(l.0, tl.0), edge(l.1, tl.1), &mut bo[0..3]);
    }
    for x in 1..=(len - 1) >> 1 {
        let (t, c) = (sample(top_uv, x), sample(cur_uv, x));
        let mix = |tl: i32, t: i32, l: i32, c: i32| {
            let avg = tl + t + l + c + 8;
            let d12 = (avg + 2 * (t + l)) >> 3;
            let d03 = (avg + 2 * (tl + c)) >> 3;
            ((d12 + tl) >> 1, (d03 + t) >> 1, (d03 + l) >> 1, (d12 + c) >> 1)
        };
        let (u0, u1, u2, u3) = mix(tl.0, t.0, l.0, c.0);
        let (v0, v1, v2, v3) = mix(tl.1, t.1, l.1, c.1);
        yuv_to_rgb(top_y[2 * x - 1], u0, v0, &mut top_out[(2 * x - 1) * 3..(2 * x) * 3]);
        yuv_to_rgb(top_y[2 * x], u1, v1, &mut top_out[(2 * x) * 3..(2 * x + 1) * 3]);
        if let Some((by, bo)) = bottom.as_mut() {
            yuv_to_rgb(by[2 * x - 1], u2, v2, &mut bo[(2 * x - 1) * 3..(2 * x) * 3]);
            yuv_to_rgb(by[2 * x], u3, v3, &mut bo[(2 * x) * 3..(2 * x + 1) * 3]);
        }
        tl = t;
        l = c;
    }
    if len & 1 == 0 {
        let x = len - 1;
        yuv_to_rgb(top_y[x], edge(tl.0, l.0), edge(tl.1, l.1), &mut top_out[x * 3..x * 3 + 3]);
        if let Some((by, bo)) = bottom.as_mut() {
            yuv_to_rgb(by[x], edge(l.0, tl.0), edge(l.1, tl.1), &mut bo[x * 3..x * 3 + 3]);
        }
    }
}

fn to_rgb(yp: &[u8], ys: usize, up: &[u8], vp: &[u8], uvs: usize, w: usize, h: usize) -> Vec<u8> {
    let mut rgb = vec![0u8; w * h * 3];
    let yrow = |r: usize| &yp[r * ys..r * ys + w];
    let uvrow = |r: usize| (&up[r * uvs..r * uvs + uvs], &vp[r * uvs..r * uvs + uvs]);
    // The first row: its chroma row's alone.
    upsample_pair(yrow(0), None, uvrow(0), uvrow(0), &mut rgb[..w * 3], None, w);
    // Then two rows at a time, between two chroma rows.
    let mut r = 1;
    while r + 1 < h {
        let k = (r + 1) / 2;
        let (a, b) = rgb[r * w * 3..(r + 2) * w * 3].split_at_mut(w * 3);
        upsample_pair(yrow(r), Some(yrow(r + 1)), uvrow(k - 1), uvrow(k), a, Some(b), w);
        r += 2;
    }
    // The last row, when there's an even number of them: its chroma row's alone.
    if h > 1 && h % 2 == 0 {
        let k = (h - 1) / 2;
        upsample_pair(yrow(h - 1), None, uvrow(k), uvrow(k), &mut rgb[(h - 1) * w * 3..], None, w);
    }
    rgb
}
