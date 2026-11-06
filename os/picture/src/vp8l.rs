//! WebP's lossless format (VP8L, RFC 9649 section 5): an image of ARGB pixels decoded from its prefix codes
//! (canonical Huffman codes, a group of five for each block of the image when it has meta prefix codes), its backward
//! references (LZ77, their distances in 2D) and its colour cache; then its transforms (predictor, colour, subtract
//! green, colour indexing) undone, last first.

use crate::webp_tables::{CODE_LENGTH_ORDER, CODE_TO_PLANE};
use crate::Error;
use alloc::vec;
use alloc::vec::Vec;

/// Bits read least significant first, as VP8L packs them. Past the end, zeros (`past_end` says so).
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    acc: u64,
    n: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Bits<'a> {
        Bits { data, at: 0, acc: 0, n: 0 }
    }

    fn fill(&mut self) {
        while self.n <= 56 {
            let b = if self.at < self.data.len() { self.data[self.at] } else { 0 };
            self.at += 1;
            self.acc |= (b as u64) << self.n;
            self.n += 8;
        }
    }

    fn read(&mut self, bits: u32) -> u32 {
        if bits == 0 {
            return 0;
        }
        self.fill();
        let v = (self.acc & ((1u64 << bits) - 1)) as u32;
        self.acc >>= bits;
        self.n -= bits;
        v
    }

    fn peek(&mut self) -> u32 {
        self.fill();
        self.acc as u32
    }

    fn skip(&mut self, bits: u32) {
        self.acc >>= bits;
        self.n -= bits;
    }

    /// Whether more bits have been read than there are.
    fn past_end(&self) -> bool {
        (self.at as u64) * 8 - self.n as u64 > self.data.len() as u64 * 8
    }
}

// --------------------------------------------------------------------------------------------- prefix codes

/// A canonical Huffman code, as a table: its first `ROOT` bits (read least significant first) give an entry, which is
/// a symbol and its length, or (for longer codes) where a second table for the bits after them starts and how many
/// bits it takes. Entries: symbol | length << 16, or start | bits << 16 | 1 << 31.
struct Code {
    table: Vec<u32>,
    /// A code of one symbol: it takes no bits.
    single: Option<u16>,
}

const ROOT: u32 = 8;
const LINK: u32 = 1 << 31;

impl Code {
    /// The code with these lengths (0: the symbol isn't in it). None if they aren't a code: no symbol, or (two or
    /// more) not using every bit pattern exactly once.
    fn from_lengths(lengths: &[u8]) -> Option<Code> {
        let mut used = lengths.iter().enumerate().filter(|(_, l)| **l > 0);
        let first = used.next()?.0;
        if used.next().is_none() {
            return Some(Code { table: Vec::new(), single: Some(first as u16) });
        }
        let mut count = [0u32; 16];
        for &l in lengths {
            if l > 15 {
                return None;
            }
            count[l as usize] += 1;
        }
        count[0] = 0;
        let mut left: i64 = 1;
        for &c in &count[1..] {
            left = left * 2 - c as i64;
            if left < 0 {
                return None;
            }
        }
        if left != 0 {
            return None;
        }
        // Each symbol's code: by length, then by symbol; reversed, as the bits come least significant first.
        let mut next = [0u32; 16];
        let mut code = 0u32;
        for l in 1..16 {
            code = (code + count[l - 1]) << 1;
            next[l] = code;
        }
        let mut codes: Vec<(u32, u32, u32)> = Vec::new(); // (code, length, symbol)
        for (s, &l) in lengths.iter().enumerate() {
            if l > 0 {
                let c = next[l as usize];
                next[l as usize] += 1;
                codes.push((reverse(c, l as u32), l as u32, s as u32));
            }
        }
        let mut table = vec![0u32; 1 << ROOT];
        for &(rc, l, s) in &codes {
            if l <= ROOT {
                let mut i = rc;
                while i < (1 << ROOT) {
                    table[i as usize] = s | l << 16;
                    i += 1 << l;
                }
            }
        }
        // Longer codes: a table for each first ROOT bits they share, as many bits as the longest of them needs.
        let mut extra = [0u32; 1 << ROOT];
        for &(rc, l, _) in &codes {
            if l > ROOT {
                let p = (rc & ((1 << ROOT) - 1)) as usize;
                extra[p] = extra[p].max(l - ROOT);
            }
        }
        for p in 0..(1usize << ROOT) {
            if extra[p] > 0 {
                let start = table.len() as u32;
                table[p] = start | extra[p] << 16 | LINK;
                table.resize(table.len() + (1 << extra[p]), 0);
            }
        }
        for &(rc, l, s) in &codes {
            if l > ROOT {
                let p = (rc & ((1 << ROOT) - 1)) as usize;
                let start = (table[p] & 0xFFFF) as usize;
                let bits = extra[p];
                let ll = l - ROOT;
                let mut i = rc >> ROOT;
                while i < (1 << bits) {
                    table[start + i as usize] = s | ll << 16;
                    i += 1 << ll;
                }
            }
        }
        Some(Code { table, single: None })
    }

