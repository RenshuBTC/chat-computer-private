//! Big numbers modulo an odd number, for checking signatures (RSA's 2048- to 4096-bit numbers, and the
//! P-256 and P-384 curves' fields): Montgomery multiplication on 64-bit limbs, least significant first.
//! Signatures and certificates are public, so these take no care to hide their values.

use alloc::vec;
use alloc::vec::Vec;

/// A number as 64-bit limbs, least significant first (as many as the modulus has).
pub type Limbs = Vec<u64>;

/// Big-endian bytes to `n` limbs (None if it doesn't fit).
pub fn from_bytes(b: &[u8], n: usize) -> Option<Limbs> {
    let b = &b[b.iter().position(|&x| x != 0).unwrap_or(b.len())..];
    if b.len() > 8 * n {
        return None;
    }
    let mut out = vec![0u64; n];
    for (i, byte) in b.iter().rev().enumerate() {
        out[i / 8] |= (*byte as u64) << (8 * (i % 8));
    }
    Some(out)
}

/// `len` big-endian bytes.
pub fn to_bytes(a: &[u64], len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    for i in 0..len.min(a.len() * 8) {
        out[len - 1 - i] = (a[i / 8] >> (8 * (i % 8))) as u8;
    }
    out
}

pub fn is_zero(a: &[u64]) -> bool {
    a.iter().all(|&x| x == 0)
}

/// a < b (same length).
pub fn less(a: &[u64], b: &[u64]) -> bool {
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return a[i] < b[i];
        }
    }
    false
}

/// a -= b, returning the borrow.
fn sub_in(a: &mut [u64], b: &[u64]) -> u64 {
    let mut borrow = 0u64;
    for i in 0..a.len() {
        let (d, b1) = a[i].overflowing_sub(b[i]);
        let (d, b2) = d.overflowing_sub(borrow);
        a[i] = d;
        borrow = (b1 | b2) as u64;
    }
    borrow
}

/// a += b, returning the carry.
fn add_in(a: &mut [u64], b: &[u64]) -> u64 {
    let mut carry = 0u64;
    for i in 0..a.len() {
        let (s, c1) = a[i].overflowing_add(b[i]);
        let (s, c2) = s.overflowing_add(carry);
        a[i] = s;
        carry = (c1 | c2) as u64;
    }
    carry
}

/// Arithmetic modulo an odd `m`.
pub struct Modulus {
    pub m: Limbs,
    /// -m^-1 mod 2^64.
    inv: u64,
    /// R^2 mod m (R = 2^(64 limbs)).
    r2: Limbs,
    pub bits: usize,
}

impl Modulus {
    pub fn new(m: Limbs) -> Option<Modulus> {
        if m.is_empty() || m[0] & 1 == 0 || is_zero(&m) {
            return None;
        }
        // Newton's iteration for m^-1 mod 2^64.
        let mut x = 1u64;
        for _ in 0..6 {
            x = x.wrapping_mul(2u64.wrapping_sub(m[0].wrapping_mul(x)));
        }
        let n = m.len();
        let bits = 64 * n - m.last().unwrap().leading_zeros() as usize;
        // R^2 mod m: 1 doubled 2*64*n times, reduced each time.
        let mut r = vec![0u64; n];
        r[0] = 1;
        for _ in 0..2 * 64 * n {
            let top = r[n - 1] >> 63;
            for i in (1..n).rev() {
                r[i] = (r[i] << 1) | (r[i - 1] >> 63);
            }
            r[0] <<= 1;
            if top == 1 || !less(&r, &m) {
                sub_in(&mut r, &m);
            }
        }
        Some(Modulus { inv: x.wrapping_neg(), r2: r, m, bits })
    }

    pub fn limbs(&self) -> usize {
        self.m.len()
    }

    /// a * b / R mod m (a, b < m).
    pub fn mul(&self, a: &[u64], b: &[u64]) -> Limbs {
        let n = self.m.len();
        let mut t = vec![0u64; n + 2];
        for &bi in b.iter().take(n) {
            let mut carry = 0u128;
            for j in 0..n {
                let v = t[j] as u128 + a[j] as u128 * bi as u128 + carry;
                t[j] = v as u64;
                carry = v >> 64;
            }
            let v = t[n] as u128 + carry;
            t[n] = v as u64;
            t[n + 1] = (v >> 64) as u64;
            let q = t[0].wrapping_mul(self.inv);
            let v = t[0] as u128 + q as u128 * self.m[0] as u128;
            let mut carry = v >> 64;
            for j in 1..n {
                let v = t[j] as u128 + q as u128 * self.m[j] as u128 + carry;
                t[j - 1] = v as u64;
                carry = v >> 64;
            }
            let v = t[n] as u128 + carry;
            t[n - 1] = v as u64;
            t[n] = t[n + 1] + (v >> 64) as u64;
        }
        let mut out: Limbs = t[..n].to_vec();
        if t[n] != 0 || !less(&out, &self.m) {
            sub_in(&mut out, &self.m);
        }
        out
    }

    /// Into Montgomery form (a < m).
    pub fn to_mont(&self, a: &[u64]) -> Limbs {
        self.mul(a, &self.r2)
    }

    pub fn from_mont(&self, a: &[u64]) -> Limbs {
        let mut one = vec![0u64; self.m.len()];
        one[0] = 1;
        self.mul(a, &one)
    }

    /// 1 in Montgomery form.
    pub fn one(&self) -> Limbs {
        let mut one = vec![0u64; self.m.len()];
        one[0] = 1;
        self.to_mont(&one)
    }

    pub fn add(&self, a: &[u64], b: &[u64]) -> Limbs {
        let mut r = a.to_vec();
        let carry = add_in(&mut r, b);
        if carry != 0 || !less(&r, &self.m) {
            sub_in(&mut r, &self.m);
        }
        r
    }

    pub fn sub(&self, a: &[u64], b: &[u64]) -> Limbs {
        let mut r = a.to_vec();
        if sub_in(&mut r, b) != 0 {
            add_in(&mut r, &self.m);
        }
        r
    }

    /// x mod m, for x with as many limbs or fewer, at most a little over m (subtracts m while it's too big).
    pub fn reduce(&self, x: &[u64]) -> Limbs {
        let mut r = vec![0u64; self.m.len()];
        r[..x.len().min(self.m.len())].copy_from_slice(&x[..x.len().min(self.m.len())]);
        while !less(&r, &self.m) {
            sub_in(&mut r, &self.m);
        }
        r
    }

    /// base^exp (both in normal form; exp as big-endian bytes), in normal form.
    pub fn pow(&self, base: &[u64], exp: &[u8]) -> Limbs {
        let b = self.to_mont(base);
        let mut r = self.one();
        for byte in exp {
            for bit in (0..8).rev() {
                r = self.mul(&r, &r);
                if (byte >> bit) & 1 == 1 {
                    r = self.mul(&r, &b);
                }
            }
        }
        self.from_mont(&r)
    }

    /// 1/a for a prime modulus (Fermat: a^(m-2)), in normal form.
    pub fn invert(&self, a: &[u64]) -> Limbs {
        let mut e = self.m.clone();
        let two = {
            let mut t = vec![0u64; e.len()];
            t[0] = 2;
            t
        };
        sub_in(&mut e, &two);
        let bytes = to_bytes(&e, e.len() * 8);
        self.pow(a, &bytes)
    }
}
