//! ZIP archives, read, and DEFLATE (RFC 1951), the compression they use: what Word, Excel and PowerPoint files are (a
//! ZIP of XML files: Office Open XML), and what PNG pictures are compressed with (DEFLATE in zlib's wrapping, RFC
//! 1950).
//!
//! An archive ends with its "central directory": each file's name, sizes, CRC-32, how it's compressed, and where its
//! "local header", and after that its bytes, are. ZIP64's bigger numbers are read where the 32-bit ones are full.
//! DEFLATE is blocks, each stored as it is or Huffman-coded (with the standard's fixed codes, or with its own,
//! described at its start), of bytes and of copies of up to 258 bytes from up to 32 KiB back.
//!
//! As PKWARE's APPNOTE and the RFCs describe them; the decoder works as zlib's "puff" does, with a table for codes of
//! up to 9 bits. Every number is checked before it's used, and what comes out is capped, so a damaged archive gives an
//! error, never a crash, and one made to blow up (a "zip bomb") stops at the cap.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Not a ZIP archive (or not a zlib stream).
    NotZip,
    /// Something in it doesn't add up: a code that can't be, a copy from before the start, a CRC that doesn't match...
    Damaged,
    /// It would come out bigger than allowed.
    TooBig,
    /// Encrypted, with a password.
    Encrypted,
    /// Compressed some other way than DEFLATE: its method's number.
    Method(u16),
}

/// Where a stream's bits come from: bytes, each read from its lowest bit up.
struct Bits<'a> {
    input: &'a [u8],
    /// The next byte to read.
    at: usize,
    /// Bits read but not used yet, the next in the lowest place, and how many.
    buf: u64,
    count: u32,
}

impl Bits<'_> {
    /// As many bytes into `buf` as fit (fewer at the input's end).
    fn fill(&mut self) {
        while self.count <= 56 && self.at < self.input.len() {
            self.buf |= (self.input[self.at] as u64) << self.count;
            self.at += 1;
            self.count += 8;
        }
    }

    /// The next `n` bits (at most 32), the first in the lowest place.
    fn need(&mut self, n: u32) -> Result<u32, Error> {
        if self.count < n {
            self.fill();
            if self.count < n {
                return Err(Error::Damaged);
            }
        }
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.count -= n;
        Ok(v)
    }

    /// To the next byte's start; the whole bytes not used yet given back to the input.
    fn align(&mut self) {
        let partial = self.count % 8;
        self.buf >>= partial;
        self.count -= partial;
        self.at -= (self.count / 8) as usize;
        self.buf = 0;
        self.count = 0;
    }

    /// How many of the input's bytes have been used (a part-used one counted).
    fn used(&self) -> usize {
        self.at - (self.count / 8) as usize
    }
}

/// How many bits the table looks up at once.
const FAST: u32 = 9;

/// A canonical Huffman code: how many codes each length has, the symbols in code order, and a table of every
/// symbol whose code is at most `FAST` bits, indexed by the next `FAST` bits (symbol << 4 | length; 0: none).
struct Code {
    counts: [u16; 16],
    symbols: Vec<u16>,
    fast: Vec<u16>,
}