    fn read(&self, b: &mut Bits) -> u32 {
        if let Some(s) = self.single {
            return s as u32;
        }
        let v = b.peek();
        let e = self.table[(v & ((1 << ROOT) - 1)) as usize];
        if e & LINK == 0 {
            b.skip(e >> 16);
            return e & 0xFFFF;
        }
        let bits = (e >> 16) & 0xFF;
        let e2 = self.table[(e & 0xFFFF) as usize + ((v >> ROOT) & ((1 << bits) - 1)) as usize];
        b.skip(ROOT + (e2 >> 16));
        e2 & 0xFFFF
    }

    fn size(&self) -> usize {
        self.table.len()
    }
}

fn reverse(mut c: u32, n: u32) -> u32 {
    let mut r = 0;
    for _ in 0..n {
        r = (r << 1) | (c & 1);
        c >>= 1;
    }
    r
}

/// A prefix code as the stream gives it: simple (one or two symbols, written out) or normal (its code lengths, in
/// turn coded).
fn read_code(b: &mut Bits, alphabet: usize) -> Result<Code, Error> {
    let mut lengths = vec![0u8; alphabet];
    if b.read(1) == 1 {
        let two = b.read(1) == 1;
        let first_bits = if b.read(1) == 1 { 8 } else { 1 };
        let mut symbols = [b.read(first_bits) as usize, usize::MAX];
        if two {
            symbols[1] = b.read(8) as usize;
        }
        // (A symbol past the alphabet counts for nothing, as libwebp has it.)
        for s in symbols {
            if s < alphabet {
                lengths[s] = 1;
            }
        }
        return Code::from_lengths(&lengths).ok_or(Error::Damaged);
    }
    let n = b.read(4) as usize + 4;
    let mut cl = [0u8; 19];
    for &k in CODE_LENGTH_ORDER.iter().take(n) {
        cl[k as usize] = b.read(3) as u8;
    }
    let lc = Code::from_lengths(&cl).ok_or(Error::Damaged)?;
    let mut max_symbol = if b.read(1) == 1 {
        let nbits = 2 + 2 * b.read(3);
        let m = 2 + b.read(nbits) as usize;
        if m > alphabet {
            return Err(Error::Damaged);
        }
        m
    } else {
        alphabet
    };
    let mut prev = 8u8;
    let mut i = 0;
    while i < alphabet {
        if max_symbol == 0 {
            break;
        }
        max_symbol -= 1;
        let c = lc.read(b);
        if c < 16 {
            lengths[i] = c as u8;
            if c != 0 {
                prev = c as u8;
            }
            i += 1;
            continue;
        }
        let (extra, offset, value) = match c {
            16 => (2, 3, prev),
            17 => (3, 3, 0),
            _ => (7, 11, 0),
        };
        let r = offset + b.read(extra) as usize;
        if i + r > alphabet {
            return Err(Error::Damaged);
        }
        lengths[i..i + r].fill(value);
        i += r;
        if b.past_end() {
            return Err(Error::Damaged);
        }
    }
    Code::from_lengths(&lengths).ok_or(Error::Damaged)
}

