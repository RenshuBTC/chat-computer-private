//! Chat Computer's own cryptography, for its own TLS 1.3: hashes (SHA-256, SHA-384, SHA-512), HMAC and HKDF,
//! the ciphers TLS 1.3 uses (AES-GCM and ChaCha20-Poly1305), X25519 for agreeing keys, and checking
//! signatures (ECDSA on P-256 and P-384, RSA with PKCS #1 v1.5 and PSS). No code from other projects: each
//! is written from its specification and checked against the specification's own test vectors.
//!
//! What handles secrets (the ciphers, X25519) takes the same time whatever the secret, using no secret as an
//! array index or a branch condition, except where noted (aes_tables: AES with tables, both ways, for data kept on
//! the computer itself, where nothing else runs). Signature checks only ever see public values.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod aes;
pub mod aes_tables;
pub mod bignum;
pub mod chacha20poly1305;
pub mod drbg;
pub mod ecdsa;
pub mod gcm;
pub mod rsa;
pub mod sha2;
pub mod x25519;

use core::ops::Deref;

pub use sha2::{Sha256, Sha384, Sha512};

/// A hash function.
pub trait Hash: Clone {
    /// The size of its blocks, in bytes (HMAC needs it).
    const BLOCK: usize;
    /// The size of its output.
    const OUT: usize;
    fn new() -> Self;
    fn update(&mut self, data: &[u8]);
    /// Writes the hash in the first `OUT` bytes of `out`.
    fn finish_into(self, out: &mut [u8]);

    fn finish(self) -> Digest {
        let mut d = Digest { bytes: [0; 64], len: Self::OUT };
        self.finish_into(&mut d.bytes);
        d
    }

    fn digest(data: &[u8]) -> Digest {
        let mut h = Self::new();
        h.update(data);
        h.finish()
    }
}

/// A hash's output (up to 64 bytes).
#[derive(Clone, Copy)]
pub struct Digest {
    bytes: [u8; 64],
    len: usize,
}

impl Digest {
    pub fn from_slice(b: &[u8]) -> Digest {
        let mut d = Digest { bytes: [0; 64], len: b.len().min(64) };
        d.bytes[..d.len].copy_from_slice(&b[..d.len]);
        d
    }
}

impl Deref for Digest {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl core::fmt::Debug for Digest {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        for b in self.iter() {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    Sha256::digest(data).iter().zip(out.iter_mut()).for_each(|(a, b)| *b = *a);
    out
}

/// Whether two byte strings are equal, taking the same time wherever they differ.
pub fn equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut d = 0u8;
    for (x, y) in a.iter().zip(b) {
        d |= x ^ y;
    }
    core::hint::black_box(d) == 0
}

/// HMAC (RFC 2104) with hash `H`.
#[derive(Clone)]
pub struct Hmac<H: Hash> {
    inner: H,
    outer: H,
}

impl<H: Hash> Hmac<H> {
    pub fn new(key: &[u8]) -> Self {
        let mut k = [0u8; 128];
        if key.len() > H::BLOCK {
            H::digest(key).iter().zip(k.iter_mut()).for_each(|(a, b)| *b = *a);
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let mut ipad = [0u8; 128];
        let mut opad = [0u8; 128];
        for i in 0..H::BLOCK {
            ipad[i] = k[i] ^ 0x36;
            opad[i] = k[i] ^ 0x5c;
        }
        let mut inner = H::new();
        inner.update(&ipad[..H::BLOCK]);
        let mut outer = H::new();
        outer.update(&opad[..H::BLOCK]);
        Hmac { inner, outer }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    pub fn finish(self) -> Digest {
        let inner = self.inner.finish();
        let mut outer = self.outer;
        outer.update(&inner);
        outer.finish()
    }
}

pub fn hmac<H: Hash>(key: &[u8], data: &[u8]) -> Digest {
    let mut m = Hmac::<H>::new(key);
    m.update(data);
    m.finish()
}

/// HKDF's extract step (RFC 5869): a pseudorandom key from input keying material.
pub fn hkdf_extract<H: Hash>(salt: &[u8], ikm: &[u8]) -> Digest {
    let zeros = [0u8; 64];
    hmac::<H>(if salt.is_empty() { &zeros[..H::OUT] } else { salt }, ikm)
}

/// HKDF's expand step: fills `out` (at most 255 hash lengths).
pub fn hkdf_expand<H: Hash>(prk: &[u8], info: &[u8], out: &mut [u8]) {
    assert!(out.len() <= 255 * H::OUT);
    let mut previous: Option<Digest> = None;
    for (i, chunk) in out.chunks_mut(H::OUT).enumerate() {
        let mut m = Hmac::<H>::new(prk);
        if let Some(p) = &previous {
            m.update(p);
        }
        m.update(info);
        m.update(&[(i + 1) as u8]);
        let t = m.finish();
        chunk.copy_from_slice(&t[..chunk.len()]);
        previous = Some(t);
    }
}

#[cfg(test)]
mod tests;
