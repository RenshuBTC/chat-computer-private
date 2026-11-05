//! Arithmetic core lacks without std: rounding, roots, powers, sines (enough for colours and layout, not to the last
//! bit).

pub fn floor(x: f32) -> f32 {
    if !x.is_finite() || x.abs() >= 8_388_608.0 {
        return x;
    }
    let t = x as i64 as f32;
    if t > x { t - 1.0 } else { t }
}

pub fn ceil(x: f32) -> f32 {
    -floor(-x)
}

pub fn round(x: f32) -> f32 {
    floor(x + 0.5)
}

pub fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo { lo } else if x > hi { hi } else { x }
}

/// e^x.
pub fn exp(x: f32) -> f32 {
    if x > 80.0 {
        return f32::MAX;
    }
    if x < -80.0 {
        return 0.0;
    }
    // e^x = 2^k · e^r, |r| ≤ ln2/2.
    let k = round(x / core::f32::consts::LN_2);
    let r = x - k * core::f32::consts::LN_2;
    let mut term = 1.0f32;
    let mut sum = 1.0f32;
    for n in 1..12 {
        term *= r / n as f32;
        sum += term;
    }
    sum * f32::from_bits(((k as i32 + 127).clamp(1, 254) as u32) << 23)
}

/// ln x, for x > 0.
pub fn ln(x: f32) -> f32 {
    if x <= 0.0 {
        return f32::MIN;
    }
    // x = m · 2^e with m in [1, 2): ln x = e·ln2 + ln m, ln m by its series in (m-1)/(m+1).
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xFF) as i32 - 127;
    let m = f32::from_bits((bits & 0x007F_FFFF) | 0x3F80_0000);
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let mut term = s;
    let mut sum = 0.0f32;
    let mut n = 1.0f32;
    for _ in 0..10 {
        sum += term / n;
        term *= s2;
        n += 2.0;
    }
    e as f32 * core::f32::consts::LN_2 + 2.0 * sum
}

/// x^y, for x ≥ 0.
pub fn pow(x: f32, y: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    exp(y * ln(x))
}

pub fn cbrt(x: f32) -> f32 {
    if x == 0.0 {
        return 0.0;
    }
    let s = if x < 0.0 { -1.0 } else { 1.0 };
    s * pow(x.abs(), 1.0 / 3.0)
}

pub fn sqrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut g = if x > 1.0 { x / 2.0 } else { 1.0 };
    for _ in 0..20 {
        g = 0.5 * (g + x / g);
    }
    g
}

/// sin x (radians).
pub fn sin(x: f32) -> f32 {
    use core::f32::consts::PI;
    let tau = 2.0 * PI;
    let mut x = x - floor(x / tau) * tau;
    if x > PI {
        x -= tau;
    }
    // (Folded into [-π/2, π/2].)
    if x > PI / 2.0 {
        x = PI - x;
    } else if x < -PI / 2.0 {
        x = -PI - x;
    }
    let x2 = x * x;
    x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0))))
}

pub fn cos(x: f32) -> f32 {
    sin(x + core::f32::consts::FRAC_PI_2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn they_are_close_enough() {
        for &(x, y) in &[(2.0f32, 0.5f32), (0.5, 2.4), (10.0, 1.0 / 3.0), (0.04, 1.0 / 2.4)] {
            let want = libm_pow(x, y);
            assert!((pow(x, y) - want).abs() < 1e-4 * want.max(1.0), "{x}^{y}");
        }
        assert!((sin(1.0) - 0.841_470_96).abs() < 1e-5);
        assert!((cos(2.0) - (-0.416_146_84)).abs() < 1e-5);
        assert!((cbrt(27.0) - 3.0).abs() < 1e-4);
        assert!((sqrt(2.0) - core::f32::consts::SQRT_2).abs() < 1e-6);
        assert_eq!(floor(-1.5), -2.0);
        assert_eq!(round(2.5), 3.0);
    }

    fn libm_pow(x: f32, y: f32) -> f32 {
        (x as f64).powf(y as f64) as f32
    }
}