impl Code {
    /// The code with these lengths (0: a symbol that isn't used), and how incomplete it is (0: complete). An
    /// over-subscribed one (more codes than its lengths have room for) is an error.
    fn new(lengths: &[u8]) -> Result<(Code, i32), Error> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        let mut left: i32 = 1;
        for &c in &counts[1..] {
            left <<= 1;
            left -= c as i32;
            if left < 0 {
                return Err(Error::Damaged);
            }
        }
        // Where each length's symbols start, in code order.
        let mut offsets = [0u16; 16];
        for len in 1..15 {
            offsets[len + 1] = offsets[len] + counts[len];
        }
        let mut symbols = vec![0u16; lengths.len()];
        // Each length's first code: after the shorter lengths' codes, doubled for the bit more (RFC 1951, 3.2.2).
        let mut next = [0u32; 16];
        let mut code = 0u32;
        for len in 1..16 {
            code = (code + if len == 1 { 0 } else { counts[len - 1] as u32 }) << 1;
            next[len] = code;
        }
        let mut fast = vec![0u16; 1 << FAST];
        for (sym, &len) in lengths.iter().enumerate() {
            let len = len as usize;
            if len == 0 {
                continue;
            }
            symbols[offsets[len] as usize] = sym as u16;
            offsets[len] += 1;
            let c = next[len];
            next[len] += 1;
            if len as u32 <= FAST {
                // The code's bits come first-bit-first from the stream: reversed, they index the table, whatever the
                // bits after them.
                let reversed = (c.reverse_bits() >> (32 - len)) as usize;
                let mut i = reversed;
                while i < 1 << FAST {
                    fast[i] = (sym as u16) << 4 | len as u16;
                    i += 1 << len;
                }
            }
        }
        Ok((Code { counts, symbols, fast }, left))
    }

    /// The next symbol.
    fn decode(&self, bits: &mut Bits) -> Result<u16, Error> {
        if bits.count < FAST {
            bits.fill();
        }
        if bits.count >= FAST {
            let e = self.fast[(bits.buf & ((1 << FAST) - 1)) as usize];
            if e != 0 {
                let len = (e & 15) as u32;
                bits.buf >>= len;
                bits.count -= len;
                return Ok(e >> 4);
            }
        }
        // A longer code (or the input's last bits): a bit at a time, through each length's codes.
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.need(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return self.symbols.get((index + code - first) as usize).copied().ok_or(Error::Damaged);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(Error::Damaged)
    }
}

/// Lengths 3 to 258 (symbols 257 to 285): the base, and how many extra bits are added.
const LENGTH_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
/// Distances 1 to 32,768 (symbols 0 to 29).
const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193,
    12289, 16385, 24577,
];
const DISTANCE_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// The order a dynamic block gives its code lengths' code lengths in.
const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// Raw DEFLATE: what `input` decompresses to, at most `most` bytes.
pub fn inflate(input: &[u8], most: usize) -> Result<Vec<u8>, Error> {
    inflate_used(input, most).map(|(out, _)| out)
}

/// Raw DEFLATE as far as it goes: what `input` decompresses to before it ends early or is damaged (at most `most`
/// bytes), as readers of damaged files (PDFs cut short) want.
pub fn inflate_partial(input: &[u8], most: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = inflate_into(input, most, &mut out);
    out
}

/// As `inflate`, and how many of the input's bytes the stream took (zlib's checksum comes after them).
fn inflate_used(input: &[u8], most: usize) -> Result<(Vec<u8>, usize), Error> {
    let mut out: Vec<u8> = Vec::new();
    let used = inflate_into(input, most, &mut out)?;
    Ok((out, used))
}

/// DEFLATE's output added to `out`; how many of the input's bytes the stream took.
fn inflate_into(input: &[u8], most: usize, out: &mut Vec<u8>) -> Result<usize, Error> {
    let mut bits = Bits { input, at: 0, buf: 0, count: 0 };
    let mut fixed: Option<(Code, Code)> = None;
    loop {
        let last = bits.need(1)?;
        match bits.need(2)? {
            0 => stored(&mut bits, out, most)?,
            1 => {
                if fixed.is_none() {
                    let mut lengths = [0u8; 288];
                    lengths[..144].fill(8);
                    lengths[144..256].fill(9);
                    lengths[256..280].fill(7);
                    lengths[280..].fill(8);
                    fixed = Some((Code::new(&lengths)?.0, Code::new(&[5u8; 30])?.0));
                }
                let (lit, dist) = fixed.as_ref().unwrap();
                codes(&mut bits, out, most, lit, dist)?;
            }
            2 => {
                let (lit, dist) = dynamic(&mut bits)?;
                codes(&mut bits, out, most, &lit, &dist)?;
            }
            _ => return Err(Error::Damaged),
        }
        if last == 1 {
            return Ok(bits.used());
        }
    }
}

