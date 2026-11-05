//! Microsoft's compound files ([MS-CFB]): a little file system inside a file, as Office keeps its files from before
//! 2007 (.doc, .xls, .ppt) and the files a password protects. The file is cut into sectors (512 bytes, or 4,096 in
//! version 4), chained by a table (the FAT) that the header's DIFAT finds; a directory names the streams and storages
//! (a red-black tree of each storage's children); and streams smaller than 4,096 bytes are kept in 64-byte sectors of
//! their own, inside the root's stream (the mini stream), chained by the mini FAT.
//!
//! Damaged files give None or shorter streams, never a crash: chains that loop or run off the file end there.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
/// The highest sector number (those above are marks: the chain's end, a FAT's or DIFAT's own sector, unused).
const MAX_SECTOR: u32 = 0xFFFF_FFFA;
const NO_ENTRY: u32 = 0xFFFF_FFFF;
const STREAM: u8 = 2;
const ROOT: u8 = 5;
/// The biggest stream read (the files read are at most 64 MB).
const MOST_STREAM: u64 = 64 << 20;

/// A directory entry: a stream or storage, its name, where its tree goes, and where its data starts.
struct Entry {
    name: String,
    kind: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: u64,
}

pub struct Compound<'a> {
    data: &'a [u8],
    shift: u32,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    mini: Vec<u8>,
    cutoff: u64,
    entries: Vec<Entry>,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

