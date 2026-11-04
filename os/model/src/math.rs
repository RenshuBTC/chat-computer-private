//! The few functions of real numbers the model needs (exp, ln, sqrt, sin and cos), written here: the crate runs where
//! there's no standard library (Chat Computer's own kernel), and needs nothing from anywhere else.

/// e^x, to within about one part in ten million (the model's softmax, SiLU, sigmoid and decays).
pub fn exp(x: f32) -> f32 {
    if x.is_nan() {
        return x;
    }
    if x > 88.7 {
        return f32::INFINITY;
    }
    if x < -103.9 {
        return 0.0;
    }
    // x = n ln 2 + r, |r| <= ln2 / 2; e^x = 2^n e^r. (ln 2 in two parts, so r is exact.)
    const LN2_HI: f32 = 0.693_145_75;
    const LN2_LO: f32 = 1.428_606_8e-6;
    let n = (x * core::f32::consts::LOG2_E + if x < 0.0 { -0.5 } else { 0.5 }) as i32;
    let r = x - n as f32 * LN2_HI - n as f32 * LN2_LO;
    // e^r by its Taylor series to r^7 (|r| < 0.35: the next term is under 2e-9).
    let p = 1.0 + r * (1.0 + r * (0.5 + r * (1.0 / 6.0 + r * (1.0 / 24.0 + r * (1.0 / 120.0 + r * (1.0 / 720.0 + r * (1.0 / 5040.0)))))));
    scale2(p, n)
}

/// x · 2^n, for n where x · 2^n is a normal number or underflows to a subnormal or 0.
fn scale2(x: f32, n: i32) -> f32 {
    if n > 127 {
        return x * f32::from_bits(254 << 23) * f32::from_bits(((n - 127 + 127) as u32) << 23);
    }
    if n < -126 {
        // In two steps, so the result can be subnormal.
        let m = n + 126;
        return x * f32::from_bits(1 << 23) * f32::from_bits(((m.max(-126) + 127) as u32) << 23);
    }
    x * f32::from_bits(((n + 127) as u32) << 23)
}

/// The natural logarithm of x (x > 0), to within a few parts in ten million.
pub fn ln(x: f32) -> f32 {
    if x.is_nan() || x < 0.0 {
        return f32::NAN;
    }
    if x == 0.0 {
        return f32::NEG_INFINITY;
    }
    if x.is_infinite() {
        return x;
    }
    let (mut m, mut e) = frexp(x as f64);
    // m in [0.5, 1): moved to [sqrt(1/2), sqrt(2)) so ln's series converges fast.
    if m < core::f64::consts::FRAC_1_SQRT_2 {
        m *= 2.0;
        e -= 1;
    }
    // ln m = 2 atanh(s), s = (m - 1) / (m + 1), |s| < 0.172.
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let atanh = s * (1.0 + s2 * (1.0 / 3.0 + s2 * (1.0 / 5.0 + s2 * (1.0 / 7.0 + s2 * (1.0 / 9.0 + s2 * (1.0 / 11.0))))));
    (2.0 * atanh + e as f64 * core::f64::consts::LN_2) as f32
}

/// x as m · 2^e, m in [0.5, 1) (x positive and finite).
fn frexp(x: f64) -> (f64, i32) {
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        // Subnormal: made normal first.
        let (m, e) = frexp(x * (1u64 << 54) as f64);
        return (m, e - 54);
    }
    let m = f64::from_bits((bits & !(0x7ffu64 << 52)) | (1022u64 << 52));
    (m, exp - 1022)
}

/// The square root of x (x >= 0).
#[inline]
pub fn sqrt(x: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::*;
        // (SSE, which every x86-64 processor has.)
        unsafe { _mm_cvtss_f32(_mm_sqrt_ss(_mm_set_ss(x))) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        sqrt64(x as f64) as f32
    }
}

/// The square root of x (x >= 0), in double precision.
pub fn sqrt64(x: f64) -> f64 {
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::*;
        unsafe { _mm_cvtsd_f64(_mm_sqrt_sd(_mm_set_sd(x), _mm_set_sd(x))) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        if x <= 0.0 {
            return 0.0;
        }
        let mut r = f64::from_bits((x.to_bits() >> 1) + (1023u64 << 51));
        for _ in 0..6 {
            r = 0.5 * (r + x / r);
        }
        r
    }
}

