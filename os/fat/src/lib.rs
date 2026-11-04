//! FAT file systems (Microsoft's FAT specification, "FAT: General Overview of On-Disk Format", 1.03): the file
//! system of every PC's EFI system partition (FAT32 when Windows makes it), and of most USB sticks and memory cards up
//! to 32 GB. Read, never changed. What Chat Computer needs from it: where a file's bytes are, sector by sector, so its
//! own memory file (made by Windows, its size fixed) can be read and written in place with nothing about the file
//! system itself ever written; and folders' lists and files' bytes, for Claude to read what's on a USB stick.
//!
//! FAT12, FAT16 and FAT32, long file names included (and 8.3 names Windows shows in small letters, by the flags it
//! keeps for them). Every number read from the disk is checked before it's used (a sector outside the volume, a
//! cluster chain that loops or runs off the end are errors, not crashes).

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[cfg(test)]
mod tests;

/// Where the volume's sectors come from: from sector `n` on (counted from the volume's first, of the volume's own
/// size), `buf.len()` bytes of them, a whole number of sectors. False if they couldn't be read.
pub trait Sectors {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> bool;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// A sector couldn't be read.
    Read,
    /// The first sector isn't a FAT volume's.
    NotFat,
    /// Something on the disk doesn't add up (a cluster outside the volume, a chain that loops...).
    Damaged,
    /// No file or folder of that name.
    NotFound,
    /// A file where a folder was wanted.
    NotAFolder,
    /// A folder where a file was wanted.
    NotAFile,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Fat12,
    Fat16,
    Fat32,
}

/// Files' and folders' attributes.
pub const READ_ONLY: u8 = 0x01;
pub const HIDDEN: u8 = 0x02;
pub const SYSTEM: u8 = 0x04;
const LABEL: u8 = 0x08;
pub const FOLDER: u8 = 0x10;
const LONG_NAME: u8 = 0x0F;

/// The most a folder holds: 65,536 entries of 32 bytes (the specification's limit).
const FOLDER_MOST: u64 = 65_536 * 32;

#[derive(Clone, Copy, Debug)]
pub struct Volume {
    pub kind: Kind,
    pub bytes_per_sector: u32,
    pub sectors_per_cluster: u32,
    /// All of the volume's sectors.
    pub sectors: u64,
    /// The FAT that's read: the first, or on FAT32 the one in use if only one is.
    fat_start: u64,
    fat_sectors: u64,
    /// The FAT12/16 root folder's sectors (a fixed region); FAT32's root is a cluster chain.
    root_start: u64,
    root_sectors: u64,
    root_cluster: u32,
    data_start: u64,
    /// Data clusters: numbered 2 to clusters + 1.
    pub clusters: u32,
}

/// A file or folder found: where its bytes start, and how many.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub first_cluster: u32,
    pub size: u32,
    pub folder: bool,
}

/// The root folder (as `find` and `list` take it).
pub fn root() -> Entry {
    Entry { first_cluster: 0, size: 0, folder: true }
}

/// A file or folder in a folder's list: its name (its long name, else its 8.3 name as Windows shows it), the entry
/// `find` gives for it, its attributes, and when it was last changed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Item {
    pub name: String,
    /// Its 8.3 name ("PROGRA~1"), which finds it too.
    pub short: String,
    pub entry: Entry,
    pub attributes: u8,
    /// Seconds since 1970, of the time as the volume keeps it: FAT keeps the PC's local time, with no time zone, so
    /// this is that time as if it were UTC (0 if the entry has no date).
    pub modified: i64,
}

