//! How weights are stored, and the arithmetic on them. A weight matrix's rows are blocks of 32 numbers:
//!
//! - Q8: a scale (f16) and 32 signed bytes: 34 bytes for 32 weights (8.5 bits each), nearly exact;
//! - Q4: a scale (f16) and 32 signed 4-bit numbers (-8 to 7), the first 16 in the low halves of 16 bytes and the
//!   last 16 in the high halves: 18 bytes for 32 weights (4.5 bits each). Each block's scale is the one (of a
//!   few tried) that keeps its weights closest to what they were;
//! - F32 for the small things (norms, the linear attention's constants) and BF16 as published.
//!
//! A row times a vector: the vector is put in Q8 blocks too, and each block's sum of products is of whole numbers,
//! scaled once: on a processor with AVX2, 32 at a time.

use crate::math;
use alloc::vec;
use alloc::vec::Vec;

pub const BLOCK: usize = 32;
pub const Q8_BYTES: usize = 2 + BLOCK;
pub const Q4_BYTES: usize = 2 + BLOCK / 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dtype {
    F32,
    BF16,
    F16,
    Q8,
    Q4,
    /// Bytes of something else (the settings, the tokenizer).
    Blob,
}

impl Dtype {
    pub fn name(self) -> &'static str {
        match self {
            Dtype::F32 => "f32",
            Dtype::BF16 => "bf16",
            Dtype::F16 => "f16",
            Dtype::Q8 => "q8",
            Dtype::Q4 => "q4",
            Dtype::Blob => "blob",
        }
    }

    pub fn from_name(s: &str) -> Option<Dtype> {
        Some(match s {
            "f32" => Dtype::F32,
            "bf16" => Dtype::BF16,
            "f16" => Dtype::F16,
            "q8" => Dtype::Q8,
            "q4" => Dtype::Q4,
            "blob" => Dtype::Blob,
            _ => return None,
        })
    }

    /// Bytes for `n` numbers (None: not a whole number of blocks).
    pub fn bytes_for(self, n: usize) -> Option<usize> {
        match self {
            Dtype::F32 => n.checked_mul(4),
            Dtype::BF16 | Dtype::F16 => n.checked_mul(2),
            Dtype::Q8 => n.is_multiple_of(BLOCK).then(|| n / BLOCK * Q8_BYTES),
            Dtype::Q4 => n.is_multiple_of(BLOCK).then(|| n / BLOCK * Q4_BYTES),
            Dtype::Blob => Some(n),
        }
    }
}

pub fn bf16_to_f32(b: u16) -> f32 {
    f32::from_bits((b as u32) << 16)
}

pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) as u32) << 31;
    let exp = ((h >> 10) & 0x1f) as u32;
    let frac = (h & 0x3ff) as u32;
    let bits = match exp {
        0 if frac == 0 => sign,
        0 => {
            // Subnormal: normalise.
            let mut e = 127 - 15 + 1;
            let mut f = frac;
            while f & 0x400 == 0 {
                f <<= 1;
                e -= 1;
            }
            sign | (e as u32) << 23 | (f & 0x3ff) << 13
        }
        31 => sign | 0xff << 23 | frac << 13,
        _ => sign | (exp + 127 - 15) << 23 | frac << 13,
    };
    f32::from_bits(bits)
}

pub fn f32_to_f16(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let frac = bits & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if frac != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        // Subnormal, rounded to nearest even.
        let m = frac | 0x80_0000;
        let shift = (14 - e) as u32;
        let mut v = m >> shift;
        let rem = m & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        if rem > half || (rem == half && v & 1 == 1) {
            v += 1;
        }
        return sign | v as u16;
    }
    let mut v = ((e as u32) << 10) | (frac >> 13);
    let rem = frac & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && v & 1 == 1) {
        v += 1; // (may carry into the exponent: still right, up to infinity)
    }
    sign | v as u16
}

/// Numbers into Q8 blocks (their count a multiple of 32).
pub fn quantize_q8(x: &[f32], out: &mut Vec<u8>) {
    for b in x.chunks_exact(BLOCK) {
        let max = b.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let d = max / 127.0;
        let h = f32_to_f16(d);
        let d = f16_to_f32(h);
        let inv = if d > 0.0 { 1.0 / d } else { 0.0 };
        out.extend_from_slice(&h.to_le_bytes());
        for &v in b {
            out.push(round(v * inv).clamp(-127.0, 127.0) as i8 as u8);
        }
    }
}

