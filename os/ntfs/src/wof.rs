//! Windows' compression of its own files ("Compact OS", the Windows Overlay Filter's: WOF). A file's bytes are in a
//! named stream of their own (WofCompressedData), in chunks of 4, 8, 16 or 32 KiB of the file each, compressed one by
//! one (a chunk that wouldn't get smaller is kept as it is), after a table of where each chunk but the first starts.
//! Chunks of 4, 8 and 16 KiB are XPRESS with Huffman codes ([MS-XCA] 2.1 and 2.2: LZ77's matches and literals, each
//! a symbol of a canonical Huffman code whose 512 lengths come first); chunks of 32 KiB are LZX as Windows' WIM files
//! have it (a 32 KiB window; verbatim, aligned-offset and uncompressed blocks; its codes' lengths sent as changes
//! from the last block's; the addresses x86 calls go to translated, as if the file were 12,000,000 bytes long). As
//! Microsoft's [MS-XCA] describes XPRESS, and its LZX format specification (with wimlib's notes on WIM's variant of
//! it) describes LZX. Whatever the bytes, a chunk decompresses to its size or gives None: never a crash.

use alloc::vec;
use alloc::vec::Vec;

/// WOF's algorithms (a file provider's): XPRESS in chunks of 4 KiB, LZX in chunks of 32 KiB, XPRESS in chunks of 8
/// KiB and of 16 KiB. The chunk size of each, if it's one of them.
pub(crate) fn chunk_size(algorithm: u32) -> Option<usize> {
    match algorithm {
        0 => Some(4096),
        1 => Some(32768),
        2 => Some(8192),
        3 => Some(16384),
        _ => None,
    }
}

/// A chunk, decompressed into `out` (its size, as the file has it), by `algorithm`.
pub(crate) fn decompress(algorithm: u32, input: &[u8], out: &mut [u8]) -> Option<()> {
    match algorithm {
        1 => lzx(input, out),
        0 | 2 | 3 => xpress(input, out),
        _ => None,
    }
}

/// A canonical Huffman code (codes given out in order of their length, then of their symbol, the first bit the top
/// one): how many codes each length has, and the symbols in that order.
struct Code {
    counts: [u16; 17],
    symbols: Vec<u16>,
}