/// A group's five codes: green (and the lengths of backward references, and colour cache indexes), red, blue,
/// alpha, distance.
struct Group {
    green: Code,
    red: Code,
    blue: Code,
    alpha: Code,
    distance: Code,
}

/// The value a length or distance prefix and its extra bits give.
fn prefix_value(b: &mut Bits, prefix: u32) -> usize {
    if prefix < 4 {
        return prefix as usize + 1;
    }
    let extra = (prefix - 2) >> 1;
    let offset = (2 + (prefix & 1)) << extra;
    (offset + b.read(extra)) as usize + 1
}

fn cache_key(p: u32, bits: u32) -> usize {
    (0x1e35_a7bdu32.wrapping_mul(p) >> (32 - bits)) as usize
}

/// An entropy-coded image `w` × `h` (ARGB): with meta prefix codes when it's the main image (`main`), its colour
/// cache if it has one.
fn entropy_image(b: &mut Bits, w: usize, h: usize, main: bool, most: u64) -> Result<Vec<u32>, Error> {
    if w as u64 * h as u64 > most {
        return Err(Error::TooBig);
    }
    let cache_bits = if b.read(1) == 1 {
        let c = b.read(4);
        if !(1..=11).contains(&c) {
            return Err(Error::Damaged);
        }
        c
    } else {
        0
    };
    // Meta prefix codes: which group of codes each block of the image takes (from its red and green). Only the
    // groups some block takes are kept (the others are read, to get past them).
    let mut meta_bits = 0;
    let mut meta: Vec<u32> = Vec::new();
    let mut meta_w = 0;
    let mut keep: Vec<u32> = vec![0];
    if main && b.read(1) == 1 {
        meta_bits = b.read(3) + 2;
        meta_w = w.div_ceil(1 << meta_bits);
        meta = entropy_image(b, meta_w, h.div_ceil(1 << meta_bits), false, most)?;
        let groups_n = meta.iter().map(|&p| ((p >> 8) & 0xFFFF) as usize).max().unwrap_or(0) + 1;
        keep = vec![u32::MAX; groups_n];
        let mut kept = 0;
        for p in meta.iter_mut() {
            let g = ((*p >> 8) & 0xFFFF) as usize;
            if keep[g] == u32::MAX {
                keep[g] = kept;
                kept += 1;
            }
            *p = keep[g];
        }
    }
    let cache_size = if cache_bits > 0 { 1usize << cache_bits } else { 0 };
    let mut groups: Vec<Option<Group>> = Vec::new();
    groups.resize_with(keep.iter().filter(|&&k| k != u32::MAX).count(), || None);
    // (The tables: no more entries than pixels, or a million for a small picture.)
    let mut table_room = (most as usize).max(1 << 20);
    for &k in &keep {
        let green = read_code(b, 256 + 24 + cache_size)?;
        let red = read_code(b, 256)?;
        let blue = read_code(b, 256)?;
        let alpha = read_code(b, 256)?;
        let distance = read_code(b, 40)?;
        if b.past_end() {
            return Err(Error::Damaged);
        }
        if k != u32::MAX {
            let size = green.size() + red.size() + blue.size() + alpha.size() + distance.size();
            table_room = table_room.checked_sub(size).ok_or(Error::TooBig)?;
            groups[k as usize] = Some(Group { green, red, blue, alpha, distance });
        }
    }
    let groups: Vec<Group> = groups.into_iter().map(|g| g.ok_or(Error::Damaged)).collect::<Result<_, _>>()?;
    let group_at = |x: usize, y: usize| -> usize {
        if meta_bits == 0 {
            0
        } else {
            meta[(y >> meta_bits) * meta_w + (x >> meta_bits)] as usize
        }
    };
    let mask = if meta_bits == 0 { usize::MAX } else { (1 << meta_bits) - 1 };
    let mut cache = vec![0u32; cache_size];
    let n = w * h;
    let mut px = vec![0u32; n];
    let (mut i, mut x, mut y) = (0, 0, 0);
    let mut g = &groups[group_at(0, 0)];
    // (Pixels are put in the colour cache when one is looked up, and after a row or a backward reference.)
    let mut cached = 0;
    while i < n {
        if x & mask == 0 {
            g = &groups[group_at(x, y)];
        }
        let s = g.green.read(b);
        if s < 256 {
            let red = g.red.read(b);
            let blue = g.blue.read(b);
            let alpha = g.alpha.read(b);
            px[i] = alpha << 24 | red << 16 | s << 8 | blue;
        } else if s < 256 + 24 {
            let len = prefix_value(b, s - 256);
            let dsym = g.distance.read(b);
            let code = prefix_value(b, dsym);
            let dist = if code > 120 {
                code - 120
            } else {
                let p = CODE_TO_PLANE[code - 1];
                let d = (p >> 4) as isize * w as isize + 8 - (p & 0xF) as isize;
                d.max(1) as usize
            };
            if dist > i || len > n - i {
                return Err(Error::Damaged);
            }
            for k in i..i + len {
                px[k] = px[k - dist];
            }
            i += len;
            x += len;
            while x >= w {
                x -= w;
                y += 1;
            }
            if i < n && x & mask != 0 {
                g = &groups[group_at(x, y)];
            }
            if cache_size > 0 {
                while cached < i {
                    cache[cache_key(px[cached], cache_bits)] = px[cached];
                    cached += 1;
                }
            }
            if b.past_end() {
                return Err(Error::Damaged);
            }
            continue;
        } else {
            let k = (s - 280) as usize;
            if k >= cache_size {
                return Err(Error::Damaged);
            }
            while cached < i {
                cache[cache_key(px[cached], cache_bits)] = px[cached];
                cached += 1;
            }
            px[i] = cache[k];
        }
        i += 1;
        x += 1;
        if x == w {
            x = 0;
            y += 1;
            if cache_size > 0 {
                while cached < i {
                    cache[cache_key(px[cached], cache_bits)] = px[cached];
                    cached += 1;
                }
            }
            if b.past_end() {
                return Err(Error::Damaged);
            }
        }
    }
    if b.past_end() {
        return Err(Error::Damaged);
    }
    Ok(px)
}