fn u16_at(b: &[u8], i: usize) -> u32 {
    u16::from_le_bytes([b[i], b[i + 1]]) as u32
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Reads the volume's first sector (its BIOS parameter block) and works out the rest, as the specification says.
pub fn open(disk: &mut impl Sectors) -> Result<Volume, Error> {
    // The sector size isn't known yet: the first 512 bytes have it.
    let mut b = vec![0u8; 512];
    if !disk.read(0, &mut b) {
        return Err(Error::Read);
    }
    let bytes_per_sector = u16_at(&b, 11);
    let sectors_per_cluster = b[13] as u32;
    let reserved = u16_at(&b, 14) as u64;
    let fats = b[16] as u64;
    let root_entries = u16_at(&b, 17) as u64;
    let total16 = u16_at(&b, 19) as u64;
    let fat16 = u16_at(&b, 22) as u64;
    let total32 = u32_at(&b, 32) as u64;
    let fat32 = u32_at(&b, 36) as u64;
    let signature = u16_at(&b, 510);
    if signature != 0xAA55
        || !matches!(bytes_per_sector, 512 | 1024 | 2048 | 4096)
        || !sectors_per_cluster.is_power_of_two()
        || sectors_per_cluster > 128
        || reserved == 0
        || fats == 0
    {
        return Err(Error::NotFat);
    }
    let bps = bytes_per_sector as u64;
    let fat_sectors = if fat16 != 0 { fat16 } else { fat32 };
    let sectors = if total16 != 0 { total16 } else { total32 };
    let root_sectors = (root_entries * 32).div_ceil(bps);
    let data_start = reserved + fats * fat_sectors + root_sectors;
    if fat_sectors == 0 || data_start >= sectors {
        return Err(Error::NotFat);
    }
    let clusters = (sectors - data_start) / sectors_per_cluster as u64;
    let kind = match clusters {
        0..4085 => Kind::Fat12,
        4085..65525 => Kind::Fat16,
        _ => Kind::Fat32,
    };
    let root_cluster = if kind == Kind::Fat32 { u32_at(&b, 44) } else { 0 };
    // Each FAT must be big enough for every cluster.
    let entry_bits = match kind {
        Kind::Fat12 => 12,
        Kind::Fat16 => 16,
        Kind::Fat32 => 32,
    };
    if (clusters + 2) * entry_bits > fat_sectors * bps * 8 || clusters > 0x0FFF_FFF5 {
        return Err(Error::Damaged);
    }
    // FAT32 may keep only one of its FATs up to date (its flags' top bit): that one is read.
    let flags = u16_at(&b, 40);
    let active = if kind == Kind::Fat32 && flags & 0x80 != 0 && ((flags & 0xF) as u64) < fats { (flags & 0xF) as u64 } else { 0 };
    let v = Volume {
        kind,
        bytes_per_sector,
        sectors_per_cluster,
        sectors,
        fat_start: reserved + active * fat_sectors,
        fat_sectors,
        root_start: reserved + fats * fat_sectors,
        root_sectors,
        root_cluster,
        data_start,
        clusters: clusters as u32,
    };
    if kind == Kind::Fat32 && !v.valid_cluster(root_cluster) {
        return Err(Error::Damaged);
    }
    Ok(v)
}

/// The FAT, read a few sectors at a time and kept, so following a chain reads each of its sectors once.
struct Table<'v> {
    v: &'v Volume,
    /// The FAT's sectors held (counted from the FAT's first), and their bytes.
    first: u64,
    bytes: Vec<u8>,
}

/// FAT sectors read at once.
const TABLE_SECTORS: u64 = 8;

