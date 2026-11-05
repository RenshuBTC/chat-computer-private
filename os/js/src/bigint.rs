//! BigInt: integers of any size, as a sign and a magnitude of 32-bit limbs (the least first, none for zero). The
//! operators as the language has them: division truncates, `%` takes the dividend's sign, `>>` rounds down, and the
//! bitwise operators work as if on infinite two's complement.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigInt {
    neg: bool,
    mag: Vec<u32>,
}

fn trim(v: &mut Vec<u32>) {
    while v.last() == Some(&0) {
        v.pop();
    }
}

fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
    if a.len() != b.len() {
        return a.len().cmp(&b.len());
    }
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return a[i].cmp(&b[i]);
        }
    }
    Ordering::Equal
}

fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
    let mut carry = 0u64;
    for i in 0..a.len().max(b.len()) {
        let s = *a.get(i).unwrap_or(&0) as u64 + *b.get(i).unwrap_or(&0) as u64 + carry;
        out.push(s as u32);
        carry = s >> 32;
    }
    if carry > 0 {
        out.push(carry as u32);
    }
    out
}

/// a - b, with a >= b.
fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for i in 0..a.len() {
        let mut d = a[i] as i64 - *b.get(i).unwrap_or(&0) as i64 - borrow;
        borrow = 0;
        if d < 0 {
            d += 1 << 32;
            borrow = 1;
        }
        out.push(d as u32);
    }
    trim(&mut out);
    out
}

fn mul_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u32; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        let mut carry = 0u64;
        for (j, &y) in b.iter().enumerate() {
            let t = out[i + j] as u64 + x as u64 * y as u64 + carry;
            out[i + j] = t as u32;
            carry = t >> 32;
        }
        let mut k = i + b.len();
        while carry > 0 {
            let t = out[k] as u64 + carry;
            out[k] = t as u32;
            carry = t >> 32;
            k += 1;
        }
    }
    trim(&mut out);
    out
}

/// Divides by a small number: the quotient, and the remainder.
fn div_small(a: &[u32], d: u32) -> (Vec<u32>, u32) {
    let mut out = vec![0u32; a.len()];
    let mut rem = 0u64;
    for i in (0..a.len()).rev() {
        let cur = (rem << 32) | a[i] as u64;
        out[i] = (cur / d as u64) as u32;
        rem = cur % d as u64;
    }
    trim(&mut out);
    (out, rem as u32)
}

/// The quotient and remainder of magnitudes (b not zero), by long division a bit at a time over limbs (Knuth's
/// algorithm D would be faster; these numbers are small).
fn div_mag(a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
    if cmp_mag(a, b) == Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    if b.len() == 1 {
        let (q, r) = div_small(a, b[0]);
        return (q, if r == 0 { Vec::new() } else { vec![r] });
    }
    let bits = a.len() * 32;
    let mut q = vec![0u32; a.len()];
    let mut r: Vec<u32> = Vec::new();
    for i in (0..bits).rev() {
        // r = r << 1 | bit i of a.
        let bit = (a[i / 32] >> (i % 32)) & 1;
        let mut carry = bit;
        for limb in r.iter_mut() {
            let next = *limb >> 31;
            *limb = (*limb << 1) | carry;
            carry = next;
        }
        if carry != 0 {
            r.push(carry);
        }
        if cmp_mag(&r, b) != Ordering::Less {
            r = sub_mag(&r, b);
            q[i / 32] |= 1 << (i % 32);
        }
    }
    trim(&mut q);
    trim(&mut r);
    (q, r)
}

impl BigInt {
    pub fn zero() -> BigInt {
        BigInt { neg: false, mag: Vec::new() }
    }

    pub fn is_zero(&self) -> bool {
        self.mag.is_empty()
    }

    pub fn is_negative(&self) -> bool {
        self.neg
    }

    fn make(neg: bool, mut mag: Vec<u32>) -> BigInt {
        trim(&mut mag);
        BigInt { neg: neg && !mag.is_empty(), mag }
    }

    pub fn from_i64(v: i64) -> BigInt {
        let m = v.unsigned_abs();
        BigInt::make(v < 0, vec![m as u32, (m >> 32) as u32])
    }

    pub fn from_u64(v: u64) -> BigInt {
        BigInt::make(false, vec![v as u32, (v >> 32) as u32])
    }

