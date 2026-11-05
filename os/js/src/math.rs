//! The math functions JavaScript's `Math` and `**` need, for `no_std` (there's no libm in the kernel): square root
//! exactly rounded from integers; exp, log, sin, cos and atan by range reduction and the classic minimax
//! polynomials; the rest from those. Accurate to an ulp or two, which is what the language asks.

use crate::value::{floor as vfloor, trunc as vtrunc};

const LN2_HI: f64 = 6.93147180369123816490e-01;
const LN2_LO: f64 = 1.90821492927058770002e-10;
const INV_LN2: f64 = 1.44269504088896338700e+00;
pub const PI: f64 = core::f64::consts::PI;
/// π/2 in three parts, the first two of 33 bits (so n times them is exact for n below 2^20).
const PIO2_1: f64 = 1.57079632673412561417e+00;
const PIO2_2: f64 = 6.07710050630396597660e-11;
const PIO2_2T: f64 = 2.02226624879595063154e-21;
/// π/2 times 2^1400, in hexadecimal (worked out by Machin's formula), for reducing huge arguments.
const PIO2_BITS: &str = "1921fb54442d18469898cc51701b839a252049c1114cf98e804177d4c76273644a29410f31c6809bbdf2a33679a748636605614dbe4be286e9fc26adadaa3848bc90b6aecc4bcfd8de89885d34c6fdad617feb96de80d6fdbdc70d7f6b5133f4b5d3e4822f8963fcc9250cca3d9c8b67b8400f97142c77e0b31b4906c38aba734d22c7f51fa499ebf06caba47b9475b2c38c5e6ac410aa5773daa520ee12d2cdace186a9c95793009e2e8d811943042";

fn bits(x: f64) -> u64 {
    x.to_bits()
}

fn from_bits(b: u64) -> f64 {
    f64::from_bits(b)
}

pub fn floor(x: f64) -> f64 {
    vfloor(x)
}

pub fn trunc(x: f64) -> f64 {
    vtrunc(x)
}

pub fn ceil(x: f64) -> f64 {
    let f = vfloor(x);
    if f == x || x.is_nan() {
        return x;
    }
    let c = f + 1.0;
    if c == 0.0 { -0.0 } else { c }
}

/// JavaScript's Math.round: halves go up, and -0.5..-0 is -0.
pub fn round(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() || x == 0.0 {
        return x;
    }
    if x > 0.0 && x < 0.5 {
        return 0.0;
    }
    if x < 0.0 && x >= -0.5 {
        return -0.0;
    }
    let f = vfloor(x);
    if x - f >= 0.5 { f + 1.0 } else { f }
}

/// x * 2^n.
pub fn ldexp(mut x: f64, mut n: i32) -> f64 {
    if x == 0.0 || x.is_nan() || x.is_infinite() {
        return x;
    }
    while n > 1000 {
        x *= from_bits(0x7E70_0000_0000_0000); // 2^1000... (2^(1023-?)): see below
        n -= 1000;
        if x.is_infinite() {
            return x;
        }
    }
    while n < -1000 {
        x *= from_bits(0x0170_0000_0000_0000); // 2^-1000
        n += 1000;
        if x == 0.0 {
            return x;
        }
    }
    // 2^n as a double (normal for -1022..=1023; split otherwise).
    if n > 1023 {
        x *= from_bits(((1023 + 1023) as u64) << 52);
        n -= 1023;
    } else if n < -1022 {
        x *= from_bits(1u64 << 52); // 2^-1022
        n += 1022;
        if n < -1022 {
            x *= from_bits(1u64 << 52);
            n += 1022;
        }
    }
    x * from_bits(((n + 1023) as u64) << 52)
}

/// The mantissa (an integer below 2^53, at least 2^52 for numbers not 0) and exponent of a finite positive x.
fn decompose(x: f64) -> (u64, i32) {
    let b = bits(x);
    let e = ((b >> 52) & 0x7FF) as i32;
    let m = b & ((1u64 << 52) - 1);
    if e == 0 {
        // Subnormal: normalize.
        let shift = m.leading_zeros() as i32 - 11;
        (m << shift, -1074 - shift)
    } else {
        (m | (1u64 << 52), e - 1075)
    }
}