impl<'a> Compound<'a> {
    /// The compound file `data` is, if it is one.
    pub fn open(data: &'a [u8]) -> Option<Compound<'a>> {
        if data.len() < 512 || data[..8] != SIGNATURE {
            return None;
        }
        let major = u16_at(data, 0x1A)?;
        let shift = u16_at(data, 0x1E)? as u32;
        if shift != 9 && shift != 12 {
            return None;
        }
        let fat_sectors = u32_at(data, 0x2C)? as usize;
        let dir_start = u32_at(data, 0x30)?;
        let cutoff = u32_at(data, 0x38)? as u64;
        let mini_fat_start = u32_at(data, 0x3C)?;
        let difat_start = u32_at(data, 0x44)?;
        let mut c = Compound { data, shift, fat: Vec::new(), mini_fat: Vec::new(), mini: Vec::new(), cutoff, entries: Vec::new() };
        // The FAT's sectors: the first 109 in the header, the rest in the DIFAT's chain of sectors (each one's last
        // number the next's).
        let most = c.sectors();
        let mut fat_list: Vec<u32> = (0..109).filter_map(|i| u32_at(data, 0x4C + 4 * i)).filter(|&s| s <= MAX_SECTOR).collect();
        let mut d = difat_start;
        let per = (1usize << shift) / 4 - 1;
        let mut steps = 0;
        while d <= MAX_SECTOR && steps < most && fat_list.len() < fat_sectors {
            let s = c.sector(d)?;
            fat_list.extend((0..per).filter_map(|k| u32_at(s, 4 * k)).filter(|&s| s <= MAX_SECTOR));
            d = u32_at(s, 4 * per)?;
            steps += 1;
        }
        fat_list.truncate(fat_sectors.max(1));
        let mut fat = Vec::new();
        for s in fat_list {
            let sec = c.sector(s)?;
            fat.extend(sec.chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])));
        }
        c.fat = fat;
        // The directory: 128 bytes an entry.
        let dir = c.chain(dir_start, u64::MAX);
        for e in dir.chunks_exact(128) {
            let len = (u16_at(e, 0x40).unwrap_or(0) as usize).min(64);
            let units: Vec<u16> = e[..len].chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).take_while(|&u| u != 0).collect();
            let mut size = u32_at(e, 0x78).unwrap_or(0) as u64 | (u32_at(e, 0x7C).unwrap_or(0) as u64) << 32;
            if major == 3 {
                // (Version 3's sizes are 32 bits: what's above may be anything.)
                size &= 0xFFFF_FFFF;
            }
            c.entries.push(Entry {
                name: String::from_utf16_lossy(&units),
                kind: e[0x42],
                left: u32_at(e, 0x44).unwrap_or(NO_ENTRY),
                right: u32_at(e, 0x48).unwrap_or(NO_ENTRY),
                child: u32_at(e, 0x4C).unwrap_or(NO_ENTRY),
                start: u32_at(e, 0x74).unwrap_or(0),
                size,
            });
        }
        let root = c.entries.first()?;
        if root.kind != ROOT {
            return None;
        }
        let (root_start, root_size) = (root.start, root.size);
        c.mini = c.chain(root_start, root_size);
        let mini_fat = c.chain(mini_fat_start, u64::MAX);
        c.mini_fat = mini_fat.chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        Some(c)
    }

    /// How many sectors the file has room for.
    fn sectors(&self) -> usize {
        (self.data.len() >> self.shift).saturating_sub(1) + 1
    }

    /// Sector `n`'s bytes (a last one cut short as it is).
    fn sector(&self, n: u32) -> Option<&'a [u8]> {
        let size = 1usize << self.shift;
        let at = (n as usize).checked_add(1)?.checked_mul(size)?;
        let end = at.checked_add(size)?.min(self.data.len());
        self.data.get(at..end).filter(|s| !s.is_empty())
    }

    /// The bytes of the chain of sectors from `start`, at most `size` of them.
    fn chain(&self, start: u32, size: u64) -> Vec<u8> {
        let mut out = Vec::new();
        let mut s = start;
        let mut steps = 0;
        let want = size.min(MOST_STREAM) as usize;
        while s <= MAX_SECTOR && out.len() < want && steps <= self.fat.len() {
            let Some(sec) = self.sector(s) else { break };
            out.extend_from_slice(sec);
            s = self.fat.get(s as usize).copied().unwrap_or(NO_ENTRY);
            steps += 1;
        }
        out.truncate(want);
        out
    }

    /// The bytes of the chain of mini sectors (64 bytes each) from `start`, `size` of them.
    fn mini_chain(&self, start: u32, size: u64) -> Vec<u8> {
        let mut out = Vec::new();
        let mut s = start;
        let mut steps = 0;
        let want = size.min(MOST_STREAM) as usize;
        while s <= MAX_SECTOR && out.len() < want && steps <= self.mini_fat.len() {
            let at = s as usize * 64;
            let Some(sec) = self.mini.get(at..(at + 64).min(self.mini.len())).filter(|b| !b.is_empty()) else { break };
            out.extend_from_slice(sec);
            s = self.mini_fat.get(s as usize).copied().unwrap_or(NO_ENTRY);
            steps += 1;
        }
        out.truncate(want);
        out
    }

    /// The entries in a storage (by its entry's number): its tree of children, in no particular order.
    fn children(&self, storage: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut seen = vec![false; self.entries.len()];
        let mut stack = Vec::new();
        if let Some(e) = self.entries.get(storage) {
            stack.push(e.child);
        }
        while let Some(i) = stack.pop() {
            let i = i as usize;
            if i >= self.entries.len() || seen[i] {
                continue;
            }
            seen[i] = true;
            out.push(i);
            stack.push(self.entries[i].left);
            stack.push(self.entries[i].right);
        }
        out
    }

    /// Whether the file has a stream (or storage) of this name at its top (names are matched ignoring case).
    pub fn has(&self, name: &str) -> bool {
        self.children(0).iter().any(|&i| self.entries[i].name.eq_ignore_ascii_case(name))
    }

    /// A stream at the file's top, by name (ignoring case).
    pub fn stream(&self, name: &str) -> Option<Vec<u8>> {
        let i = self.children(0).into_iter().find(|&i| self.entries[i].kind == STREAM && self.entries[i].name.eq_ignore_ascii_case(name))?;
        let e = &self.entries[i];
        Some(if e.size < self.cutoff { self.mini_chain(e.start, e.size) } else { self.chain(e.start, e.size) })
    }

}