// --------------------------------------------------------------------------------------------- transforms

enum Transform {
    /// Each block's predictor (in its green) for the pixels of an image `width` wide.
    Predictor { bits: u32, modes: Vec<u32>, width: usize },
    /// Each block's colour transform (green to red in its blue, green to blue in its green, red to blue in its red).
    Colour { bits: u32, data: Vec<u32>, width: usize },
    SubtractGreen,
    /// The palette (as many colours as an index can take, those past its own black and clear), and how many
    /// pixels were packed in each: 1 << bits (1, 2, 4 or 8), into an image `width` wide.
    Index { palette: Vec<u32>, bits: u32, width: usize },
}

/// A VP8L chunk's image: its header (signature, size, whether alpha is used, version), then its stream. Its size
/// and its pixels, ARGB.
pub(crate) fn decode(data: &[u8], most: u64) -> Result<(usize, usize, Vec<u32>), Error> {
    let (w, h) = size(data).ok_or(Error::Damaged)?;
    let mut b = Bits::new(&data[5..]);
    let px = decode_stream(&mut b, w as usize, h as usize, most)?;
    Ok((w as usize, h as usize, px))
}

/// The pixels of a stream with no header, of a size given (as the alpha of a lossy WebP has its own).
pub(crate) fn decode_headless(data: &[u8], w: usize, h: usize, most: u64) -> Result<Vec<u32>, Error> {
    decode_stream(&mut Bits::new(data), w, h, most)
}

