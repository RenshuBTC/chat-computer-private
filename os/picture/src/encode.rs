//! JPEG, written: baseline, YCbCr with its colour at half the resolution each way (4:2:0, as cameras save photos),
//! the standard's example Huffman tables (ITU T.81, annex K) and its quantization tables scaled by quality as libjpeg
//! scales them, libjpeg's accurate integer forward DCT. For a photo too big to send to Claude as it is: decoded smaller,
//! then written again, a few hundred kilobytes.

use crate::Picture;
use alloc::vec::Vec;

/// Annex K's quantization tables, in natural order: luminance's and chrominance's.
const LUMA_Q: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56, 14, 17, 22, 29, 51, 87,
    80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92,
    95, 98, 112, 100, 103, 99,
];
const CHROMA_Q: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99, 47, 66, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99,
];

/// Annex K's Huffman tables: how many codes of each length, then the symbols.
const DC_LUMA: ([u8; 16], &[u8]) = ([0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0], &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
const DC_CHROMA: ([u8; 16], &[u8]) = ([0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0], &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
const AC_LUMA: ([u8; 16], &[u8]) = (
    [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d],
    &[
        0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32,
        0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16,
        0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45,
        0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
        0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94,
        0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6,
        0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8,
        0xd9, 0xda, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
        0xf9, 0xfa,
    ],
);
const AC_CHROMA: ([u8; 16], &[u8]) = (
    [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77],
    &[
        0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22, 0x32, 0x81,
        0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0, 0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34,
        0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44,
        0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
        0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92,
        0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4,
        0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6,
        0xd7, 0xd8, 0xd9, 0xda, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
        0xf9, 0xfa,
    ],
);

/// Where the zigzag order's coefficients come from in a block.
const NATURAL: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55,
    62, 63,
];

/// A Huffman table's codes, by symbol: the code and its length.
struct Codes([(u16, u8); 256]);

impl Codes {
    fn new(counts: &[u8; 16], symbols: &[u8]) -> Codes {
        let mut out = [(0u16, 0u8); 256];
        let (mut code, mut k) = (0u16, 0usize);
        for (len, &n) in counts.iter().enumerate() {
            for _ in 0..n {
                out[symbols[k] as usize] = (code, len as u8 + 1);
                code += 1;
                k += 1;
            }
            code <<= 1;
        }
        Codes(out)
    }
}

/// The entropy-coded data being written: bits first bit first, a 0xFF followed by a 0x00.
struct Writer {
    out: Vec<u8>,
    buf: u32,
    count: u32,
}

impl Writer {
    fn put(&mut self, value: u32, bits: u32) {
        if bits == 0 {
            return;
        }
        self.buf = self.buf << bits | (value & ((1 << bits) - 1));
        self.count += bits;
        while self.count >= 8 {
            let byte = (self.buf >> (self.count - 8)) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0);
            }
            self.count -= 8;
        }
        self.buf &= (1 << self.count) - 1;
    }

    /// The last byte filled with ones.
    fn flush(&mut self) {
        if self.count > 0 {
            let pad = 8 - self.count;
            self.put((1 << pad) - 1, pad);
        }
    }
}

/// The quality's quantization table (as libjpeg scales annex K's), in natural order.
fn quantization(base: &[u16; 64], quality: u8) -> [u16; 64] {
    let q = quality.clamp(1, 100) as u32;
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    core::array::from_fn(|i| ((base[i] as u32 * scale + 50) / 100).clamp(1, 255) as u16)
}

const CONST_BITS: i32 = 13;
const PASS1_BITS: i32 = 2;

fn descale(x: i32, n: i32) -> i32 {
    (x + (1 << (n - 1))) >> n
}

/// libjpeg's accurate integer forward DCT (jfdctint.c), on 8 values; `rows` for the first pass.
fn fdct_1d(d: [i32; 8], first: bool) -> [i32; 8] {
    let (tmp0, tmp7, tmp1, tmp6) = (d[0] + d[7], d[0] - d[7], d[1] + d[6], d[1] - d[6]);
    let (tmp2, tmp5, tmp3, tmp4) = (d[2] + d[5], d[2] - d[5], d[3] + d[4], d[3] - d[4]);
    let (tmp10, tmp13, tmp11, tmp12) = (tmp0 + tmp3, tmp0 - tmp3, tmp1 + tmp2, tmp1 - tmp2);
    let shift = if first { CONST_BITS - PASS1_BITS } else { CONST_BITS + PASS1_BITS };
    let mut out = [0i32; 8];
    if first {
        out[0] = (tmp10 + tmp11) << PASS1_BITS;
        out[4] = (tmp10 - tmp11) << PASS1_BITS;
    } else {
        out[0] = descale(tmp10 + tmp11, PASS1_BITS);
        out[4] = descale(tmp10 - tmp11, PASS1_BITS);
    }
    let z1 = (tmp12 + tmp13) * 4433;
    out[2] = descale(z1 + tmp13 * 6270, shift);
    out[6] = descale(z1 + tmp12 * -15137, shift);
    let (mut z1, mut z2, mut z3, mut z4) = (tmp4 + tmp7, tmp5 + tmp6, tmp4 + tmp6, tmp5 + tmp7);
    let z5 = (z3 + z4) * 9633;
    let (t4, t5, t6, t7) = (tmp4 * 2446, tmp5 * 16819, tmp6 * 25172, tmp7 * 12299);
    z1 *= -7373;
    z2 *= -20995;
    z3 *= -16069;
    z4 *= -3196;
    z3 += z5;
    z4 += z5;
    out[7] = descale(t4 + z1 + z3, shift);
    out[5] = descale(t5 + z2 + z4, shift);
    out[3] = descale(t6 + z2 + z3, shift);
    out[1] = descale(t7 + z1 + z4, shift);
    out
}