impl<'v> Table<'v> {
    fn new(v: &'v Volume) -> Table<'v> {
        Table { v, first: 0, bytes: Vec::new() }
    }

    /// The FAT's entry for cluster `c`: the next cluster, or None at the chain's end.
    fn next(&mut self, disk: &mut impl Sectors, c: u32) -> Result<Option<u32>, Error> {
        let v = self.v;
        if !v.valid_cluster(c) {
            return Err(Error::Damaged);
        }
        let bps = v.bytes_per_sector as u64;
        let (offset, end) = match v.kind {
            Kind::Fat12 => (c as u64 + c as u64 / 2, 0xFF8),
            Kind::Fat16 => (c as u64 * 2, 0xFFF8),
            Kind::Fat32 => (c as u64 * 4, 0x0FFF_FFF8),
        };
        // The entry's bytes (a FAT12 entry's two can be in two sectors), read if they aren't held.
        let width = if v.kind == Kind::Fat32 { 4 } else { 2 };
        let start = self.first * bps;
        if offset < start || offset + width > start + self.bytes.len() as u64 {
            let first = offset / bps;
            let count = TABLE_SECTORS.min(v.fat_sectors.saturating_sub(first));
            if (count * bps) < offset % bps + width || v.fat_start + first + count > v.sectors {
                return Err(Error::Damaged);
            }
            let mut bytes = vec![0u8; (count * bps) as usize];
            if !disk.read(v.fat_start + first, &mut bytes) {
                return Err(Error::Read);
            }
            (self.first, self.bytes) = (first, bytes);
        }
        let at = (offset - self.first * bps) as usize;
        let b = |i: usize| self.bytes[at + i] as u32;
        let value = match v.kind {
            Kind::Fat12 => {
                let x = b(0) | b(1) << 8;
                if c & 1 == 1 { x >> 4 } else { x & 0xFFF }
            }
            Kind::Fat16 => b(0) | b(1) << 8,
            Kind::Fat32 => (b(0) | b(1) << 8 | b(2) << 16 | b(3) << 24) & 0x0FFF_FFFF,
        };
        if value >= end {
            return Ok(None);
        }
        if !v.valid_cluster(value) {
            return Err(Error::Damaged);
        }
        Ok(Some(value))
    }
}

impl Volume {
    fn valid_cluster(&self, c: u32) -> bool {
        c >= 2 && c - 2 < self.clusters
    }

    /// The first sector of cluster `c`.
    pub fn cluster_sector(&self, c: u32) -> u64 {
        self.data_start + (c as u64 - 2) * self.sectors_per_cluster as u64
    }

    fn cluster_bytes(&self) -> u64 {
        self.bytes_per_sector as u64 * self.sectors_per_cluster as u64
    }

    /// Reads whole sectors from `first`, checked to be in the volume.
    fn sectors_at(&self, disk: &mut impl Sectors, first: u64, buf: &mut [u8]) -> Result<(), Error> {
        let count = buf.len() as u64 / self.bytes_per_sector as u64;
        if first.checked_add(count).is_none_or(|end| end > self.sectors) {
            return Err(Error::Damaged);
        }
        if disk.read(first, buf) { Ok(()) } else { Err(Error::Read) }
    }

    /// A cluster chain from `first`, in order (a chain longer than the volume has clusters loops: damaged).
    pub fn chain(&self, disk: &mut impl Sectors, first: u32) -> Result<Vec<u32>, Error> {
        if !self.valid_cluster(first) {
            return Err(Error::Damaged);
        }
        let mut table = Table::new(self);
        let mut chain = vec![first];
        let mut c = first;
        while let Some(next) = table.next(disk, c)? {
            if chain.len() as u32 >= self.clusters {
                return Err(Error::Damaged);
            }
            chain.push(next);
            c = next;
        }
        Ok(chain)
    }

    /// A folder's bytes (its 32-byte entries), up to its end mark: FAT12/16's fixed root, or a cluster chain's.
    fn folder_bytes(&self, disk: &mut impl Sectors, folder: &Entry) -> Result<Vec<u8>, Error> {
        if !folder.folder {
            return Err(Error::NotAFolder);
        }
        let ends = |b: &[u8]| b.chunks_exact(32).any(|e| e[0] == 0);
        let mut out = Vec::new();
        if folder.first_cluster == 0 && self.kind != Kind::Fat32 {
            // A sector at a time, to the end mark.
            let bps = self.bytes_per_sector as usize;
            for s in self.root_start..self.root_start + self.root_sectors {
                let at = out.len();
                out.resize(at + bps, 0);
                self.sectors_at(disk, s, &mut out[at..])?;
                if ends(&out[at..]) {
                    break;
                }
            }
            return Ok(out);
        }
        let first = if folder.first_cluster == 0 { self.root_cluster } else { folder.first_cluster };
        if !self.valid_cluster(first) {
            return Err(Error::Damaged);
        }
        let mut table = Table::new(self);
        let mut c = first;
        loop {
            let at = out.len();
            out.resize(at + self.cluster_bytes() as usize, 0);
            self.sectors_at(disk, self.cluster_sector(c), &mut out[at..])?;
            if ends(&out[at..]) || out.len() as u64 >= FOLDER_MOST {
                return Ok(out);
            }
            match table.next(disk, c)? {
                Some(next) => c = next,
                None => return Ok(out),
            }
        }
    }

