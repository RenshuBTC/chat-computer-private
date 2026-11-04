//! AES (FIPS 197) both ways, with tables: fast enough to read a drive through (aes.rs, which TLS uses, computes its
//! S-box to keep its timing secret, and only encrypts). For data kept on the person's own computer, which runs nothing
//! but Chat Computer, so nothing could watch the caches: BitLocker's drives, encrypted PDFs. The tables are worked out
//! when Chat Computer is built.

/// Multiplication in GF(2^8) modulo x^8 + x^4 + x^3 + x + 1.
const fn mul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        a = (a << 1) ^ if a & 0x80 != 0 { 0x1b } else { 0 };
        b >>= 1;
    }
    p
}

const fn sbox() -> [u8; 256] {
    // Powers of 3 (a generator of GF(2^8)'s multiplicative group) and their logarithms: x's inverse is 3^(255 − log x).
    let mut exp = [0u8; 256];
    let mut log = [0u8; 256];
    let (mut x, mut i) = (1u8, 0usize);
    while i < 255 {
        exp[i] = x;
        log[x as usize] = i as u8;
        x = mul(x, 3);
        i += 1;
    }
    let mut s = [0u8; 256];
    let mut v = 0usize;
    while v < 256 {
        let inv = if v == 0 { 0 } else { exp[(255 - log[v] as usize) % 255] };
        s[v] = inv ^ inv.rotate_left(1) ^ inv.rotate_left(2) ^ inv.rotate_left(3) ^ inv.rotate_left(4) ^ 0x63;
        v += 1;
    }
    s
}

const S: [u8; 256] = sbox();

const fn inverse(s: &[u8; 256]) -> [u8; 256] {
    let mut out = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        out[s[i] as usize] = i as u8;
        i += 1;
    }
    out
}

const SI: [u8; 256] = inverse(&S);

/// A round's SubBytes, ShiftRows and MixColumns for one byte, as a column (big-endian), rotated for each row.
const fn tables(decrypt: bool) -> [[u32; 256]; 4] {
    let mut t = [[0u32; 256]; 4];
    let mut x = 0usize;
    while x < 256 {
        let w = if decrypt {
            let s = SI[x];
            (mul(s, 14) as u32) << 24 | (mul(s, 9) as u32) << 16 | (mul(s, 13) as u32) << 8 | mul(s, 11) as u32
        } else {
            let s = S[x];
            (mul(s, 2) as u32) << 24 | (s as u32) << 16 | (s as u32) << 8 | mul(s, 3) as u32
        };
        t[0][x] = w;
        t[1][x] = w.rotate_right(8);
        t[2][x] = w.rotate_right(16);
        t[3][x] = w.rotate_right(24);
        x += 1;
    }
    t
}

const TE: [[u32; 256]; 4] = tables(false);
const TD: [[u32; 256]; 4] = tables(true);

#[derive(Clone)]
pub struct Aes {
    rounds: usize,
    ek: [u32; 60],
    dk: [u32; 60],
}

impl Drop for Aes {
    fn drop(&mut self) {
        for w in self.ek.iter_mut().chain(self.dk.iter_mut()) {
            unsafe { core::ptr::write_volatile(w, 0) };
        }
    }
}

fn sub_word(w: u32) -> u32 {
    (S[(w >> 24) as usize] as u32) << 24 | (S[(w >> 16 & 0xff) as usize] as u32) << 16 | (S[(w >> 8 & 0xff) as usize] as u32) << 8 | S[(w & 0xff) as usize] as u32
}

