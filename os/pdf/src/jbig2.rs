//! JBIG2's black-and-white pictures (ITU-T T.88), as PDF's JBIG2Decode filter has them (ISO 32000-1, 7.4.7): what
//! scanners and Acrobat write scanned pages in, each a page's segments (with the global ones, often its symbol
//! dictionary, in the JBIG2Globals stream). Read here: generic regions (arithmetic-coded, with any of the four
//! templates, their adaptive pixels and typical prediction; or MMR-coded, as fax's Group 4), refinement regions,
//! symbol dictionaries (their symbols arithmetic-coded, refined or made of others) and text regions (symbols placed
//! in strips, any corner, transposed or not, refined), each region put on the page by its combination operator.
//! Not read: Huffman-coded symbols and text, and halftone regions (left white).
//!
//! The arithmetic coder is the MQ coder (T.88 Annex E), its integers as Annex A has them. The result is PDF's: a
//! bit a pixel, each line its own bytes, black 0.

use crate::ccitt;
use alloc::vec;
use alloc::vec::Vec;

/// The most pixels the regions and symbols of one picture may take together.
const MOST_PIXELS: usize = 160 << 20;
/// The most symbols a dictionary may make, and the longest symbol code.
const MOST_SYMBOLS: u32 = 1 << 20;
const MOST_CODE_LEN: u32 = 22;

/// A bitmap: a byte a pixel, 1 black.
#[derive(Clone, Default)]
struct Bitmap {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

impl Bitmap {
    fn new(w: usize, h: usize, fill: u8, budget: &mut usize) -> Option<Bitmap> {
        let n = w.checked_mul(h)?;
        *budget = budget.checked_sub(n)?;
        Some(Bitmap { w, h, px: vec![fill; n] })
    }

    #[inline]
    fn get(&self, x: isize, y: isize) -> u32 {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            0
        } else {
            self.px[y as usize * self.w + x as usize] as u32
        }
    }