/// The size a VP8L chunk's header gives.
pub(crate) fn size(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 5 || data[0] != 0x2F {
        return None;
    }
    let v = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
    if v >> 29 != 0 {
        // (A version other than 0.)
        return None;
    }
    Some(((v & 0x3FFF) + 1, ((v >> 14) & 0x3FFF) + 1))
}

fn decode_stream(b: &mut Bits, w: usize, h: usize, most: u64) -> Result<Vec<u32>, Error> {
    if w as u64 * h as u64 > most {
        return Err(Error::TooBig);
    }
    let mut transforms: Vec<Transform> = Vec::new();
    let mut seen = 0u32;
    let mut xsize = w;
    while b.read(1) == 1 {
        let kind = b.read(2);
        if seen & (1 << kind) != 0 {
            return Err(Error::Damaged);
        }
        seen |= 1 << kind;
        match kind {
            0 | 1 => {
                let bits = b.read(3) + 2;
                let data = entropy_image(b, xsize.div_ceil(1 << bits), h.div_ceil(1 << bits), false, most)?;
                transforms.push(if kind == 0 {
                    Transform::Predictor { bits, modes: data, width: xsize }
                } else {
                    Transform::Colour { bits, data, width: xsize }
                });
            }
            2 => transforms.push(Transform::SubtractGreen),
            _ => {
                let n = b.read(8) as usize + 1;
                let colours = entropy_image(b, n, 1, false, most)?;
                let bits = match n {
                    0..=2 => 3,
                    3..=4 => 2,
                    5..=16 => 1,
                    _ => 0,
                };
                // (Each colour stored as its difference from the one before.)
                let mut palette = vec![0u32; 1 << (8 >> bits)];
                palette[0] = colours[0];
                for k in 1..n {
                    palette[k] = add_pixels(colours[k], palette[k - 1]);
                }
                transforms.push(Transform::Index { palette, bits, width: xsize });
                xsize = xsize.div_ceil(1 << bits);
            }
        }
        if b.past_end() {
            return Err(Error::Damaged);
        }
    }
    let mut px = entropy_image(b, xsize, h, true, most)?;
    for t in transforms.iter().rev() {
        match t {
            Transform::Predictor { bits, modes, width } => unpredict(&mut px, *width, h, *bits, modes),
            Transform::Colour { bits, data, width } => uncolour(&mut px, *width, *bits, data),
            Transform::SubtractGreen => {
                for p in px.iter_mut() {
                    let g = (*p >> 8) & 0xFF;
                    let rb = ((*p & 0x00FF_00FF) + (g << 16 | g)) & 0x00FF_00FF;
                    *p = (*p & 0xFF00_FF00) | rb;
                }
            }
            Transform::Index { palette, bits, width } => px = unindex(&px, *width, h, *bits, palette),
        }
    }
    Ok(px)
}

/// Two pixels added, each channel modulo 256.
fn add_pixels(a: u32, b: u32) -> u32 {
    let ag = (a & 0xFF00_FF00).wrapping_add(b & 0xFF00_FF00) & 0xFF00_FF00;
    let rb = (a & 0x00FF_00FF).wrapping_add(b & 0x00FF_00FF) & 0x00FF_00FF;
    ag | rb
}

/// Each channel's mean, rounded down.
fn average2(a: u32, b: u32) -> u32 {
    (((a ^ b) & 0xFEFE_FEFE) >> 1) + (a & b)
}

fn channels(p: u32) -> [i32; 4] {
    [(p >> 24) as i32, ((p >> 16) & 0xFF) as i32, ((p >> 8) & 0xFF) as i32, (p & 0xFF) as i32]
}

fn from_channels(c: [i32; 4]) -> u32 {
    (c[0].clamp(0, 255) as u32) << 24
        | (c[1].clamp(0, 255) as u32) << 16
        | (c[2].clamp(0, 255) as u32) << 8
        | c[3].clamp(0, 255) as u32
}

