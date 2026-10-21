//! ChaCha20 and Poly1305, and the two together as an AEAD (RFC 8439). Additions, rotations and exclusive-ors
//! only: the same time whatever the key or the data.

fn quarter(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(7);
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// One 64-byte block of ChaCha20's key stream.
pub fn block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut s = [0u32; 16];
    s[0] = 0x6170_7865;
    s[1] = 0x3320_646e;
    s[2] = 0x7962_2d32;
    s[3] = 0x6b20_6574;
    for i in 0..8 {
        s[4 + i] = le32(&key[4 * i..]);
    }
    s[12] = counter;
    for i in 0..3 {
        s[13 + i] = le32(&nonce[4 * i..]);
    }
    let start = s;
    for _ in 0..10 {
        quarter(&mut s, 0, 4, 8, 12);
        quarter(&mut s, 1, 5, 9, 13);
        quarter(&mut s, 2, 6, 10, 14);
        quarter(&mut s, 3, 7, 11, 15);
        quarter(&mut s, 0, 5, 10, 15);
        quarter(&mut s, 1, 6, 11, 12);
        quarter(&mut s, 2, 7, 8, 13);
        quarter(&mut s, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[4 * i..4 * i + 4].copy_from_slice(&s[i].wrapping_add(start[i]).to_le_bytes());
    }
    out
}

/// Encrypts or decrypts `data` where it is, its key stream starting at block `counter`.
pub fn chacha20(key: &[u8; 32], counter: u32, nonce: &[u8; 12], data: &mut [u8]) {
    for (i, chunk) in data.chunks_mut(64).enumerate() {
        let k = block(key, counter.wrapping_add(i as u32), nonce);
        for (b, x) in chunk.iter_mut().zip(k) {
            *b ^= x;
        }
    }
}

/// Poly1305, in 26-bit limbs.
pub struct Poly1305 {
    r: [u32; 5],
    s: [u32; 4],
    h: [u32; 5],
    buf: [u8; 16],
    used: usize,
}

impl Poly1305 {
    pub fn new(key: &[u8; 32]) -> Poly1305 {
        let r = [
            le32(&key[0..]) & 0x3ff_ffff,
            (le32(&key[3..]) >> 2) & 0x3ff_ff03,
            (le32(&key[6..]) >> 4) & 0x3ff_c0ff,
            (le32(&key[9..]) >> 6) & 0x3f0_3fff,
            (le32(&key[12..]) >> 8) & 0x00f_ffff,
        ];
        let s = [le32(&key[16..]), le32(&key[20..]), le32(&key[24..]), le32(&key[28..])];
        Poly1305 { r, s, h: [0; 5], buf: [0; 16], used: 0 }
    }

    /// One 16-byte block; `hibit`: 1 << 24 for a whole block, 0 for the last, padded one.
    fn block(&mut self, m: &[u8; 16], hibit: u32) {
        let [r0, r1, r2, r3, r4] = self.r.map(|x| x as u64);
        let (s1, s2, s3, s4) = (r1 * 5, r2 * 5, r3 * 5, r4 * 5);
        let mut h = self.h.map(|x| x as u64);
        h[0] += (le32(&m[0..]) & 0x3ff_ffff) as u64;
        h[1] += ((le32(&m[3..]) >> 2) & 0x3ff_ffff) as u64;
        h[2] += ((le32(&m[6..]) >> 4) & 0x3ff_ffff) as u64;
        h[3] += ((le32(&m[9..]) >> 6) & 0x3ff_ffff) as u64;
        h[4] += ((le32(&m[12..]) >> 8) | hibit) as u64;
        let d0 = h[0] * r0 + h[1] * s4 + h[2] * s3 + h[3] * s2 + h[4] * s1;
        let mut d1 = h[0] * r1 + h[1] * r0 + h[2] * s4 + h[3] * s3 + h[4] * s2;
        let mut d2 = h[0] * r2 + h[1] * r1 + h[2] * r0 + h[3] * s4 + h[4] * s3;
        let mut d3 = h[0] * r3 + h[1] * r2 + h[2] * r1 + h[3] * r0 + h[4] * s4;
        let mut d4 = h[0] * r4 + h[1] * r3 + h[2] * r2 + h[3] * r1 + h[4] * r0;
        let mut c = d0 >> 26;
        h[0] = d0 & 0x3ff_ffff;
        d1 += c;
        c = d1 >> 26;
        h[1] = d1 & 0x3ff_ffff;
        d2 += c;
        c = d2 >> 26;
        h[2] = d2 & 0x3ff_ffff;
        d3 += c;
        c = d3 >> 26;
        h[3] = d3 & 0x3ff_ffff;
        d4 += c;
        c = d4 >> 26;
        h[4] = d4 & 0x3ff_ffff;
        h[0] += c * 5;
        c = h[0] >> 26;
        h[0] &= 0x3ff_ffff;
        h[1] += c;
        self.h = h.map(|x| x as u32);
    }

    pub fn update(&mut self, mut data: &[u8]) {
        if self.used > 0 {
            let n = (16 - self.used).min(data.len());
            self.buf[self.used..self.used + n].copy_from_slice(&data[..n]);
            self.used += n;
            data = &data[n..];
            if self.used < 16 {
                return;
            }
            let b = self.buf;
            self.block(&b, 1 << 24);
            self.used = 0;
        }
        while data.len() >= 16 {
            let mut b = [0u8; 16];
            b.copy_from_slice(&data[..16]);
            self.block(&b, 1 << 24);
            data = &data[16..];
        }
        self.buf[..data.len()].copy_from_slice(data);
        self.used = data.len();
    }

    pub fn finish(mut self) -> [u8; 16] {
        if self.used > 0 {
            let mut b = [0u8; 16];
            b[..self.used].copy_from_slice(&self.buf[..self.used]);
            b[self.used] = 1;
            self.block(&b, 0);
        }
        let mut h = self.h;
        // Fully carried.
        let mut c = h[1] >> 26;
        h[1] &= 0x3ff_ffff;
        h[2] += c;
        c = h[2] >> 26;
        h[2] &= 0x3ff_ffff;
        h[3] += c;
        c = h[3] >> 26;
        h[3] &= 0x3ff_ffff;
        h[4] += c;
        c = h[4] >> 26;
        h[4] &= 0x3ff_ffff;
        h[0] += c * 5;
        c = h[0] >> 26;
        h[0] &= 0x3ff_ffff;
        h[1] += c;
        // h - p, taken if it doesn't go below zero (chosen by mask, not by branch).
        let mut g = [0u32; 5];
        g[0] = h[0].wrapping_add(5);
        c = g[0] >> 26;
        g[0] &= 0x3ff_ffff;
        g[1] = h[1].wrapping_add(c);
        c = g[1] >> 26;
        g[1] &= 0x3ff_ffff;
        g[2] = h[2].wrapping_add(c);
        c = g[2] >> 26;
        g[2] &= 0x3ff_ffff;
        g[3] = h[3].wrapping_add(c);
        c = g[3] >> 26;
        g[3] &= 0x3ff_ffff;
        g[4] = h[4].wrapping_add(c).wrapping_sub(1 << 26);
        let mask = (g[4] >> 31).wrapping_sub(1);
        for i in 0..5 {
            h[i] = (h[i] & !mask) | (g[i] & mask);
        }
        // To 128 bits, plus s.
        let h0 = (h[0] | (h[1] << 26)) as u64;
        let h1 = ((h[1] >> 6) | (h[2] << 20)) as u64;
        let h2 = ((h[2] >> 12) | (h[3] << 14)) as u64;
        let h3 = ((h[3] >> 18) | (h[4] << 8)) as u64;
        let mut f = h0 + self.s[0] as u64;
        let w0 = f as u32;
        f = h1 + self.s[1] as u64 + (f >> 32);
        let w1 = f as u32;
        f = h2 + self.s[2] as u64 + (f >> 32);
        let w2 = f as u32;
        f = h3 + self.s[3] as u64 + (f >> 32);
        let w3 = f as u32;
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&w0.to_le_bytes());
        out[4..8].copy_from_slice(&w1.to_le_bytes());
        out[8..12].copy_from_slice(&w2.to_le_bytes());
        out[12..16].copy_from_slice(&w3.to_le_bytes());
        out
    }
}