/// Numbers into Q4 blocks: each block's scale chosen, of a few around its largest weight over 8, to keep its
/// weights' squared error least.
pub fn quantize_q4(x: &[f32], out: &mut Vec<u8>) {
    for b in x.chunks_exact(BLOCK) {
        // The weight with the largest size, signed: it's the one that gets -8 (the code that has no +8).
        let max = b.iter().fold(0.0f32, |m, &v| if v.abs() > m.abs() { v } else { m });
        let mut best = (f32::INFINITY, 0.0f32);
        if max == 0.0 {
            best.1 = 0.0;
        } else {
            for k in 0..=12 {
                // d from max/-8 (the largest exactly representable), coarser in steps (fewer weights rounded off).
                let d = max / -(8.0 - 0.1 * k as f32);
                let dh = f16_to_f32(f32_to_f16(d));
                if dh == 0.0 {
                    continue;
                }
                let inv = 1.0 / dh;
                let err: f32 = b.iter().map(|&v| {
                    let q = round(v * inv).clamp(-8.0, 7.0);
                    let e = v - q * dh;
                    e * e
                }).sum();
                if err < best.0 {
                    best = (err, dh);
                }
            }
        }
        let d = best.1;
        let inv = if d != 0.0 { 1.0 / d } else { 0.0 };
        out.extend_from_slice(&f32_to_f16(d).to_le_bytes());
        let q: Vec<u8> = b.iter().map(|&v| (round(v * inv).clamp(-8.0, 7.0) as i32 + 8) as u8).collect();
        for j in 0..BLOCK / 2 {
            out.push(q[j] | q[j + BLOCK / 2] << 4);
        }
    }
}

#[inline]
fn round(x: f32) -> f32 {
    // Half away from zero, as C's roundf.
    if x >= 0.0 { (x + 0.5) as i32 as f32 } else { (x - 0.5) as i32 as f32 }
}

/// Row `r` (of `cols` numbers) of a matrix of `dtype`, as f32s.
pub fn row_f32(dtype: Dtype, data: &[u8], cols: usize, r: usize, out: &mut [f32]) {
    let bytes = dtype.bytes_for(cols).unwrap_or(0);
    let row = &data[r * bytes..(r + 1) * bytes];
    match dtype {
        Dtype::F32 => {
            for (o, c) in out.iter_mut().zip(row.chunks_exact(4)) {
                *o = f32::from_le_bytes(c.try_into().unwrap());
            }
        }
        Dtype::BF16 => {
            for (o, c) in out.iter_mut().zip(row.chunks_exact(2)) {
                *o = bf16_to_f32(u16::from_le_bytes([c[0], c[1]]));
            }
        }
        Dtype::F16 => {
            for (o, c) in out.iter_mut().zip(row.chunks_exact(2)) {
                *o = f16_to_f32(u16::from_le_bytes([c[0], c[1]]));
            }
        }
        Dtype::Q8 => {
            for (o, blk) in out.chunks_exact_mut(BLOCK).zip(row.chunks_exact(Q8_BYTES)) {
                let d = f16_to_f32(u16::from_le_bytes([blk[0], blk[1]]));
                for (v, &q) in o.iter_mut().zip(&blk[2..]) {
                    *v = d * q as i8 as f32;
                }
            }
        }
        Dtype::Q4 => {
            for (o, blk) in out.chunks_exact_mut(BLOCK).zip(row.chunks_exact(Q4_BYTES)) {
                let d = f16_to_f32(u16::from_le_bytes([blk[0], blk[1]]));
                for j in 0..BLOCK / 2 {
                    o[j] = d * ((blk[2 + j] & 15) as i32 - 8) as f32;
                    o[j + BLOCK / 2] = d * ((blk[2 + j] >> 4) as i32 - 8) as f32;
                }
            }
        }
        Dtype::Blob => {}
    }
}

/// A vector made ready to be multiplied by rows of any type: as f32s, and (for Q8 and Q4 rows) in Q8 blocks.
pub struct Prepared {
    pub f: Vec<f32>,
    pub q: Vec<u8>,
    /// Each Q8 block's scale, as f32.
    pub scales: Vec<f32>,
    /// With VNNI: each Q8 block's 32 numbers plus 128 (unsigned bytes, which VNNI multiplies by signed ones).
    pub qu: Vec<u8>,
}

