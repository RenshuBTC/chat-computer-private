//! exFAT, read: the file system of most USB sticks bigger than 32 GB, SD cards, and drives shared between Windows and
//! Macs. Never written.
//!
//! As Microsoft's exFAT specification describes it: a boot sector ("EXFAT   ", where the FAT and the cluster heap
//! are, the cluster's size, the root folder's first cluster); a FAT of 32-bit entries (each cluster's next, or the end
//! of a chain); folders of 32-byte entries, a file or folder a set of them: its primary entry (its attributes and
//! times), its stream extension (its size, its first cluster, whether its clusters are in one run, so not in the
//! FAT), and its name, 15 UTF-16 units an entry. The root folder also has the volume's name and its up-case table
//! (names are compared in capitals, by that table, as Windows compares them). Every number read from the disk is
//! checked before it's used, so a damaged volume gives an error, never a crash; a chain of clusters that loops is cut
//! off.

#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Where the volume's bytes come from: `buf.len()` bytes at byte `offset` of it.
pub trait Disk {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    Read,
    /// Not exFAT.
    NotExfat,
    /// Something in it doesn't add up.
    Damaged,
    NotFound,
    NotAFolder,
    NotAFile,
}

/// File attributes, as FAT's.
pub const READ_ONLY: u16 = 0x01;
pub const HIDDEN: u16 = 0x02;
pub const SYSTEM: u16 = 0x04;
pub const FOLDER: u16 = 0x10;

/// Directory entries' types (in use).
const UPCASE: u8 = 0x82;
const LABEL: u8 = 0x83;
const FILE: u8 = 0x85;
const STREAM: u8 = 0xC0;
const NAME: u8 = 0xC1;

/// The most entries a folder is read for (a folder of 256 MB of entries: far more than any has).
const MOST_ENTRIES: u64 = 8 << 20;

pub struct Volume {
    pub bytes_per_sector: u32,
    pub cluster_size: u32,
    pub clusters: u32,
    /// Where the (first) FAT and the cluster heap start, in bytes.
    fat: u64,
    heap: u64,
    root: u32,
    /// Each UTF-16 unit in capitals, by the volume's own up-case table (empty: ASCII's only).
    upcase: Vec<u16>,
    /// The volume's name.
    pub label: String,
}

/// A file or folder in a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub folder: bool,
    /// Its bytes (the end past what's been written reads as zeros).
    pub size: u64,
    pub attributes: u16,
    /// When it was last changed, in seconds since 1970 (UTC where the volume says the time's offset, else as the PC's
    /// clock had it).
    pub modified: i64,
    first: u32,
    /// Its clusters in one run, not in the FAT.
    contiguous: bool,
    /// How much has been written.
    valid: u64,
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
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

/// An exFAT timestamp (FAT's date and time: the date in its top 16 bits) and its offset from UTC (bit 7: there is
/// one; the rest, in quarters of an hour, signed), as seconds since 1970.
fn unix_time(stamp: u32, offset: u8) -> i64 {
    let (date, time) = (stamp >> 16, stamp & 0xFFFF);
    let (y, m, d) = (1980 + (date >> 9) as i64, ((date >> 5) & 15).clamp(1, 12) as i64, (date & 31).max(1) as i64);
    let secs = (time >> 11) as i64 * 3600 + ((time >> 5) & 63) as i64 * 60 + (time & 31) as i64 * 2;
    let local = days(y, m, d) * 86_400 + secs;
    if offset & 0x80 != 0 {
        // (Seven bits, signed.)
        let quarters = ((offset << 1) as i8 >> 1) as i64;
        local - quarters * 900
    } else {
        local
    }
}