/// A stored block: its length, that length's complement, then the bytes as they are.
fn stored(bits: &mut Bits, out: &mut Vec<u8>, most: usize) -> Result<(), Error> {
    bits.align();
    let at = bits.at;
    let header = bits.input.get(at..at + 4).ok_or(Error::Damaged)?;
    let len = u16::from_le_bytes([header[0], header[1]]);
    if len != !u16::from_le_bytes([header[2], header[3]]) {
        return Err(Error::Damaged);
    }
    let data = bits.input.get(at + 4..at + 4 + len as usize).ok_or(Error::Damaged)?;
    if out.len() + data.len() > most {
        return Err(Error::TooBig);
    }
    out.extend_from_slice(data);
    bits.at = at + 4 + len as usize;
    Ok(())
}

/// A dynamic block's codes: how many of each kind, the code lengths' own code, then the lengths (with runs of
/// repeats and zeros).
fn dynamic(bits: &mut Bits) -> Result<(Code, Code), Error> {
    let nlen = bits.need(5)? as usize + 257;
    let ndist = bits.need(5)? as usize + 1;
    let ncode = bits.need(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err(Error::Damaged);
    }
    let mut lengths = [0u8; 320];
    for &i in &ORDER[..ncode] {
        lengths[i] = bits.need(3)? as u8;
    }
    let (lencode, left) = Code::new(&lengths[..19])?;
    if left != 0 {
        return Err(Error::Damaged);
    }
    lengths = [0u8; 320];
    let mut index = 0;
    while index < nlen + ndist {
        let sym = lencode.decode(bits)?;
        if sym < 16 {
            lengths[index] = sym as u8;
            index += 1;
            continue;
        }
        let (value, count) = match sym {
            16 => (*lengths.get(index.wrapping_sub(1)).ok_or(Error::Damaged)?, 3 + bits.need(2)? as usize),
            17 => (0, 3 + bits.need(3)? as usize),
            _ => (0, 11 + bits.need(7)? as usize),
        };
        if index + count > nlen + ndist {
            return Err(Error::Damaged);
        }
        lengths[index..index + count].fill(value);
        index += count;
    }
    // (Without an end-of-block code, a block couldn't end.)
    if lengths[256] == 0 {
        return Err(Error::Damaged);
    }
    // An incomplete code is allowed only as a single code of one bit.
    let (lit, left) = Code::new(&lengths[..nlen])?;
    if left > 0 && nlen != (lit.counts[0] + lit.counts[1]) as usize {
        return Err(Error::Damaged);
    }
    let (dist, left) = Code::new(&lengths[nlen..nlen + ndist])?;
    if left > 0 && ndist != (dist.counts[0] + dist.counts[1]) as usize {
        return Err(Error::Damaged);
    }
    Ok((lit, dist))
}

/// A Huffman-coded block's bytes and copies, to its end.
fn codes(bits: &mut Bits, out: &mut Vec<u8>, most: usize, lit: &Code, dist: &Code) -> Result<(), Error> {
    loop {
        let sym = lit.decode(bits)? as usize;
        if sym < 256 {
            if out.len() >= most {
                return Err(Error::TooBig);
            }
            out.push(sym as u8);
            continue;
        }
        if sym == 256 {
            return Ok(());
        }
        let l = sym - 257;
        if l >= 29 {
            return Err(Error::Damaged);
        }
        let len = LENGTH_BASE[l] as usize + bits.need(LENGTH_EXTRA[l] as u32)? as usize;
        let d = dist.decode(bits)? as usize;
        if d >= 30 {
            return Err(Error::Damaged);
        }
        let back = DISTANCE_BASE[d] as usize + bits.need(DISTANCE_EXTRA[d] as u32)? as usize;
        if back > out.len() {
            return Err(Error::Damaged);
        }
        if out.len() + len > most {
            return Err(Error::TooBig);
        }
        let from = out.len() - back;
        if back >= len {
            out.extend_from_within(from..from + len);
        } else {
            // Overlapping: each byte copied may be one this copy made.
            for i in 0..len {
                let b = out[from + i];
                out.push(b);
            }
        }
    }
}