impl Code {
    /// The code with these lengths (0: a symbol that has none), at most `max` bits each; None for lengths no code
    /// can have (more codes of a length than there's room for). One with fewer is used as far as it goes (and one
    /// with none at all is a code never used).
    fn new(lens: &[u8], max: u32) -> Option<Code> {
        let mut counts = [0u16; 17];
        for &l in lens {
            if l as u32 > max {
                return None;
            }
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut room: i32 = 1;
        for &count in &counts[1..=max as usize] {
            room = (room << 1) - count as i32;
            if room < 0 {
                return None;
            }
        }
        // Where each length's symbols start.
        let mut at = [0u16; 18];
        for len in 1..=16 {
            at[len + 1] = at[len] + counts[len];
        }
        let mut symbols = vec![0u16; at[17] as usize];
        for (s, &l) in lens.iter().enumerate() {
            if l != 0 {
                symbols[at[l as usize] as usize] = s as u16;
                at[l as usize] += 1;
            }
        }
        Some(Code { counts, symbols })
    }

    /// The symbol whose code `bits` (the next `max` bits, the first at the top) begins with, and the code's length.
    /// None for bits no code of it begins (an incomplete code's).
    fn decode(&self, bits: u32, max: u32) -> Option<(u16, u32)> {
        let mut first = 0u32;
        let mut index = 0usize;
        for len in 1..=max {
            let code = bits >> (max - len);
            let count = self.counts[len as usize] as u32;
            if code < first + count {
                return self.symbols.get(index + (code - first) as usize).map(|&s| (s, len));
            }
            index += count as usize;
            first = (first + count) << 1;
        }
        None
    }
}

/// A 16-bit little-endian word of the input at `at` (0 past its end).
fn word(input: &[u8], at: usize) -> u32 {
    match input.get(at..at + 2) {
        Some(b) => u16::from_le_bytes([b[0], b[1]]) as u32,
        None => 0,
    }
}

/// XPRESS with Huffman codes ([MS-XCA] 2.2), into `out` (as many bytes as it holds). Each 64 KiB of what comes out
/// starts with its code: 512 lengths of 4 bits (literals 0 to 255; then matches, a symbol's low 4 bits the match's
/// length less 3 (15: longer, in a byte, or 16 or 32 bits, after), its high 4 the bits of its offset after the top
/// one). Its bits come 16 at a time (little-endian words, the first bit the top one), in a register of 32 that's
/// refilled once fewer than 16 are left; the bytes of a long match's length are read from where the words have got
/// to, between them.
pub(crate) fn xpress(input: &[u8], out: &mut [u8]) -> Option<()> {
    let mut at = 0usize;
    let mut done = 0usize;
    while done < out.len() {
        let table = input.get(at..at + 256)?;
        at += 256;
        let mut lens = [0u8; 512];
        for (i, &b) in table.iter().enumerate() {
            lens[2 * i] = b & 15;
            lens[2 * i + 1] = b >> 4;
        }
        let code = Code::new(&lens, 15)?;
        let mut bits: u32 = (word(input, at) << 16) | word(input, at + 2);
        at += 4;
        let mut extra: i32 = 16;
        let block_end = done.saturating_add(65536).min(out.len());
        while done < block_end {
            let (symbol, len) = code.decode(bits >> 17, 15)?;
            bits <<= len;
            extra -= len as i32;
            if extra < 0 {
                bits |= word(input, at) << (-extra) as u32;
                extra += 16;
                at += 2;
            }
            if symbol < 256 {
                out[done] = symbol as u8;
                done += 1;
                continue;
            }
            let symbol = symbol - 256;
            let mut length = (symbol & 15) as usize;
            let offset_bits = (symbol >> 4) as u32;
            if length == 15 {
                length = *input.get(at)? as usize;
                at += 1;
                if length == 255 {
                    length = u16::from_le_bytes([*input.get(at)?, *input.get(at + 1)?]) as usize;
                    at += 2;
                    if length == 0 {
                        let b = input.get(at..at + 4)?;
                        length = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
                        at += 4;
                    }
                    length = length.checked_sub(15)?;
                }
                length += 15;
            }
            length += 3;
            let offset = if offset_bits == 0 { 1 } else { ((bits >> (32 - offset_bits)) | (1 << offset_bits)) as usize };
            bits = if offset_bits == 0 { bits } else { bits << offset_bits };
            extra -= offset_bits as i32;
            if extra < 0 {
                bits |= word(input, at) << (-extra) as u32;
                extra += 16;
                at += 2;
            }
            if offset > done || length > out.len() - done {
                return None;
            }
            for _ in 0..length {
                out[done] = out[done - offset];
                done += 1;
            }
        }
    }
    Some(())
}

/// LZX's sizes for a 32 KiB window: literals (256), then matches' headers (8 for each of 30 offset slots), the code
/// of matches' longer lengths (249 symbols), of aligned offsets' last 3 bits (8), and of the codes' lengths (20).
const SLOTS: usize = 30;
const MAIN: usize = 256 + 8 * SLOTS;
const LENGTHS: usize = 249;
/// Where each offset slot's (formatted) offsets start, and how many extra bits say which: the first three slots are
/// the three offsets used last, and the rest are 2 more than the offset itself.
const SLOT_BASE: [u32; SLOTS] = [
    0, 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096, 6144,
    8192, 12288, 16384, 24576,
];
const SLOT_BITS: [u32; SLOTS] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// The size WIM's LZX takes a chunk to be part of, for its calls' translated addresses.
const E8_SIZE: i32 = 12_000_000;

/// LZX's bits: 16 at a time (little-endian words), the first bit the top one; zeros past the input's end.
struct Bits<'a> {
    input: &'a [u8],
    /// Where the next word is.
    at: usize,
    /// The bits to come, the next at the top, and how many there are.
    buf: u64,
    count: u32,
}

impl Bits<'_> {
    fn fill(&mut self) {
        while self.count <= 48 {
            self.buf |= (word(self.input, self.at) as u64) << (48 - self.count);
            self.at += 2;
            self.count += 16;
        }
    }

    /// The next `n` bits (1 to 32), not taken.
    fn peek(&mut self, n: u32) -> u32 {
        if self.count < n {
            self.fill();
        }
        (self.buf >> (64 - n)) as u32
    }

    fn skip(&mut self, n: u32) {
        self.buf <<= n;
        self.count -= n;
    }

    fn read(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.peek(n);
        self.skip(n);
        v
    }

    /// A symbol of `code` (whose codes are at most `max` bits long).
    fn symbol(&mut self, code: &Code, max: u32) -> Option<u16> {
        let (s, len) = code.decode(self.peek(max), max)?;
        self.skip(len);
        Some(s)
    }

    /// Where an uncompressed block's bytes start: at the next word, the rest of this one left (or, at a word's start,
    /// the whole word after it).
    fn align(&self) -> usize {
        let taken = self.at * 8 - self.count as usize;
        (taken / 16 + 1) * 2
    }

    /// Bits again from `at`, after an uncompressed block.
    fn restart(&mut self, at: usize) {
        self.at = at;
        self.buf = 0;
        self.count = 0;
    }
}

