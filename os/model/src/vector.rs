//! Arithmetic on many numbers at once, for attention over long inputs (a recording's 1,500 positions): e^x over a
//! row of scores, GELU over a layer's numbers, the scores of a few queries against every key, and the values
//! weighted by them. Eight numbers at a time with AVX2 (and FMA) where the processor has them (cpu.rs), else one
//! at a time; the same answers to within rounding either way.

use crate::math;

/// x[i] = e^(x[i] - max), and their sum. (max: at least every x[i], as for a softmax.)
pub fn exp_shifted(x: &mut [f32], max: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    if crate::cpu::avx2() {
        return unsafe { avx2::exp_shifted(x, max) };
    }
    let mut sum = 0.0f32;
    for v in x.iter_mut() {
        *v = math::exp(*v - max);
        sum += *v;
    }
    sum
}

/// GELU (exactly: x · Φ(x), by erf) of each number.
pub fn gelu(x: &mut [f32]) {
    #[cfg(target_arch = "x86_64")]
    if crate::cpu::avx2() {
        unsafe { avx2::gelu(x) };
        return;
    }
    for v in x.iter_mut() {
        *v = crate::vision::gelu(*v);
    }
}

/// The scores of `qs` (queries of `hd` numbers, one after another) against `n` keys laid out by dimension (`kt`:
/// hd rows of n numbers, each row a dimension of every key), times `scale`: into `out` (a row of n per query).
pub fn scores(qs: &[f32], hd: usize, kt: &[f32], n: usize, scale: f32, out: &mut [f32]) {
    #[cfg(target_arch = "x86_64")]
    if crate::cpu::avx2() {
        unsafe { avx2::scores(qs, hd, kt, n, scale, out) };
        return;
    }
    for (q, row) in qs.chunks_exact(hd).zip(out.chunks_exact_mut(n)) {
        row.fill(0.0);
        for (d, &qd) in q.iter().enumerate() {
            let k = &kt[d * n..(d + 1) * n];
            for (o, &kv) in row.iter_mut().zip(k) {
                *o += qd * kv;
            }
        }
        for o in row.iter_mut() {
            *o *= scale;
        }
    }
}