fn isqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x: u128 = 1u128 << ((128 - n.leading_zeros()) / 2 + 1);
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// The square root, rounded exactly.
pub fn sqrt(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return if x == 0.0 { x } else { f64::NAN };
    }
    if x == 0.0 || x.is_infinite() {
        return x;
    }
    let (mut m, mut e) = decompose(x);
    if e & 1 != 0 {
        m <<= 1;
        e -= 1;
    }
    let n = (m as u128) << 52;
    let r = isqrt(n);
    let rem = n - r * r;
    // (r + 1/2)^2 = r^2 + r + 1/4: round up when the remainder is more than r.
    let r = if rem > r { r + 1 } else { r };
    ldexp(r as f64, (e - 52) / 2)
}

pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > 709.782712893383973096 {
        return f64::INFINITY;
    }
    if x < -745.13321910194110842 {
        return 0.0;
    }
    if x.abs() < 3.7252902984e-09 {
        return 1.0 + x;
    }
    let k = if x.abs() < 0.5 * LN2_HI { 0 } else { (x * INV_LN2 + if x < 0.0 { -0.5 } else { 0.5 }) as i32 };
    let kf = k as f64;
    let hi = x - kf * LN2_HI;
    let lo = kf * LN2_LO;
    let r = hi - lo;
    let t = r * r;
    let c = r - t * (1.66666666666666019037e-01 + t * (-2.77777777770155933842e-03 + t * (6.61375632143793436117e-05 + t * (-1.65339022054652515390e-06 + t * 4.13813679705723846039e-08))));
    let y = 1.0 - ((lo - (r * c) / (2.0 - c)) - hi);
    if k == 0 { y } else { ldexp(y, k) }
}

pub fn log(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() {
        return x;
    }
    let (m, e) = decompose(x);
    // x = f1 * 2^k with f1 in [sqrt(2)/2, sqrt(2)).
    let mut k = e + 52;
    let mut f1 = from_bits((m & ((1u64 << 52) - 1)) | (1023u64 << 52));
    if f1 > core::f64::consts::SQRT_2 {
        f1 *= 0.5;
        k += 1;
    }
    let f = f1 - 1.0;
    let hfsq = 0.5 * f * f;
    let s = f / (2.0 + f);
    let z = s * s;
    let w = z * z;
    let t1 = w * (3.999999999940941908e-01 + w * (2.222219843214978396e-01 + w * 1.531383769920937332e-01));
    let t2 = z * (6.666666666666735130e-01 + w * (2.857142874366239149e-01 + w * (1.818357216161805012e-01 + w * 1.479819860511658591e-01)));
    let r = t2 + t1;
    let kf = k as f64;
    kf * LN2_HI - ((hfsq - (s * (hfsq + r) + kf * LN2_LO)) - f)
}

pub fn log2(x: f64) -> f64 {
    if x > 0.0 && x.is_finite() {
        let (m, e) = decompose(x);
        if m == 1u64 << 52 {
            return (e + 52) as f64;
        }
    }
    log(x) * core::f64::consts::LOG2_E
}

pub fn log10(x: f64) -> f64 {
    let r = log(x) * core::f64::consts::LOG10_E;
    if r.is_finite() {
        let n = round(r);
        if (n - r).abs() < 1e-9 && (0.0..=22.0).contains(&n) && pow10(n as i32) == x {
            return n;
        }
    }
    r
}

fn pow10(n: i32) -> f64 {
    let mut p = 1.0;
    for _ in 0..n {
        p *= 10.0;
    }
    p
}

/// exp(x) - 1, precise near 0 (Kahan's way).
pub fn expm1(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 {
        return x;
    }
    if x.abs() < 1e-5 {
        return x + x * x / 2.0 + x * x * x / 6.0;
    }
    let u = exp(x);
    if u == 1.0 {
        return x;
    }
    let um1 = u - 1.0;
    if um1 == -1.0 {
        return -1.0;
    }
    if u.is_infinite() {
        return u;
    }
    um1 * x / log(u)
}