fn predict(mode: u32, l: u32, t: u32, tl: u32, tr: u32) -> u32 {
    match mode {
        1 => l,
        2 => t,
        3 => tr,
        4 => tl,
        5 => average2(average2(l, tr), t),
        6 => average2(l, tl),
        7 => average2(l, t),
        8 => average2(tl, t),
        9 => average2(t, tr),
        10 => average2(average2(l, tl), average2(t, tr)),
        11 => {
            // Left or top, whichever is nearer (in the sum of each channel's distance) to left + top - top-left.
            let (cl, ct, ctl) = (channels(l), channels(t), channels(tl));
            let mut to_t = 0;
            let mut to_l = 0;
            for k in 0..4 {
                to_t += (cl[k] - ctl[k]).abs();
                to_l += (ct[k] - ctl[k]).abs();
            }
            if to_l < to_t { l } else { t }
        }
        12 => {
            let (cl, ct, ctl) = (channels(l), channels(t), channels(tl));
            from_channels(core::array::from_fn(|k| cl[k] + ct[k] - ctl[k]))
        }
        13 => {
            let (ca, ctl) = (channels(average2(l, t)), channels(tl));
            from_channels(core::array::from_fn(|k| ca[k] + (ca[k] - ctl[k]) / 2))
        }
        // (0, and 14 and 15, which libwebp takes as 0.)
        _ => 0xFF00_0000,
    }
}

fn unpredict(px: &mut [u32], w: usize, h: usize, bits: u32, modes: &[u32]) {
    let bw = w.div_ceil(1 << bits);
    // The first row: from black, then from the left.
    px[0] = add_pixels(px[0], 0xFF00_0000);
    for x in 1..w {
        px[x] = add_pixels(px[x], px[x - 1]);
    }
    for y in 1..h {
        let row = y * w;
        // The first column: from the top.
        px[row] = add_pixels(px[row], px[row - w]);
        let mrow = &modes[(y >> bits) * bw..];
        for x in 1..w {
            let mode = (mrow[x >> bits] >> 8) & 0xF;
            let i = row + x;
            // (The pixel above to the right, for the last column: the first of this row.)
            let p = predict(mode, px[i - 1], px[i - w], px[i - w - 1], px[i - w + 1]);
            px[i] = add_pixels(px[i], p);
        }
    }
}

fn uncolour(px: &mut [u32], w: usize, bits: u32, data: &[u32]) {
    let bw = w.div_ceil(1 << bits);
    for (i, p) in px.iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        let m = data[(y >> bits) * bw + (x >> bits)];
        let (g2r, g2b, r2b) = (m as u8 as i8 as i32, (m >> 8) as u8 as i8 as i32, (m >> 16) as u8 as i8 as i32);
        let argb = *p;
        let green = (argb >> 8) as u8 as i8 as i32;
        let red = (((argb >> 16) & 0xFF) as i32 + ((g2r * green) >> 5)) & 0xFF;
        let blue = ((argb & 0xFF) as i32 + ((g2b * green) >> 5) + ((r2b * (red as u8 as i8 as i32)) >> 5)) & 0xFF;
        *p = (argb & 0xFF00_FF00) | (red as u32) << 16 | blue as u32;
    }
}

fn unindex(px: &[u32], w: usize, h: usize, bits: u32, palette: &[u32]) -> Vec<u32> {
    let mut out = vec![0u32; w * h];
    if bits == 0 {
        for (o, p) in out.iter_mut().zip(px) {
            *o = palette[((p >> 8) & 0xFF) as usize];
        }
        return out;
    }
    let in_w = w.div_ceil(1 << bits);
    let per = 8 >> bits;
    let mask = (1u32 << per) - 1;
    for y in 0..h {
        for x in 0..w {
            let packed = (px[y * in_w + (x >> bits)] >> 8) & 0xFF;
            let idx = (packed >> ((x & ((1 << bits) - 1)) as u32 * per)) & mask;
            out[y * w + x] = palette[idx as usize];
        }
    }
    out
}
