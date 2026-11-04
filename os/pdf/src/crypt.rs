//! Encrypted PDFs that open without a password (ISO 32000-2, 7.6): the standard security handler's files with only an
//! owner's password (which limits printing, copying or changing them, as many banks' and offices' documents have),
//! which every reader opens as they are. The key for an empty password is worked out and checked against the file's
//! (revisions 2 to 4: MD5 and RC4, 40 to 128 bits; 5 and 6: SHA-256, SHA-384, SHA-512 and AES-256), then each object's
//! strings and streams are decrypted with it (RC4, or AES-128 or AES-256 in CBC mode). A file that needs a password to
//! open stays shut: no password is ever guessed.

use crate::lex::{Dict, Obj};
use alloc::vec::Vec;
use crypto::aes_tables::Aes;
use crypto::{Hash, Sha256, Sha384, Sha512};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    /// Not encrypted.
    Identity,
    Rc4,
    Aes128,
    Aes256,
}

pub struct Crypt {
    key: Vec<u8>,
    strings: Method,
    streams: Method,
    /// Whether the document's metadata (XMP) is encrypted.
    pub metadata: bool,
    /// The encryption dictionary's own object (its strings aren't encrypted).
    pub own: Option<u32>,
}

/// Algorithm 2's padding: what an empty password is made up to 32 bytes with.
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6,
    0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

impl Crypt {
    /// The file's key for an empty password, if it opens with one: from its encryption dictionary (`encrypt`, its
    /// references already followed by `resolve`) and the first of its IDs.
    pub fn open(encrypt: &Dict, id: &[u8], own: Option<u32>, resolve: &dyn Fn(&Obj) -> Obj) -> Option<Crypt> {
        let get = |k: &str| encrypt.get(k).map(resolve).unwrap_or(Obj::Null);
        if get("Filter").name() != Some(b"Standard") {
            return None;
        }
        let v = get("V").int().unwrap_or(0);
        let r = get("R").int().unwrap_or(0);
        let o = get("O").string().map(<[u8]>::to_vec).unwrap_or_default();
        let u = get("U").string().map(<[u8]>::to_vec).unwrap_or_default();
        let metadata = !matches!(get("EncryptMetadata"), Obj::Bool(false));
        // (V 4 and 5 name their methods for strings and streams in /CF.)
        let named = |key: &str, default: Method| -> Method {
            if v < 4 {
                return default;
            }
            let Some(name) = get(key).name().map(<[u8]>::to_vec) else { return Method::Identity };
            if name == b"Identity" {
                return Method::Identity;
            }
            let cf = get("CF");
            let filter = cf.dict().and_then(|d| d.get(core::str::from_utf8(&name).ok()?)).map(resolve);
            match filter.as_ref().and_then(|f| f.dict()).and_then(|f| f.get("CFM")).map(resolve).and_then(|m| m.name().map(<[u8]>::to_vec)) {
                Some(m) if m == b"V2" => Method::Rc4,
                Some(m) if m == b"AESV2" => Method::Aes128,
                Some(m) if m == b"AESV3" => Method::Aes256,
                Some(m) if m == b"None" => Method::Identity,
                _ => default,
            }
        };
        let key = match r {
            2..=4 => {
                if o.len() < 32 || u.len() < 32 {
                    return None;
                }
                let p = (get("P").int().unwrap_or(0) as i32 as u32).to_le_bytes();
                let bits = get("Length").int().unwrap_or(40);
                let n = if r == 2 { 5 } else if v >= 4 { 16 } else { (bits / 8).clamp(5, 16) as usize };
                let mut md5 = Md5::new();
                md5.update(&PAD);
                md5.update(&o[..32]);
                md5.update(&p);
                md5.update(id);
                if r >= 4 && !metadata {
                    md5.update(&[0xFF; 4]);
                }
                let mut key = md5.finish().to_vec();
                if r >= 3 {
                    for _ in 0..50 {
                        key = md5_of(&key[..n]).to_vec();
                    }
                }
                key.truncate(n);
                // The user's password checked (algorithms 4 and 5): an empty one opens it, or nothing does.
                let opens = if r == 2 {
                    rc4(&key, &PAD) == u[..32]
                } else {
                    let mut md5 = Md5::new();
                    md5.update(&PAD);
                    md5.update(id);
                    let mut x = rc4(&key, &md5.finish());
                    for i in 1..=19u8 {
                        let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
                        x = rc4(&k, &x);
                    }
                    x[..] == u[..16]
                };
                if !opens {
                    return None;
                }
                key
            }
            5 | 6 => {
                let ue = get("UE").string().map(<[u8]>::to_vec).unwrap_or_default();
                if u.len() < 48 || ue.len() < 32 {
                    return None;
                }
                let hash = |salt: &[u8]| if r == 5 { crypto::sha256(salt).to_vec() } else { hash_2b(b"", salt, b"") };
                if hash(&u[32..40]) != u[..32] {
                    return None;
                }
                let middle = hash(&u[40..48]);
                let mut key = cbc_decrypt(&middle, &[0; 16], &ue[..32]);
                key.truncate(32);
                if key.len() != 32 {
                    return None;
                }
                key
            }
            _ => return None,
        };
        let default = if r >= 5 { Method::Aes256 } else { Method::Rc4 };
        Some(Crypt { key, strings: named("StrF", default), streams: named("StmF", default), metadata, own })
    }