/// Reads the volume's boot sector, then its root folder's volume name and up-case table.
pub fn open(disk: &mut impl Disk) -> Result<Volume, Error> {
    let mut b = [0u8; 512];
    if !disk.read(0, &mut b) {
        return Err(Error::Read);
    }
    if &b[3..11] != b"EXFAT   " || b[510..512] != [0x55, 0xAA] {
        return Err(Error::NotExfat);
    }
    let (sector_shift, cluster_shift) = (b[108] as u32, b[109] as u32);
    if !(9..=12).contains(&sector_shift) || sector_shift + cluster_shift > 25 {
        return Err(Error::Damaged);
    }
    let bytes_per_sector = 1u32 << sector_shift;
    let cluster_size = 1u32 << (sector_shift + cluster_shift);
    let (fat, heap, clusters, root) = (u32_at(&b, 80), u32_at(&b, 88), u32_at(&b, 92), u32_at(&b, 96));
    if clusters == 0 || clusters > 0xFFFF_FFF5 || root < 2 || root > clusters + 1 || fat == 0 || heap == 0 {
        return Err(Error::Damaged);
    }
    let mut v = Volume {
        bytes_per_sector,
        cluster_size,
        clusters,
        fat: fat as u64 * bytes_per_sector as u64,
        heap: heap as u64 * bytes_per_sector as u64,
        root,
        upcase: Vec::new(),
        label: String::new(),
    };
    // The root folder's own entries: the volume's name, and the up-case table (its runs of mappings; FFFFh and a
    // count: that many units that are their own capitals).
    let root = root_entry(&v);
    let raw = v.entries(disk, &root)?;
    for e in raw.chunks_exact(32) {
        match e[0] {
            0 => break,
            LABEL => {
                let n = (e[1] as usize).min(11);
                v.label = utf16(&e[2..2 + 2 * n]);
            }
            UPCASE => {
                let (first, size) = (u32_at(e, 20), u64_at(e, 24));
                if size == 0 || size > 1 << 17 || first < 2 || first > v.clusters + 1 {
                    continue;
                }
                let table = Entry { name: String::new(), folder: false, size, attributes: 0, modified: 0, first, contiguous: false, valid: size };
                let mut packed = vec![0u8; size as usize];
                if v.read(disk, &table, 0, &mut packed)? != packed.len() {
                    return Err(Error::Damaged);
                }
                let units: Vec<u16> = packed.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                let mut up: Vec<u16> = Vec::with_capacity(1 << 16);
                let mut i = 0;
                while i < units.len() && up.len() < 1 << 16 {
                    if units[i] == 0xFFFF && i + 1 < units.len() {
                        for _ in 0..units[i + 1] {
                            if up.len() == 1 << 16 {
                                break;
                            }
                            up.push(up.len() as u16);
                        }
                        i += 2;
                    } else {
                        up.push(units[i]);
                        i += 1;
                    }
                }
                while up.len() < 1 << 16 {
                    up.push(up.len() as u16);
                }
                v.upcase = up;
            }
            _ => {}
        }
    }
    Ok(v)
}

fn utf16(b: &[u8]) -> String {
    let units = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
    char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect()
}

/// The root folder, as an entry (it isn't in a folder: its clusters are in the FAT).
fn root_entry(v: &Volume) -> Entry {
    Entry { name: String::new(), folder: true, size: 0, attributes: FOLDER, modified: 0, first: v.root, contiguous: false, valid: 0 }
}

/// The FAT, read 4 KiB (1024 entries) at a time and kept, so following a chain reads each part of it once.
struct Table {
    /// The first entry held, and the entries.
    at: u32,
    entries: Vec<u32>,
}

impl Table {
    /// Cluster `c`'s entry: the next cluster in its chain, or FFFFFFFFh at its end.
    fn next(&mut self, v: &Volume, disk: &mut impl Disk, c: u32) -> Result<u32, Error> {
        if c < self.at || c - self.at >= self.entries.len() as u32 {
            let at = c & !1023;
            let count = (1024u64).min(v.clusters as u64 + 2 - at as u64) as usize;
            let mut b = vec![0u8; 4 * count];
            if !disk.read(v.fat + 4 * at as u64, &mut b) {
                return Err(Error::Read);
            }
            self.at = at;
            self.entries = b.chunks_exact(4).map(|e| u32::from_le_bytes(e.try_into().unwrap())).collect();
        }
        self.entries.get((c - self.at) as usize).copied().ok_or(Error::Damaged)
    }
}

