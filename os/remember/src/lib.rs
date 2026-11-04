//! Chat Computer's memory file (`\EFI\chatcomputer-own\memory.bin` on the start-up partition): the conversation,
//! kept across restarts. Windows' setup (windows/lib/OwnOs.ps1) makes the file at its full size, so Chat Computer's
//! own system never changes the file system, only the file's own bytes, in place.
//!
//! The file is made of 512-byte chunks. Each starts with a 16-byte tag: "CCMEMORY", the file's id (a random
//! number setup chose, also in the settings file) and the chunk's number in the file. Before anything is written, the
//! sector it goes to is read and all its tags checked: a sector that isn't this file's, at this place, is never
//! written, whatever went wrong in finding it. The other 496 bytes of each chunk carry data.
//!
//! The file is two halves, written in turn, so that one of them always holds a whole copy, even if the PC stops in
//! the middle of a write. Each half starts with a header: "CCMEMDAT", the format's version, a sequence number (the
//! newest copy has the highest), the data's length and its hash (FNV-1a, 64 bits). The data follows, then the
//! header is written last.

#![no_std]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

#[cfg(test)]
mod tests;

pub const CHUNK: usize = 512;
pub const TAG: usize = 16;
/// Data bytes in a chunk.
pub const DATA: usize = CHUNK - TAG;
const TAG_MAGIC: &[u8; 8] = b"CCMEMORY";
const HEADER_MAGIC: &[u8; 8] = b"CCMEMDAT";
const VERSION: u32 = 1;
/// Magic, version, sequence, length, hash.
const HEADER: usize = 8 + 4 + 8 + 4 + 8;

/// The file's sectors (of the drive's block size, a whole number of chunks), numbered from its start.
pub trait Store {
    fn sector_size(&self) -> usize;
    fn sectors(&self) -> usize;
    fn read(&mut self, sector: usize, buf: &mut [u8]) -> bool;
    fn write(&mut self, sector: usize, buf: &[u8]) -> bool;
    fn flush(&mut self) -> bool;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    Read,
    Write,
    /// A sector's tags aren't this file's at that place: nothing was written to it.
    NotOurs,
    /// The data is bigger than half the file.
    TooBig,
    /// The file is too small to be a memory file, or its sectors aren't whole chunks.
    Shape,
}

/// Chunk `index`'s tag in the file with `id`.
pub fn tag(id: u32, index: u32) -> [u8; TAG] {
    let mut t = [0u8; TAG];
    t[..8].copy_from_slice(TAG_MAGIC);
    t[8..12].copy_from_slice(&id.to_le_bytes());
    t[12..].copy_from_slice(&index.to_le_bytes());
    t
}

/// The file as setup makes it: every chunk tagged, no data.
pub fn blank(id: u32, bytes: usize) -> Vec<u8> {
    let mut f = vec![0u8; bytes / CHUNK * CHUNK];
    for (i, c) in f.chunks_exact_mut(CHUNK).enumerate() {
        c[..TAG].copy_from_slice(&tag(id, i as u32));
    }
    f
}