impl Prepared {
    pub fn new(x: &[f32]) -> Prepared {
        let mut q = Vec::with_capacity(x.len() / BLOCK * Q8_BYTES);
        if x.len().is_multiple_of(BLOCK) {
            quantize_q8(x, &mut q);
        }
        let scales = q.chunks_exact(Q8_BYTES).map(|b| f16_to_f32(u16::from_le_bytes([b[0], b[1]]))).collect();
        let qu = if crate::cpu::vnni() {
            q.chunks_exact(Q8_BYTES).flat_map(|b| b[2..].iter().map(|v| v ^ 0x80)).collect()
        } else {
            Vec::new()
        };
        Prepared { f: x.to_vec(), q, scales, qu }
    }
}
#[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
/// Row `r` of a `dtype` matrix times each of several prepared vectors (a token's each): out[t] for xs[t]. Four
/// at a time where the processor can, each block of the row read once for the four.
pub fn dot_row_many(dtype: Dtype, data: &[u8], cols: usize, r: usize, xs: &[&Prepared], out: &mut [f32]) {
    dot_row_many_at(crate::cpu::level(), dtype, data, cols, r, xs, out);
}

/// The same with the instructions of `level` (at most what the processor has), for the kernels to be compared.
pub fn dot_row_many_at(level: crate::cpu::Level, dtype: Dtype, data: &[u8], cols: usize, r: usize, xs: &[&Prepared], out: &mut [f32]) {
    let level = level.min(crate::cpu::level());
    let n = xs.len();
    #[allow(unused_mut)]
    let mut t = 0;
    #[cfg(target_arch = "x86_64")]
    if matches!(dtype, Dtype::Q8 | Dtype::Q4) && level >= crate::cpu::Level::Avx2 && cols.is_multiple_of(BLOCK) {
        let q4 = dtype == Dtype::Q4;
        let bytes = if q4 { Q4_BYTES } else { Q8_BYTES };
        let row = &data[r * cols / BLOCK * bytes..(r + 1) * cols / BLOCK * bytes];
        while t + 4 <= n {
            let four = [xs[t], xs[t + 1], xs[t + 2], xs[t + 3]];
            if four.iter().any(|x| x.q.len() != cols / BLOCK * Q8_BYTES || (level >= crate::cpu::Level::AvxVnni && x.qu.len() != cols)) {
                break;
            }
            let res = unsafe {
                match (level, q4) {
                    (crate::cpu::Level::AvxVnni, false) => simd::dot4_avxvnni::<false>(row, four),
                    (crate::cpu::Level::AvxVnni, true) => simd::dot4_avxvnni::<true>(row, four),
                    (crate::cpu::Level::Avx512Vnni, false) => simd::dot4_avx512vnni::<false>(row, four),
                    (crate::cpu::Level::Avx512Vnni, true) => simd::dot4_avx512vnni::<true>(row, four),
                    (_, false) => simd::dot4_avx2::<false>(row, four),
                    (_, true) => simd::dot4_avx2::<true>(row, four),
                }
            };
            out[t..t + 4].copy_from_slice(&res);
            t += 4;
        }
    }
    for (o, x) in out[t..n].iter_mut().zip(&xs[t..]) {
        *o = if level >= crate::cpu::Level::Avx2 { dot_row(dtype, data, cols, r, x) } else { dot_row_plain(dtype, data, cols, r, x) };
    }
}

/// A row times a vector without AVX2 (for comparing; the plain arithmetic dot_row uses where there's no AVX2).
pub fn dot_row_plain(dtype: Dtype, data: &[u8], cols: usize, r: usize, x: &Prepared) -> f32 {
    match dtype {
        Dtype::Q8 => {
            let row = &data[r * cols / BLOCK * Q8_BYTES..(r + 1) * cols / BLOCK * Q8_BYTES];
            dot_q8_plain(row, &x.q, &x.scales)
        }
        Dtype::Q4 => {
            let row = &data[r * cols / BLOCK * Q4_BYTES..(r + 1) * cols / BLOCK * Q4_BYTES];
            dot_q4_plain(row, &x.q, &x.scales)
        }
        _ => dot_row(dtype, data, cols, r, x),
    }
}