pub struct ChaCha20Poly1305 {
    key: [u8; 32],
}

impl Drop for ChaCha20Poly1305 {
    fn drop(&mut self) {
        unsafe { core::ptr::write_volatile(&mut self.key, [0; 32]) };
    }
}

impl ChaCha20Poly1305 {
    pub fn new(key: &[u8]) -> ChaCha20Poly1305 {
        let mut k = [0u8; 32];
        k.copy_from_slice(&key[..32]);
        ChaCha20Poly1305 { key: k }
    }

    fn tag(&self, nonce: &[u8; 12], aad: &[u8], ciphertext: &[u8]) -> [u8; 16] {
        let block0 = block(&self.key, 0, nonce);
        let mut otk = [0u8; 32];
        otk.copy_from_slice(&block0[..32]);
        let mut p = Poly1305::new(&otk);
        let pad = [0u8; 16];
        p.update(aad);
        p.update(&pad[..(16 - aad.len() % 16) % 16]);
        p.update(ciphertext);
        p.update(&pad[..(16 - ciphertext.len() % 16) % 16]);
        p.update(&(aad.len() as u64).to_le_bytes());
        p.update(&(ciphertext.len() as u64).to_le_bytes());
        p.finish()
    }

    pub fn seal(&self, nonce: &[u8; 12], aad: &[u8], data: &mut [u8]) -> [u8; 16] {
        chacha20(&self.key, 1, nonce, data);
        self.tag(nonce, aad, data)
    }

    pub fn open(&self, nonce: &[u8; 12], aad: &[u8], data: &mut [u8], tag: &[u8]) -> bool {
        if !crate::equal(&self.tag(nonce, aad, data), tag) {
            return false;
        }
        chacha20(&self.key, 1, nonce, data);
        true
    }
}