/// A zlib stream (PNG's): a two-byte header (DEFLATE, a window of at most 32 KiB, no preset dictionary), DEFLATE,
/// then the Adler-32 checksum of what comes out.
pub fn zlib(input: &[u8], most: usize) -> Result<Vec<u8>, Error> {
    let (Some(&cmf), Some(&flg)) = (input.first(), input.get(1)) else { return Err(Error::NotZip) };
    if cmf & 0x0F != 8 || cmf >> 4 > 7 || (u16::from(cmf) << 8 | u16::from(flg)) % 31 != 0 {
        return Err(Error::NotZip);
    }
    if flg & 0x20 != 0 {
        return Err(Error::Damaged);
    }
    let (out, used) = inflate_used(&input[2..], most)?;
    let sum = input.get(2 + used..2 + used + 4).ok_or(Error::Damaged)?;
    if adler32(&out) != u32::from_be_bytes([sum[0], sum[1], sum[2], sum[3]]) {
        return Err(Error::Damaged);
    }
    Ok(out)
}

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    // (5,552 bytes at most between reductions keeps b from overflowing.)
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

/// CRC-32 (IEEE 802.3's, as ZIP and PNG use it), a byte at a time from a table.
pub fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0u32, |c, &b| CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8))
}

const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

/// A file in an archive, as its central directory has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// Its size, and its size compressed.
    pub size: u64,
    pub packed: u64,
    method: u16,
    flags: u16,
    crc: u32,
    /// Where its local header is.
    offset: u64,
}

impl Entry {
    /// A folder (its name ends in /), not a file.
    pub fn is_folder(&self) -> bool {
        self.name.ends_with('/')
    }
}

pub struct Archive<'a> {
    data: &'a [u8],
    entries: Vec<Entry>,
}

/// The most files an archive is believed to have.
const MOST_ENTRIES: usize = 1 << 20;