/// Row `r` of a `dtype` matrix with `cols` columns, times the prepared vector.
#[inline]
pub fn dot_row(dtype: Dtype, data: &[u8], cols: usize, r: usize, x: &Prepared) -> f32 {
    match dtype {
        Dtype::Q8 => {
            let row = &data[r * cols / BLOCK * Q8_BYTES..(r + 1) * cols / BLOCK * Q8_BYTES];
            dot_q8(row, &x.q, &x.scales)
        }
        Dtype::Q4 => {
            let row = &data[r * cols / BLOCK * Q4_BYTES..(r + 1) * cols / BLOCK * Q4_BYTES];
            dot_q4(row, &x.q, &x.scales)
        }
        Dtype::F32 => {
            let row = &data[r * cols * 4..(r + 1) * cols * 4];
            row.chunks_exact(4).zip(&x.f).map(|(c, v)| f32::from_le_bytes(c.try_into().unwrap()) * v).sum()
        }
        Dtype::BF16 => {
            let row = &data[r * cols * 2..(r + 1) * cols * 2];
            let mut s = 0.0f32;
            for (c, v) in row.chunks_exact(2).zip(&x.f) {
                s += bf16_to_f32(u16::from_le_bytes([c[0], c[1]])) * v;
            }
            s
        }
        Dtype::F16 => {
            let row = &data[r * cols * 2..(r + 1) * cols * 2];
            let mut s = 0.0f32;
            for (c, v) in row.chunks_exact(2).zip(&x.f) {
                s += f16_to_f32(u16::from_le_bytes([c[0], c[1]])) * v;
            }
            s
        }
        Dtype::Blob => 0.0,
    }
}

/// A Q8 row times a Q8 vector.
fn dot_q8(row: &[u8], x: &[u8], xs: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    if crate::cpu::avx2() {
        return unsafe { simd::dot_q8_avx2(row, x, xs) };
    }
    dot_q8_plain(row, x, xs)
}
#[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
fn dot_q8_plain(row: &[u8], x: &[u8], xs: &[f32]) -> f32 {
    let mut sum = 0.0f32;
    for ((w, v), &dx) in row.chunks_exact(Q8_BYTES).zip(x.chunks_exact(Q8_BYTES)).zip(xs) {
        let dw = f16_to_f32(u16::from_le_bytes([w[0], w[1]]));
        let mut s = 0i32;
        for j in 0..BLOCK {
            s += w[2 + j] as i8 as i32 * v[2 + j] as i8 as i32;
        }
        sum += dw * dx * s as f32;
    }
    sum
}

/// A Q4 row times a Q8 vector.
fn dot_q4(row: &[u8], x: &[u8], xs: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    if crate::cpu::avx2() {
        return unsafe { simd::dot_q4_avx2(row, x, xs) };
    }
    dot_q4_plain(row, x, xs)
}
#[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
fn dot_q4_plain(row: &[u8], x: &[u8], xs: &[f32]) -> f32 {
    let mut sum = 0.0f32;
    for ((w, v), &dx) in row.chunks_exact(Q4_BYTES).zip(x.chunks_exact(Q8_BYTES)).zip(xs) {
        let dw = f16_to_f32(u16::from_le_bytes([w[0], w[1]]));
        let mut s = 0i32;
        for j in 0..BLOCK / 2 {
            s += ((w[2 + j] & 15) as i32 - 8) * v[2 + j] as i8 as i32;
            s += ((w[2 + j] >> 4) as i32 - 8) * v[2 + j + BLOCK / 2] as i8 as i32;
        }
        sum += dw * dx * s as f32;
    }
    sum
}

#[cfg(target_arch = "x86_64")]
mod simd {
    use super::*;
    use core::arch::x86_64::*;

    /// The sum of a block's 32 products of signed bytes, as 8 sums of 4 (in 32-bit lanes).
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn products(a: __m256i, b: __m256i) -> __m256i {
        // maddubs multiplies unsigned by signed bytes: |a| times b with a's sign.
        let ax = _mm256_sign_epi8(a, a);
        let sy = _mm256_sign_epi8(b, a);
        let dot = _mm256_maddubs_epi16(ax, sy);
        _mm256_madd_epi16(dot, _mm256_set1_epi16(1))
    }

    #[inline]
    #[target_feature(enable = "avx2,fma")]
    unsafe fn sum8(v: __m256) -> f32 {
        let hi = _mm256_extractf128_ps(v, 1);
        let lo = _mm256_castps256_ps128(v);
        let s = _mm_add_ps(hi, lo);
        let s = _mm_add_ps(s, _mm_movehl_ps(s, s));
        let s = _mm_add_ss(s, _mm_shuffle_ps(s, s, 1));
        _mm_cvtss_f32(s)
    }

