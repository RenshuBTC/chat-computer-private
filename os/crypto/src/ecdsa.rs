//! Checking ECDSA signatures (FIPS 186-4) on the curves TLS servers and certificate authorities use: P-256 and
//! P-384. Points in Jacobian coordinates, numbers in Montgomery form. Only public values pass through here.

use crate::bignum::{self, Limbs, Modulus};
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Curve {
    P256,
    P384,
}

struct Params {
    p: &'static str,
    b: &'static str,
    gx: &'static str,
    gy: &'static str,
    n: &'static str,
    bytes: usize,
}

const P256: Params = Params {
    p: "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff",
    b: "5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b",
    gx: "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296",
    gy: "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5",
    n: "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551",
    bytes: 32,
};

const P384: Params = Params {
    p: "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff",
    b: "b3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aef",
    gx: "aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7",
    gy: "3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f",
    n: "ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973",
    bytes: 48,
};

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// A point in Jacobian coordinates (Montgomery form); z = 0 is the point at infinity.
#[derive(Clone)]
struct Point {
    x: Limbs,
    y: Limbs,
    z: Limbs,
}

struct Field {
    f: Modulus,
}

impl Field {
    fn zero(&self) -> Limbs {
        alloc::vec![0u64; self.f.limbs()]
    }

    fn double(&self, a: &Point) -> Point {
        let f = &self.f;
        if bignum::is_zero(&a.z) || bignum::is_zero(&a.y) {
            return Point { x: f.one(), y: f.one(), z: self.zero() };
        }
        // a = -3: alpha = 3 (X - Z^2)(X + Z^2).
        let delta = f.mul(&a.z, &a.z);
        let gamma = f.mul(&a.y, &a.y);
        let beta = f.mul(&a.x, &gamma);
        let t = f.mul(&f.sub(&a.x, &delta), &f.add(&a.x, &delta));
        let alpha = f.add(&f.add(&t, &t), &t);
        let beta4 = f.add(&f.add(&beta, &beta), &f.add(&beta, &beta));
        let beta8 = f.add(&beta4, &beta4);
        let x3 = f.sub(&f.mul(&alpha, &alpha), &beta8);
        let yz = f.add(&a.y, &a.z);
        let z3 = f.sub(&f.sub(&f.mul(&yz, &yz), &gamma), &delta);
        let g2 = f.mul(&gamma, &gamma);
        let g8 = {
            let g2x2 = f.add(&g2, &g2);
            let g2x4 = f.add(&g2x2, &g2x2);
            f.add(&g2x4, &g2x4)
        };
        let y3 = f.sub(&f.mul(&alpha, &f.sub(&beta4, &x3)), &g8);
        Point { x: x3, y: y3, z: z3 }
    }

    fn add(&self, a: &Point, b: &Point) -> Point {
        let f = &self.f;
        if bignum::is_zero(&a.z) {
            return b.clone();
        }
        if bignum::is_zero(&b.z) {
            return a.clone();
        }
        let z1z1 = f.mul(&a.z, &a.z);
        let z2z2 = f.mul(&b.z, &b.z);
        let u1 = f.mul(&a.x, &z2z2);
        let u2 = f.mul(&b.x, &z1z1);
        let s1 = f.mul(&f.mul(&a.y, &b.z), &z2z2);
        let s2 = f.mul(&f.mul(&b.y, &a.z), &z1z1);
        let h = f.sub(&u2, &u1);
        let r0 = f.sub(&s2, &s1);
        if bignum::is_zero(&h) {
            return if bignum::is_zero(&r0) { self.double(a) } else { Point { x: f.one(), y: f.one(), z: self.zero() } };
        }
        let r = f.add(&r0, &r0);
        let h2 = f.add(&h, &h);
        let i = f.mul(&h2, &h2);
        let j = f.mul(&h, &i);
        let v = f.mul(&u1, &i);
        let x3 = f.sub(&f.sub(&f.mul(&r, &r), &j), &f.add(&v, &v));
        let s1j = f.mul(&s1, &j);
        let y3 = f.sub(&f.mul(&r, &f.sub(&v, &x3)), &f.add(&s1j, &s1j));
        let zz = f.add(&a.z, &b.z);
        let z3 = f.mul(&f.sub(&f.sub(&f.mul(&zz, &zz), &z1z1), &z2z2), &h);
        Point { x: x3, y: y3, z: z3 }
    }