    /// Object `n` (generation `g`)'s string, decrypted.
    pub fn string(&self, n: u32, g: u16, s: &[u8]) -> Vec<u8> {
        self.decrypt(self.strings, n, g, s)
    }

    /// Object `n` (generation `g`)'s stream data, decrypted.
    pub fn stream(&self, n: u32, g: u16, d: &[u8]) -> Vec<u8> {
        self.decrypt(self.streams, n, g, d)
    }

    /// Every string in `o` (object `n`'s value), decrypted.
    pub fn strings_in(&self, n: u32, g: u16, o: &mut Obj) {
        if self.own == Some(n) || self.strings == Method::Identity {
            return;
        }
        match o {
            Obj::Str(s) => *s = self.string(n, g, s),
            Obj::Array(items) => items.iter_mut().for_each(|i| self.strings_in(n, g, i)),
            Obj::Dict(d) | Obj::Stream(d, _) => d.0.iter_mut().for_each(|(_, v)| self.strings_in(n, g, v)),
            _ => {}
        }
    }

    fn decrypt(&self, method: Method, n: u32, g: u16, data: &[u8]) -> Vec<u8> {
        // Algorithm 1: each object's own key (but AES-256's, the file's).
        let object_key = |aes: bool| {
            let mut md5 = Md5::new();
            md5.update(&self.key);
            md5.update(&n.to_le_bytes()[..3]);
            md5.update(&g.to_le_bytes());
            if aes {
                md5.update(b"sAlT");
            }
            let mut k = md5.finish().to_vec();
            k.truncate((self.key.len() + 5).min(16));
            k
        };
        match method {
            Method::Identity => data.to_vec(),
            Method::Rc4 => rc4(&object_key(false), data),
            Method::Aes128 => aes_data(&object_key(true), data),
            Method::Aes256 => aes_data(&self.key, data),
        }
    }
}

/// AES data as PDF has it: a 16-byte IV, then CBC's blocks, the last padded (PKCS #7: its padding taken off).
fn aes_data(key: &[u8], data: &[u8]) -> Vec<u8> {
    if data.len() < 32 {
        return Vec::new();
    }
    let (iv, rest) = data.split_at(16);
    let mut out = cbc_decrypt(key, iv, &rest[..rest.len() / 16 * 16]);
    if let Some(&pad) = out.last()
        && (1..=16).contains(&pad)
        && out.len() >= pad as usize
        && out[out.len() - pad as usize..].iter().all(|&b| b == pad)
    {
        out.truncate(out.len() - pad as usize);
    }
    out
}