    /// Each file and folder in a folder, in order, given to `each` until it returns false. Volume labels, "." and
    /// "..", and deleted entries are passed over.
    fn each_item(&self, disk: &mut impl Sectors, folder: &Entry, mut each: impl FnMut(Item) -> bool) -> Result<(), Error> {
        let raw = self.folder_bytes(disk, folder)?;
        let mut long = LongName::default();
        for e in raw.chunks_exact(32) {
            match e[0] {
                0x00 => break, // the folder's end
                0xE5 => {
                    long = LongName::default();
                    continue;
                }
                _ => {}
            }
            let attributes = e[11];
            if attributes & 0x3F == LONG_NAME {
                long.piece(e);
                continue;
            }
            let short: [u8; 11] = e[..11].try_into().unwrap();
            let name = long.take(&short);
            if attributes & LABEL != 0 || short[0] == b'.' {
                continue;
            }
            let first = u16_at(e, 20) << 16 | u16_at(e, 26);
            let first_cluster = if self.kind == Kind::Fat32 { first } else { first & 0xFFFF };
            let folder = attributes & FOLDER != 0;
            let entry = Entry { first_cluster, size: if folder { 0 } else { u32_at(e, 28) }, folder };
            let plain = short_name(&short, e[12]);
            let item = Item { name: name.unwrap_or_else(|| plain.clone()), short: plain, entry, attributes, modified: time(u16_at(e, 24), u16_at(e, 22)) };
            if !each(item) {
                break;
            }
        }
        Ok(())
    }

    /// The files and folders in a folder (`root()` for the root), in the folder's order.
    pub fn list(&self, disk: &mut impl Sectors, folder: &Entry) -> Result<Vec<Item>, Error> {
        let mut out = Vec::new();
        self.each_item(disk, folder, |item| {
            out.push(item);
            true
        })?;
        Ok(out)
    }

    /// The file or folder named `name` in a folder: by its long name or its 8.3 one, whatever their case.
    pub fn child(&self, disk: &mut impl Sectors, folder: &Entry, name: &str) -> Result<Item, Error> {
        let mut found = None;
        self.each_item(disk, folder, |item| {
            if same_name(&item.name, name) || same_name(&item.short, name) {
                found = Some(item);
                return false;
            }
            true
        })?;
        found.ok_or(Error::NotFound)
    }

    /// Finds `name` (its long name or its 8.3 one, in any case) in the folder that starts at `folder` (0: the root).
    pub fn find_in(&self, disk: &mut impl Sectors, folder: u32, name: &str) -> Result<Entry, Error> {
        Ok(self.child(disk, &Entry { first_cluster: folder, size: 0, folder: true }, name)?.entry)
    }

    /// Finds a file or folder by its path from the root ("EFI", "chatcomputer-own", "memory.bin").
    pub fn find(&self, disk: &mut impl Sectors, path: &[&str]) -> Result<Entry, Error> {
        let mut at = root();
        for name in path {
            if !at.folder {
                return Err(Error::NotFound);
            }
            at = self.find_in(disk, at.first_cluster, name)?;
        }
        Ok(at)
    }

    /// The sectors that hold a file's bytes, in order (just enough for its size).
    pub fn file_sectors(&self, disk: &mut impl Sectors, file: &Entry) -> Result<Vec<u64>, Error> {
        let needed = (file.size as u64).div_ceil(self.bytes_per_sector as u64);
        if needed == 0 {
            return Ok(Vec::new());
        }
        let mut sectors = Vec::new();
        for c in self.chain(disk, file.first_cluster)? {
            let s = self.cluster_sector(c);
            sectors.extend(s..s + self.sectors_per_cluster as u64);
            if sectors.len() as u64 >= needed {
                sectors.truncate(needed as usize);
                return Ok(sectors);
            }
        }
        Err(Error::Damaged) // its chain is shorter than its size
    }