/// Codes' lengths, as changes from the last block's (`lens`, changed in place): a code of the changes first (20
/// lengths of 4 bits), then each length's change (0 to 16, taken from the old one, mod 17), or runs: of 4 to 19
/// zeros (17), of 20 to 51 (18), or of 4 or 5 lengths the same change makes (19).
fn read_lens(bits: &mut Bits, lens: &mut [u8]) -> Option<()> {
    let mut pre = [0u8; 20];
    for p in &mut pre {
        *p = bits.read(4) as u8;
    }
    let code = Code::new(&pre, 15)?;
    let changed = |old: u8, by: u16| ((old as i32 - by as i32 + 17) % 17) as u8;
    let mut i = 0;
    while i < lens.len() {
        let s = bits.symbol(&code, 15)?;
        let (run, len) = match s {
            0..=16 => (1, changed(lens[i], s)),
            17 => (4 + bits.read(4) as usize, 0),
            18 => (20 + bits.read(5) as usize, 0),
            _ => {
                let run = 4 + bits.read(1) as usize;
                let by = bits.symbol(&code, 15)?;
                if by > 17 {
                    return None;
                }
                (run, changed(lens[i], by))
            }
        };
        let end = (i + run).min(lens.len());
        lens[i..end].fill(len);
        i = end;
    }
    Some(())
}

/// LZX, as WIM files (and WOF) have it, into `out` (as many bytes as it holds, 32 KiB at most).
pub(crate) fn lzx(input: &[u8], out: &mut [u8]) -> Option<()> {
    let mut bits = Bits { input, at: 0, buf: 0, count: 0 };
    let mut main_lens = [0u8; MAIN];
    let mut length_lens = [0u8; LENGTHS];
    let mut recent = [1u32; 3];
    let mut done = 0usize;
    // Where the blocks so far end (a match may run on into the next).
    let mut block_end = 0usize;
    while done < out.len() {
        let kind = bits.read(3);
        let size = if bits.read(1) == 1 { 32768 } else { bits.read(16) as usize };
        let block_start = block_end;
        block_end = block_end.saturating_add(size).min(out.len());
        match kind {
            // Verbatim and aligned-offset blocks: their codes, then literals and matches.
            1 | 2 => {
                let aligned = if kind == 2 {
                    let mut l = [0u8; 8];
                    for x in &mut l {
                        *x = bits.read(3) as u8;
                    }
                    Some(Code::new(&l, 7)?)
                } else {
                    None
                };
                read_lens(&mut bits, &mut main_lens[..256])?;
                read_lens(&mut bits, &mut main_lens[256..])?;
                read_lens(&mut bits, &mut length_lens)?;
                let main = Code::new(&main_lens, 16)?;
                let lengths = Code::new(&length_lens, 16)?;
                while done < block_end {
                    let s = bits.symbol(&main, 16)? as usize;
                    if s < 256 {
                        out[done] = s as u8;
                        done += 1;
                        continue;
                    }
                    let header = s - 256;
                    let mut length = (header & 7) + 2;
                    if header & 7 == 7 {
                        length += bits.symbol(&lengths, 16)? as usize;
                    }
                    let slot = header >> 3;
                    let offset = if slot < 3 {
                        let o = recent[slot];
                        recent[slot] = recent[0];
                        recent[0] = o;
                        o
                    } else {
                        let extra = SLOT_BITS[slot];
                        let mut o = SLOT_BASE[slot] - 2;
                        match &aligned {
                            Some(a) if extra >= 3 => {
                                o += bits.read(extra - 3) << 3;
                                o += bits.symbol(a, 7)? as u32;
                            }
                            _ => o += bits.read(extra),
                        }
                        recent[2] = recent[1];
                        recent[1] = recent[0];
                        recent[0] = o;
                        o
                    };
                    let offset = offset as usize;
                    if offset == 0 || offset > done || length > out.len() - done {
                        return None;
                    }
                    for _ in 0..length {
                        out[done] = out[done - offset];
                        done += 1;
                    }
                }
            }
            // An uncompressed block: at the next 16-bit word, the three offsets used last, then its bytes (and one
            // more if they're odd, for the words after to stay in step).
            3 => {
                if done != block_start {
                    return None;
                }
                let at = bits.align();
                let le32 = |i: usize| input.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                recent = [le32(at)?, le32(at + 4)?, le32(at + 8)?];
                if recent.contains(&0) {
                    return None;
                }
                let n = block_end - done;
                out[done..block_end].copy_from_slice(input.get(at + 12..at + 12 + n)?);
                done = block_end;
                bits.restart(at + 12 + size + (size & 1));
            }
            _ => return None,
        }
    }
    undo_e8(out);
    Some(())
}

/// The addresses x86 calls go to (an E8 byte, then 4), back as they were: the compressor made each an absolute one
/// (its place in the chunk added) if it was within the chunk's "file" of E8_SIZE bytes; not in the last 10 bytes.
fn undo_e8(data: &mut [u8]) {
    if data.len() <= 10 {
        return;
    }
    let end = data.len() - 10;
    let mut i = 0;
    while i < end {
        if data[i] != 0xE8 {
            i += 1;
            continue;
        }
        let at = i as i32;
        let v = i32::from_le_bytes([data[i + 1], data[i + 2], data[i + 3], data[i + 4]]);
        let was = if v >= 0 {
            (v < E8_SIZE).then(|| v - at)
        } else {
            (v >= -at).then(|| v + E8_SIZE)
        };
        if let Some(w) = was {
            data[i + 1..i + 5].copy_from_slice(&w.to_le_bytes());
        }
        i += 5;
    }
}