    /// An integral number, exactly (None if it isn't an integer).
    pub fn from_f64(x: f64) -> Option<BigInt> {
        if !x.is_finite() || crate::value::trunc(x) != x {
            return None;
        }
        let neg = x < 0.0;
        let bits = x.abs().to_bits();
        let exp = ((bits >> 52) & 0x7FF) as i32;
        if exp == 0 {
            return Some(BigInt::zero());
        }
        let mant = (bits & ((1 << 52) - 1)) | (1 << 52);
        let shift = exp - 1075;
        let m = BigInt::from_u64(mant);
        let r = if shift >= 0 { m.shl_bits(shift as u32) } else { BigInt::make(false, div_mag(&m.mag, &BigInt::from_u64(1).shl_bits((-shift) as u32).mag).0) };
        Some(if neg { r.neg() } else { r })
    }

    /// The nearest number (ties to even).
    pub fn to_f64(&self) -> f64 {
        if self.mag.is_empty() {
            return 0.0;
        }
        let bits = self.bit_len();
        let v = if bits <= 64 {
            let lo = self.mag[0] as u64 | (*self.mag.get(1).unwrap_or(&0) as u64) << 32;
            lo as f64
        } else {
            // The top 64 bits, then whether anything below them is set (for rounding), scaled.
            let shift = bits - 64;
            let top = self.shr_mag(shift);
            let mut t = top[0] as u64 | (*top.get(1).unwrap_or(&0) as u64) << 32;
            let sticky = (0..shift).any(|i| (self.mag[(i / 32) as usize] >> (i % 32)) & 1 != 0);
            if sticky {
                t |= 1;
            }
            let f = t as f64;
            if shift > 1100 { f64::INFINITY } else { mul_pow2(f, shift as i32) }
        };
        if self.neg { -v } else { v }
    }

    fn bit_len(&self) -> u32 {
        match self.mag.last() {
            None => 0,
            Some(&top) => (self.mag.len() as u32 - 1) * 32 + (32 - top.leading_zeros()),
        }
    }

    pub fn neg(&self) -> BigInt {
        BigInt::make(!self.neg, self.mag.clone())
    }

    pub fn add(&self, o: &BigInt) -> BigInt {
        if self.neg == o.neg {
            return BigInt::make(self.neg, add_mag(&self.mag, &o.mag));
        }
        match cmp_mag(&self.mag, &o.mag) {
            Ordering::Less => BigInt::make(o.neg, sub_mag(&o.mag, &self.mag)),
            _ => BigInt::make(self.neg, sub_mag(&self.mag, &o.mag)),
        }
    }

    pub fn sub(&self, o: &BigInt) -> BigInt {
        self.add(&o.neg())
    }

    pub fn mul(&self, o: &BigInt) -> BigInt {
        BigInt::make(self.neg != o.neg, mul_mag(&self.mag, &o.mag))
    }

    /// Truncating division (None: division by zero).
    pub fn div(&self, o: &BigInt) -> Option<BigInt> {
        if o.is_zero() {
            return None;
        }
        Some(BigInt::make(self.neg != o.neg, div_mag(&self.mag, &o.mag).0))
    }

    /// The remainder, with the dividend's sign (None: division by zero).
    pub fn rem(&self, o: &BigInt) -> Option<BigInt> {
        if o.is_zero() {
            return None;
        }
        Some(BigInt::make(self.neg, div_mag(&self.mag, &o.mag).1))
    }

    /// self ** e (None: a negative exponent).
    pub fn pow(&self, e: &BigInt) -> Option<BigInt> {
        if e.neg {
            return None;
        }
        let mut result = BigInt::from_i64(1);
        let mut base = self.clone();
        let bits = e.bit_len();
        for i in 0..bits {
            if (e.mag[(i / 32) as usize] >> (i % 32)) & 1 == 1 {
                result = result.mul(&base);
            }
            if i + 1 < bits {
                base = base.mul(&base);
            }
        }
        Some(result)
    }

    fn shl_bits(&self, n: u32) -> BigInt {
        let (limbs, bits) = ((n / 32) as usize, n % 32);
        let mut out = vec![0u32; limbs];
        let mut carry = 0u32;
        for &l in &self.mag {
            if bits == 0 {
                out.push(l);
            } else {
                out.push((l << bits) | carry);
                carry = l >> (32 - bits);
            }
        }
        if carry != 0 {
            out.push(carry);
        }
        BigInt::make(self.neg, out)
    }

    fn shr_mag(&self, n: u32) -> Vec<u32> {
        let (limbs, bits) = ((n / 32) as usize, n % 32);
        if limbs >= self.mag.len() {
            return Vec::new();
        }
        let src = &self.mag[limbs..];
        let mut out = Vec::with_capacity(src.len());
        for i in 0..src.len() {
            let lo = src[i] >> bits;
            let hi = if bits == 0 { 0 } else { src.get(i + 1).map_or(0, |h| h << (32 - bits)) };
            out.push(lo | hi);
        }
        trim(&mut out);
        out
    }