/// A block of samples (0 to 255) as quantized coefficients, in natural order.
fn block_coefficients(samples: &[u8; 64], q: &[u16; 64]) -> [i32; 64] {
    let mut d: [i32; 64] = core::array::from_fn(|i| samples[i] as i32 - 128);
    for r in 0..8 {
        let row: [i32; 8] = core::array::from_fn(|c| d[r * 8 + c]);
        let out = fdct_1d(row, true);
        d[r * 8..r * 8 + 8].copy_from_slice(&out);
    }
    for c in 0..8 {
        let col: [i32; 8] = core::array::from_fn(|r| d[r * 8 + c]);
        let out = fdct_1d(col, false);
        for r in 0..8 {
            d[r * 8 + c] = out[r];
        }
    }
    // (The DCT comes out 8 times too big: divided by 8 × the quantizer, rounded.)
    core::array::from_fn(|i| {
        let q8 = q[i] as i32 * 8;
        if d[i] < 0 { -((-d[i] + q8 / 2) / q8) } else { (d[i] + q8 / 2) / q8 }
    })
}

/// How many bits a coefficient's value takes, and the bits (a negative one as JPEG writes it: less one).
fn magnitude(v: i32) -> (u32, u32) {
    let bits = 32 - v.unsigned_abs().leading_zeros();
    let code = if v < 0 { (v - 1) as u32 & ((1 << bits) - 1) } else { v as u32 };
    (bits, code)
}

fn encode_block(w: &mut Writer, coefficients: &[i32; 64], previous_dc: &mut i32, dc: &Codes, ac: &Codes) {
    let diff = coefficients[0] - *previous_dc;
    *previous_dc = coefficients[0];
    let (bits, code) = magnitude(diff);
    let (c, l) = dc.0[bits as usize];
    w.put(c as u32, l as u32);
    w.put(code, bits);
    let mut run = 0;
    for k in 1..64 {
        let v = coefficients[NATURAL[k]];
        if v == 0 {
            run += 1;
            continue;
        }
        while run > 15 {
            let (c, l) = ac.0[0xF0];
            w.put(c as u32, l as u32);
            run -= 16;
        }
        let (bits, code) = magnitude(v);
        let (c, l) = ac.0[(run << 4 | bits) as usize];
        w.put(c as u32, l as u32);
        w.put(code, bits);
        run = 0;
    }
    if run > 0 {
        let (c, l) = ac.0[0x00];
        w.put(c as u32, l as u32);
    }
}

/// The picture's colour as JPEG keeps it: Y at full resolution, Cb and Cr at half each way, each as a plane of whole
/// blocks (8 × 8 samples), its edges filled out as libjpeg fills them: the full-size planes' right edge repeated before
/// the colour is halved (each 2 × 2 averaged, rounded alternately down and up as libjpeg does), the halved planes'
/// bottom row repeated after; and with libjpeg's arithmetic for the colour itself (jccolor.c, jcsample.c), so the
/// same picture comes out as the same JPEG.
struct Planes {
    y: Vec<u8>,
    cb: Vec<u8>,
    cr: Vec<u8>,
    /// Y's blocks across and down, and Cb's and Cr's.
    y_blocks: (usize, usize),
    c_blocks: (usize, usize),
}

