//! X25519 (RFC 7748): agreeing on a shared secret over the network. Numbers modulo 2^255 - 19 in five 51-bit
//! limbs; the Montgomery ladder, its swaps chosen by mask, never by branch.

type Fe = [u64; 5];

const MASK: u64 = (1 << 51) - 1;

fn load(b: &[u8; 32]) -> Fe {
    let w = |i: usize| u64::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3], b[i + 4], b[i + 5], b[i + 6], b[i + 7]]);
    [w(0) & MASK, (w(6) >> 3) & MASK, (w(12) >> 6) & MASK, (w(19) >> 1) & MASK, (w(24) >> 12) & MASK]
}

fn carry(mut h: [u128; 5]) -> Fe {
    for i in 0..4 {
        h[i + 1] += h[i] >> 51;
        h[i] &= MASK as u128;
    }
    h[0] += (h[4] >> 51) * 19;
    h[4] &= MASK as u128;
    h[1] += h[0] >> 51;
    h[0] &= MASK as u128;
    [h[0] as u64, h[1] as u64, h[2] as u64, h[3] as u64, h[4] as u64]
}

fn add(a: &Fe, b: &Fe) -> Fe {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3], a[4] + b[4]]
}

/// a - b, with 2p added so nothing goes below zero.
fn sub(a: &Fe, b: &Fe) -> Fe {
    const P2: [u64; 5] = [0xFFFFFFFFFFFDA, 0xFFFFFFFFFFFFE, 0xFFFFFFFFFFFFE, 0xFFFFFFFFFFFFE, 0xFFFFFFFFFFFFE];
    let r = [a[0] + P2[0] - b[0], a[1] + P2[1] - b[1], a[2] + P2[2] - b[2], a[3] + P2[3] - b[3], a[4] + P2[4] - b[4]];
    carry(r.map(|x| x as u128))
}

fn mul(a: &Fe, b: &Fe) -> Fe {
    let m = |x: u64, y: u64| x as u128 * y as u128;
    let b19 = [b[0], b[1] * 19, b[2] * 19, b[3] * 19, b[4] * 19];
    carry([
        m(a[0], b[0]) + m(a[1], b19[4]) + m(a[2], b19[3]) + m(a[3], b19[2]) + m(a[4], b19[1]),
        m(a[0], b[1]) + m(a[1], b[0]) + m(a[2], b19[4]) + m(a[3], b19[3]) + m(a[4], b19[2]),
        m(a[0], b[2]) + m(a[1], b[1]) + m(a[2], b[0]) + m(a[3], b19[4]) + m(a[4], b19[3]),
        m(a[0], b[3]) + m(a[1], b[2]) + m(a[2], b[1]) + m(a[3], b[0]) + m(a[4], b19[4]),
        m(a[0], b[4]) + m(a[1], b[3]) + m(a[2], b[2]) + m(a[3], b[1]) + m(a[4], b[0]),
    ])
}

fn square(a: &Fe) -> Fe {
    mul(a, a)
}

fn mul_small(a: &Fe, k: u64) -> Fe {
    carry(a.map(|x| x as u128 * k as u128))
}

/// a^(p-2) = 1/a.
fn invert(a: &Fe) -> Fe {
    // p - 2 = 2^255 - 21: the usual addition chain.
    let z2 = square(a);
    let z8 = square(&square(&z2));
    let z9 = mul(&z8, a);
    let z11 = mul(&z9, &z2);
    let z22 = square(&z11);
    let z_5_0 = mul(&z22, &z9);
    let mut t = square(&z_5_0);
    for _ in 1..5 {
        t = square(&t);
    }
    let z_10_0 = mul(&t, &z_5_0);
    t = square(&z_10_0);
    for _ in 1..10 {
        t = square(&t);
    }
    let z_20_0 = mul(&t, &z_10_0);
    t = square(&z_20_0);
    for _ in 1..20 {
        t = square(&t);
    }
    let z_40_0 = mul(&t, &z_20_0);
    t = square(&z_40_0);
    for _ in 1..10 {
        t = square(&t);
    }
    let z_50_0 = mul(&t, &z_10_0);
    t = square(&z_50_0);
    for _ in 1..50 {
        t = square(&t);
    }
    let z_100_0 = mul(&t, &z_50_0);
    t = square(&z_100_0);
    for _ in 1..100 {
        t = square(&t);
    }
    let z_200_0 = mul(&t, &z_100_0);
    t = square(&z_200_0);
    for _ in 1..50 {
        t = square(&t);
    }
    let z_250_0 = mul(&t, &z_50_0);
    t = square(&z_250_0);
    for _ in 1..5 {
        t = square(&t);
    }
    mul(&t, &z11)
}