/// log(1 + x), precise near 0.
pub fn log1p(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 {
        return x;
    }
    if x == -1.0 {
        return f64::NEG_INFINITY;
    }
    if x < -1.0 {
        return f64::NAN;
    }
    if x.is_infinite() {
        return x;
    }
    let u = 1.0 + x;
    if u == 1.0 {
        return x;
    }
    log(u) * x / (u - 1.0)
}

pub fn pow(x: f64, y: f64) -> f64 {
    if y.is_nan() {
        return f64::NAN;
    }
    if y == 0.0 {
        return 1.0;
    }
    if x.is_nan() {
        return f64::NAN;
    }
    let ax = x.abs();
    if y.is_infinite() {
        if ax == 1.0 {
            return f64::NAN;
        }
        return if (ax > 1.0) == (y > 0.0) { f64::INFINITY } else { 0.0 };
    }
    let y_int = vtrunc(y) == y;
    let y_odd = y_int && y.abs() < 9007199254740992.0 && (vtrunc(y / 2.0) * 2.0 != y);
    if x == 0.0 {
        let neg = x.is_sign_negative() && y_odd;
        let r = if y > 0.0 { 0.0 } else { f64::INFINITY };
        return if neg { -r } else { r };
    }
    if x.is_infinite() {
        let neg = x < 0.0 && y_odd;
        let r = if y > 0.0 { f64::INFINITY } else { 0.0 };
        return if neg { -r } else { r };
    }
    if x < 0.0 && !y_int {
        return f64::NAN;
    }
    // Small integer powers by squaring (exact when the result is).
    if y_int && y.abs() <= 64.0 {
        let mut n = y.abs() as u32;
        let mut base = ax;
        let mut r = 1.0;
        while n > 0 {
            if n & 1 != 0 {
                r *= base;
            }
            base *= base;
            n >>= 1;
        }
        let r = if y < 0.0 { 1.0 / r } else { r };
        return if x < 0.0 && y_odd { -r } else { r };
    }
    let r = pow_pos(ax, y);
    if x < 0.0 && y_odd { -r } else { r }
}

/// a + b exactly, as a sum and its error.
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    (s, (a - (s - bb)) + (b - bb))
}

/// The halves of a number (Veltkamp), for products without rounding.
fn halves(a: f64) -> (f64, f64) {
    let c = 134217729.0 * a;
    let h = c - (c - a);
    (h, a - h)
}

/// a * b exactly, as a product and its error.
fn two_prod(a: f64, b: f64) -> (f64, f64) {
    let p = a * b;
    let (ah, al) = halves(a);
    let (bh, bl) = halves(b);
    (p, ((ah * bh - p) + ah * bl + al * bh) + al * bl)
}

/// x^y for x > 0, finite: exp(y log x), log x in two parts (log(1+f) = 2s + s R(s^2), s = f/(2+f), with 2s exact).
fn pow_pos(x: f64, y: f64) -> f64 {
    let (m, e) = decompose(x);
    let mut k = e + 52;
    let mut f1 = from_bits((m & ((1u64 << 52) - 1)) | (1023u64 << 52));
    if f1 > core::f64::consts::SQRT_2 {
        f1 *= 0.5;
        k += 1;
    }
    let f = f1 - 1.0;
    let (d, de) = two_sum(2.0, f);
    let sh = f / d;
    let (p, pe) = two_prod(sh, d);
    let sl = (((f - p) - pe) - sh * de) / d;
    let z = sh * sh;
    let w = z * z;
    let t1 = w * (3.999999999940941908e-01 + w * (2.222219843214978396e-01 + w * 1.531383769920937332e-01));
    let t2 = z * (6.666666666666735130e-01 + w * (2.857142874366239149e-01 + w * (1.818357216161805012e-01 + w * 1.479819860511658591e-01)));
    let tail = 2.0 * sl + sh * (t1 + t2);
    let kf = k as f64;
    let (a, ae) = two_sum(kf * LN2_HI, 2.0 * sh);
    let (lh, le) = two_sum(a, ae + tail + kf * LN2_LO);
    let (p, pe) = two_prod(y, lh);
    let pe = pe + y * le;
    let s = p + pe;
    if s > 709.782712893383973096 {
        return f64::INFINITY;
    }
    if s < -745.13321910194110842 {
        return 0.0;
    }
    let s_err = pe - (s - p);
    let r = exp(s);
    r + r * s_err
}