impl Volume {
    /// The root folder.
    pub fn root(&self) -> Entry {
        root_entry(self)
    }

    /// `count` of a file's or folder's clusters, in order, from its `skip`th: in one run (`contiguous`), or along its
    /// chain in the FAT, which mustn't end sooner, loop or leave the volume. `count` None: a folder's, to the chain's
    /// end (the root folder, whose size isn't kept, or another without one), but no further than the most entries a
    /// folder is read for: a chain that loops is otherwise only caught after as many steps as the volume says it has
    /// clusters, which it can say is four billion (16 GB of cluster numbers, from a stick of any size).
    fn clusters_of(&self, disk: &mut impl Disk, e: &Entry, skip: u64, count: Option<u64>) -> Result<Vec<u32>, Error> {
        let most = MOST_ENTRIES * 32 / self.cluster_size as u64 + 1;
        if e.first == 0 {
            return Ok(Vec::new());
        }
        let last = self.clusters as u64 + 1;
        if e.first < 2 || e.first as u64 > last {
            return Err(Error::Damaged);
        }
        if e.contiguous {
            let count = count.ok_or(Error::Damaged)?;
            if e.first as u64 + skip + count > last + 1 {
                return Err(Error::Damaged);
            }
            return Ok((0..count).map(|i| (e.first as u64 + skip + i) as u32).collect());
        }
        let mut table = Table { at: 0, entries: Vec::new() };
        let mut c = e.first;
        let mut steps = 0u64;
        let mut out = Vec::new();
        loop {
            if steps >= skip {
                out.push(c);
                if count.is_some_and(|n| out.len() as u64 == n) || count.is_none() && out.len() as u64 >= most {
                    return Ok(out);
                }
            }
            steps += 1;
            if steps > self.clusters as u64 {
                return Err(Error::Damaged); // it loops
            }
            match table.next(self, disk, c)? {
                0xFFFF_FFFF if count.is_none() => return Ok(out),
                n if n >= 2 && n as u64 <= last => c = n,
                _ => return Err(Error::Damaged),
            }
        }
    }

    /// A folder's entries, raw (32 bytes each), to its end or its first end-of-folder entry.
    fn entries(&self, disk: &mut impl Disk, folder: &Entry) -> Result<Vec<u8>, Error> {
        if folder.size > MOST_ENTRIES * 32 {
            return Err(Error::Damaged);
        }
        // A folder is a chain of clusters ending at its end-of-directory entry, not a length: its DataLength is 0 for the
        // root (which has no stream entry) and can be 0 for any folder. Only a folder with a length stops at that many
        // clusters.
        let count = if folder.folder && folder.size == 0 { None } else { Some(folder.size.div_ceil(self.cluster_size as u64)) };
        let mut out = Vec::new();
        for c in self.clusters_of(disk, folder, 0, count)? {
            let at = out.len();
            out.resize(at + self.cluster_size as usize, 0);
            if !disk.read(self.heap + (c as u64 - 2) * self.cluster_size as u64, &mut out[at..]) {
                return Err(Error::Read);
            }
            if out[at..].chunks_exact(32).any(|e| e[0] == 0) || out.len() as u64 > MOST_ENTRIES * 32 {
                break;
            }
        }
        Ok(out)
    }