/// The sine and cosine of x (radians), in double precision (rotary positions: x up to a few hundred thousand).
pub fn sin_cos(x: f64) -> (f64, f64) {
    use core::f64::consts::{FRAC_PI_2, PI};
    // x = k π/2 + r, |r| <= π/4: π/2 in three parts, so r stays exact for large x.
    const P1: f64 = 1.570_796_326_734_125_6;
    const P2: f64 = 6.077_100_506_303_966e-11;
    const P3: f64 = 2.022_266_248_795_950_7e-21;
    let k = (x / FRAC_PI_2 + if x < 0.0 { -0.5 } else { 0.5 }) as i64;
    let kf = k as f64;
    let r = ((x - kf * P1) - kf * P2) - kf * P3;
    let r2 = r * r;
    let s = r * (1.0 - r2 / 6.0 * (1.0 - r2 / 20.0 * (1.0 - r2 / 42.0 * (1.0 - r2 / 72.0 * (1.0 - r2 / 110.0 * (1.0 - r2 / 156.0))))));
    let c = 1.0 - r2 / 2.0 * (1.0 - r2 / 12.0 * (1.0 - r2 / 30.0 * (1.0 - r2 / 56.0 * (1.0 - r2 / 90.0 * (1.0 - r2 / 132.0)))));
    let _ = PI;
    match k.rem_euclid(4) {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

/// The natural logarithm of x (x > 0), in double precision.
pub fn ln_f64(x: f64) -> f64 {
    ln64(x)
}

/// e^x, in double precision.
pub fn exp_f64(x: f64) -> f64 {
    exp64(x)
}

/// x to the power y, x > 0 (rotary frequencies: θ^(-2i/d)).
pub fn powf64(x: f64, y: f64) -> f64 {
    exp64(y * ln64(x))
}

fn ln64(x: f64) -> f64 {
    let (mut m, mut e) = frexp(x);
    if m < core::f64::consts::FRAC_1_SQRT_2 {
        m *= 2.0;
        e -= 1;
    }
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let mut term = s;
    let mut sum = 0.0;
    let mut k = 1.0;
    while k < 40.0 {
        sum += term / k;
        term *= s2;
        k += 2.0;
    }
    2.0 * sum + e as f64 * core::f64::consts::LN_2
}

fn exp64(x: f64) -> f64 {
    if x > 709.0 {
        return f64::INFINITY;
    }
    if x < -745.0 {
        return 0.0;
    }
    let n = (x / core::f64::consts::LN_2 + if x < 0.0 { -0.5 } else { 0.5 }) as i64;
    let r = x - n as f64 * core::f64::consts::LN_2;
    let mut term = 1.0;
    let mut sum = 1.0;
    for i in 1..20 {
        term *= r / i as f64;
        sum += term;
    }
    // 2^n, in up to two steps (n can be below the normal range).
    let (a, b) = if n < -1000 { (n + 500, -500) } else if n > 1000 { (n - 500, 500) } else { (n, 0) };
    sum * f64::from_bits(((a + 1023) as u64) << 52) * f64::from_bits(((b + 1023) as u64) << 52)
}

/// SiLU (x · sigmoid x).
#[inline]
pub fn silu(x: f32) -> f32 {
    x / (1.0 + exp(-x))
}

#[inline]
pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + exp(-x))
}

/// softplus: ln(1 + e^x).
pub fn softplus(x: f32) -> f32 {
    if x > 20.0 { x } else if x < -20.0 { exp(x) } else { ln(1.0 + exp(x)) }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn as_the_standard_library_gives_them() {
        let mut worst = (0.0f64, 0.0f64, 0.0f64);
        // (Down to where e^x is still a normal number: below, an f32 has fewer digits to be right in.)
        let mut x = -87.0f32;
        while x < 88.0 {
            let ours = exp(x) as f64;
            let theirs = (x as f64).exp();
            let rel = ((ours - theirs) / theirs).abs();
            if rel > worst.0 {
                worst.0 = rel;
            }
            x += 0.0137;
        }
        let mut x = 1e-30f32;
        while x < 1e30 {
            // (Relative to the answer, or absolute below 1: an f32's own precision.)
            let rel = ((ln(x) as f64) - (x as f64).ln()).abs() / (x as f64).ln().abs().max(1.0);
            if rel > worst.1 {
                worst.1 = rel;
            }
            x *= 1.37;
        }
        let mut x = -300000.0f64;
        while x < 300000.0 {
            let (s, c) = sin_cos(x);
            worst.2 = worst.2.max((s - x.sin()).abs()).max((c - x.cos()).abs());
            x += 77.7;
        }
        assert!(worst.0 < 3e-7, "exp {worst:?}");
        assert!(worst.1 < 3e-7, "ln {worst:?}");
        assert!(worst.2 < 1e-9, "sin cos {worst:?}");
        for x in [0.0f32, 1e-8, 0.25, 2.0, 1e6, 123_456.79] {
            assert_eq!(sqrt(x), x.sqrt());
        }
        let p = powf64(10_000_000.0, -0.25);
        assert!((p - 10_000_000f64.powf(-0.25)).abs() < 1e-15, "{p}");
        assert!((softplus(0.3) - (1.0f32 + 0.3f32.exp()).ln()).abs() < 1e-6);
    }
}