/// out = the sum over i of p[i] times value i (`hd` numbers at v[i * stride..]).
pub fn weighted_sum(p: &[f32], v: &[f32], stride: usize, hd: usize, out: &mut [f32]) {
    #[cfg(target_arch = "x86_64")]
    if crate::cpu::avx2() && hd.is_multiple_of(8) && hd <= 128 {
        unsafe { avx2::weighted_sum(p, v, stride, hd, out) };
        return;
    }
    out[..hd].fill(0.0);
    for (i, &w) in p.iter().enumerate() {
        for (o, &x) in out[..hd].iter_mut().zip(&v[i * stride..i * stride + hd]) {
            *o += w * x;
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use core::arch::x86_64::*;

    /// e^x of eight numbers, as math::exp works it out (x = n ln 2 + r, e^r by its series to r^7); below about
    /// -87.3, 0 (where e^x is too small for a normal f32).
    #[inline]
    #[target_feature(enable = "avx2,fma")]
    unsafe fn exp8(x: __m256) -> __m256 {
        let x = _mm256_min_ps(x, _mm256_set1_ps(88.7));
        let tiny = _mm256_cmp_ps(x, _mm256_set1_ps(-87.3), _CMP_LT_OQ);
        let x = _mm256_max_ps(x, _mm256_set1_ps(-87.3));
        let n = _mm256_round_ps(_mm256_mul_ps(x, _mm256_set1_ps(core::f32::consts::LOG2_E)), _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC);
        let r = _mm256_fnmadd_ps(n, _mm256_set1_ps(0.693_145_75), x);
        let r = _mm256_fnmadd_ps(n, _mm256_set1_ps(1.428_606_8e-6), r);
        let mut p = _mm256_set1_ps(1.0 / 5040.0);
        for c in [1.0 / 720.0, 1.0 / 120.0, 1.0 / 24.0, 1.0 / 6.0, 0.5, 1.0, 1.0] {
            p = _mm256_fmadd_ps(p, r, _mm256_set1_ps(c));
        }
        // 2^n, by the exponent's bits (n from -126 to 127 here).
        let n = _mm256_cvtps_epi32(n);
        let two_n = _mm256_castsi256_ps(_mm256_slli_epi32(_mm256_add_epi32(n, _mm256_set1_epi32(127)), 23));
        _mm256_andnot_ps(tiny, _mm256_mul_ps(p, two_n))
    }

    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn exp_shifted(x: &mut [f32], max: f32) -> f32 {
        unsafe {
            let m = _mm256_set1_ps(max);
            let mut acc = _mm256_setzero_ps();
            let mut i = 0;
            while i + 8 <= x.len() {
                let e = exp8(_mm256_sub_ps(_mm256_loadu_ps(x.as_ptr().add(i)), m));
                _mm256_storeu_ps(x.as_mut_ptr().add(i), e);
                acc = _mm256_add_ps(acc, e);
                i += 8;
            }
            let mut sum = sum8(acc);
            while i < x.len() {
                x[i] = crate::math::exp(x[i] - max);
                sum += x[i];
                i += 1;
            }
            sum
        }
    }

    /// GELU of eight numbers: x/2 · (1 + erf(x/√2)), erf by Abramowitz and Stegun's 7.1.26 (within 1.5e-7).
    #[inline]
    #[target_feature(enable = "avx2,fma")]
    unsafe fn gelu8(x: __m256) -> __m256 {
        let z = _mm256_mul_ps(x, _mm256_set1_ps(core::f32::consts::FRAC_1_SQRT_2));
        let sign = _mm256_and_ps(z, _mm256_set1_ps(-0.0));
        let a = _mm256_andnot_ps(_mm256_set1_ps(-0.0), z);
        let t = _mm256_div_ps(_mm256_set1_ps(1.0), _mm256_fmadd_ps(a, _mm256_set1_ps(0.327_591_1), _mm256_set1_ps(1.0)));
        let mut p = _mm256_set1_ps(1.061_405_4);
        for c in [-1.453_152_1, 1.421_413_7, -0.284_496_74, 0.254_829_6] {
            p = _mm256_fmadd_ps(p, t, _mm256_set1_ps(c));
        }
        p = _mm256_mul_ps(p, t);
        let e = unsafe { exp8(_mm256_sub_ps(_mm256_setzero_ps(), _mm256_mul_ps(a, a))) };
        let erf = _mm256_or_ps(_mm256_fnmadd_ps(p, e, _mm256_set1_ps(1.0)), sign);
        _mm256_mul_ps(_mm256_mul_ps(x, _mm256_set1_ps(0.5)), _mm256_add_ps(erf, _mm256_set1_ps(1.0)))
    }

    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn gelu(x: &mut [f32]) {
        unsafe {
            let mut i = 0;
            while i + 8 <= x.len() {
                let g = gelu8(_mm256_loadu_ps(x.as_ptr().add(i)));
                _mm256_storeu_ps(x.as_mut_ptr().add(i), g);
                i += 8;
            }
            if i < x.len() {
                // (The last few through the same arithmetic, padded.)
                let mut last = [0.0f32; 8];
                last[..x.len() - i].copy_from_slice(&x[i..]);
                let g = gelu8(_mm256_loadu_ps(last.as_ptr()));
                _mm256_storeu_ps(last.as_mut_ptr(), g);
                let rest = x.len() - i;
                x[i..].copy_from_slice(&last[..rest]);
            }
        }
    }

    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn scores(qs: &[f32], hd: usize, kt: &[f32], n: usize, scale: f32, out: &mut [f32]) {
        unsafe {
            let queries = qs.len() / hd;
            let mut j = 0;
            // Four queries at a time, eight keys at a time: each row of keys read once for the four.
            while j < queries {
                let count = (queries - j).min(4);
                let q = |a: usize, d: usize| -> f32 { if a < count { qs[(j + a) * hd + d] * scale } else { 0.0 } };
                let mut i = 0;
                while i + 8 <= n {
                    let (mut a0, mut a1, mut a2, mut a3) = (_mm256_setzero_ps(), _mm256_setzero_ps(), _mm256_setzero_ps(), _mm256_setzero_ps());
                    for d in 0..hd {
                        let k = _mm256_loadu_ps(kt.as_ptr().add(d * n + i));
                        a0 = _mm256_fmadd_ps(_mm256_set1_ps(q(0, d)), k, a0);
                        a1 = _mm256_fmadd_ps(_mm256_set1_ps(q(1, d)), k, a1);
                        a2 = _mm256_fmadd_ps(_mm256_set1_ps(q(2, d)), k, a2);
                        a3 = _mm256_fmadd_ps(_mm256_set1_ps(q(3, d)), k, a3);
                    }
                    for (a, acc) in [a0, a1, a2, a3].into_iter().enumerate().take(count) {
                        _mm256_storeu_ps(out.as_mut_ptr().add((j + a) * n + i), acc);
                    }
                    i += 8;
                }
                for a in 0..count {
                    for key in i..n {
                        let mut s = 0.0f32;
                        for d in 0..hd {
                            s += q(a, d) * kt[d * n + key];
                        }
                        out[(j + a) * n + key] = s;
                    }
                }
                j += count;
            }
        }
    }

    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn weighted_sum(p: &[f32], v: &[f32], stride: usize, hd: usize, out: &mut [f32]) {
        unsafe {
            let lanes = hd / 8;
            let mut acc = [_mm256_setzero_ps(); 16];
            for (i, &w) in p.iter().enumerate() {
                let wv = _mm256_set1_ps(w);
                let row = v.as_ptr().add(i * stride);
                for (l, a) in acc.iter_mut().enumerate().take(lanes) {
                    *a = _mm256_fmadd_ps(wv, _mm256_loadu_ps(row.add(l * 8)), *a);
                }
            }
            for (l, a) in acc.iter().enumerate().take(lanes) {
                _mm256_storeu_ps(out.as_mut_ptr().add(l * 8), *a);
            }
        }
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
}
