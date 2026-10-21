//! A generator of random bytes for keys: ChaCha20's key stream, its key replaced after every request ("fast key
//! erasure"), so what it gave out can't be worked out later even from the generator's state. Seeded (and
//! reseeded) by the kernel from the processor's random number instructions and timing jitter.

use crate::chacha20poly1305::block;
use crate::{Hash, Sha256};

pub struct Drbg {
    key: [u8; 32],
}

impl Drop for Drbg {
    fn drop(&mut self) {
        unsafe { core::ptr::write_volatile(&mut self.key, [0; 32]) };
    }
}

impl Drbg {
    pub fn new(seed: &[u8]) -> Drbg {
        let mut d = Drbg { key: [0; 32] };
        d.reseed(seed);
        d
    }

    /// Mixes more hard-to-guess bytes in.
    pub fn reseed(&mut self, seed: &[u8]) {
        let mut h = Sha256::new();
        h.update(b"Chat Computer random bytes");
        h.update(&self.key);
        h.update(seed);
        self.key.copy_from_slice(&h.finish());
    }

    pub fn fill(&mut self, out: &mut [u8]) {
        let nonce = [0u8; 12];
        let first = block(&self.key, 0, &nonce);
        let mut next_key = [0u8; 32];
        next_key.copy_from_slice(&first[..32]);
        let mut at = 0;
        let mut counter = 1u32;
        // (the first block's other half goes out too)
        let n = out.len().min(32);
        out[..n].copy_from_slice(&first[32..32 + n]);
        at += n;
        while at < out.len() {
            let b = block(&self.key, counter, &nonce);
            let n = (out.len() - at).min(64);
            out[at..at + n].copy_from_slice(&b[..n]);
            at += n;
            counter += 1;
        }
        self.key = next_key;
    }

    pub fn bytes<const N: usize>(&mut self) -> [u8; N] {
        let mut b = [0u8; N];
        self.fill(&mut b);
        b
    }
}