    /// self << n (n negative: >>, rounding down).
    pub fn shl(&self, n: i64) -> BigInt {
        if n >= 0 {
            return self.shl_bits(n.min(u32::MAX as i64) as u32);
        }
        let n = (-n).min(u32::MAX as i64) as u32;
        let q = BigInt::make(self.neg, self.shr_mag(n));
        // (A negative number shifted right rounds down: -1 more if anything shifted out was set.)
        if self.neg && (0..n.min(self.bit_len())).any(|i| (self.mag[(i / 32) as usize] >> (i % 32)) & 1 != 0) {
            return q.sub(&BigInt::from_i64(1));
        }
        q
    }

    /// Two's complement limbs, `n` of them (enough for the sign).
    fn twos(&self, n: usize) -> Vec<u32> {
        let mut v = self.mag.clone();
        v.resize(n, 0);
        if self.neg {
            for l in v.iter_mut() {
                *l = !*l;
            }
            for l in v.iter_mut() {
                let (s, c) = l.overflowing_add(1);
                *l = s;
                if !c {
                    break;
                }
            }
        }
        v
    }

    fn from_twos(mut v: Vec<u32>) -> BigInt {
        let neg = v.last().is_some_and(|l| l >> 31 == 1);
        if neg {
            for l in v.iter_mut() {
                *l = !*l;
            }
            for l in v.iter_mut() {
                let (s, c) = l.overflowing_add(1);
                *l = s;
                if !c {
                    break;
                }
            }
        }
        BigInt::make(neg, v)
    }

    fn bitwise(&self, o: &BigInt, f: impl Fn(u32, u32) -> u32) -> BigInt {
        let n = self.mag.len().max(o.mag.len()) + 1;
        let (a, b) = (self.twos(n), o.twos(n));
        BigInt::from_twos(a.iter().zip(&b).map(|(x, y)| f(*x, *y)).collect())
    }

    pub fn and(&self, o: &BigInt) -> BigInt {
        self.bitwise(o, |a, b| a & b)
    }

    pub fn or(&self, o: &BigInt) -> BigInt {
        self.bitwise(o, |a, b| a | b)
    }

    pub fn xor(&self, o: &BigInt) -> BigInt {
        self.bitwise(o, |a, b| a ^ b)
    }

    /// ~x: -x - 1.
    pub fn not(&self) -> BigInt {
        self.neg().sub(&BigInt::from_i64(1))
    }

    /// BigInt.asUintN(bits, self).
    pub fn as_uint_n(&self, bits: u32) -> BigInt {
        let n = (bits as usize).div_ceil(32) + 1;
        let mut v = self.twos(n.max(self.mag.len() + 1));
        v.truncate((bits as usize).div_ceil(32));
        if bits % 32 != 0
            && let Some(last) = v.last_mut()
        {
            *last &= (1u32 << (bits % 32)) - 1;
        }
        BigInt::make(false, v)
    }

    /// BigInt.asIntN(bits, self).
    pub fn as_int_n(&self, bits: u32) -> BigInt {
        if bits == 0 {
            return BigInt::zero();
        }
        let u = self.as_uint_n(bits);
        // (The top bit set: it's negative, 2^bits less.)
        if u.bit_len() == bits {
            return u.sub(&BigInt::from_i64(1).shl_bits(bits));
        }
        u
    }