    /// `buf.len()` bytes of a file from byte `offset` (fewer at its end): how many were read.
    pub fn read(&self, disk: &mut impl Sectors, file: &Entry, offset: u64, buf: &mut [u8]) -> Result<usize, Error> {
        if file.folder {
            return Err(Error::NotAFile);
        }
        let size = file.size as u64;
        if offset >= size || buf.is_empty() {
            return Ok(0);
        }
        let n = (buf.len() as u64).min(size - offset) as usize;
        let cb = self.cluster_bytes();
        // The clusters the bytes wanted are in: the chain followed to the first, then on to the last.
        let mut table = Table::new(self);
        let mut c = file.first_cluster;
        if !self.valid_cluster(c) {
            return Err(Error::Damaged);
        }
        for _ in 0..offset / cb {
            c = table.next(disk, c)?.ok_or(Error::Damaged)?;
        }
        let count = (offset % cb + n as u64).div_ceil(cb) as usize;
        let mut clusters = Vec::with_capacity(count);
        clusters.push(c);
        while clusters.len() < count {
            c = table.next(disk, c)?.ok_or(Error::Damaged)?;
            clusters.push(c);
        }
        // Read a run of clusters in a row at a time.
        let mut done = 0usize;
        let mut skip = offset % cb;
        let mut i = 0;
        while done < n {
            let mut run = 1;
            while i + run < clusters.len() && clusters[i + run] == clusters[i] + run as u32 {
                run += 1;
            }
            let take = ((run as u64 * cb - skip) as usize).min(n - done);
            self.bytes_at(disk, self.cluster_sector(clusters[i]), skip, &mut buf[done..done + take])?;
            done += take;
            skip = 0;
            i += run;
        }
        Ok(n)
    }

    /// `out.len()` bytes from `skip` bytes into sector `first`: whole sectors straight into `out`, the parts of
    /// sectors at either end through a sector of its own.
    fn bytes_at(&self, disk: &mut impl Sectors, first: u64, skip: u64, out: &mut [u8]) -> Result<(), Error> {
        let bps = self.bytes_per_sector as u64;
        let mut sector = first + skip / bps;
        let mut within = (skip % bps) as usize;
        let mut done = 0;
        let mut one = Vec::new();
        while done < out.len() {
            let left = out.len() - done;
            if within == 0 && left as u64 >= bps {
                let whole = (left as u64 / bps * bps) as usize;
                self.sectors_at(disk, sector, &mut out[done..done + whole])?;
                done += whole;
                sector += whole as u64 / bps;
            } else {
                one.resize(bps as usize, 0);
                self.sectors_at(disk, sector, &mut one)?;
                let take = (bps as usize - within).min(left);
                out[done..done + take].copy_from_slice(&one[within..within + take]);
                done += take;
                within = 0;
                sector += 1;
            }
        }
        Ok(())
    }