fn sin_kernel(x: f64, y: f64) -> f64 {
    let z = x * x;
    let v = z * x;
    let r = 8.33333333332248946124e-03 + z * (-1.98412698298579493134e-04 + z * (2.75573137070700676789e-06 + z * (-2.50507602534068634195e-08 + z * 1.58969099521155010221e-10)));
    x - ((z * (0.5 * y - v * r) - y) - v * -1.66666666666666324348e-01)
}

fn cos_kernel(x: f64, y: f64) -> f64 {
    let z = x * x;
    let r = z * (4.16666666666666019037e-02 + z * (-1.38888888888741095749e-03 + z * (2.48015872894767294178e-05 + z * (-2.75573143513906633035e-07 + z * (2.08757232129817482790e-09 + z * -1.13596475577881948265e-11)))));
    let hz = 0.5 * z;
    let w = 1.0 - hz;
    w + (((1.0 - w) - hz) + (z * r - x * y))
}

/// x reduced by multiples of π/2: (quadrant, hi, lo).
fn reduce(x: f64) -> (i64, f64, f64) {
    if x.abs() < 823549.0 {
        let n = round(x * core::f64::consts::FRAC_2_PI);
        let t = x - n * PIO2_1;
        let w = n * PIO2_2;
        let r = t - w;
        let lo1 = (t - r) - w;
        let w2 = n * PIO2_2T;
        let hi = r - w2;
        let lo = ((r - hi) - w2) + lo1;
        return ((n as i64) & 3, hi, lo);
    }
    // Huge: exactly, in integers (x is m 2^e; π/2 to 1400 bits).
    use crate::bigint::BigInt;
    let (m, e) = decompose(x.abs());
    let p = BigInt::parse_radix(PIO2_BITS, 16).unwrap_or_else(BigInt::zero);
    let big = BigInt::from_u64(m).shl(e as i64 + 1400);
    let (Some(q), Some(mut r)) = (big.div(&p), big.rem(&p)) else { return (0, 0.0, 0.0) };
    let mut quadrant = q.as_uint_n(2).to_f64() as i64;
    if r.shl(1).cmp(&p) == core::cmp::Ordering::Greater {
        r = r.sub(&p);
        quadrant += 1;
    }
    let t = r.shl(-1230);
    let hi = t.to_f64();
    let lo = BigInt::from_f64(hi).map_or(0.0, |h| t.sub(&h).to_f64());
    let (hi, lo) = (ldexp(hi, -170), ldexp(lo, -170));
    if x < 0.0 { ((4 - quadrant) & 3, -hi, -lo) } else { (quadrant & 3, hi, lo) }
}

pub fn fmod(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() || x.is_infinite() || y == 0.0 {
        return f64::NAN;
    }
    if y.is_infinite() || x == 0.0 {
        return x;
    }
    // Exact remainder by long division on the mantissas.
    let (mx, ex) = decompose(x.abs());
    let (my, ey) = decompose(y.abs());
    if ex < ey || (ex == ey && mx < my) {
        return x;
    }
    let mut r = mx as u128;
    let d = my as u128;
    let mut e = ex;
    while e > ey {
        r %= d;
        let shift = (e - ey).min(60);
        r <<= shift;
        e -= shift;
    }
    r %= d;
    let v = ldexp(r as f64, ey);
    if x < 0.0 { -v } else { v }
}

pub fn sin(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() {
        return f64::NAN;
    }
    if x.abs() < 7.4505805969238281e-09 {
        return x;
    }
    let (q, hi, lo) = reduce(x);
    match q {
        0 => sin_kernel(hi, lo),
        1 => cos_kernel(hi, lo),
        2 => -sin_kernel(hi, lo),
        _ => -cos_kernel(hi, lo),
    }
}

