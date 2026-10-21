//! FAT file systems (Microsoft's FAT specification, "FAT: General Overview of On-Disk Format", 1.03): the file
//! system of every PC's EFI system partition (FAT32 when Windows makes it), read but never changed. What Chat
//! Computer needs from it: where a file's bytes are, sector by sector, so its own memory file (made by Windows, its
//! size fixed) can be read and written in place, with nothing about the file system itself ever written.
//!
//! FAT12, FAT16 and FAT32, long file names included. Every number read from the disk is checked before it's used
//! (a sector outside the volume, a cluster chain that loops or runs off the end are errors, not crashes).

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[cfg(test)]
mod tests;

/// Where the volume's sectors come from: sector `n` (counted from the volume's first, of the volume's own size)
/// into `buf`. False if it couldn't be read.
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
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Fat12,
    Fat16,
    Fat32,
}

#[derive(Clone, Copy, Debug)]
pub struct Volume {
    pub kind: Kind,
    pub bytes_per_sector: u32,
    pub sectors_per_cluster: u32,
    /// All of the volume's sectors.
    pub sectors: u64,
    fat_start: u64,
    /// The FAT12/16 root folder's sectors (a fixed region); FAT32's root is a cluster chain.
    root_start: u64,
    root_sectors: u64,
    root_cluster: u32,
    data_start: u64,
    /// Data clusters: numbered 2 to clusters + 1.
    pub clusters: u32,
}

/// A file or folder found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub first_cluster: u32,
    pub size: u32,
    pub folder: bool,
}