/// The number's bytes, fully reduced.
fn store(a: &Fe) -> [u8; 32] {
    let mut h = carry(a.map(|x| x as u128));
    // Now h < 2^255 + small; subtract p if h >= p, by mask.
    let mut q = (h[0] + 19) >> 51;
    q = (h[1] + q) >> 51;
    q = (h[2] + q) >> 51;
    q = (h[3] + q) >> 51;
    q = (h[4] + q) >> 51;
    h[0] += 19 * q;
    for i in 0..4 {
        h[i + 1] += h[i] >> 51;
        h[i] &= MASK;
    }
    h[4] &= MASK;
    let mut out = [0u8; 32];
    let mut acc: u128 = 0;
    let mut bits = 0;
    let mut at = 0;
    for limb in h {
        acc |= (limb as u128) << bits;
        bits += 51;
        while bits >= 8 && at < 32 {
            out[at] = acc as u8;
            acc >>= 8;
            bits -= 8;
            at += 1;
        }
    }
    if at < 32 {
        out[at] = acc as u8;
    }
    out
}

fn swap(c: u64, a: &mut Fe, b: &mut Fe) {
    let mask = 0u64.wrapping_sub(c);
    for i in 0..5 {
        let t = mask & (a[i] ^ b[i]);
        a[i] ^= t;
        b[i] ^= t;
    }
}

/// The X25519 function: `scalar` times the point whose u-coordinate is `u`.
pub fn x25519(scalar: &[u8; 32], u: &[u8; 32]) -> [u8; 32] {
    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;
    let mut ub = *u;
    ub[31] &= 127;
    let x1 = load(&ub);
    let (mut x2, mut z2, mut x3, mut z3) = ([1, 0, 0, 0, 0], [0u64; 5], x1, [1, 0, 0, 0, 0]);
    let mut swapped = 0u64;
    for t in (0..255).rev() {
        let bit = ((k[t / 8] >> (t % 8)) & 1) as u64;
        swapped ^= bit;
        swap(swapped, &mut x2, &mut x3);
        swap(swapped, &mut z2, &mut z3);
        swapped = bit;
        let a = add(&x2, &z2);
        let aa = square(&a);
        let b = sub(&x2, &z2);
        let bb = square(&b);
        let e = sub(&aa, &bb);
        let c = add(&x3, &z3);
        let d = sub(&x3, &z3);
        let da = mul(&d, &a);
        let cb = mul(&c, &b);
        x3 = square(&add(&da, &cb));
        z3 = mul(&x1, &square(&sub(&da, &cb)));
        x2 = mul(&aa, &bb);
        z2 = mul(&e, &add(&aa, &mul_small(&e, 121665)));
    }
    swap(swapped, &mut x2, &mut x3);
    swap(swapped, &mut z2, &mut z3);
    let out = store(&mul(&x2, &invert(&z2)));
    unsafe { core::ptr::write_volatile(&mut k, [0; 32]) };
    out
}

/// The public key for a private one: the private key times the base point (u = 9).
pub fn public(private: &[u8; 32]) -> [u8; 32] {
    let mut nine = [0u8; 32];
    nine[0] = 9;
    x25519(private, &nine)
}

/// The shared secret, or None when the other side's key makes it all zeros (RFC 7748 asks to check).
pub fn shared(private: &[u8; 32], theirs: &[u8; 32]) -> Option<[u8; 32]> {
    let s = x25519(private, theirs);
    let mut any = 0u8;
    for b in s {
        any |= b;
    }
    (any != 0).then_some(s)
}