    /// The files and folders in a folder, in its order.
    pub fn list(&self, disk: &mut impl Disk, folder: &Entry) -> Result<Vec<Entry>, Error> {
        if !folder.folder {
            return Err(Error::NotAFolder);
        }
        let raw = self.entries(disk, folder)?;
        let mut out = Vec::new();
        let mut at = 0;
        while at + 32 <= raw.len() {
            let e = &raw[at..at + 32];
            if e[0] == 0 {
                break;
            }
            if e[0] != FILE {
                at += 32;
                continue;
            }
            // The set: the file's entry, its stream extension, its name's entries.
            let count = e[1] as usize;
            if count < 2 || at + 32 * (count + 1) > raw.len() {
                at += 32;
                continue;
            }
            let set = &raw[at..at + 32 * (count + 1)];
            at += 32 * (count + 1);
            let stream = &set[32..64];
            if stream[0] != STREAM {
                continue;
            }
            let name_len = stream[3] as usize;
            let mut units = Vec::with_capacity(name_len);
            for n in set[64..].chunks_exact(32).take_while(|n| n[0] == NAME) {
                units.extend(n[2..32].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])));
            }
            if units.len() < name_len {
                continue;
            }
            units.truncate(name_len);
            let name: String = char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect();
            let attributes = u16_at(e, 4);
            let size = u64_at(stream, 24);
            out.push(Entry {
                name,
                folder: attributes & FOLDER != 0,
                size,
                attributes,
                // (Its time to two seconds, then hundredths of a second more: up to 1.99 s.)
                modified: unix_time(u32_at(e, 12), e[23]) + e[21] as i64 / 100,
                first: u32_at(stream, 20),
                contiguous: stream[1] & 2 != 0,
                valid: u64_at(stream, 8).min(size),
            });
        }
        Ok(out)
    }

    /// A name in capitals, as the volume compares names.
    fn upper(&self, name: &str) -> Vec<u16> {
        name.encode_utf16()
            .map(|u| match self.upcase.get(u as usize) {
                Some(&up) => up,
                None => if (b'a' as u16..=b'z' as u16).contains(&u) { u - 32 } else { u },
            })
            .collect()
    }

    /// The file or folder of this name in a folder (whatever its case).
    pub fn child(&self, disk: &mut impl Disk, folder: &Entry, name: &str) -> Result<Entry, Error> {
        let want = self.upper(name);
        self.list(disk, folder)?.into_iter().find(|e| self.upper(&e.name) == want).ok_or(Error::NotFound)
    }

    /// The file or folder at a path from the root ("Photos\2024/beach.jpg").
    pub fn find(&self, disk: &mut impl Disk, path: &str) -> Result<Entry, Error> {
        let mut here = self.root();
        for part in path.split(['\\', '/']).filter(|p| !p.is_empty()) {
            if !here.folder {
                return Err(Error::NotAFolder);
            }
            here = self.child(disk, &here, part)?;
        }
        Ok(here)
    }

    /// `buf.len()` bytes of a file from byte `offset` (fewer at its end): how many were read. Past what's been
    /// written, zeros.
    pub fn read(&self, disk: &mut impl Disk, file: &Entry, offset: u64, buf: &mut [u8]) -> Result<usize, Error> {
        if file.folder {
            return Err(Error::NotAFile);
        }
        if offset >= file.size || buf.is_empty() {
            return Ok(0);
        }
        let n = (buf.len() as u64).min(file.size - offset) as usize;
        let cs = self.cluster_size as u64;
        // What's been written is read, a run of clusters in a row at a time; past it, zeros.
        let written = ((file.valid.max(offset) - offset) as usize).min(n);
        buf[written..n].fill(0);
        if written == 0 {
            return Ok(n);
        }
        let skip = offset % cs;
        let clusters = self.clusters_of(disk, file, offset / cs, Some((skip + written as u64).div_ceil(cs)))?;
        let (mut done, mut within, mut i) = (0usize, skip, 0usize);
        while done < written {
            let mut run = 1;
            while i + run < clusters.len() && clusters[i + run] == clusters[i] + run as u32 {
                run += 1;
            }
            let take = ((run as u64 * cs - within) as usize).min(written - done);
            if !disk.read(self.heap + (clusters[i] as u64 - 2) * cs + within, &mut buf[done..done + take]) {
                return Err(Error::Read);
            }
            done += take;
            within = 0;
            i += run;
        }
        Ok(n)
    }
}