pub fn cos(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() {
        return f64::NAN;
    }
    let (q, hi, lo) = reduce(x);
    match q {
        0 => cos_kernel(hi, lo),
        1 => -sin_kernel(hi, lo),
        2 => -cos_kernel(hi, lo),
        _ => sin_kernel(hi, lo),
    }
}

pub fn tan(x: f64) -> f64 {
    if x.is_nan() || x.is_infinite() {
        return f64::NAN;
    }
    if x.abs() < 7.4505805969238281e-09 {
        return x;
    }
    let (q, hi, lo) = reduce(x);
    let s = sin_kernel(hi, lo);
    let c = cos_kernel(hi, lo);
    if q & 1 == 0 { s / c } else { -c / s }
}

const ATAN_HI: [f64; 4] = [4.63647609000806093515e-01, 7.85398163397448278999e-01, 9.82793723247329054082e-01, 1.57079632679489655800e+00];
const ATAN_LO: [f64; 4] = [2.26987774529616870924e-17, 3.06161699786838301793e-17, 1.39033110312309984516e-17, 6.12323399573676603587e-17];
const AT: [f64; 11] = [
    3.33333333333329318027e-01,
    -1.99999999998764832476e-01,
    1.42857142725034663711e-01,
    -1.11111104054623557880e-01,
    9.09088713343650656196e-02,
    -7.69187620504482999495e-02,
    6.66107313738753120669e-02,
    -5.83357013379057348645e-02,
    4.97687799461593236017e-02,
    -3.65315727442169155270e-02,
    1.62858201153657823623e-02,
];

pub fn atan(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    let neg = x < 0.0;
    let mut a = x.abs();
    if a >= 7.3786976294838206464e+19 {
        let r = ATAN_HI[3] + ATAN_LO[3];
        return if neg { -r } else { r };
    }
    let id: i32;
    if a < 0.4375 {
        if a < 7.4505805969238281e-09 {
            return x;
        }
        id = -1;
    } else if a < 1.1875 {
        if a < 0.6875 {
            id = 0;
            a = (2.0 * a - 1.0) / (2.0 + a);
        } else {
            id = 1;
            a = (a - 1.0) / (a + 1.0);
        }
    } else if a < 2.4375 {
        id = 2;
        a = (a - 1.5) / (1.0 + 1.5 * a);
    } else {
        id = 3;
        a = -1.0 / a;
    }
    let z = a * a;
    let w = z * z;
    let s1 = z * (AT[0] + w * (AT[2] + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
    let s2 = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));
    let r = if id < 0 {
        a - a * (s1 + s2)
    } else {
        let i = id as usize;
        ATAN_HI[i] - ((a * (s1 + s2) - ATAN_LO[i]) - a)
    };
    if neg { -r } else { r }
}

pub fn atan2(y: f64, x: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if y == 0.0 {
        if x > 0.0 || (x == 0.0 && !x.is_sign_negative()) {
            return y;
        }
        return if y.is_sign_negative() { -PI } else { PI };
    }
    if x == 0.0 {
        return if y > 0.0 { PI / 2.0 } else { -PI / 2.0 };
    }
    if x.is_infinite() {
        if y.is_infinite() {
            let r = if x > 0.0 { PI / 4.0 } else { 3.0 * PI / 4.0 };
            return if y > 0.0 { r } else { -r };
        }
        let r = if x > 0.0 { 0.0 } else { PI };
        return if y > 0.0 { r } else { -r };
    }
    if y.is_infinite() {
        return if y > 0.0 { PI / 2.0 } else { -PI / 2.0 };
    }
    let a = atan((y / x).abs());
    match (x > 0.0, y > 0.0) {
        (true, true) => a,
        (true, false) => -a,
        (false, true) => PI - a,
        (false, false) => a - PI,
    }
}

pub fn asin(x: f64) -> f64 {
    if x.is_nan() || x.abs() > 1.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return x;
    }
    atan2(x, sqrt((1.0 - x) * (1.0 + x)))
}