    /// A block's scale (an f16 at p) as f32.
    #[inline]
    #[target_feature(enable = "avx2,f16c")]
    unsafe fn half(p: *const u8) -> f32 {
        unsafe { _mm_cvtss_f32(_mm_cvtph_ps(_mm_cvtsi32_si128(u16::from_le_bytes([*p, *p.add(1)]) as i32))) }
    }

    /// A Q4 block's 32 weights (16 bytes of nibbles at p: the low halves are numbers 0-15, the high halves 16-31)
    /// as signed bytes.
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn unpack_q4(p: *const u8) -> __m256i {
        unsafe {
            let packed = _mm_loadu_si128(p as *const __m128i);
            let both = _mm256_set_m128i(_mm_srli_epi16(packed, 4), packed);
            _mm256_sub_epi8(_mm256_and_si256(both, _mm256_set1_epi8(15)), _mm256_set1_epi8(8))
        }
    }

    /// A Q8 or Q4 block's weights as signed bytes.
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn weights<const Q4: bool>(w: *const u8) -> __m256i {
        unsafe { if Q4 { unpack_q4(w.add(2)) } else { _mm256_loadu_si256(w.add(2) as *const __m256i) } }
    }

    #[target_feature(enable = "avx2,fma,f16c")]
    pub unsafe fn dot_q8_avx2(row: &[u8], x: &[u8], xs: &[f32]) -> f32 {
        unsafe {
            let mut acc = _mm256_setzero_ps();
            for i in 0..xs.len() {
                let w = row.as_ptr().add(i * Q8_BYTES);
                let v = x.as_ptr().add(i * Q8_BYTES);
                let a = _mm256_loadu_si256(w.add(2) as *const __m256i);
                let b = _mm256_loadu_si256(v.add(2) as *const __m256i);
                let p = _mm256_cvtepi32_ps(products(a, b));
                acc = _mm256_fmadd_ps(_mm256_set1_ps(half(w) * *xs.get_unchecked(i)), p, acc);
            }
            sum8(acc)
        }
    }

    #[target_feature(enable = "avx2,fma,f16c")]
    pub unsafe fn dot_q4_avx2(row: &[u8], x: &[u8], xs: &[f32]) -> f32 {
        unsafe {
            let mut acc = _mm256_setzero_ps();
            for i in 0..xs.len() {
                let w = row.as_ptr().add(i * Q4_BYTES);
                let v = x.as_ptr().add(i * Q8_BYTES);
                let q = unpack_q4(w.add(2));
                let b = _mm256_loadu_si256(v.add(2) as *const __m256i);
                let p = _mm256_cvtepi32_ps(products(q, b));
                acc = _mm256_fmadd_ps(_mm256_set1_ps(half(w) * *xs.get_unchecked(i)), p, acc);
            }
            sum8(acc)
        }
    }

    /// A row times four vectors: each of the row's blocks read and unpacked once for the four.
    #[target_feature(enable = "avx2,fma,f16c")]
    pub unsafe fn dot4_avx2<const Q4: bool>(row: &[u8], xs: [&Prepared; 4]) -> [f32; 4] {
        unsafe {
            let bytes = if Q4 { Q4_BYTES } else { Q8_BYTES };
            let (q0, q1, q2, q3) = (xs[0].q.as_ptr().add(2), xs[1].q.as_ptr().add(2), xs[2].q.as_ptr().add(2), xs[3].q.as_ptr().add(2));
            let (s0, s1, s2, s3) = (xs[0].scales.as_ptr(), xs[1].scales.as_ptr(), xs[2].scales.as_ptr(), xs[3].scales.as_ptr());
            let (mut a0, mut a1, mut a2, mut a3) = (_mm256_setzero_ps(), _mm256_setzero_ps(), _mm256_setzero_ps(), _mm256_setzero_ps());
            let ones = _mm256_set1_epi16(1);
            let mut w = row.as_ptr();
            for i in 0..row.len() / bytes {
                let dw = half(w);
                let a = weights::<Q4>(w);
                let ax = _mm256_sign_epi8(a, a);
                let at = i * Q8_BYTES;
                let dot = |q: *const u8| {
                    let b = _mm256_loadu_si256(q.add(at) as *const __m256i);
                    _mm256_cvtepi32_ps(_mm256_madd_epi16(_mm256_maddubs_epi16(ax, _mm256_sign_epi8(b, a)), ones))
                };
                a0 = _mm256_fmadd_ps(_mm256_set1_ps(dw * *s0.add(i)), dot(q0), a0);
                a1 = _mm256_fmadd_ps(_mm256_set1_ps(dw * *s1.add(i)), dot(q1), a1);
                a2 = _mm256_fmadd_ps(_mm256_set1_ps(dw * *s2.add(i)), dot(q2), a2);
                a3 = _mm256_fmadd_ps(_mm256_set1_ps(dw * *s3.add(i)), dot(q3), a3);
                w = w.add(bytes);
            }
            [sum8(a0), sum8(a1), sum8(a2), sum8(a3)]
        }
    }

