//! Checking RSA signatures (RFC 8017): PKCS #1 v1.5, which certificates are mostly signed with, and PSS, which
//! TLS 1.3 servers with RSA keys sign their handshakes with. Keys of 2048 to 8192 bits.

use crate::bignum::{self, Modulus};
use crate::{Hash, Sha256, Sha384, Sha512};
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Digest {
    Sha256,
    Sha384,
    Sha512,
}

impl Digest {
    pub fn hash(self, data: &[u8]) -> Vec<u8> {
        match self {
            Digest::Sha256 => Sha256::digest(data).to_vec(),
            Digest::Sha384 => Sha384::digest(data).to_vec(),
            Digest::Sha512 => Sha512::digest(data).to_vec(),
        }
    }

    fn len(self) -> usize {
        match self {
            Digest::Sha256 => 32,
            Digest::Sha384 => 48,
            Digest::Sha512 => 64,
        }
    }

    /// DER's DigestInfo before the hash, for PKCS #1 v1.5.
    fn prefix(self) -> &'static [u8] {
        match self {
            Digest::Sha256 => &[0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00, 0x04, 0x20],
            Digest::Sha384 => &[0x30, 0x41, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02, 0x05, 0x00, 0x04, 0x30],
            Digest::Sha512 => &[0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03, 0x05, 0x00, 0x04, 0x40],
        }
    }
}

/// signature^e mod n, as many bytes as n has; None for a signature that isn't a number below n, or a key
/// that's too small or too big.
fn open(n: &[u8], e: &[u8], signature: &[u8]) -> Option<Vec<u8>> {
    let n = &n[n.iter().position(|&b| b != 0)?..];
    if n.len() < 256 || n.len() > 1024 || signature.len() != n.len() || e.is_empty() || e.len() > 8 {
        return None;
    }
    let limbs = n.len().div_ceil(8);
    let m = Modulus::new(bignum::from_bytes(n, limbs)?)?;
    let s = bignum::from_bytes(signature, limbs)?;
    if !bignum::less(&s, &m.m) {
        return None;
    }
    Some(bignum::to_bytes(&m.pow(&s, e), n.len()))
}

/// PKCS #1 v1.5: 00 01 FF..FF 00 DigestInfo hash.
pub fn verify_pkcs1(n: &[u8], e: &[u8], digest: Digest, message: &[u8], signature: &[u8]) -> bool {
    let Some(em) = open(n, e, signature) else { return false };
    let hash = digest.hash(message);
    let t_len = digest.prefix().len() + hash.len();
    if em.len() < t_len + 11 {
        return false;
    }
    let mut want = vec![0xFFu8; em.len()];
    want[0] = 0;
    want[1] = 1;
    let at = em.len() - t_len;
    want[at - 1] = 0;
    want[at..at + digest.prefix().len()].copy_from_slice(digest.prefix());
    want[em.len() - hash.len()..].copy_from_slice(&hash);
    crate::equal(&em, &want)
}

fn mgf1(digest: Digest, seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 64);
    let mut counter = 0u32;
    while out.len() < len {
        let mut input = seed.to_vec();
        input.extend_from_slice(&counter.to_be_bytes());
        out.extend_from_slice(&digest.hash(&input));
        counter += 1;
    }
    out.truncate(len);
    out
}

/// PSS with MGF1 of the same hash and a salt as long as the hash (as TLS 1.3 requires).
pub fn verify_pss(n: &[u8], e: &[u8], digest: Digest, message: &[u8], signature: &[u8]) -> bool {
    let Some(m) = open(n, e, signature) else { return false };
    let n_trimmed = &n[n.iter().position(|&b| b != 0).unwrap_or(0)..];
    let mod_bits = 8 * n_trimmed.len() - n_trimmed[0].leading_zeros() as usize;
    let em_bits = mod_bits - 1;
    let em_len = em_bits.div_ceil(8);
    // m has as many bytes as n; EM is its last em_len (the first byte must be 0 when they differ).
    if m.len() > em_len && m[..m.len() - em_len].iter().any(|&b| b != 0) {
        return false;
    }
    let em = &m[m.len() - em_len..];
    let h_len = digest.len();
    let s_len = h_len;
    if em_len < h_len + s_len + 2 || em[em_len - 1] != 0xbc {
        return false;
    }
    let db_len = em_len - h_len - 1;
    let (masked, h) = (&em[..db_len], &em[db_len..em_len - 1]);
    let top_bits = 8 * em_len - em_bits;
    if top_bits > 0 && masked[0] >> (8 - top_bits) != 0 {
        return false;
    }
    let mut db: Vec<u8> = masked.iter().zip(mgf1(digest, h, db_len)).map(|(a, b)| a ^ b).collect();
    if top_bits > 0 {
        db[0] &= 0xFF >> top_bits;
    }
    let ps = em_len - h_len - s_len - 2;
    if db[..ps].iter().any(|&b| b != 0) || db[ps] != 1 {
        return false;
    }
    let salt = &db[db_len - s_len..];
    let mut m2 = vec![0u8; 8];
    m2.extend_from_slice(&digest.hash(message));
    m2.extend_from_slice(salt);
    crate::equal(&digest.hash(&m2), h)
}