impl<'a> Archive<'a> {
    /// The archive `data` is: its central directory read, from the record at its end that says where that is.
    pub fn open(data: &'a [u8]) -> Result<Archive<'a>, Error> {
        // The end record: 22 bytes, then a comment of at most 64 KiB.
        let lowest = data.len().saturating_sub(22 + 65_535);
        let end = (lowest..=data.len().saturating_sub(22))
            .rev()
            .find(|&i| u32_at(data, i) == Some(0x0605_4B50) && 22 + u16_at(data, i + 20).unwrap_or(0) as usize <= data.len() - i)
            .ok_or(Error::NotZip)?;
        let mut count = u16_at(data, end + 10).ok_or(Error::NotZip)? as u64;
        let mut size = u32_at(data, end + 12).ok_or(Error::NotZip)? as u64;
        let mut start = u32_at(data, end + 16).ok_or(Error::NotZip)? as u64;
        // ZIP64: a locator just before it says where the bigger end record is.
        if end >= 20 && u32_at(data, end - 20) == Some(0x0706_4B50) {
            let at = u64_at(data, end - 12).ok_or(Error::Damaged)? as usize;
            if u32_at(data, at) != Some(0x0606_4B50) {
                return Err(Error::Damaged);
            }
            count = u64_at(data, at + 32).ok_or(Error::Damaged)?;
            size = u64_at(data, at + 40).ok_or(Error::Damaged)?;
            start = u64_at(data, at + 48).ok_or(Error::Damaged)?;
        }
        if count as usize > MOST_ENTRIES || start.checked_add(size).is_none_or(|e| e > data.len() as u64) {
            return Err(Error::Damaged);
        }
        let directory = &data[start as usize..(start + size) as usize];
        let mut entries = Vec::new();
        let mut at = 0usize;
        for _ in 0..count {
            if u32_at(directory, at) != Some(0x0201_4B50) {
                return Err(Error::Damaged);
            }
            let field = |i: usize| u16_at(directory, at + i).ok_or(Error::Damaged);
            let (flags, method) = (field(8)?, field(10)?);
            let crc = u32_at(directory, at + 16).ok_or(Error::Damaged)?;
            let mut packed = u32_at(directory, at + 20).ok_or(Error::Damaged)? as u64;
            let mut size = u32_at(directory, at + 24).ok_or(Error::Damaged)? as u64;
            let (name_len, extra_len, comment_len) = (field(28)? as usize, field(30)? as usize, field(32)? as usize);
            let mut offset = u32_at(directory, at + 42).ok_or(Error::Damaged)? as u64;
            let name = directory.get(at + 46..at + 46 + name_len).ok_or(Error::Damaged)?;
            let extra = directory.get(at + 46 + name_len..at + 46 + name_len + extra_len).ok_or(Error::Damaged)?;
            // ZIP64's extra field: in order, the 64-bit values of whichever of these were full.
            let mut e = 0;
            while e + 4 <= extra.len() {
                let (id, len) = (u16_at(extra, e).unwrap_or(0), u16_at(extra, e + 2).unwrap_or(0) as usize);
                let body = extra.get(e + 4..e + 4 + len).ok_or(Error::Damaged)?;
                if id == 0x0001 {
                    let mut k = 0;
                    for v in [&mut size, &mut packed, &mut offset] {
                        if *v == 0xFFFF_FFFF {
                            *v = u64_at(body, k).ok_or(Error::Damaged)?;
                            k += 8;
                        }
                    }
                }
                e += 4 + len;
            }
            // Names are UTF-8 when flag 11 says so (and, being ASCII, in nearly every Office file); else IBM's code
            // page 437.
            let name = if flags & 0x0800 != 0 {
                String::from_utf8_lossy(name).into_owned()
            } else {
                name.iter().map(|&b| if b < 0x80 { b as char } else { CP437[(b - 0x80) as usize] }).collect()
            };
            entries.push(Entry { name, size, packed, method, flags, crc, offset });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Archive { data, entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The file called `name` (as Office files' parts are named: whatever the case of its ASCII letters, with or
    /// without a / at the start).
    pub fn find(&self, name: &str) -> Option<&Entry> {
        let name = name.trim_start_matches('/');
        self.entries.iter().find(|e| e.name.trim_start_matches('/').eq_ignore_ascii_case(name))
    }

    /// A file's bytes (at most `most` of them), its CRC checked.
    pub fn read(&self, e: &Entry, most: usize) -> Result<Vec<u8>, Error> {
        if e.flags & 0x0001 != 0 {
            return Err(Error::Encrypted);
        }
        if e.size > most as u64 {
            return Err(Error::TooBig);
        }
        // Its local header: 30 bytes, its name and an extra field (whose length can differ from the directory's).
        let at = usize::try_from(e.offset).map_err(|_| Error::Damaged)?;
        if u32_at(self.data, at) != Some(0x0403_4B50) {
            return Err(Error::Damaged);
        }
        let skip = 30 + u16_at(self.data, at + 26).ok_or(Error::Damaged)? as usize + u16_at(self.data, at + 28).ok_or(Error::Damaged)? as usize;
        let from = at.checked_add(skip).ok_or(Error::Damaged)?;
        let packed = usize::try_from(e.packed).map_err(|_| Error::Damaged)?;
        let raw = self.data.get(from..from.checked_add(packed).ok_or(Error::Damaged)?).ok_or(Error::Damaged)?;
        let out = match e.method {
            0 => raw.to_vec(),
            8 => inflate(raw, e.size as usize)?,
            m => return Err(Error::Method(m)),
        };
        if out.len() as u64 != e.size || crc32(&out) != e.crc {
            return Err(Error::Damaged);
        }
        Ok(out)
    }
}

/// IBM's code page 437, bytes 0x80 to 0xFF: the characters of names in older archives.
const CP437: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ',
    'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', 'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', '░', '▒',
    '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩',
    '╦', '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', 'α', 'ß', 'Γ', 'π',
    'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ',
    '²', '■', '\u{A0}',
];