pub fn acos(x: f64) -> f64 {
    if x.is_nan() || x.abs() > 1.0 {
        return f64::NAN;
    }
    atan2(sqrt((1.0 - x) * (1.0 + x)), x)
}

pub fn sinh(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 || x.is_infinite() {
        return x;
    }
    let a = x.abs();
    let r = if a < 22.0 {
        let t = expm1(a);
        0.5 * (t + t / (t + 1.0))
    } else if a < 709.78 {
        0.5 * exp(a)
    } else {
        let w = exp(0.5 * a);
        0.5 * w * w
    };
    if x < 0.0 { -r } else { r }
}

pub fn cosh(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    let a = x.abs();
    if a < 22.0 {
        let t = exp(a);
        return 0.5 * (t + 1.0 / t);
    }
    if a < 709.78 {
        return 0.5 * exp(a);
    }
    let w = exp(0.5 * a);
    0.5 * w * w
}

pub fn tanh(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 {
        return x;
    }
    if x.is_infinite() {
        return x.signum();
    }
    let a = x.abs();
    let r = if a > 22.0 {
        1.0
    } else {
        let t = expm1(2.0 * a);
        t / (t + 2.0)
    };
    if x < 0.0 { -r } else { r }
}

pub fn asinh(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 || x.is_infinite() {
        return x;
    }
    let a = x.abs();
    let r = if a > 1e8 { log(a) + core::f64::consts::LN_2 } else { log1p(a + a * a / (1.0 + sqrt(1.0 + a * a))) };
    if x < 0.0 { -r } else { r }
}

pub fn acosh(x: f64) -> f64 {
    if x.is_nan() || x < 1.0 {
        return f64::NAN;
    }
    if x == 1.0 {
        return 0.0;
    }
    if x.is_infinite() {
        return x;
    }
    if x > 1e8 {
        return log(x) + core::f64::consts::LN_2;
    }
    log(x + sqrt((x - 1.0) * (x + 1.0)))
}

pub fn atanh(x: f64) -> f64 {
    if x.is_nan() || x.abs() > 1.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return x;
    }
    if x == 1.0 {
        return f64::INFINITY;
    }
    if x == -1.0 {
        return f64::NEG_INFINITY;
    }
    0.5 * log1p(2.0 * x / (1.0 - x))
}

pub fn cbrt(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 || x.is_infinite() {
        return x;
    }
    let a = x.abs();
    let (m, e) = decompose(a);
    // A first guess from the exponent, then Newton.
    let mut r = ldexp(1.0, (e + 52) / 3) * (1.0 + (m as f64 / (1u64 << 52) as f64 - 1.0) / 3.0);
    for _ in 0..6 {
        r = r - (r * r * r - a) / (3.0 * r * r);
    }
    // The nearer of r and its neighbours.
    let best = [from_bits(bits(r) - 1), r, from_bits(bits(r) + 1)]
        .into_iter()
        .min_by(|p, q| {
            let ep = (p * p * p - a).abs();
            let eq = (q * q * q - a).abs();
            ep.partial_cmp(&eq).unwrap_or(core::cmp::Ordering::Equal)
        })
        .unwrap_or(r);
    if x < 0.0 { -best } else { best }
}