    pub fn cmp(&self, o: &BigInt) -> Ordering {
        match (self.neg, o.neg) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => cmp_mag(&self.mag, &o.mag),
            (true, true) => cmp_mag(&o.mag, &self.mag),
        }
    }

    /// Compared with a number (None: it's NaN).
    pub fn cmp_f64(&self, x: f64) -> Option<Ordering> {
        if x.is_nan() {
            return None;
        }
        if x == f64::INFINITY {
            return Some(Ordering::Less);
        }
        if x == f64::NEG_INFINITY {
            return Some(Ordering::Greater);
        }
        let t = crate::value::trunc(x);
        let ti = BigInt::from_f64(t)?;
        match self.cmp(&ti) {
            Ordering::Equal => Some(if x > t { Ordering::Less } else if x < t { Ordering::Greater } else { Ordering::Equal }),
            o => Some(o),
        }
    }

    /// Its digits in `radix` (2 to 36), with a minus sign if it's negative.
    pub fn to_string_radix(&self, radix: u32) -> String {
        if self.mag.is_empty() {
            return String::from("0");
        }
        let mut digits = Vec::new();
        let mut m = self.mag.clone();
        while !m.is_empty() {
            let (q, r) = div_small(&m, radix);
            digits.push(core::char::from_digit(r, radix).unwrap_or('0'));
            m = q;
        }
        let mut out = String::new();
        if self.neg {
            out.push('-');
        }
        out.extend(digits.iter().rev());
        out
    }

    /// Digits of `radix` (no sign, no prefix, no separators), or None.
    pub fn parse_radix(s: &str, radix: u32) -> Option<BigInt> {
        if s.is_empty() {
            return None;
        }
        let mut mag: Vec<u32> = Vec::new();
        for c in s.chars() {
            let d = c.to_digit(radix)?;
            let mut carry = d as u64;
            for l in mag.iter_mut() {
                let v = *l as u64 * radix as u64 + carry;
                *l = v as u32;
                carry = v >> 32;
            }
            if carry > 0 {
                mag.push(carry as u32);
            }
        }
        Some(BigInt::make(false, mag))
    }

    /// StringToBigInt: white space around, decimal with a sign, or 0x / 0o / 0b; empty is 0; else None.
    pub fn parse_js(s: &str) -> Option<BigInt> {
        let t = s.trim_matches(|c: char| crate::string::is_space(c as u32) || crate::string::is_line_terminator(c as u32));
        if t.is_empty() {
            return Some(BigInt::zero());
        }
        for (p, r) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
            if let Some(rest) = t.strip_prefix(p) {
                return BigInt::parse_radix(rest, r);
            }
        }
        let (neg, body) = match t.as_bytes()[0] {
            b'-' => (true, &t[1..]),
            b'+' => (false, &t[1..]),
            _ => (false, t),
        };
        let v = BigInt::parse_radix(body, 10)?;
        Some(if neg { v.neg() } else { v })
    }

    /// Its value as an i64, if it fits.
    pub fn to_i64(&self) -> Option<i64> {
        if self.mag.len() > 2 {
            return None;
        }
        let m = self.mag.first().copied().unwrap_or(0) as u64 | (self.mag.get(1).copied().unwrap_or(0) as u64) << 32;
        if self.neg {
            if m <= 1 << 63 { Some((m as i64).wrapping_neg()) } else { None }
        } else if m < 1 << 63 {
            Some(m as i64)
        } else {
            None
        }
    }

    /// Its low 64 bits, as two's complement (BigInt64Array's elements).
    pub fn to_u64_wrapping(&self) -> u64 {
        let v = self.twos(self.mag.len().max(2) + 1);
        v[0] as u64 | (v[1] as u64) << 32
    }
}

/// x × 2^e, exactly where it can be.
fn mul_pow2(mut x: f64, mut e: i32) -> f64 {
    while e > 0 {
        let step = e.min(1000);
        x *= f64::from_bits(((1023 + step) as u64) << 52);
        e -= step;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(s: &str) -> BigInt {
        BigInt::parse_js(s).unwrap()
    }

    #[test]
    fn arithmetic() {
        assert_eq!(b("123456789012345678901234567890").mul(&b("987654321")).to_string_radix(10), "121932631124828532112482853211126352690");
        assert_eq!(b("-7").div(&b("2")).unwrap().to_string_radix(10), "-3");
        assert_eq!(b("-7").rem(&b("2")).unwrap().to_string_radix(10), "-1");
        assert_eq!(b("2").pow(&b("100")).unwrap().to_string_radix(10), "1267650600228229401496703205376");
        assert_eq!(b("-5").shl(-1).to_string_radix(10), "-3");
        assert_eq!(b("-1").and(&b("255")).to_string_radix(10), "255");
        assert_eq!(b("-256").or(&b("15")).to_string_radix(10), "-241");
        assert_eq!(b("5").not().to_string_radix(10), "-6");
        assert_eq!(b("255").as_int_n(8).to_string_radix(10), "-1");
        assert_eq!(b("-1").as_uint_n(8).to_string_radix(10), "255");
        assert_eq!(b("0xff").to_string_radix(16), "ff");
        assert_eq!(BigInt::from_f64(1e20).unwrap().to_string_radix(10), "100000000000000000000");
        assert_eq!(b("9007199254740993").to_f64(), 9007199254740992.0);
        assert_eq!(b("12345678901234567890123").to_f64(), 1.2345678901234568e22);
        assert_eq!(b("10").cmp_f64(10.5), Some(Ordering::Less));
        assert!(BigInt::parse_js("1.5").is_none());
    }
}