fn u16_at(b: &[u8], i: usize) -> u32 {
    u16::from_le_bytes([b[i], b[i + 1]]) as u32
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Reads the volume's first sector (its BIOS parameter block) and works out the rest, as the specification says.
pub fn open(disk: &mut impl Sectors) -> Result<Volume, Error> {
    // The sector size isn't known yet: read the largest a FAT volume may have.
    let mut b = vec![0u8; 4096];
    if !disk.read(0, &mut b[..512]) {
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
    let v = Volume {
        kind,
        bytes_per_sector,
        sectors_per_cluster,
        sectors,
        fat_start: reserved,
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

impl Volume {
    fn valid_cluster(&self, c: u32) -> bool {
        c >= 2 && c - 2 < self.clusters
    }

    /// The first sector of cluster `c`.
    pub fn cluster_sector(&self, c: u32) -> u64 {
        self.data_start + (c as u64 - 2) * self.sectors_per_cluster as u64
    }

    /// Reads one sector, checked to be in the volume.
    fn sector(&self, disk: &mut impl Sectors, n: u64, buf: &mut [u8]) -> Result<(), Error> {
        if n >= self.sectors {
            return Err(Error::Damaged);
        }
        if disk.read(n, &mut buf[..self.bytes_per_sector as usize]) { Ok(()) } else { Err(Error::Read) }
    }

    /// The FAT's entry for cluster `c`: the next cluster, or None at the chain's end.
    fn next(&self, disk: &mut impl Sectors, c: u32) -> Result<Option<u32>, Error> {
        let bps = self.bytes_per_sector as u64;
        let mut buf = vec![0u8; 2 * bps as usize];
        let (offset, end) = match self.kind {
            Kind::Fat12 => (c as u64 + c as u64 / 2, 0xFF8),
            Kind::Fat16 => (c as u64 * 2, 0xFFF8),
            Kind::Fat32 => (c as u64 * 4, 0x0FFF_FFF8),
        };
        let first = self.fat_start + offset / bps;
        let at = (offset % bps) as usize;
        self.sector(disk, first, &mut buf[..bps as usize])?;
        // A FAT12 entry can straddle two sectors.
        if at + 4 > bps as usize && first + 1 < self.root_start {
            let (_, second) = buf.split_at_mut(bps as usize);
            self.sector(disk, first + 1, second)?;
        }
        let value = match self.kind {
            Kind::Fat12 => {
                let v = u16_at(&buf, at);
                if c & 1 == 1 { v >> 4 } else { v & 0xFFF }
            }
            Kind::Fat16 => u16_at(&buf, at),
            Kind::Fat32 => u32_at(&buf, at) & 0x0FFF_FFFF,
        };
        if value >= end {
            return Ok(None);
        }
        if !self.valid_cluster(value) {
            return Err(Error::Damaged);
        }
        Ok(Some(value))
    }

    /// A cluster chain from `first`, in order (a chain longer than the volume has clusters loops: damaged).
    pub fn chain(&self, disk: &mut impl Sectors, first: u32) -> Result<Vec<u32>, Error> {
        if !self.valid_cluster(first) {
            return Err(Error::Damaged);
        }
        let mut chain = vec![first];
        let mut c = first;
        while let Some(next) = self.next(disk, c)? {
            if chain.len() as u32 >= self.clusters {
                return Err(Error::Damaged);
            }
            chain.push(next);
            c = next;
        }
        Ok(chain)
    }

    /// A folder's sectors: FAT12/16's fixed root, or a cluster chain's.
    fn folder_sectors(&self, disk: &mut impl Sectors, first_cluster: u32) -> Result<Vec<u64>, Error> {
        if first_cluster == 0 {
            if self.kind == Kind::Fat32 {
                return self.folder_sectors(disk, self.root_cluster);
            }
            return Ok((self.root_start..self.root_start + self.root_sectors).collect());
        }
        let mut sectors = Vec::new();
        for c in self.chain(disk, first_cluster)? {
            let s = self.cluster_sector(c);
            sectors.extend(s..s + self.sectors_per_cluster as u64);
        }
        Ok(sectors)
    }

    /// Finds `name` (its long name or its 8.3 one, in any case) in the folder that starts at `folder` (0: the root).
    pub fn find_in(&self, disk: &mut impl Sectors, folder: u32, name: &str) -> Result<Entry, Error> {
        let mut buf = vec![0u8; self.bytes_per_sector as usize];
        let mut long: Vec<u16> = Vec::new();
        let mut long_sum: Option<u8> = None;
        for s in self.folder_sectors(disk, folder)? {
            self.sector(disk, s, &mut buf)?;
            for e in buf.chunks_exact(32) {
                match e[0] {
                    0x00 => return Err(Error::NotFound), // the folder's end
                    0xE5 => {
                        long.clear();
                        long_sum = None;
                        continue;
                    }
                    _ => {}
                }
                let attributes = e[11];
                if attributes & 0x3F == 0x0F {
                    // A piece of a long name: 13 UTF-16 characters, the pieces stored last first.
                    let order = e[0] & 0x1F;
                    if e[0] & 0x40 != 0 {
                        long.clear();
                    }
                    let mut piece = Vec::with_capacity(13);
                    for i in [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30] {
                        let ch = u16_at(e, i) as u16;
                        if ch == 0 || ch == 0xFFFF {
                            break;
                        }
                        piece.push(ch);
                    }
                    if order == 0 {
                        long.clear();
                        continue;
                    }
                    let mut joined = piece;
                    joined.extend_from_slice(&long);
                    long = joined;
                    long_sum = Some(e[13]);
                    continue;
                }
                let short: [u8; 11] = e[..11].try_into().unwrap();
                let long_name = match long_sum {
                    Some(sum) if sum == short_checksum(&short) => Some(String::from_utf16_lossy(&long)),
                    _ => None,
                };
                long.clear();
                long_sum = None;
                if attributes & 0x08 != 0 {
                    continue; // the volume's label
                }
                let matches = long_name.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(name))
                    || short_name(&short).eq_ignore_ascii_case(name);
                if matches {
                    let first = u16_at(e, 20) << 16 | u16_at(e, 26);
                    let first = if self.kind == Kind::Fat32 { first } else { first & 0xFFFF };
                    return Ok(Entry { first_cluster: first, size: u32_at(e, 28), folder: attributes & 0x10 != 0 });
                }
            }
        }
        Err(Error::NotFound)
    }

    /// Finds a file or folder by its path from the root ("EFI", "chatcomputer-own", "memory.bin").
    pub fn find(&self, disk: &mut impl Sectors, path: &[&str]) -> Result<Entry, Error> {
        let mut at = Entry { first_cluster: 0, size: 0, folder: true };
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
}

/// An 8.3 name as "NAME.EXT" (spaces taken off; no dot without an extension).
fn short_name(n: &[u8; 11]) -> String {
    let base: String = n[..8].iter().map(|&b| if b == 0x05 { 0xE5 } else { b } as char).collect();
    let ext: String = n[8..].iter().map(|&b| b as char).collect();
    let (base, ext) = (base.trim_end(), ext.trim_end());
    if ext.is_empty() { String::from(base) } else { alloc::format!("{base}.{ext}") }
}

/// The checksum long-name pieces carry of their 8.3 name.
fn short_checksum(n: &[u8; 11]) -> u8 {
    n.iter().fold(0u8, |sum, &b| sum.rotate_right(1).wrapping_add(b))
}