fn planes(p: &Picture) -> Planes {
    let (w, h) = (p.width as usize, p.height as usize);
    let (yw, yh) = (w.div_ceil(8), h.div_ceil(8));
    let (cw, ch) = (w.div_ceil(2).div_ceil(8), h.div_ceil(2).div_ceil(8));
    // Full size, its right edge repeated as far as the halved planes' blocks need.
    let fw = (yw * 8).max(cw * 16);
    let mut full = [Vec::new(), Vec::new(), Vec::new()];
    for plane in &mut full {
        *plane = alloc::vec![0u8; fw * h];
    }
    for y in 0..h {
        for x in 0..fw {
            let i = (y * w + x.min(w - 1)) * 3;
            let (r, g, b) = (p.rgb[i] as i32, p.rgb[i + 1] as i32, p.rgb[i + 2] as i32);
            full[0][y * fw + x] = ((19_595 * r + 38_470 * g + 7_471 * b + 32_768) >> 16) as u8;
            full[1][y * fw + x] = ((-11_059 * r - 21_709 * g + 32_768 * b + (128 << 16) + 32_767) >> 16) as u8;
            full[2][y * fw + x] = ((32_768 * r - 27_439 * g - 5_329 * b + (128 << 16) + 32_767) >> 16) as u8;
        }
    }
    let mut y_plane = alloc::vec![0u8; yw * 8 * yh * 8];
    for y in 0..yh * 8 {
        let from = y.min(h - 1) * fw;
        y_plane[y * yw * 8..(y + 1) * yw * 8].copy_from_slice(&full[0][from..from + yw * 8]);
    }
    let last = h.div_ceil(2) - 1;
    let halved = |plane: &Vec<u8>| -> Vec<u8> {
        let mut out = alloc::vec![0u8; cw * 8 * ch * 8];
        for y in 0..ch * 8 {
            let row = y.min(last);
            let (a, b) = (2 * row * fw, (2 * row + 1).min(h - 1) * fw);
            for x in 0..cw * 8 {
                let sum = plane[a + 2 * x] as u32 + plane[a + 2 * x + 1] as u32 + plane[b + 2 * x] as u32 + plane[b + 2 * x + 1] as u32;
                out[y * cw * 8 + x] = ((sum + 1 + (x as u32 & 1)) >> 2) as u8;
            }
        }
        out
    };
    Planes { cb: halved(&full[1]), cr: halved(&full[2]), y: y_plane, y_blocks: (yw, yh), c_blocks: (cw, ch) }
}

/// A plane's block: its 64 samples.
fn block(plane: &[u8], across: usize, bx: usize, by: usize) -> [u8; 64] {
    core::array::from_fn(|i| plane[(by * 8 + i / 8) * across * 8 + bx * 8 + i % 8])
}

/// The picture as a baseline JPEG of this quality (1 to 100), as libjpeg writes it with 4:2:0 colour, its standard
/// tables and its accurate integer DCT (as Pillow saves one, by default).
pub fn jpeg(p: &Picture, quality: u8) -> Vec<u8> {
    let (w, h) = (p.width as usize, p.height as usize);
    let (lq, cq) = (quantization(&LUMA_Q, quality), quantization(&CHROMA_Q, quality));
    let mut out = Vec::new();
    out.extend_from_slice(&[0xFF, 0xD8]);
    // JFIF: version 1.01, no density.
    out.extend_from_slice(&[0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0]);
    for (id, table) in [(0u8, &lq), (1, &cq)] {
        out.extend_from_slice(&[0xFF, 0xDB, 0, 67, id]);
        out.extend(NATURAL.iter().map(|&n| table[n] as u8));
    }
    // The frame: 8-bit, its size, three components (Y at 2 × 2, Cb and Cr at 1 × 1).
    out.extend_from_slice(&[0xFF, 0xC0, 0, 17, 8]);
    out.extend_from_slice(&(h as u16).to_be_bytes());
    out.extend_from_slice(&(w as u16).to_be_bytes());
    out.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    for (class_id, (counts, symbols)) in [(0x00u8, DC_LUMA), (0x10, AC_LUMA), (0x01, DC_CHROMA), (0x11, AC_CHROMA)] {
        let len = 2 + 1 + 16 + symbols.len();
        out.extend_from_slice(&[0xFF, 0xC4]);
        out.extend_from_slice(&(len as u16).to_be_bytes());
        out.push(class_id);
        out.extend_from_slice(&counts);
        out.extend_from_slice(symbols);
    }
    out.extend_from_slice(&[0xFF, 0xDA, 0, 12, 3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);
    let (dcl, acl, dcc, acc) = (Codes::new(&DC_LUMA.0, DC_LUMA.1), Codes::new(&AC_LUMA.0, AC_LUMA.1), Codes::new(&DC_CHROMA.0, DC_CHROMA.1), Codes::new(&AC_CHROMA.0, AC_CHROMA.1));
    let mut writer = Writer { out, buf: 0, count: 0 };
    let mut previous = [0i32; 3];
    let pl = planes(p);
    let ((yw, yh), (cw, _)) = (pl.y_blocks, pl.c_blocks);
    // Each MCU (16 × 16 pixels): Y's four blocks, then Cb's and Cr's. Past Y's last block across or down, a block
    // is a dummy, as libjpeg makes one (jccoefct.c): nothing but the DC of the block before it in the MCU.
    for my in 0..h.div_ceil(16) {
        for mx in 0..w.div_ceil(16) {
            let mut before = 0;
            for row in 0..2 {
                for col in 0..2 {
                    let (bx, by) = (2 * mx + col, 2 * my + row);
                    let coefficients = if bx < yw && by < yh {
                        block_coefficients(&block(&pl.y, yw, bx, by), &lq)
                    } else {
                        let mut dummy = [0i32; 64];
                        dummy[0] = before;
                        dummy
                    };
                    before = coefficients[0];
                    encode_block(&mut writer, &coefficients, &mut previous[0], &dcl, &acl);
                }
            }
            encode_block(&mut writer, &block_coefficients(&block(&pl.cb, cw, mx, my), &cq), &mut previous[1], &dcc, &acc);
            encode_block(&mut writer, &block_coefficients(&block(&pl.cr, cw, mx, my), &cq), &mut previous[2], &dcc, &acc);
        }
    }
    writer.flush();
    let mut out = writer.out;
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}
