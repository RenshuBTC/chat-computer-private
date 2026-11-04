//! AES (FIPS 197), encryption only (GCM never decrypts with the block cipher), with 128- and 256-bit keys.
//!
//! The S-box is computed, not looked up: each byte's inverse in GF(2^8) (as x^254, by multiplications that
//! take the same steps whatever the bytes) and then the affine map. So no secret byte is ever an array index,
//! and how long it takes, or what's in the processor's caches, says nothing about the key or the data. It is
//! slower than a table (a few megabytes a second), which is plenty for a conversation.

/// Multiplication in GF(2^8) modulo x^8 + x^4 + x^3 + x + 1, in constant time.
fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    for _ in 0..8 {
        p ^= a & 0u8.wrapping_sub(b & 1);
        let hi = a >> 7;
        a = (a << 1) ^ (0x1b & 0u8.wrapping_sub(hi));
        b >>= 1;
    }
    p
}

/// The S-box: the inverse (0 for 0) then the affine transformation.
fn sbox(x: u8) -> u8 {
    // x^254 = x^-1: 254 = 11111110 in binary.
    let x2 = gmul(x, x);
    let x3 = gmul(x2, x);
    let x6 = gmul(x3, x3);
    let x12 = gmul(x6, x6);
    let x15 = gmul(x12, x3);
    let x30 = gmul(x15, x15);
    let x60 = gmul(x30, x30);
    let x120 = gmul(x60, x60);
    let x126 = gmul(x120, x6);
    let x127 = gmul(x126, x);
    let x254 = gmul(x127, x127);
    let b = x254;
    b ^ b.rotate_left(1) ^ b.rotate_left(2) ^ b.rotate_left(3) ^ b.rotate_left(4) ^ 0x63
}

fn xtime(x: u8) -> u8 {
    (x << 1) ^ (0x1b & 0u8.wrapping_sub(x >> 7))
}

#[derive(Clone)]
pub struct Aes {
    rounds: usize,
    keys: [[u8; 16]; 15],
}

impl Drop for Aes {
    fn drop(&mut self) {
        for k in self.keys.iter_mut() {
            unsafe { core::ptr::write_volatile(k, [0; 16]) };
        }
    }
}

impl Aes {
    /// A 16-, 24- or 32-byte key.
    pub fn new(key: &[u8]) -> Aes {
        let nk = key.len() / 4;
        assert!(matches!(key.len(), 16 | 24 | 32));
        let rounds = nk + 6;
        let mut w = [[0u8; 4]; 60];
        for (i, word) in w.iter_mut().take(nk).enumerate() {
            word.copy_from_slice(&key[4 * i..4 * i + 4]);
        }
        let mut rcon = 1u8;
        for i in nk..4 * (rounds + 1) {
            let mut t = w[i - 1];
            if i % nk == 0 {
                t = [sbox(t[1]) ^ rcon, sbox(t[2]), sbox(t[3]), sbox(t[0])];
                rcon = xtime(rcon);
            } else if nk > 6 && i % nk == 4 {
                t = [sbox(t[0]), sbox(t[1]), sbox(t[2]), sbox(t[3])];
            }
            for j in 0..4 {
                w[i][j] = w[i - nk][j] ^ t[j];
            }
        }
        let mut keys = [[0u8; 16]; 15];
        for (r, k) in keys.iter_mut().enumerate().take(rounds + 1) {
            for c in 0..4 {
                k[4 * c..4 * c + 4].copy_from_slice(&w[4 * r + c]);
            }
        }
        Aes { rounds, keys }
    }

    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        let mut s = *block;
        for (b, k) in s.iter_mut().zip(self.keys[0]) {
            *b ^= k;
        }
        for r in 1..=self.rounds {
            // SubBytes and ShiftRows together: row i of column c comes from column c + i.
            let mut t = [0u8; 16];
            for c in 0..4 {
                for i in 0..4 {
                    t[4 * c + i] = sbox(s[4 * ((c + i) % 4) + i]);
                }
            }
            if r < self.rounds {
                for c in 0..4 {
                    let a = [t[4 * c], t[4 * c + 1], t[4 * c + 2], t[4 * c + 3]];
                    let all = a[0] ^ a[1] ^ a[2] ^ a[3];
                    for i in 0..4 {
                        t[4 * c + i] = a[i] ^ all ^ xtime(a[i] ^ a[(i + 1) % 4]);
                    }
                }
            }
            for (b, (x, k)) in s.iter_mut().zip(t.iter().zip(self.keys[r])) {
                *b = x ^ k;
            }
        }
        *block = s;
    }
}