    /// `src` put on this at (x, y), by a combination operator (0 OR, 1 AND, 2 XOR, 3 XNOR, 4 REPLACE).
    fn compose(&mut self, src: &Bitmap, x: i64, y: i64, op: u8) {
        for sy in 0..src.h {
            let ty = y + sy as i64;
            if ty < 0 || ty >= self.h as i64 {
                continue;
            }
            for sx in 0..src.w {
                let tx = x + sx as i64;
                if tx < 0 || tx >= self.w as i64 {
                    continue;
                }
                let s = src.px[sy * src.w + sx];
                let d = &mut self.px[ty as usize * self.w + tx as usize];
                *d = match op {
                    0 => *d | s,
                    1 => *d & s,
                    2 => *d ^ s,
                    3 => !(*d ^ s) & 1,
                    _ => s,
                };
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------- the MQ coder

/// The MQ coder's probability states: Qe, the next state after an MPS and after an LPS, and whether an LPS switches
/// which symbol is more probable.
const QE: [(u16, u8, u8, bool); 47] = [
    (0x5601, 1, 1, true), (0x3401, 2, 6, false), (0x1801, 3, 9, false), (0x0AC1, 4, 12, false), (0x0521, 5, 29, false),
    (0x0221, 38, 33, false), (0x5601, 7, 6, true), (0x5401, 8, 14, false), (0x4801, 9, 14, false), (0x3801, 10, 14, false),
    (0x3001, 11, 17, false), (0x2401, 12, 18, false), (0x1C01, 13, 20, false), (0x1601, 29, 21, false), (0x5601, 15, 14, true),
    (0x5401, 16, 14, false), (0x5101, 17, 15, false), (0x4801, 18, 16, false), (0x3801, 19, 17, false), (0x3401, 20, 18, false),
    (0x3001, 21, 19, false), (0x2801, 22, 19, false), (0x2401, 23, 20, false), (0x2201, 24, 21, false), (0x1C01, 25, 22, false),
    (0x1801, 26, 23, false), (0x1601, 27, 24, false), (0x1401, 28, 25, false), (0x1201, 29, 26, false), (0x1101, 30, 27, false),
    (0x0AC1, 31, 28, false), (0x09C1, 32, 29, false), (0x08A1, 33, 30, false), (0x0521, 34, 31, false), (0x0441, 35, 32, false),
    (0x02A1, 36, 33, false), (0x0221, 37, 34, false), (0x0141, 38, 35, false), (0x0111, 39, 36, false), (0x0085, 40, 37, false),
    (0x0049, 41, 38, false), (0x0025, 42, 39, false), (0x0015, 43, 40, false), (0x0009, 44, 41, false), (0x0005, 45, 42, false),
    (0x0001, 45, 43, false), (0x5601, 46, 46, false),
];

/// The MQ decoder (T.88 E.3, as its software conventions have it), over a segment's data.
pub(crate) struct Mq<'a> {
    data: &'a [u8],
    bp: usize,
    chigh: u32,
    clow: u32,
    ct: i32,
    a: u32,
}

impl<'a> Mq<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Mq<'a> {
        let mut m = Mq { data, bp: 0, chigh: *data.first().unwrap_or(&0xFF) as u32, clow: 0, ct: 0, a: 0x8000 };
        m.byte_in();
        m.chigh = ((m.chigh << 7) & 0xFFFF) | ((m.clow >> 9) & 0x7F);
        m.clow = (m.clow << 7) & 0xFFFF;
        m.ct -= 7;
        m
    }

    fn byte(&self, i: usize) -> u32 {
        // (Past the end: 0xFF, as the standard has it.)
        *self.data.get(i).unwrap_or(&0xFF) as u32
    }

    fn byte_in(&mut self) {
        if self.byte(self.bp) == 0xFF {
            if self.byte(self.bp + 1) > 0x8F {
                self.clow += 0xFF00;
                self.ct = 8;
            } else {
                self.bp += 1;
                self.clow += self.byte(self.bp) << 9;
                self.ct = 7;
            }
        } else {
            self.bp += 1;
            self.clow += self.byte(self.bp) << 8;
            self.ct = 8;
        }
        if self.clow > 0xFFFF {
            self.chigh += self.clow >> 16;
            self.clow &= 0xFFFF;
        }
    }

    /// A bit, in context `cx` (its state: the index in QE, shifted, and the more probable symbol).
    pub(crate) fn bit(&mut self, cx: &mut u8) -> u32 {
        let (mut index, mut mps) = ((*cx >> 1) as usize, (*cx & 1) as u32);
        let (qe, nmps, nlps, switch) = QE[index.min(46)];
        let qe = qe as u32;
        let mut a = self.a.wrapping_sub(qe);
        let d;
        if self.chigh < qe {
            // (The LPS's interval, or the MPS's if it's the bigger.)
            if a < qe {
                d = mps;
                index = nmps as usize;
            } else {
                d = 1 ^ mps;
                if switch {
                    mps = d;
                }
                index = nlps as usize;
            }
            a = qe;
        } else {
            self.chigh -= qe;
            if a & 0x8000 != 0 {
                self.a = a;
                return mps;
            }
            if a < qe {
                d = 1 ^ mps;
                if switch {
                    mps = d;
                }
                index = nlps as usize;
            } else {
                d = mps;
                index = nmps as usize;
            }
        }
        loop {
            if self.ct == 0 {
                self.byte_in();
            }
            a <<= 1;
            self.chigh = ((self.chigh << 1) & 0xFFFF) | ((self.clow >> 15) & 1);
            self.clow = (self.clow << 1) & 0xFFFF;
            self.ct -= 1;
            if a & 0x8000 != 0 {
                break;
            }
        }
        self.a = a;
        *cx = ((index as u8) << 1) | mps as u8;
        d
    }
}

/// An integer arithmetic decoding procedure's contexts (T.88 A.2: IADH, IADW and the others).
#[derive(Clone)]
struct IntCx(Vec<u8>);

impl IntCx {
    fn new() -> IntCx {
        IntCx(vec![0; 512])
    }

    /// An integer, or None for OOB.
    fn decode(&mut self, mq: &mut Mq) -> Option<i64> {
        let mut prev = 1usize;
        let mut bits = |n: u32, mq: &mut Mq, cx: &mut [u8]| -> u64 {
            let mut v = 0u64;
            for _ in 0..n {
                let b = mq.bit(&mut cx[prev]) as usize;
                prev = if prev < 256 { (prev << 1) | b } else { (((prev << 1) | b) & 511) | 256 };
                v = (v << 1) | b as u64;
            }
            v
        };
        let cx = &mut self.0;
        let sign = bits(1, mq, cx);
        let value = if bits(1, mq, cx) == 0 {
            bits(2, mq, cx)
        } else if bits(1, mq, cx) == 0 {
            bits(4, mq, cx) + 4
        } else if bits(1, mq, cx) == 0 {
            bits(6, mq, cx) + 20
        } else if bits(1, mq, cx) == 0 {
            bits(8, mq, cx) + 84
        } else if bits(1, mq, cx) == 0 {
            bits(12, mq, cx) + 340
        } else {
            bits(32, mq, cx) + 4436
        } as i64;
        match (sign, value) {
            (0, v) => Some(v),
            (_, 0) => None,
            (_, v) => Some(-v),
        }
    }
}

/// The symbol ID procedure's contexts (IAID, T.88 A.3): `len` bits.
struct IdCx {
    len: u32,
    cx: Vec<u8>,
}

impl IdCx {
    fn new(len: u32) -> IdCx {
        IdCx { len, cx: vec![0; 1 << (len + 1)] }
    }

    fn decode(&mut self, mq: &mut Mq) -> usize {
        let mut prev = 1usize;
        for _ in 0..self.len {
            let b = mq.bit(&mut self.cx[prev]) as usize;
            prev = (prev << 1) | b;
        }
        prev - (1 << self.len)
    }
}

// --------------------------------------------------------------------------------------------------- regions

/// A generic region's way of being coded (T.88 6.2).
struct Generic {
    template: u8,
    tpgdon: bool,
    at: [(i8, i8); 4],
}

/// The contexts typical prediction decodes its "the row is the row above" bit in, by template (T.88 6.2.5.7).
const SLTP: [usize; 4] = [0x9B25, 0x0795, 0x00E5, 0x0195];

/// A generic region, arithmetic-coded (T.88 6.2.5): each pixel in the context of those before it, as its template
/// takes them (the bits as the standard numbers them).
fn generic(mq: &mut Mq, cx: &mut [u8], w: usize, h: usize, g: &Generic, budget: &mut usize) -> Option<Bitmap> {
    let mut b = Bitmap::new(w, h, 0, budget)?;
    let at = g.at.map(|(x, y)| (x as isize, y as isize));
    let mut ltp = 0u32;
    for y in 0..h {
        let yi = y as isize;
        if g.tpgdon {
            ltp ^= mq.bit(&mut cx[SLTP[g.template as usize & 3]]);
            if ltp == 1 {
                if y > 0 {
                    let (above, here) = b.px.split_at_mut(y * w);
                    here[..w].copy_from_slice(&above[(y - 1) * w..]);
                }
                continue;
            }
        }
        for x in 0..w {
            let xi = x as isize;
            let p = |dx: isize, dy: isize| b.get(xi + dx, yi + dy);
            let c = match g.template {
                0 => {
                    p(-1, 0) | p(-2, 0) << 1 | p(-3, 0) << 2 | p(-4, 0) << 3 | p(at[0].0, at[0].1) << 4 | p(2, -1) << 5
                        | p(1, -1) << 6 | p(0, -1) << 7 | p(-1, -1) << 8 | p(-2, -1) << 9 | p(at[1].0, at[1].1) << 10
                        | p(at[2].0, at[2].1) << 11 | p(1, -2) << 12 | p(0, -2) << 13 | p(-1, -2) << 14 | p(at[3].0, at[3].1) << 15
                }
                1 => {
                    p(-1, 0) | p(-2, 0) << 1 | p(-3, 0) << 2 | p(at[0].0, at[0].1) << 3 | p(2, -1) << 4 | p(1, -1) << 5
                        | p(0, -1) << 6 | p(-1, -1) << 7 | p(-2, -1) << 8 | p(2, -2) << 9 | p(1, -2) << 10 | p(0, -2) << 11
                        | p(-1, -2) << 12
                }
                2 => {
                    p(-1, 0) | p(-2, 0) << 1 | p(at[0].0, at[0].1) << 2 | p(1, -1) << 3 | p(0, -1) << 4 | p(-1, -1) << 5
                        | p(-2, -1) << 6 | p(1, -2) << 7 | p(0, -2) << 8 | p(-1, -2) << 9
                }
                _ => {
                    p(-1, 0) | p(-2, 0) << 1 | p(-3, 0) << 2 | p(-4, 0) << 3 | p(at[0].0, at[0].1) << 4 | p(1, -1) << 5
                        | p(0, -1) << 6 | p(-1, -1) << 7 | p(-2, -1) << 8 | p(-3, -1) << 9
                }
            };
            b.px[y * w + x] = mq.bit(&mut cx[c as usize]) as u8;
        }
    }
    Some(b)
}

/// A generic region coded as MMR (fax's Group 4, black 1).
fn generic_mmr(data: &[u8], w: usize, h: usize, budget: &mut usize) -> Option<Bitmap> {
    if w == 0 || h == 0 || w > 1 << 16 || h > 1 << 16 {
        return Bitmap::new(w, h, 0, budget);
    }
    let p = ccitt::Params { k: -1, columns: w, rows: h, byte_align: false, black_is_1: true };
    let packed = ccitt::decode(data, &p, h)?;
    let mut b = Bitmap::new(w, h, 0, budget)?;
    let stride = w.div_ceil(8);
    for y in 0..h {
        for x in 0..w {
            b.px[y * w + x] = (packed.get(y * stride + x / 8).copied().unwrap_or(0) >> (7 - x % 8)) & 1;
        }
    }
    Some(b)
}

/// A refinement's way of being coded (T.88 6.3).
struct Refine {
    template: u8,
    at: [(i8, i8); 2],
}

/// A refinement (T.88 6.3.5): a new bitmap, each pixel in the context of those before it and of the reference's
/// around where it is (the reference's (0, 0) at (dx, dy)); with typical prediction, a pixel whose reference is the
/// same all round it is that.
#[allow(clippy::too_many_arguments)]
fn refine(mq: &mut Mq, cx: &mut [u8], w: usize, h: usize, r: &Refine, reference: &Bitmap, dx: i64, dy: i64, tpgron: bool, budget: &mut usize) -> Option<Bitmap> {
    let mut b = Bitmap::new(w, h, 0, budget)?;
    let (dx, dy) = (dx as isize, dy as isize);
    let at = r.at.map(|(x, y)| (x as isize, y as isize));
    let mut ltp = 0u32;
    for y in 0..h {
        let yi = y as isize;
        if tpgron {
            // (Its context as jbig2dec has it, which T.88's conformance streams were checked with.)
            ltp ^= mq.bit(&mut cx[if r.template == 0 { 0x100 } else { 0x040 }]);
        }
        for x in 0..w {
            let xi = x as isize;
            let rf = |ox: isize, oy: isize| reference.get(xi - dx + ox, yi - dy + oy);
            if ltp == 1 {
                let around: u32 = (-1..=1).flat_map(|oy| (-1..=1).map(move |ox| (ox, oy))).map(|(ox, oy)| rf(ox, oy)).sum();
                if around == 0 || around == 9 {
                    b.px[y * w + x] = (around == 9) as u8;
                    continue;
                }
            }
            let p = |ox: isize, oy: isize| b.get(xi + ox, yi + oy);
            let c = if r.template == 0 {
                p(-1, 0) | p(1, -1) << 1 | p(0, -1) << 2 | p(at[0].0, at[0].1) << 3 | rf(1, 1) << 4 | rf(0, 1) << 5
                    | rf(-1, 1) << 6 | rf(1, 0) << 7 | rf(0, 0) << 8 | rf(-1, 0) << 9 | rf(1, -1) << 10 | rf(0, -1) << 11
                    | rf(at[1].0, at[1].1) << 12
            } else {
                p(-1, 0) | p(1, -1) << 1 | p(0, -1) << 2 | p(-1, -1) << 3 | rf(1, 1) << 4 | rf(0, 1) << 5 | rf(1, 0) << 6
                    | rf(0, 0) << 7 | rf(-1, 0) << 8 | rf(0, -1) << 9
            };
            b.px[y * w + x] = mq.bit(&mut cx[c as usize]) as u8;
        }
    }
    Some(b)
}

/// The integer contexts a symbol dictionary's and a text region's decoding share (each segment's own).
struct Ints {
    dh: IntCx,
    dw: IntCx,
    ex: IntCx,
    ai: IntCx,
    dt: IntCx,
    fs: IntCx,
    ds: IntCx,
    it: IntCx,
    ri: IntCx,
    rdw: IntCx,
    rdh: IntCx,
    rdx: IntCx,
    rdy: IntCx,
}

impl Ints {
    fn new() -> Ints {
        let n = IntCx::new;
        Ints { dh: n(), dw: n(), ex: n(), ai: n(), dt: n(), fs: n(), ds: n(), it: n(), ri: n(), rdw: n(), rdh: n(), rdx: n(), rdy: n() }
    }
}

/// A text region's parameters (T.88 6.4.2).
struct Text {
    w: usize,
    h: usize,
    instances: u32,
    strips: u32,
    refine: bool,
    transposed: bool,
    corner: u8,
    op: u8,
    default: u8,
    ds_offset: i64,
    refinement: Refine,
}

/// Symbols placed in a region (T.88 6.4.5): strip after strip, each symbol's ID, where along the strip it goes, and
/// perhaps a refinement of it.
#[allow(clippy::too_many_arguments)]
fn text(mq: &mut Mq, ints: &mut Ints, id: &mut IdCx, gr: &mut [u8], t: &Text, symbols: &[&Bitmap], budget: &mut usize) -> Option<Bitmap> {
    let mut region = Bitmap::new(t.w, t.h, t.default, budget)?;
    let strips = t.strips as i64;
    let mut strip_t = -ints.dt.decode(mq)? * strips;
    let mut first_s = 0i64;
    let mut placed = 0u32;
    while placed < t.instances {
        strip_t += ints.dt.decode(mq)? * strips;
        first_s += ints.fs.decode(mq)?;
        let mut cur_s = first_s;
        // (Each strip's symbols until its S's OOB, the last one's too.)
        loop {
            let cur_t = if t.strips == 1 { 0 } else { ints.it.decode(mq)? };
            let ti = strip_t + cur_t;
            let symbol = symbols.get(id.decode(mq)).copied()?;
            let refined;
            let ib = if t.refine && ints.ri.decode(mq)? != 0 {
                let (rdw, rdh, rdx, rdy) = (ints.rdw.decode(mq)?, ints.rdh.decode(mq)?, ints.rdx.decode(mq)?, ints.rdy.decode(mq)?);
                let (w, h) = (symbol.w as i64 + rdw, symbol.h as i64 + rdh);
                if w < 0 || h < 0 {
                    return None;
                }
                // (Each refinement its own room, given back once it's placed.)
                let mut room = 16 << 20;
                refined = refine(mq, gr, w as usize, h as usize, &t.refinement, symbol, rdw.div_euclid(2) + rdx, rdh.div_euclid(2) + rdy, false, &mut room)?;
                &refined
            } else {
                symbol
            };
            let (wi, hi) = (ib.w as i64, ib.h as i64);
            let right = t.corner & 2 != 0;
            let bottom = t.corner & 1 == 0;
            if !t.transposed && right {
                cur_s += wi - 1;
            } else if t.transposed && bottom {
                cur_s += hi - 1;
            }
            let (sx, sy) = if t.transposed { (ti, cur_s) } else { (cur_s, ti) };
            let x = sx - if right { wi - 1 } else { 0 };
            let y = sy - if bottom { hi - 1 } else { 0 };
            region.compose(ib, x, y, t.op);
            if !t.transposed && !right {
                cur_s += wi - 1;
            } else if t.transposed && !bottom {
                cur_s += hi - 1;
            }
            placed += 1;
            match ints.ds.decode(mq) {
                Some(ds) if placed < t.instances => cur_s += ds + t.ds_offset,
                _ => break,
            }
        }
    }
    Some(region)
}

// -------------------------------------------------------------------------------------------------- segments

/// What a segment left for those that refer to it.
enum Kept {
    /// A symbol dictionary's exported symbols, and its contexts if it kept them (for the next to use).
    Symbols(Vec<Bitmap>, Option<(Vec<u8>, Vec<u8>)>),
    /// An intermediate region's bitmap.
    Region(Bitmap),
}

/// A segment's header (T.88 7.2).
struct Segment {
    number: u32,
    kind: u8,
    refers: Vec<u32>,
    data_len: u32,
}

fn be16(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// A segment's header at `at`: it, and where its data starts.
fn header(b: &[u8], at: usize) -> Option<(Segment, usize)> {
    let number = be32(b, at)?;
    let flags = *b.get(at + 4)?;
    let mut i = at + 5;
    let first = *b.get(i)?;
    let count = if first >> 5 == 7 {
        let n = be32(b, i)? & 0x1FFF_FFFF;
        i += 4 + (n as usize + 1).div_ceil(8);
        n
    } else {
        i += 1;
        (first >> 5) as u32
    };
    if count > 10_000 {
        return None;
    }
    let size = if number <= 256 { 1 } else if number <= 65_536 { 2 } else { 4 };
    let mut refers = Vec::with_capacity(count as usize);
    for _ in 0..count {
        refers.push(match size {
            1 => *b.get(i)? as u32,
            2 => be16(b, i)? as u32,
            _ => be32(b, i)?,
        });
        i += size;
    }
    i += if flags & 0x40 != 0 { 4 } else { 1 };
    let data_len = be32(b, i)?;
    i += 4;
    Some((Segment { number, kind: flags & 0x3F, refers, data_len }, i))
}

/// A segment whose data's length isn't known (an immediate generic region's: T.88 7.2.7): its end found by its end
/// marker (0xFFAC, or 0x0000 for MMR) and its row count.
fn unknown_length(b: &[u8], start: usize) -> Option<u32> {
    let h = be32(b, start + 4)?;
    let mmr = b.get(start + 17)? & 1 != 0;
    let mut pattern = [0u8; 6];
    if !mmr {
        pattern[..2].copy_from_slice(&[0xFF, 0xAC]);
    }
    pattern[2..].copy_from_slice(&h.to_be_bytes());
    let at = b.get(start..)?.windows(6).position(|w| w == pattern)?;
    Some((at + 6) as u32)
}

/// The segments of a stream, in order, each with its data: sequentially (header, data, header, data), or a JBIG2
/// file's random-access organisation (every header, then every segment's data).
fn segments(b: &[u8]) -> Vec<(Segment, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0;
    let mut sequential = true;
    // (A file's header, which a stream shouldn't have but some do.)
    if b.starts_with(&[0x97, 0x4A, 0x42, 0x32, 0x0D, 0x0A, 0x1A, 0x0A]) {
        let flags = b.get(8).copied().unwrap_or(1);
        sequential = flags & 1 != 0;
        at = if flags & 2 == 0 { 13 } else { 9 };
    }
    if sequential {
        while let Some((mut s, start)) = header(b, at) {
            if s.data_len == u32::MAX {
                let Some(n) = unknown_length(b, start) else { break };
                s.data_len = n;
            }
            let end = start.saturating_add(s.data_len as usize).min(b.len());
            let kind = s.kind;
            out.push((s, &b[start.min(end)..end]));
            at = end;
            if kind == 51 || at >= b.len() || out.len() > 100_000 {
                break;
            }
        }
    } else {
        let mut headers = Vec::new();
        while let Some((s, next)) = header(b, at) {
            let end = s.kind == 51;
            headers.push(s);
            at = next;
            if end || headers.len() > 100_000 {
                break;
            }
        }
        for s in headers {
            let end = at.saturating_add(s.data_len as usize).min(b.len());
            out.push((s, &b[at.min(end)..end]));
            at = end;
        }
    }
    out
}

/// A region segment's information field (T.88 7.4.1): its width, height, place and combination operator.
fn region_info(d: &[u8]) -> Option<(usize, usize, i64, i64, u8)> {
    Some((be32(d, 0)? as usize, be32(d, 4)? as usize, be32(d, 8)? as i64, be32(d, 12)? as i64, d.get(16)? & 7))
}

fn at_pixels<const N: usize>(d: &[u8], at: usize) -> Option<[(i8, i8); N]> {
    let mut out = [(0i8, 0i8); N];
    for (k, p) in out.iter_mut().enumerate() {
        *p = (*d.get(at + 2 * k)? as i8, *d.get(at + 2 * k + 1)? as i8);
    }
    Some(out)
}

/// The decoding of a page: its bitmap, and what each segment kept.
struct Page {
    bitmap: Option<Bitmap>,
    striped: bool,
    kept: Vec<(u32, Kept)>,
    budget: usize,
}

impl Page {
    fn kept(&self, n: u32) -> Option<&Kept> {
        self.kept.iter().rev().find(|(k, _)| *k == n).map(|(_, v)| v)
    }

    /// The symbols the segments `refers` to export, in order.
    fn symbols(&self, refers: &[u32]) -> Vec<&Bitmap> {
        let mut out = Vec::new();
        for r in refers {
            if let Some(Kept::Symbols(s, _)) = self.kept(*r) {
                out.extend(s.iter());
            }
        }
        out
    }

    /// A region decoded: on the page (an immediate one), or kept (an intermediate one).
    fn place(&mut self, s: &Segment, b: Bitmap, x: i64, y: i64, op: u8) {
        if matches!(s.kind, 4 | 20 | 36 | 40) {
            self.kept.push((s.number, Kept::Region(b)));
            return;
        }
        if self.striped
            && let Some(page) = self.bitmap.as_mut()
            && (y + b.h as i64) as usize > page.h
        {
            // (A page of unknown height, striped: it grows to take the region.)
            let h = (y + b.h as i64) as usize;
            let more = (h - page.h) * page.w;
            if self.budget >= more {
                self.budget -= more;
                page.px.resize(h * page.w, 0);
                page.h = h;
            }
        }
        if let Some(page) = self.bitmap.as_mut() {
            page.compose(&b, x, y, op);
        }
    }

    fn segment(&mut self, s: &Segment, d: &[u8]) -> Option<()> {
        match s.kind {
            48 => {
                let (w, h) = (be32(d, 0)? as usize, be32(d, 4)?);
                let flags = *d.get(16)?;
                let striping = be16(d, 17).unwrap_or(0);
                self.striped = h == u32::MAX || striping & 0x8000 != 0;
                let h = if h == u32::MAX { 0 } else { h as usize };
                self.bitmap = Some(Bitmap::new(w, h, (flags >> 2) & 1, &mut self.budget)?);
            }
            50 => {
                // An end of stripe: the page is at least this tall.
                let end = be32(d, 0)? as usize + 1;
                if let Some(page) = self.bitmap.as_mut()
                    && end > page.h
                    && self.striped
                {
                    let more = (end - page.h).checked_mul(page.w)?;
                    self.budget = self.budget.checked_sub(more)?;
                    page.px.resize(end * page.w, 0);
                    page.h = end;
                }
            }
            0 => {
                let kept = self.symbol_dictionary(s, d)?;
                self.kept.push((s.number, kept));
            }
            4 | 6 | 7 => {
                let (w, h, x, y, op) = region_info(d)?;
                let b = self.text_region(s, &d[17..], w, h)?;
                self.place(s, b, x, y, op);
            }
            36 | 38 | 39 => {
                let (w, h, x, y, op) = region_info(d)?;
                let flags = *d.get(17)?;
                let b = if flags & 1 != 0 {
                    generic_mmr(d.get(18..)?, w, h, &mut self.budget)?
                } else {
                    let template = (flags >> 1) & 3;
                    let n = if template == 0 { 4 } else { 1 };
                    let mut at = at_pixels::<4>(d, 18).unwrap_or_default();
                    if n == 1 {
                        at = [at[0], (0, 0), (0, 0), (0, 0)];
                    }
                    let g = Generic { template, tpgdon: flags & 8 != 0, at };
                    let mut cx = vec![0u8; 1 << 16];
                    let mut mq = Mq::new(d.get(18 + 2 * n..)?);
                    generic(&mut mq, &mut cx, w, h, &g, &mut self.budget)?
                };
                self.place(s, b, x, y, op);
            }
            40 | 42 | 43 => {
                let (w, h, x, y, op) = region_info(d)?;
                let flags = *d.get(17)?;
                let template = flags & 1;
                let at = if template == 0 { at_pixels::<2>(d, 18)? } else { [(0, 0); 2] };
                let data = d.get(if template == 0 { 22 } else { 18 }..)?;
                // The reference: the region it refers to, or the page where it's going.
                let reference = match s.refers.first().and_then(|r| self.kept(*r)) {
                    Some(Kept::Region(b)) => b.clone(),
                    _ => {
                        let page = self.bitmap.as_ref()?;
                        let mut b = Bitmap::new(w, h, 0, &mut self.budget)?;
                        for yy in 0..h {
                            for xx in 0..w {
                                b.px[yy * w + xx] = page.get(x as isize + xx as isize, y as isize + yy as isize) as u8;
                            }
                        }
                        b
                    }
                };
                let mut cx = vec![0u8; 1 << 13];
                let mut mq = Mq::new(data);
                let r = Refine { template, at };
                let b = refine(&mut mq, &mut cx, w, h, &r, &reference, 0, 0, flags & 2 != 0, &mut self.budget)?;
                if matches!(s.kind, 42 | 43) && s.refers.is_empty() {
                    // (A refinement of the page itself replaces what was there.)
                    self.place(s, b, x, y, 4);
                } else {
                    self.place(s, b, x, y, op);
                }
            }
            _ => {}
        }
        Some(())
    }

    /// A symbol dictionary (T.88 6.5, 7.4.3), arithmetic-coded: its height classes' symbols, each its own generic
    /// region or a refinement or aggregate of others; then which of them it exports.
    fn symbol_dictionary(&mut self, s: &Segment, d: &[u8]) -> Option<Kept> {
        let flags = be16(d, 0)?;
        let (huffman, refagg) = (flags & 1 != 0, flags & 2 != 0);
        if huffman {
            return Some(Kept::Symbols(Vec::new(), None));
        }
        let template = ((flags >> 10) & 3) as u8;
        let rtemplate = ((flags >> 12) & 1) as u8;
        let mut i = 2;
        let mut at = at_pixels::<4>(d, i)?;
        if template != 0 {
            at = [at[0], (0, 0), (0, 0), (0, 0)];
        }
        i += if template == 0 { 8 } else { 2 };
        let rat = if refagg && rtemplate == 0 {
            let a = at_pixels::<2>(d, i)?;
            i += 4;
            a
        } else {
            [(0, 0); 2]
        };
        let exported_count = be32(d, i)?;
        let new_count = be32(d, i + 4)?.min(MOST_SYMBOLS);
        i += 8;
        let inputs: Vec<Bitmap> = self.symbols(&s.refers).into_iter().cloned().collect();
        // (The contexts the last dictionary it refers to kept, if it says it uses them.)
        let used = flags & 0x100 != 0;
        let retained = flags & 0x200 != 0;
        let previous = if used {
            s.refers.iter().rev().find_map(|r| match self.kept(*r) {
                Some(Kept::Symbols(_, Some(c))) => Some(c.clone()),
                _ => None,
            })
        } else {
            None
        };
        let (mut gb, mut gr) = previous.unwrap_or_else(|| (vec![0u8; 1 << 16], vec![0u8; 1 << 13]));
        let mut mq = Mq::new(d.get(i..)?);
        let mut ints = Ints::new();
        let total = inputs.len() as u32 + new_count;
        let code_len = 32 - total.saturating_sub(1).leading_zeros();
        if code_len > MOST_CODE_LEN {
            return None;
        }
        let mut id = IdCx::new(code_len);
        let g = Generic { template, tpgdon: false, at };
        let r = Refine { template: rtemplate, at: rat };
        let mut new: Vec<Bitmap> = Vec::new();
        let mut height = 0i64;
        let mut classes = 0u32;
        while (new.len() as u32) < new_count {
            classes += 1;
            if classes > new_count + 64 {
                return None;
            }
            height += ints.dh.decode(&mut mq)?;
            let mut width = 0i64;
            loop {
                let Some(dw) = ints.dw.decode(&mut mq) else { break };
                width += dw;
                if width < 0 || height < 0 || (new.len() as u32) >= new_count {
                    return None;
                }
                let (w, h) = (width as usize, height as usize);
                let symbol = if !refagg {
                    generic(&mut mq, &mut gb, w, h, &g, &mut self.budget)?
                } else {
                    let instances = ints.ai.decode(&mut mq)?;
                    if instances > 1 {
                        let all: Vec<&Bitmap> = inputs.iter().chain(new.iter()).collect();
                        let t = Text {
                            w,
                            h,
                            instances: instances.min(u32::MAX as i64) as u32,
                            strips: 1,
                            refine: true,
                            transposed: false,
                            corner: 1,
                            op: 0,
                            default: 0,
                            ds_offset: 0,
                            refinement: Refine { template: rtemplate, at: rat },
                        };
                        text(&mut mq, &mut ints, &mut id, &mut gr, &t, &all, &mut self.budget)?
                    } else {
                        let n = id.decode(&mut mq);
                        let (rdx, rdy) = (ints.rdx.decode(&mut mq)?, ints.rdy.decode(&mut mq)?);
                        let reference = if n < inputs.len() { inputs.get(n)? } else { new.get(n - inputs.len())? };
                        refine(&mut mq, &mut gr, w, h, &r, reference, rdx, rdy, false, &mut self.budget)?
                    }
                };
                new.push(symbol);
            }
        }
        // Which are exported: runs of not and then of exported, in turn, through them all.
        let mut exported = Vec::new();
        let mut index = 0u32;
        let mut flag = false;
        let mut runs = 0u32;
        while index < total {
            runs += 1;
            if runs > total + 64 {
                return None;
            }
            let run = ints.ex.decode(&mut mq)?;
            if run < 0 || index as i64 + run > total as i64 {
                return None;
            }
            if flag {
                for k in index..index + run as u32 {
                    let k = k as usize;
                    exported.push(if k < inputs.len() { inputs[k].clone() } else { new[k - inputs.len()].clone() });
                }
            }
            index += run as u32;
            flag = !flag;
        }
        let _ = exported_count;
        Some(Kept::Symbols(exported, retained.then_some((gb, gr))))
    }

    /// A text region's segment (T.88 7.4.4), arithmetic-coded: its flags, then its symbols placed.
    fn text_region(&mut self, s: &Segment, d: &[u8], w: usize, h: usize) -> Option<Bitmap> {
        let flags = be16(d, 0)?;
        if flags & 1 != 0 {
            // (Huffman-coded: left white.)
            return Bitmap::new(w, h, 0, &mut self.budget);
        }
        let refine_on = flags & 2 != 0;
        let strips = 1u32 << ((flags >> 2) & 3);
        let corner = ((flags >> 4) & 3) as u8;
        let transposed = flags & 0x40 != 0;
        let op = ((flags >> 7) & 3) as u8;
        let default = ((flags >> 9) & 1) as u8;
        let mut ds_offset = ((flags >> 10) & 0x1F) as i64;
        if ds_offset >= 16 {
            ds_offset -= 32;
        }
        let rtemplate = ((flags >> 15) & 1) as u8;
        let mut i = 2;
        let rat = if refine_on && rtemplate == 0 {
            let a = at_pixels::<2>(d, i)?;
            i += 4;
            a
        } else {
            [(0, 0); 2]
        };
        let instances = be32(d, i)?;
        i += 4;
        let symbols = self.symbols(&s.refers);
        let code_len = 32 - (symbols.len() as u32).saturating_sub(1).leading_zeros();
        if code_len > MOST_CODE_LEN {
            return None;
        }
        let symbols: Vec<Bitmap> = symbols.into_iter().cloned().collect();
        let refs: Vec<&Bitmap> = symbols.iter().collect();
        let t = Text { w, h, instances, strips, refine: refine_on, transposed, corner, op, default, ds_offset, refinement: Refine { template: rtemplate, at: rat } };
        let mut mq = Mq::new(d.get(i..)?);
        let mut gr = vec![0u8; 1 << 13];
        let mut budget = self.budget;
        let b = text(&mut mq, &mut Ints::new(), &mut IdCx::new(code_len), &mut gr, &t, &refs, &mut budget);
        self.budget = budget;
        b
    }
}

/// A JBIG2Decode stream's picture (with its JBIG2Globals' segments first), `width` × `height`: a bit a pixel, each
/// line its own bytes, black 0 (as PDF's DeviceGray has it). None if it isn't one.
pub fn decode(data: &[u8], globals: Option<&[u8]>, width: usize, height: usize) -> Option<Vec<u8>> {
    // (Room for the page three times over, and its symbols.)
    let budget = width.saturating_mul(height).saturating_mul(3).saturating_add(16 << 20).min(MOST_PIXELS);
    let mut page = Page { bitmap: None, striped: false, kept: Vec::new(), budget };
    for (s, d) in globals.map(segments).unwrap_or_default().into_iter().chain(segments(data)) {
        if matches!(s.kind, 49 | 51) {
            break;
        }
        // (A damaged segment leaves what it would have made out; the others still come.)
        let _ = page.segment(&s, d);
    }
    let bitmap = page.bitmap?;
    let stride = width.div_ceil(8);
    let mut out = vec![0xFFu8; stride * height];
    for y in 0..height.min(bitmap.h) {
        for x in 0..width.min(bitmap.w) {
            if bitmap.px[y * bitmap.w + x] != 0 {
                out[y * stride + x / 8] &= !(0x80 >> (x % 8));
            }
        }
    }
    Some(out)
}


#[cfg(test)]
mod fuzz {
    extern crate std;
    use crate::file::File;
    use crate::lex::Obj;

    /// Every fourth byte of each page's stream (and its globals') changed in turn: none crashes it (minutes long, so
    /// only when asked: cargo test -p pdf -- --ignored).
    #[test]
    #[ignore]
    fn every_byte_changed() {
        let data = std::fs::read(std::format!("{}/tests/scan-jbig2.pdf", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let f = File::open(&data);
        let mut n = 0;
        for num in f.numbers() {
            let Some((_, bytes, Some(codec))) = f.image_stream(&Obj::Ref(num, 0)) else { continue };
            let globals = codec.1.as_ref().and_then(|p| p.get("JBIG2Globals")).and_then(|g| f.stream(g));
            for i in (0..bytes.len()).step_by(4) {
                for flip in [0x10u8, 0xFF] {
                    let mut d = bytes.clone();
                    d[i] ^= flip;
                    let _ = super::decode(&d, globals.as_deref(), 480, 320);
                    n += 1;
                }
            }
            if let Some(g) = &globals {
                for i in 0..g.len() {
                    let mut d = g.clone();
                    d[i] ^= 0x41;
                    let _ = super::decode(&bytes, Some(&d), 480, 320);
                    n += 1;
                }
            }
        }
        std::println!("{n} decodes, no crash");
    }
}