pub fn hypot(values: &[f64]) -> f64 {
    let mut inf = false;
    let mut nan = false;
    let mut max = 0.0f64;
    for v in values {
        if v.is_infinite() {
            inf = true;
        } else if v.is_nan() {
            nan = true;
        } else if v.abs() > max {
            max = v.abs();
        }
    }
    if inf {
        return f64::INFINITY;
    }
    if nan {
        return f64::NAN;
    }
    if max == 0.0 {
        return 0.0;
    }
    // Scaled sum (Kahan-compensated).
    let mut sum = 0.0;
    let mut comp = 0.0;
    for v in values {
        let s = v / max;
        let y = s * s - comp;
        let t = sum + y;
        comp = (t - sum) - y;
        sum = t;
    }
    max * sqrt(sum)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    fn ulps(a: f64, b: f64) -> u64 {
        if a == b || (a.is_nan() && b.is_nan()) {
            return 0;
        }
        (a.to_bits() as i64 - b.to_bits() as i64).unsigned_abs()
    }

    fn samples() -> std::vec::Vec<f64> {
        let mut v = std::vec![0.5, 1.0, 2.0, 3.0, 10.0, 0.1, 1e-10, 1e10, 123.456, 0.75, 1.5, 2.5, 100.0, 700.0, -700.0, 1e-300, 5e-324, 1e300];
        let mut x = 0.001;
        while x < 1000.0 {
            v.push(x);
            v.push(-x);
            x *= 1.37;
        }
        v
    }

    #[test]
    fn square_roots_are_exact() {
        for x in samples() {
            let (a, b) = (sqrt(x), x.sqrt());
            assert!(a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()), "sqrt {x}");
        }
        assert_eq!(sqrt(2.0), std::f64::consts::SQRT_2);
        assert!(sqrt(-1.0).is_nan());
        assert!(sqrt(-0.0).is_sign_negative());
    }

    #[test]
    fn functions_are_within_an_ulp_or_two() {
        type F = (fn(f64) -> f64, fn(f64) -> f64, &'static str);
        let fs: [F; 17] = [
            (exp, f64::exp, "exp"),
            (log, f64::ln, "log"),
            (sin, f64::sin, "sin"),
            (cos, f64::cos, "cos"),
            (tan, f64::tan, "tan"),
            (atan, f64::atan, "atan"),
            (asin, f64::asin, "asin"),
            (acos, f64::acos, "acos"),
            (sinh, f64::sinh, "sinh"),
            (cosh, f64::cosh, "cosh"),
            (tanh, f64::tanh, "tanh"),
            (expm1, f64::exp_m1, "expm1"),
            (log1p, f64::ln_1p, "log1p"),
            (cbrt, f64::cbrt, "cbrt"),
            (log2, f64::log2, "log2"),
            (log10, f64::log10, "log10"),
            (asinh, f64::asinh, "asinh"),
        ];
        for (mine, theirs, name) in fs {
            for x in samples() {
                let (a, b) = (mine(x), theirs(x));
                if b.is_nan() {
                    assert!(a.is_nan(), "{name}({x}) = {a}, not NaN");
                    continue;
                }
                let tolerance = if b.abs() < 1e-300 { 1 << 20 } else { 3 };
                assert!(ulps(a, b) <= tolerance, "{name}({x}) = {a}, should be {b}");
            }
        }
    }

    #[test]
    fn powers() {
        assert_eq!(pow(2.0, 10.0), 1024.0);
        assert_eq!(pow(3.0, 20.0), 3486784401.0);
        assert_eq!(pow(-2.0, 3.0), -8.0);
        assert!(pow(-2.0, 0.5).is_nan());
        assert!(pow(1.0, f64::INFINITY).is_nan());
        assert_eq!(pow(0.0, -1.0), f64::INFINITY);
        assert_eq!(pow(-0.0, -3.0), f64::NEG_INFINITY);
        for (x, y) in [(2.0, 0.5), (10.0, -3.5), (1.5, 100.5), (7.0, 1.0 / 3.0), (0.3, 77.7), (123.0, 45.6)] {
            let (a, b) = (pow(x, y), f64::powf(x, y));
            assert!(ulps(a, b) <= 2, "pow({x}, {y}) = {a}, should be {b}");
        }
        assert_eq!(fmod(5.5, 2.0), 1.5);
        assert_eq!(fmod(-5.5, 2.0), -1.5);
        assert_eq!(fmod(1e300, 3.0), 1e300 % 3.0);
        assert_eq!(atan2(1.0, 1.0), std::f64::consts::FRAC_PI_4);
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(0.49999999999999994), 0.0);
        assert_eq!(ceil(-0.5).to_bits(), (-0.0f64).to_bits());
        assert_eq!(hypot(&[3.0, 4.0]), 5.0);
        assert_eq!(cbrt(27.0), 3.0);
        assert_eq!(log10(1000.0), 3.0);
        assert_eq!(log2(8.0), 3.0);
    }
}