    /// The same with VNNI: the activations as unsigned bytes (plus 128) times the weights, in one instruction;
    /// 128 times the weights' sums taken off again.
    macro_rules! dot4_vnni {
        ($name:ident, $features:literal, $dp:ident) => {
            #[target_feature(enable = $features)]
            pub unsafe fn $name<const Q4: bool>(row: &[u8], xs: [&Prepared; 4]) -> [f32; 4] {
                unsafe {
                    let bytes = if Q4 { Q4_BYTES } else { Q8_BYTES };
                    let (u0, u1, u2, u3) = (xs[0].qu.as_ptr(), xs[1].qu.as_ptr(), xs[2].qu.as_ptr(), xs[3].qu.as_ptr());
                    let (s0, s1, s2, s3) = (xs[0].scales.as_ptr(), xs[1].scales.as_ptr(), xs[2].scales.as_ptr(), xs[3].scales.as_ptr());
                    let (mut a0, mut a1, mut a2, mut a3) = (_mm256_setzero_ps(), _mm256_setzero_ps(), _mm256_setzero_ps(), _mm256_setzero_ps());
                    let offset = _mm256_set1_epi8(-128i8);
                    let zero = _mm256_setzero_si256();
                    let mut w = row.as_ptr();
                    for i in 0..row.len() / bytes {
                        let dw = half(w);
                        let a = weights::<Q4>(w);
                        let corr = $dp(zero, offset, a);
                        let at = i * BLOCK;
                        let dot = |u: *const u8| {
                            let x = _mm256_loadu_si256(u.add(at) as *const __m256i);
                            _mm256_cvtepi32_ps(_mm256_sub_epi32($dp(zero, x, a), corr))
                        };
                        a0 = _mm256_fmadd_ps(_mm256_set1_ps(dw * *s0.add(i)), dot(u0), a0);
                        a1 = _mm256_fmadd_ps(_mm256_set1_ps(dw * *s1.add(i)), dot(u1), a1);
                        a2 = _mm256_fmadd_ps(_mm256_set1_ps(dw * *s2.add(i)), dot(u2), a2);
                        a3 = _mm256_fmadd_ps(_mm256_set1_ps(dw * *s3.add(i)), dot(u3), a3);
                        w = w.add(bytes);
                    }
                    [sum8(a0), sum8(a1), sum8(a2), sum8(a3)]
                }
            }
        };
    }
    dot4_vnni!(dot4_avxvnni, "avx2,fma,f16c,avxvnni", _mm256_dpbusd_avx_epi32);
    dot4_vnni!(dot4_avx512vnni, "avx2,fma,f16c,avx512vnni,avx512vl", _mm256_dpbusd_epi32);
}

/// A plain f32 dot product.
#[inline]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    if crate::cpu::avx2() && a.len() >= 16 {
        return unsafe { dot_avx2(a, b) };
    }
    let mut s = 0.0f32;
    for (x, y) in a.iter().zip(b) {
        s += x * y;
    }
    s
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn dot_avx2(a: &[f32], b: &[f32]) -> f32 {
    unsafe {
        use core::arch::x86_64::*;
        let n = a.len().min(b.len());
        let mut acc = _mm256_setzero_ps();
        let mut i = 0;
        while i + 8 <= n {
            acc = _mm256_fmadd_ps(_mm256_loadu_ps(a.as_ptr().add(i)), _mm256_loadu_ps(b.as_ptr().add(i)), acc);
            i += 8;
        }
        let hi = _mm256_extractf128_ps(acc, 1);
        let s = _mm_add_ps(_mm256_castps256_ps128(acc), hi);
        let s = _mm_add_ps(s, _mm_movehl_ps(s, s));
        let s = _mm_add_ss(s, _mm_shuffle_ps(s, s, 1));
        let mut sum = _mm_cvtss_f32(s);
        while i < n {
            sum += a[i] * b[i];
            i += 1;
        }
        sum
    }
}