pub fn hash(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// The memory file: its id, and how it's laid out on its store.
pub struct Memory {
    id: u32,
    chunks_per_sector: usize,
    /// Chunks in each half.
    half: usize,
}

impl Memory {
    pub fn new(store: &impl Store, id: u32) -> Result<Memory, Error> {
        let size = store.sector_size();
        if size < CHUNK || !size.is_multiple_of(CHUNK) {
            return Err(Error::Shape);
        }
        let chunks = store.sectors() * (size / CHUNK);
        let half = chunks / 2;
        // Each half: whole sectors, room for the header and something.
        if half < 2 || !half.is_multiple_of(size / CHUNK) {
            return Err(Error::Shape);
        }
        Ok(Memory { id, chunks_per_sector: size / CHUNK, half })
    }

    /// The most data a half holds.
    pub fn capacity(&self) -> usize {
        self.half * DATA - HEADER
    }

    /// Reads a sector and checks every chunk in it is this file's, at this place.
    fn sector(&self, store: &mut impl Store, sector: usize) -> Result<Vec<u8>, Error> {
        let mut buf = vec![0u8; store.sector_size()];
        if !store.read(sector, &mut buf) {
            return Err(Error::Read);
        }
        for (k, c) in buf.chunks_exact(CHUNK).enumerate() {
            if c[..TAG] != tag(self.id, (sector * self.chunks_per_sector + k) as u32) {
                return Err(Error::NotOurs);
            }
        }
        Ok(buf)
    }

    /// Half `h`'s copy, if it holds a whole one: its sequence number and data.
    fn load_half(&self, store: &mut impl Store, h: usize) -> Result<Option<(u64, Vec<u8>)>, Error> {
        let first_chunk = h * self.half;
        let mut chunks = self.chunks(store, first_chunk, 1)?;
        let c = &chunks[0];
        if c[..8] != *HEADER_MAGIC || u32::from_le_bytes(c[8..12].try_into().unwrap()) != VERSION {
            return Ok(None);
        }
        let sequence = u64::from_le_bytes(c[12..20].try_into().unwrap());
        let length = u32::from_le_bytes(c[20..24].try_into().unwrap()) as usize;
        let expected = u64::from_le_bytes(c[24..32].try_into().unwrap());
        if length > self.capacity() {
            return Ok(None);
        }
        let needed = (HEADER + length).div_ceil(DATA);
        if needed > 1 {
            chunks.extend(self.chunks(store, first_chunk + 1, needed - 1)?);
        }
        let mut data: Vec<u8> = chunks.concat();
        data.drain(..HEADER);
        data.truncate(length);
        Ok((hash(&data) == expected).then_some((sequence, data)))
    }

    /// The data areas of `count` chunks from chunk `first`, each checked.
    fn chunks(&self, store: &mut impl Store, first: usize, count: usize) -> Result<Vec<Vec<u8>>, Error> {
        let mut out = Vec::with_capacity(count);
        let mut sector = usize::MAX;
        let mut buf = Vec::new();
        for c in first..first + count {
            if c / self.chunks_per_sector != sector {
                sector = c / self.chunks_per_sector;
                buf = self.sector(store, sector)?;
            }
            let at = (c % self.chunks_per_sector) * CHUNK;
            out.push(buf[at + TAG..at + CHUNK].to_vec());
        }
        Ok(out)
    }

    /// The newest whole copy: its data, and which half and sequence number it has (None: nothing kept yet).
    pub fn load(&self, store: &mut impl Store) -> Result<Option<(usize, u64, Vec<u8>)>, Error> {
        let a = self.load_half(store, 0)?;
        let b = self.load_half(store, 1)?;
        Ok(match (a, b) {
            (Some((sa, da)), Some((sb, _))) if sa >= sb => Some((0, sa, da)),
            (_, Some((sb, db))) => Some((1, sb, db)),
            (Some((sa, da)), None) => Some((0, sa, da)),
            (None, None) => None,
        })
    }

    /// Keeps `data`, in the half that doesn't hold the newest copy (which stays whole until this one is). The data's
    /// sectors are written first, then the header's, each flushed to the drive.
    pub fn save(&self, store: &mut impl Store, data: &[u8]) -> Result<(), Error> {
        if data.len() > self.capacity() {
            return Err(Error::TooBig);
        }
        let (half, sequence) = match self.load(store)? {
            Some((h, s, _)) => (1 - h, s + 1),
            None => (0, 1),
        };
        let mut bytes = Vec::with_capacity(HEADER + data.len());
        bytes.extend_from_slice(HEADER_MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&sequence.to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&hash(data).to_le_bytes());
        bytes.extend_from_slice(data);
        let first_chunk = half * self.half;
        let chunks = bytes.len().div_ceil(DATA);
        let first_sector = first_chunk / self.chunks_per_sector;
        let last_sector = (first_chunk + chunks - 1) / self.chunks_per_sector;
        // Every sector to be written is read and checked first: if one isn't this file's, none is written.
        let mut sectors = Vec::new();
        for sector in first_sector..=last_sector {
            sectors.push((sector, self.sector(store, sector)?));
        }
        // The header's sector last.
        sectors.rotate_left(1);
        for (sector, mut buf) in sectors {
            for k in 0..self.chunks_per_sector {
                let c = sector * self.chunks_per_sector + k;
                if c < first_chunk || c >= first_chunk + chunks {
                    continue;
                }
                let from = (c - first_chunk) * DATA;
                let piece = &bytes[from..(from + DATA).min(bytes.len())];
                let at = k * CHUNK + TAG;
                buf[at..at + piece.len()].copy_from_slice(piece);
                buf[at + piece.len()..(k + 1) * CHUNK].fill(0);
            }
            if !store.write(sector, &buf) {
                return Err(Error::Write);
            }
            if sector == last_sector && !store.flush() {
                return Err(Error::Write);
            }
        }
        if !store.flush() {
            return Err(Error::Write);
        }
        Ok(())
    }
}