impl Aes {
    /// A 16-, 24- or 32-byte key (None for any other length).
    pub fn new(key: &[u8]) -> Option<Aes> {
        if !matches!(key.len(), 16 | 24 | 32) {
            return None;
        }
        let nk = key.len() / 4;
        let rounds = nk + 6;
        let mut ek = [0u32; 60];
        for (i, w) in ek.iter_mut().take(nk).enumerate() {
            *w = u32::from_be_bytes(key[4 * i..4 * i + 4].try_into().unwrap());
        }
        let mut rcon = 1u8;
        for i in nk..4 * (rounds + 1) {
            let mut t = ek[i - 1];
            if i % nk == 0 {
                t = sub_word(t.rotate_left(8)) ^ (rcon as u32) << 24;
                rcon = mul(rcon, 2);
            } else if nk > 6 && i % nk == 4 {
                t = sub_word(t);
            }
            ek[i] = ek[i - nk] ^ t;
        }
        // The equivalent inverse cipher's keys: in reverse, the middle rounds' through InvMixColumns.
        let mut dk = [0u32; 60];
        for r in 0..=rounds {
            for c in 0..4 {
                let w = ek[4 * (rounds - r) + c];
                dk[4 * r + c] = if r == 0 || r == rounds {
                    w
                } else {
                    TD[0][S[(w >> 24) as usize] as usize]
                        ^ TD[1][S[(w >> 16 & 0xff) as usize] as usize]
                        ^ TD[2][S[(w >> 8 & 0xff) as usize] as usize]
                        ^ TD[3][S[(w & 0xff) as usize] as usize]
                };
            }
        }
        Some(Aes { rounds, ek, dk })
    }

    pub fn encrypt(&self, block: &mut [u8; 16]) {
        let k = &self.ek;
        let mut s: [u32; 4] = core::array::from_fn(|c| u32::from_be_bytes(block[4 * c..4 * c + 4].try_into().unwrap()) ^ k[c]);
        for r in 1..self.rounds {
            let b = |w: u32, n: u32| (w >> n & 0xff) as usize;
            s = core::array::from_fn(|c| {
                TE[0][b(s[c], 24)] ^ TE[1][b(s[(c + 1) % 4], 16)] ^ TE[2][b(s[(c + 2) % 4], 8)] ^ TE[3][b(s[(c + 3) % 4], 0)] ^ k[4 * r + c]
            });
        }
        let r = self.rounds;
        for c in 0..4 {
            let w = (S[(s[c] >> 24) as usize] as u32) << 24
                | (S[(s[(c + 1) % 4] >> 16 & 0xff) as usize] as u32) << 16
                | (S[(s[(c + 2) % 4] >> 8 & 0xff) as usize] as u32) << 8
                | S[(s[(c + 3) % 4] & 0xff) as usize] as u32;
            block[4 * c..4 * c + 4].copy_from_slice(&(w ^ k[4 * r + c]).to_be_bytes());
        }
    }

    pub fn decrypt(&self, block: &mut [u8; 16]) {
        let k = &self.dk;
        let mut s: [u32; 4] = core::array::from_fn(|c| u32::from_be_bytes(block[4 * c..4 * c + 4].try_into().unwrap()) ^ k[c]);
        for r in 1..self.rounds {
            let b = |w: u32, n: u32| (w >> n & 0xff) as usize;
            // (InvShiftRows takes row i of column c from column c − i.)
            s = core::array::from_fn(|c| {
                TD[0][b(s[c], 24)] ^ TD[1][b(s[(c + 3) % 4], 16)] ^ TD[2][b(s[(c + 2) % 4], 8)] ^ TD[3][b(s[(c + 1) % 4], 0)] ^ k[4 * r + c]
            });
        }
        let r = self.rounds;
        for c in 0..4 {
            let w = (SI[(s[c] >> 24) as usize] as u32) << 24
                | (SI[(s[(c + 3) % 4] >> 16 & 0xff) as usize] as u32) << 16
                | (SI[(s[(c + 2) % 4] >> 8 & 0xff) as usize] as u32) << 8
                | SI[(s[(c + 1) % 4] & 0xff) as usize] as u32;
            block[4 * c..4 * c + 4].copy_from_slice(&(w ^ k[4 * r + c]).to_be_bytes());
        }
    }
}