    /// The volume's name, as Windows shows it: the root folder's label entry, else the one in its first sector
    /// (unless that's "NO NAME"). Empty if it has none.
    pub fn label(&self, disk: &mut impl Sectors) -> Result<String, Error> {
        let raw = self.folder_bytes(disk, &root())?;
        for e in raw.chunks_exact(32) {
            if e[0] == 0 {
                break;
            }
            if e[0] != 0xE5 && e[11] & 0x3F != LONG_NAME && e[11] & LABEL != 0 {
                return Ok(String::from(oem(&e[..11]).trim_end()));
            }
        }
        let mut b = vec![0u8; 512];
        if !disk.read(0, &mut b) {
            return Err(Error::Read);
        }
        let at = if self.kind == Kind::Fat32 { 64 } else { 36 };
        if b[at + 2] != 0x29 {
            return Ok(String::new());
        }
        let label = String::from(oem(&b[at + 7..at + 18]).trim_end());
        Ok(if label == "NO NAME" { String::new() } else { label })
    }
}

/// A long name's pieces, gathered as they come (last piece first) until the 8.3 entry they belong to.
#[derive(Default)]
struct LongName {
    units: Vec<u16>,
    /// The piece expected next (they count down to 1), and the checksum of the 8.3 name they all carry.
    next: u8,
    sum: u8,
}

impl LongName {
    fn piece(&mut self, e: &[u8]) {
        let order = e[0] & 0x1F;
        if e[0] & 0x40 != 0 {
            // The last piece, which comes first: 13 UTF-16 units a piece, at most 20 pieces.
            if order == 0 || order > 20 {
                *self = LongName::default();
                return;
            }
            *self = LongName { units: vec![0xFFFF; 13 * order as usize], next: order, sum: e[13] };
        } else if order == 0 || order != self.next || e[13] != self.sum {
            *self = LongName::default();
            return;
        }
        let at = 13 * (order as usize - 1);
        for (k, i) in [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30].into_iter().enumerate() {
            self.units[at + k] = u16_at(e, i) as u16;
        }
        self.next = order - 1;
    }

    /// The long name, if every piece came and they belong to this 8.3 name; the pieces are cleared either way.
    fn take(&mut self, short: &[u8; 11]) -> Option<String> {
        let me = core::mem::take(self);
        if me.units.is_empty() || me.next != 0 || me.sum != short_checksum(short) {
            return None;
        }
        let end = me.units.iter().position(|&u| u == 0 || u == 0xFFFF).unwrap_or(me.units.len());
        let name: String = char::decode_utf16(me.units[..end].iter().copied()).map(|c| c.unwrap_or('\u{FFFD}')).collect();
        (!name.is_empty()).then_some(name)
    }
}

/// Bytes of an 8.3 name or a label as characters (bytes over 127 as Latin-1's: the PC's code page isn't known).
fn oem(b: &[u8]) -> String {
    b.iter().map(|&b| b as char).collect()
}

/// An 8.3 name as Windows shows it: "NAME.EXT", spaces taken off, no dot without an extension; in small letters
/// where `case` (byte 12 of the entry, which Windows NT and later keep) says the name or the extension is.
fn short_name(n: &[u8; 11], case: u8) -> String {
    let first = if n[0] == 0x05 { 0xE5 } else { n[0] };
    let mut base = oem(&[&[first][..], &n[1..8]].concat());
    let mut ext = oem(&n[8..]);
    if case & 0x08 != 0 {
        base = base.to_ascii_lowercase();
    }
    if case & 0x10 != 0 {
        ext = ext.to_ascii_lowercase();
    }
    let (base, ext) = (base.trim_end(), ext.trim_end());
    if ext.is_empty() { String::from(base) } else { alloc::format!("{base}.{ext}") }
}

/// The checksum long-name pieces carry of their 8.3 name.
fn short_checksum(n: &[u8; 11]) -> u8 {
    n.iter().fold(0u8, |sum, &b| sum.rotate_right(1).wrapping_add(b))
}

/// Whether two names are the same but for case (as Windows compares names on FAT).
fn same_name(a: &str, b: &str) -> bool {
    a.chars().flat_map(char::to_uppercase).eq(b.chars().flat_map(char::to_uppercase))
}

/// Days from 1970-01-01 to a date (the proleptic Gregorian calendar).
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// A FAT date (years from 1980, month, day) and time (hours, minutes, seconds halved) as seconds since 1970, as if
/// UTC; 0 for no date.
fn time(date: u32, time: u32) -> i64 {
    if date == 0 {
        return 0;
    }
    let (y, m, d) = (1980 + (date >> 9) as i64, ((date >> 5) & 15).clamp(1, 12) as i64, (date & 31).max(1) as i64);
    days(y, m, d) * 86_400 + (time >> 11) as i64 * 3600 + ((time >> 5) & 63) as i64 * 60 + (time & 31) as i64 * 2
}