/// y += a · x.
#[inline]
pub fn axpy(a: f32, x: &[f32], y: &mut [f32]) {
    for (yi, xi) in y.iter_mut().zip(x) {
        *yi += a * xi;
    }
}

/// Zeros of length n.
pub fn zeros(n: usize) -> Vec<f32> {
    vec![0.0; n]
}

/// Root mean square normalisation, times (1 + w) (Qwen's "zero-centred" norm) or times w.
pub fn rms_norm(x: &[f32], w: &[f32], plus_one: bool, eps: f32, out: &mut [f32]) {
    let ms = x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32;
    let r = 1.0 / math::sqrt(ms + eps);
    for ((o, &v), &g) in out.iter_mut().zip(x).zip(w) {
        *o = v * r * if plus_one { 1.0 + g } else { g };
    }
}

// In a web page, with WebAssembly's 128-bit SIMD (os/webengine: chatcomputer.ai's chat runs the engine in the
// visitor's browser), a row times vectors is done by these instead of dot_row_many and the plain row-times-vector
// functions above: each block's 32 products of whole numbers summed 16 at a time (four tokens' at once, each block of
// the row read once for the four), then scaled and added up block after block as the plain ones do, so they give
// exactly what the plain arithmetic gives (a sum of whole numbers is the same in any order). (Their `cfg`s above took
// the blank lines that were there, so nothing compiled for a PC moved, and it builds to the same bytes.)
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
pub use wasm::dot_row_many;
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
use wasm::{dot_q4_plain, dot_q8_plain};

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
mod wasm {
    use super::{BLOCK, Dtype, Prepared, Q4_BYTES, Q8_BYTES, dot_row_plain, f16_to_f32};
    use core::arch::wasm32::*;

    /// A block's products, in four sums: its weights (32 signed bytes, as two halves) times the 32 at `v`. Each
    /// product, and the sum of two, fits 16 bits (the vector's numbers are within ±127: quantize_q8 keeps them
    /// there); the rest is summed in 32.
    ///
    /// # Safety
    /// `v` must point at 32 readable bytes.
    #[inline(always)]
    unsafe fn products(w0: v128, w1: v128, v: *const u8) -> v128 {
        unsafe {
            let (v0, v1) = (v128_load(v as *const v128), v128_load(v.add(16) as *const v128));
            let p0 = i16x8_add(i16x8_extmul_low_i8x16(w0, v0), i16x8_extmul_high_i8x16(w0, v0));
            let p1 = i16x8_add(i16x8_extmul_low_i8x16(w1, v1), i16x8_extmul_high_i8x16(w1, v1));
            i32x4_add(i32x4_extadd_pairwise_i16x8(p0), i32x4_extadd_pairwise_i16x8(p1))
        }
    }

    /// A block's sum of products (as `products`).
    ///
    /// # Safety
    /// As `products`.
    #[inline(always)]
    unsafe fn block(w0: v128, w1: v128, v: *const u8) -> i32 {
        let s = unsafe { products(w0, w1, v) };
        let s = i32x4_add(s, i32x4_shuffle::<2, 3, 0, 1>(s, s));
        let s = i32x4_add(s, i32x4_shuffle::<1, 0, 3, 2>(s, s));
        i32x4_extract_lane::<0>(s)
    }

    /// A block's 32 weights as signed bytes, in two halves: Q8's 32 bytes; or Q4's 16 (the first 16 numbers in their
    /// low halves, the last 16 in their high halves, each 8 more than it is).
    ///
    /// # Safety
    /// `w` must point at the block's numbers (after its scale): 32 readable bytes for Q8, 16 for Q4.
    #[inline(always)]
    unsafe fn weights<const Q4: bool>(w: *const u8) -> (v128, v128) {
        unsafe {
            if Q4 {
                let packed = v128_load(w as *const v128);
                (i8x16_sub(v128_and(packed, i8x16_splat(15)), i8x16_splat(8)), i8x16_sub(u8x16_shr(packed, 4), i8x16_splat(8)))
            } else {
                (v128_load(w as *const v128), v128_load(w.add(16) as *const v128))
            }
        }
    }