    /// k1 * p1 + k2 * p2 (k as normal-form limbs), both at once (Shamir's trick).
    fn twin(&self, k1: &[u64], p1: &Point, k2: &[u64], p2: &Point) -> Point {
        let both = self.add(p1, p2);
        let mut r = Point { x: self.f.one(), y: self.f.one(), z: self.zero() };
        for bit in (0..64 * k1.len()).rev() {
            r = self.double(&r);
            let b1 = (k1[bit / 64] >> (bit % 64)) & 1;
            let b2 = (k2[bit / 64] >> (bit % 64)) & 1;
            match (b1, b2) {
                (1, 1) => r = self.add(&r, &both),
                (1, 0) => r = self.add(&r, p1),
                (0, 1) => r = self.add(&r, p2),
                _ => {}
            }
        }
        r
    }
}

/// Checks `signature` (r and s, big-endian, each the curve's size) of the message whose hash is `hash`, by the
/// public key `point` (uncompressed: 0x04, x, y).
pub fn verify(curve: Curve, point: &[u8], hash: &[u8], r: &[u8], s: &[u8]) -> bool {
    let params = match curve {
        Curve::P256 => &P256,
        Curve::P384 => &P384,
    };
    let len = params.bytes;
    let limbs = len / 8;
    if point.len() != 1 + 2 * len || point[0] != 4 {
        return false;
    }
    let (Some(field), Some(order)) =
        (bignum::from_bytes(&hex(params.p), limbs).and_then(Modulus::new), bignum::from_bytes(&hex(params.n), limbs).and_then(Modulus::new))
    else {
        return false;
    };
    let (Some(r), Some(s)) = (bignum::from_bytes(r, limbs), bignum::from_bytes(s, limbs)) else { return false };
    if bignum::is_zero(&r) || bignum::is_zero(&s) || !bignum::less(&r, &order.m) || !bignum::less(&s, &order.m) {
        return false;
    }
    // The key's point must be on the curve: y^2 = x^3 - 3x + b.
    let (Some(qx), Some(qy)) = (bignum::from_bytes(&point[1..1 + len], limbs), bignum::from_bytes(&point[1 + len..], limbs)) else {
        return false;
    };
    if !bignum::less(&qx, &field.m) || !bignum::less(&qy, &field.m) {
        return false;
    }
    let fq = Field { f: field };
    let f = &fq.f;
    let (x, y) = (f.to_mont(&qx), f.to_mont(&qy));
    let b = f.to_mont(&bignum::from_bytes(&hex(params.b), limbs).unwrap());
    let x3 = f.mul(&f.mul(&x, &x), &x);
    let three_x = f.add(&f.add(&x, &x), &x);
    if f.add(&f.sub(&x3, &three_x), &b) != f.mul(&y, &y) {
        return false;
    }
    // e: the hash's leftmost bits, as many as the order has (both orders are whole bytes long).
    let e = order.reduce(&bignum::from_bytes(&hash[..hash.len().min(len)], limbs).unwrap());
    let w = order.invert(&s);
    let u1 = order.from_mont(&order.mul(&order.to_mont(&e), &order.to_mont(&w)));
    let u2 = order.from_mont(&order.mul(&order.to_mont(&r), &order.to_mont(&w)));
    let g = Point {
        x: f.to_mont(&bignum::from_bytes(&hex(params.gx), limbs).unwrap()),
        y: f.to_mont(&bignum::from_bytes(&hex(params.gy), limbs).unwrap()),
        z: f.one(),
    };
    let q = Point { x, y, z: f.one() };
    let sum = fq.twin(&u1, &g, &u2, &q);
    if bignum::is_zero(&sum.z) {
        return false;
    }
    // Its x (affine): X / Z^2, then mod n.
    let zinv = f.to_mont(&f.invert(&f.from_mont(&sum.z)));
    let ax = f.from_mont(&f.mul(&sum.x, &f.mul(&zinv, &zinv)));
    order.reduce(&ax) == r
}