/// AES-CBC decrypted (whole blocks only; nothing for a key AES doesn't take).
fn cbc_decrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Vec<u8> {
    let Some(aes) = Aes::new(key) else { return Vec::new() };
    let mut prev = [0u8; 16];
    prev.copy_from_slice(&iv[..16]);
    let mut out = Vec::with_capacity(data.len());
    for block in data.chunks_exact(16) {
        let mut b = [0u8; 16];
        b.copy_from_slice(block);
        aes.decrypt(&mut b);
        for (x, p) in b.iter_mut().zip(prev) {
            *x ^= p;
        }
        out.extend_from_slice(&b);
        prev.copy_from_slice(block);
    }
    out
}

/// AES-128-CBC encrypted, no padding (whole blocks).
fn cbc_encrypt(aes: &Aes, iv: &[u8], data: &[u8]) -> Vec<u8> {
    let mut prev = [0u8; 16];
    prev.copy_from_slice(&iv[..16]);
    let mut out = Vec::with_capacity(data.len());
    for block in data.chunks_exact(16) {
        let mut b = [0u8; 16];
        for i in 0..16 {
            b[i] = block[i] ^ prev[i];
        }
        aes.encrypt(&mut b);
        out.extend_from_slice(&b);
        prev = b;
    }
    out
}

/// Algorithm 2.B (revision 6): a password's hash, with a salt (and, for the owner's, the user's key).
fn hash_2b(password: &[u8], salt: &[u8], user: &[u8]) -> Vec<u8> {
    let mut first = Sha256::new();
    first.update(password);
    first.update(salt);
    first.update(user);
    let mut k: Vec<u8> = first.finish().to_vec();
    let mut round = 0u32;
    loop {
        let mut k1 = Vec::with_capacity(64 * (password.len() + k.len() + user.len()));
        for _ in 0..64 {
            k1.extend_from_slice(password);
            k1.extend_from_slice(&k);
            k1.extend_from_slice(user);
        }
        let Some(aes) = Aes::new(&k[..16]) else { return Vec::new() };
        let e = cbc_encrypt(&aes, &k[16..32], &k1);
        // (The first 16 bytes as a number, modulo 3: as 256 is 1 modulo 3, their sum's.)
        k = match e[..16].iter().map(|&b| b as u32).sum::<u32>() % 3 {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        round += 1;
        let last = *e.last().unwrap_or(&0) as u32;
        if round >= 64 && last + 32 <= round {
            break;
        }
        if round > 4096 {
            break;
        }
    }
    k.truncate(32);
    k
}

/// RC4 (an empty key leaves the data as it is).
pub fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    let mut s: [u8; 256] = core::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|&b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

/// MD5 (RFC 1321), which PDF's older encryption works out its keys with.
pub struct Md5 {
    state: [u32; 4],
    block: [u8; 64],
    used: usize,
    length: u64,
}

const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];
const MD5_S: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];

impl Md5 {
    pub fn new() -> Md5 {
        Md5 { state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476], block: [0; 64], used: 0, length: 0 }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.length = self.length.wrapping_add(data.len() as u64);
        while !data.is_empty() {
            let n = (64 - self.used).min(data.len());
            self.block[self.used..self.used + n].copy_from_slice(&data[..n]);
            self.used += n;
            data = &data[n..];
            if self.used == 64 {
                let block = self.block;
                self.compress(&block);
                self.used = 0;
            }
        }
    }

    pub fn finish(mut self) -> [u8; 16] {
        let bits = self.length.wrapping_mul(8);
        self.update(&[0x80]);
        while self.used != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_le_bytes());
        let mut out = [0u8; 16];
        for (i, s) in self.state.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&s.to_le_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let m: [u32; 16] = core::array::from_fn(|i| u32::from_le_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]));
        let [mut a, mut b, mut c, mut d] = self.state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(MD5_K[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(MD5_S[(i / 16) * 4 + i % 4]));
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
}

pub fn md5_of(data: &[u8]) -> [u8; 16] {
    let mut m = Md5::new();
    m.update(data);
    m.finish()
}