    fn dot<const Q4: bool>(row: &[u8], x: &[u8], xs: &[f32]) -> f32 {
        let bytes = if Q4 { Q4_BYTES } else { Q8_BYTES };
        let mut sum = 0.0f32;
        for ((w, v), &dx) in row.chunks_exact(bytes).zip(x.chunks_exact(Q8_BYTES)).zip(xs) {
            let dw = f16_to_f32(u16::from_le_bytes([w[0], w[1]]));
            // (Each block is its scale, then its numbers: whole, so the loads stay inside it.)
            let s = unsafe {
                let (w0, w1) = weights::<Q4>(w.as_ptr().add(2));
                block(w0, w1, v.as_ptr().add(2))
            };
            sum += dw * dx * s as f32;
        }
        sum
    }

    pub(super) fn dot_q8_plain(row: &[u8], x: &[u8], xs: &[f32]) -> f32 {
        dot::<false>(row, x, xs)
    }

    pub(super) fn dot_q4_plain(row: &[u8], x: &[u8], xs: &[f32]) -> f32 {
        dot::<true>(row, x, xs)
    }

    /// A row times four vectors: each block's weights read (and its scale made a number) once for the four; the
    /// four blocks' sums of products made one vector, and scaled and added up as the plain arithmetic does, a lane
    /// each.
    fn dot4<const Q4: bool>(row: &[u8], xs: [&Prepared; 4]) -> [f32; 4] {
        let bytes = if Q4 { Q4_BYTES } else { Q8_BYTES };
        let (q0, q1, q2, q3) = (xs[0].q.as_ptr(), xs[1].q.as_ptr(), xs[2].q.as_ptr(), xs[3].q.as_ptr());
        let mut acc = f32x4_splat(0.0);
        for (i, w) in row.chunks_exact(bytes).enumerate() {
            let dw = f16_to_f32(u16::from_le_bytes([w[0], w[1]]));
            let at = i * Q8_BYTES + 2;
            // (Each vector has as many blocks as the row: dot_row_many checks.)
            let s = unsafe {
                let (w0, w1) = weights::<Q4>(w.as_ptr().add(2));
                let (a, b) = (products(w0, w1, q0.add(at)), products(w0, w1, q1.add(at)));
                let (c, d) = (products(w0, w1, q2.add(at)), products(w0, w1, q3.add(at)));
                let ab = i32x4_add(i32x4_shuffle::<0, 4, 1, 5>(a, b), i32x4_shuffle::<2, 6, 3, 7>(a, b));
                let cd = i32x4_add(i32x4_shuffle::<0, 4, 1, 5>(c, d), i32x4_shuffle::<2, 6, 3, 7>(c, d));
                i32x4_add(i32x4_shuffle::<0, 1, 4, 5>(ab, cd), i32x4_shuffle::<2, 3, 6, 7>(ab, cd))
            };
            let dx = f32x4(xs[0].scales[i], xs[1].scales[i], xs[2].scales[i], xs[3].scales[i]);
            acc = f32x4_add(acc, f32x4_mul(f32x4_mul(f32x4_splat(dw), dx), f32x4_convert_i32x4(s)));
        }
        [f32x4_extract_lane::<0>(acc), f32x4_extract_lane::<1>(acc), f32x4_extract_lane::<2>(acc), f32x4_extract_lane::<3>(acc)]
    }

    /// Row `r` of a `dtype` matrix times each of several prepared vectors (a token's each): out[t] for xs[t], four at
    /// a time.
    pub fn dot_row_many(dtype: Dtype, data: &[u8], cols: usize, r: usize, xs: &[&Prepared], out: &mut [f32]) {
        let n = xs.len();
        let mut t = 0;
        if matches!(dtype, Dtype::Q8 | Dtype::Q4) && cols.is_multiple_of(BLOCK) {
            let q4 = dtype == Dtype::Q4;
            let bytes = if q4 { Q4_BYTES } else { Q8_BYTES };
            let row = &data[r * cols / BLOCK * bytes..(r + 1) * cols / BLOCK * bytes];
            while t + 4 <= n {
                let four = [xs[t], xs[t + 1], xs[t + 2], xs[t + 3]];
                if four.iter().any(|x| x.q.len() != cols / BLOCK * Q8_BYTES) {
                    break;
                }
                out[t..t + 4].copy_from_slice(&if q4 { dot4::<true>(row, four) } else { dot4::<false>(row, four) });
                t += 4;
            }
        }
        for (o, x) in out[t..n].iter_mut().zip(&xs[t..]) {
            *o = dot_row_plain(dtype, data, cols, r, x);
        }
    }
}
