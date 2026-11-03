//! NTFS, the file system of Windows' drive, read but never changed: its folders, and its files' bytes. What Chat
//! Computer's own system needs to let Claude look at the person's files, as the main Chat Computer can.
//!
//! Everything about a file is in its record in the master file table ($MFT, itself a file, record 0): attributes,
//! each kept in the record ("resident") or in clusters elsewhere, listed as runs of clusters. A file's name is a
//! $FILE_NAME attribute; its bytes, its unnamed $DATA; a folder's contents, an index ($I30) of its files' names, a
//! B-tree whose root is in the record and whose other nodes are blocks of clusters. A record that can't hold all of
//! its file's attributes (a file in hundreds of pieces) lists them in an $ATTRIBUTE_LIST and keeps some in other
//! records. Records and index blocks carry an "update sequence" to show a sector was written whole: the last two
//! bytes of each 512 are kept in the block's header, and put back on reading. Files can be sparse (runs of clusters
//! that aren't there are zeros) or compressed (16 clusters at a time, LZNT1).
//!
//! A file can also have a reparse point, which a filter in Windows acts on when it's opened, its tag saying whose:
//! a link to another file or folder (a symbolic link, or a junction such as "My Documents"), listed with where it
//! goes (but not followed here: that's for the caller); a file synced with OneDrive (or another cloud service), whose
//! bytes are on the PC or, for an "online-only" file, aren't (its clusters missing: it's sparse); or a file Windows
//! compressed itself ("Compact OS"), its bytes in a stream of their own.
//!
//! As described by Microsoft's documentation of its structures and by the Linux-NTFS project's description of the
//! format. Every number read from the disk is checked before it's used: a damaged volume gives an error, never a
//! crash. Encrypted files (EFS) aren't read.

#![no_std]

extern crate alloc;

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[cfg(test)]
mod tests;

/// Where the volume's bytes come from: `buf.len()` bytes at byte `offset` of the volume (offsets and lengths are
/// multiples of 512). False if they couldn't be read.
pub trait Disk {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// The disk couldn't be read.
    Read,
    /// Not an NTFS volume.
    NotNtfs,
    /// A volume BitLocker encrypts: its key would be needed.
    BitLocker,
    /// Something on the disk doesn't add up.
    Damaged,
    /// No file or folder of that name.
    NotFound,
    /// A file, where a folder was wanted (or the other way round).
    NotAFolder,
    NotAFile,
    /// Encrypted by Windows (EFS): only Windows, signed in as its owner, can read it.
    Encrypted,
    /// A link to another file or folder (`link_target` says where), not followed.
    Link,
    /// A placeholder whose bytes aren't all on the PC: an "online-only" OneDrive file, say. Its reparse tag.
    Online(u32),
    /// Compressed by Windows itself ("Compact OS"), which isn't read yet.
    Compacted,
    /// A folder with more in it than one listing holds (`MOST_LISTING_BYTES` of entries: millions of files).
    TooMany,
}

/// Reparse tags. Bit 29 marks a "name surrogate": a link to another file or folder. Others are about the file's own
/// bytes.
const NAME_SURROGATE: u32 = 0x2000_0000;
pub const TAG_SYMLINK: u32 = 0xA000_000C;
pub const TAG_JUNCTION: u32 = 0xA000_0003;
/// A link made by Linux under Windows (WSL): its target in UTF-8, / between names.
const TAG_LX_SYMLINK: u32 = 0xA000_001D;
/// Windows' compression of its own files (the Windows Overlay Filter's).
pub const TAG_WOF: u32 = 0x8000_0017;

/// Whether a reparse tag is a cloud service's (OneDrive's, and others' that use Windows' Cloud Files): 0x9000_x01A.
pub fn is_cloud(tag: u32) -> bool {
    tag & 0xFFFF_0FFF == 0x9000_001A
}

/// Attribute types.
const STANDARD_INFORMATION: u32 = 0x10;
const ATTRIBUTE_LIST: u32 = 0x20;
const DATA: u32 = 0x80;
const INDEX_ROOT: u32 = 0x90;
const INDEX_ALLOCATION: u32 = 0xA0;
const REPARSE_POINT: u32 = 0xC0;
const VOLUME_NAME: u32 = 0x60;
const END: u32 = 0xFFFF_FFFF;
/// Attribute flags.
const COMPRESSED: u16 = 0x0001;
const ENCRYPTED_DATA: u16 = 0x4000;
/// A record's flags.
const IN_USE: u16 = 0x0001;
const FOLDER_RECORD: u16 = 0x0002;
/// File attributes (as in $STANDARD_INFORMATION and $FILE_NAME).
pub const HIDDEN: u32 = 0x0002;
pub const SYSTEM: u32 = 0x0004;
const REPARSE: u32 = 0x0400;
/// Encrypted by Windows (EFS).
pub const ENCRYPTED: u32 = 0x4000;
/// A file whose bytes are elsewhere, or not all here (a cloud service sets these on its placeholders).
const OFFLINE: u32 = 0x1000;
const RECALL_ON_OPEN: u32 = 0x0004_0000;
const RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
const FOLDER: u32 = 0x1000_0000;
/// Index entry flags.
const SUBNODE: u16 = 0x0001;
const LAST: u16 = 0x0002;
/// $FILE_NAME namespaces: a name only in its 8.3 form (the long one is listed too).
const DOS_ONLY: u8 = 2;
/// The records NTFS keeps for itself ($MFT to $Extend and those reserved after): not listed.
const FIRST_USER_RECORD: u64 = 16;
/// The root folder's record.
pub const ROOT: u64 = 5;
/// $Volume: the volume's name and version.
const VOLUME: u64 = 3;
const UPCASE: u64 = 10;
/// The most runs, attributes and index levels believed (so damaged numbers can't make it loop or use all memory).
const MOST_RUNS: usize = 1 << 20;
const MOST_LEVELS: u32 = 32;
/// The most index nodes one folder is listed from: a crafted index whose entries point back at their own blocks would
/// otherwise fan out for ever (each level multiplies the reads).
const MOST_INDEX_NODES: u32 = 1 << 20;
/// The most memory one folder's listing takes, its entries and their names: a folder of two or three million files,
/// more than any Windows makes. Without it, an index whose nodes all point at the same few nodes (or whose runs put
/// many nodes on the same clusters) lists the same entries over and over until memory runs out, long before the
/// nodes' budget is spent.
const MOST_LISTING_BYTES: usize = 256 << 20;

/// What one folder's listing has spent: index nodes read, each at most once (a valid index is a tree: no node is
/// reached twice), and memory.
struct Walk {
    nodes: u32,
    seen: BTreeSet<u64>,
    bytes: usize,
}

/// A reparse point's value: its tag, and its data (after the 8-byte header). A value shorter than the header, or
/// than the length it gives, is damaged (slicing from 8 would panic on a 6- or 7-byte one).
fn reparse_value(value: &[u8]) -> Result<(u32, Vec<u8>), Error> {
    let tag = u32_at(value, 0).ok_or(Error::Damaged)?;
    if value.len() < 8 {
        return Err(Error::Damaged);
    }
    let len = (u16_at(value, 4).ok_or(Error::Damaged)? as usize).min(value.len() - 8);
    Ok((tag, value[8..8 + len].to_vec()))
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

/// A run of a non-resident attribute's clusters: `len` clusters from virtual cluster `vcn`, at logical cluster `lcn`
/// on the volume (None: sparse, zeros).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Run {
    vcn: u64,
    lcn: Option<u64>,
    len: u64,
}

/// An attribute's value: in the record, or in clusters.
#[derive(Clone, Debug)]
enum Value {
    Resident(Vec<u8>),
    Clusters { runs: Vec<Run>, size: u64, initialized: u64, compression_unit: u32 },
}

#[derive(Clone, Debug)]
struct Attribute {
    kind: u32,
    name: Vec<u16>,
    flags: u16,
    /// The first virtual cluster this piece covers (non-resident attributes split over records).
    first_vcn: u64,
    value: Value,
}

/// A file or folder in a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// Its record in the master file table.
    pub record: u64,
    pub folder: bool,
    /// A link to another file or folder (`Volume::link_target`), not followed.
    pub link: bool,
    /// Its reparse tag (0: none): a link's, OneDrive's, Windows' compression's...
    pub tag: u32,
    /// Its size as its folder's index has it (Windows keeps it up to date, near enough; `File` has the exact one).
    pub size: u64,
    /// When it was last changed, in seconds since 1970 (UTC).
    pub modified: i64,
    /// Its file attributes (hidden, system...).
    pub attributes: u32,
}

/// A file opened: its bytes' attribute.
#[derive(Clone, Debug)]
pub struct File {
    pub record: u64,
    pub size: u64,
    data: Value,
    compressed: bool,
}

pub struct Volume {
    pub bytes_per_sector: u32,
    pub cluster_size: u32,
    pub clusters: u64,
    record_size: usize,
    /// The master file table's own runs.
    mft: Vec<Run>,
    /// Each UTF-16 unit in capitals, as the volume compares names ($UpCase).
    upcase: Vec<u16>,
}

/// An NTFS time (100 ns since 1601) in seconds since 1970.
fn unix_time(t: u64) -> i64 {
    (t / 10_000_000) as i64 - 11_644_473_600
}

/// Reads the volume's boot sector, then its master file table's own record (for where the rest of the table is),
/// and the table of capitals that names are compared by.
pub fn open(disk: &mut impl Disk) -> Result<Volume, Error> {
    let mut boot = [0u8; 512];
    if !disk.read(0, &mut boot) {
        return Err(Error::Read);
    }
    if &boot[3..11] == b"-FVE-FS-" {
        return Err(Error::BitLocker);
    }
    if &boot[3..11] != b"NTFS    " || u16_at(&boot, 510) != Some(0xAA55) {
        return Err(Error::NotNtfs);
    }
    let bytes_per_sector = u16_at(&boot, 0x0B).ok_or(Error::Damaged)? as u32;
    if !bytes_per_sector.is_power_of_two() || !(512..=4096).contains(&bytes_per_sector) {
        return Err(Error::Damaged);
    }
    // Sectors a cluster: as it is, or (above 128) 2 to the power of 256 less it, for clusters of 64 KiB and more.
    let spc = boot[0x0D] as u32;
    let sectors_per_cluster = if spc > 128 { 1u32.checked_shl(256 - spc).ok_or(Error::Damaged)? } else { spc };
    let cluster_size = bytes_per_sector.checked_mul(sectors_per_cluster).ok_or(Error::Damaged)?;
    if cluster_size == 0 || !cluster_size.is_power_of_two() || cluster_size > 2 << 20 {
        return Err(Error::Damaged);
    }
    let sectors = u64_at(&boot, 0x28).ok_or(Error::Damaged)?;
    let clusters = sectors / sectors_per_cluster as u64;
    let mft_lcn = u64_at(&boot, 0x30).ok_or(Error::Damaged)?;
    // A record's and an index block's size: in clusters, or (negative) 2 to the power of minus it, in bytes.
    let size = |b: u8| -> Option<usize> {
        let v = b as i8;
        let s = if v < 0 { 1usize.checked_shl((-(v as i32)) as u32)? } else { (v as usize).checked_mul(cluster_size as usize)? };
        (s.is_power_of_two() && (512..=65536).contains(&s)).then_some(s)
    };
    let record_size = size(boot[0x40]).ok_or(Error::Damaged)?;
    if mft_lcn >= clusters {
        return Err(Error::Damaged);
    }
    let mut v = Volume { bytes_per_sector, cluster_size, clusters, record_size, mft: Vec::new(), upcase: Vec::new() };
    // The table's own record first, where it begins; then all of the table's runs, from it.
    let mut first = vec![0u8; record_size];
    if !disk.read(mft_lcn * cluster_size as u64, &mut first) {
        return Err(Error::Read);
    }
    fix_up(&mut first, b"FILE")?;
    v.mft = vec![Run { vcn: 0, lcn: Some(mft_lcn), len: (record_size as u64).div_ceil(cluster_size as u64) }];
    let attrs = attributes_of(&first)?;
    let base_data = attrs.iter().find(|a| a.kind == DATA && a.name.is_empty()).ok_or(Error::Damaged)?;
    if let Value::Clusters { runs, .. } = &base_data.value {
        v.mft = runs.clone();
    }
    let data = v.attribute_from(disk, 0, &first, DATA)?.ok_or(Error::Damaged)?;
    match data.value {
        Value::Clusters { runs, .. } => v.mft = runs,
        Value::Resident(_) => return Err(Error::Damaged),
    }
    // $UpCase: 65536 capitals.
    let upcase = v.open_file(disk, UPCASE)?;
    let mut table = vec![0u8; 131_072];
    let got = v.read(disk, &upcase, 0, &mut table)?;
    if got != table.len() {
        return Err(Error::Damaged);
    }
    v.upcase = table.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Ok(v)
}

/// Puts back the last two bytes of each 512 of a record or index block, checking each held the update sequence
/// number (so it was written whole); and checks its signature.
fn fix_up(block: &mut [u8], magic: &[u8; 4]) -> Result<(), Error> {
    if block.get(0..4) != Some(&magic[..]) {
        return Err(Error::Damaged);
    }
    let at = u16_at(block, 4).ok_or(Error::Damaged)? as usize;
    let count = u16_at(block, 6).ok_or(Error::Damaged)? as usize;
    if count == 0 || (count - 1) * 512 > block.len() || at + 2 * count > block.len() {
        return Err(Error::Damaged);
    }
    let usn = [block[at], block[at + 1]];
    for i in 1..count {
        let end = i * 512 - 2;
        if block[end..end + 2] != usn {
            return Err(Error::Damaged);
        }
        block[end] = block[at + 2 * i];
        block[end + 1] = block[at + 2 * i + 1];
    }
    Ok(())
}

/// A non-resident attribute's runs, from its mapping pairs: each a header byte (the sizes of the length and of the
/// offset, in bytes), the length in clusters, and the start's offset from the run before's (none: sparse).
fn decode_runs(b: &[u8], first_vcn: u64) -> Option<Vec<Run>> {
    let mut runs = Vec::new();
    let (mut at, mut vcn, mut lcn) = (0usize, first_vcn, 0i64);
    loop {
        let header = *b.get(at)?;
        if header == 0 {
            return Some(runs);
        }
        let (len_size, off_size) = ((header & 0x0F) as usize, (header >> 4) as usize);
        if len_size == 0 || len_size > 8 || off_size > 8 {
            return None;
        }
        let field = |from: usize, n: usize| -> Option<u64> {
            let bytes = b.get(from..from + n)?;
            Some(bytes.iter().rev().fold(0u64, |v, &x| v << 8 | x as u64))
        };
        let len = field(at + 1, len_size)?;
        let lcn_here = if off_size == 0 {
            None
        } else {
            let raw = field(at + 1 + len_size, off_size)?;
            // Signed: sign-extended from its top byte.
            let shift = 64 - 8 * off_size as u32;
            let delta = ((raw << shift) as i64) >> shift;
            lcn = lcn.checked_add(delta)?;
            if lcn < 0 {
                return None;
            }
            Some(lcn as u64)
        };
        if len == 0 || runs.len() >= MOST_RUNS {
            return None;
        }
        runs.push(Run { vcn, lcn: lcn_here, len });
        vcn = vcn.checked_add(len)?;
        at += 1 + len_size + off_size;
    }
}

/// The attributes in a record (fixed up): each one's type, name, flags and value.
fn attributes_of(record: &[u8]) -> Result<Vec<Attribute>, Error> {
    let flags = u16_at(record, 0x16).ok_or(Error::Damaged)?;
    if flags & IN_USE == 0 {
        return Err(Error::NotFound);
    }
    let used = (u32_at(record, 0x18).ok_or(Error::Damaged)? as usize).min(record.len());
    let mut at = u16_at(record, 0x14).ok_or(Error::Damaged)? as usize;
    let mut out = Vec::new();
    while at + 8 <= used {
        let kind = u32_at(record, at).ok_or(Error::Damaged)?;
        if kind == END {
            break;
        }
        let len = u32_at(record, at + 4).ok_or(Error::Damaged)? as usize;
        if len < 16 || at + len > used {
            return Err(Error::Damaged);
        }
        let a = &record[at..at + len];
        let resident = a[8] == 0;
        let (name_len, name_at) = (a[9] as usize, u16_at(a, 10).ok_or(Error::Damaged)? as usize);
        let name: Vec<u16> = (0..name_len).map(|i| u16_at(a, name_at + 2 * i)).collect::<Option<_>>().ok_or(Error::Damaged)?;
        let flags = u16_at(a, 12).ok_or(Error::Damaged)?;
        let (first_vcn, value) = if resident {
            let (vlen, vat) = (u32_at(a, 16).ok_or(Error::Damaged)? as usize, u16_at(a, 20).ok_or(Error::Damaged)? as usize);
            (0, Value::Resident(a.get(vat..vat + vlen).ok_or(Error::Damaged)?.to_vec()))
        } else {
            let first = u64_at(a, 16).ok_or(Error::Damaged)?;
            let runs_at = u16_at(a, 32).ok_or(Error::Damaged)? as usize;
            let unit = u16_at(a, 34).ok_or(Error::Damaged)? as u32;
            let size = u64_at(a, 48).ok_or(Error::Damaged)?;
            let initialized = u64_at(a, 56).ok_or(Error::Damaged)?;
            let runs = decode_runs(a.get(runs_at..).ok_or(Error::Damaged)?, first).ok_or(Error::Damaged)?;
            (first, Value::Clusters { runs, size, initialized, compression_unit: unit })
        };
        out.push(Attribute { kind, name, flags, first_vcn, value });
        at += len;
    }
    Ok(out)
}

impl Volume {
    /// Reads `buf.len()` bytes of a non-resident value at byte `offset`: from its runs (sparse runs, clusters it
    /// doesn't have, and anything past `initialized`, as zeros).
    fn read_runs(&self, disk: &mut impl Disk, runs: &[Run], initialized: u64, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        let cs = self.cluster_size as u64;
        let mut done = 0usize;
        while done < buf.len() {
            let pos = offset + done as u64;
            let vcn = pos / cs;
            let run = runs.iter().find(|r| vcn >= r.vcn && vcn < r.vcn + r.len);
            // Within this run, to its end or the buffer's.
            let (lcn, left_in_run) = match run {
                Some(r) => (r.lcn.map(|l| l + (vcn - r.vcn)), (r.vcn + r.len - vcn) * cs - pos % cs),
                None => (None, cs - pos % cs),
            };
            let n = (left_in_run as usize).min(buf.len() - done);
            let part = &mut buf[done..done + n];
            match lcn {
                Some(l) if pos < initialized => {
                    if l.checked_add((pos % cs + n as u64).div_ceil(cs)).is_none_or(|end| end > self.clusters) {
                        return Err(Error::Damaged);
                    }
                    self.read_disk(disk, l * cs + pos % cs, part)?;
                    // (Past the initialized size, zeros, whatever the clusters hold.)
                    let valid = initialized.saturating_sub(pos);
                    if (n as u64) > valid {
                        part[valid as usize..].fill(0);
                    }
                }
                _ => part.fill(0),
            }
            done += n;
        }
        Ok(())
    }

    /// Reads from the disk at any byte offset and length (the disk is read in whole 512s around them).
    fn read_disk(&self, disk: &mut impl Disk, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        let start = offset & !511;
        let end = (offset + buf.len() as u64).div_ceil(512) * 512;
        if start == offset && end == offset + buf.len() as u64 {
            return if disk.read(offset, buf) { Ok(()) } else { Err(Error::Read) };
        }
        let mut whole = vec![0u8; (end - start) as usize];
        if !disk.read(start, &mut whole) {
            return Err(Error::Read);
        }
        let from = (offset - start) as usize;
        buf.copy_from_slice(&whole[from..from + buf.len()]);
        Ok(())
    }

    /// Record `n` of the master file table, fixed up.
    fn record(&self, disk: &mut impl Disk, n: u64) -> Result<Vec<u8>, Error> {
        let mut r = vec![0u8; self.record_size];
        let offset = n.checked_mul(self.record_size as u64).ok_or(Error::Damaged)?;
        let end: u64 = self.mft.iter().map(|r| (r.vcn + r.len) * self.cluster_size as u64).max().unwrap_or(0);
        if offset + self.record_size as u64 > end {
            return Err(Error::NotFound);
        }
        self.read_runs(disk, &self.mft, u64::MAX, offset, &mut r)?;
        fix_up(&mut r, b"FILE")?;
        Ok(r)
    }

    /// File `n`'s attributes of type `kind` (unnamed, or all for an index's), wherever its records keep them: in its
    /// own record, or, where an attribute list says, in others. A non-resident attribute split over records comes back
    /// whole, its runs joined.
    fn attributes(&self, disk: &mut impl Disk, n: u64, kind: u32) -> Result<Vec<Attribute>, Error> {
        let base = self.record(disk, n)?;
        self.attributes_in(disk, n, &attributes_of(&base)?, kind)
    }

    /// As `attributes`, with the attributes of the file's own record (`own`) already read.
    fn attributes_in(&self, disk: &mut impl Disk, n: u64, own: &[Attribute], kind: u32) -> Result<Vec<Attribute>, Error> {
        let Some(list) = own.iter().find(|a| a.kind == ATTRIBUTE_LIST) else {
            return Ok(own.iter().filter(|a| a.kind == kind).cloned().collect());
        };
        let list = match &list.value {
            Value::Resident(b) => b.clone(),
            Value::Clusters { runs, size, initialized, .. } => {
                let mut b = vec![0u8; (*size).min(1 << 24) as usize];
                self.read_runs(disk, runs, *initialized, 0, &mut b)?;
                b
            }
        };
        // Each entry: type, length, name's length and place, first VCN, the record it's in, its id.
        let mut records: Vec<u64> = Vec::new();
        let mut at = 0;
        while at + 26 <= list.len() {
            let t = u32_at(&list, at).ok_or(Error::Damaged)?;
            let len = u16_at(&list, at + 4).ok_or(Error::Damaged)? as usize;
            if len < 26 {
                return Err(Error::Damaged);
            }
            let record = u64_at(&list, at + 16).ok_or(Error::Damaged)? & 0xFFFF_FFFF_FFFF;
            if t == kind && !records.contains(&record) {
                records.push(record);
            }
            at += len;
        }
        let mut found: Vec<Attribute> = Vec::new();
        for r in records {
            let attrs = if r == n { own.to_vec() } else { attributes_of(&self.record(disk, r)?)? };
            found.extend(attrs.into_iter().filter(|a| a.kind == kind));
        }
        // Pieces of the same attribute (same name), in order, joined.
        found.sort_by_key(|a| a.first_vcn);
        let mut joined: Vec<Attribute> = Vec::new();
        for a in found {
            match joined.iter_mut().find(|j| j.name == a.name) {
                Some(j) => {
                    if let (Value::Clusters { runs, .. }, Value::Clusters { runs: more, .. }) = (&mut j.value, a.value) {
                        if runs.len() + more.len() > MOST_RUNS {
                            return Err(Error::Damaged);
                        }
                        runs.extend(more);
                    }
                }
                None => joined.push(a),
            }
        }
        Ok(joined)
    }

    /// The master file table's own $DATA, from record 0 (`first`, already read), through its attribute list if it has
    /// one (its other pieces' records are in the part already known).
    fn attribute_from(&self, disk: &mut impl Disk, n: u64, first: &[u8], kind: u32) -> Result<Option<Attribute>, Error> {
        if !attributes_of(first)?.iter().any(|a| a.kind == ATTRIBUTE_LIST) {
            return Ok(attributes_of(first)?.into_iter().find(|a| a.kind == kind && a.name.is_empty()));
        }
        Ok(self.attributes(disk, n, kind)?.into_iter().find(|a| a.name.is_empty()))
    }

    /// File `n`, to read its bytes (its unnamed $DATA stream). Not a link (`Error::Link`), a placeholder whose bytes
    /// aren't all here (`Error::Online`), or a file Windows compressed itself (`Error::Compacted`).
    pub fn open_file(&self, disk: &mut impl Disk, n: u64) -> Result<File, Error> {
        let base = self.record(disk, n)?;
        if u16_at(&base, 0x16).unwrap_or(0) & FOLDER_RECORD != 0 {
            return Err(Error::NotAFile);
        }
        let own = attributes_of(&base)?;
        let tag = self.reparse_in(disk, n, &own)?.map_or(0, |r| r.0);
        if tag & NAME_SURROGATE != 0 {
            return Err(Error::Link);
        }
        if tag == TAG_WOF {
            return Err(Error::Compacted);
        }
        let attrs = self.attributes_in(disk, n, &own, DATA)?;
        let data = attrs.into_iter().find(|a| a.name.is_empty()).ok_or(Error::Damaged)?;
        if data.flags & ENCRYPTED_DATA != 0 {
            return Err(Error::Encrypted);
        }
        let size = match &data.value {
            Value::Resident(b) => b.len() as u64,
            Value::Clusters { size, .. } => *size,
        };
        let compressed = data.flags & COMPRESSED != 0;
        if tag != 0 && !self.all_here(&own, &data.value, compressed, size) {
            return Err(Error::Online(tag));
        }
        Ok(File { record: n, size, data: data.value, compressed })
    }

    /// Whether a placeholder's bytes are all on the PC: its file attributes don't say they're elsewhere, and every
    /// cluster up to its size is on the disk (a cloud service's placeholder is sparse where its bytes aren't
    /// downloaded). (A compressed file has no clusters for a unit of zeros, so only its attributes can say.)
    fn all_here(&self, own: &[Attribute], data: &Value, compressed: bool, size: u64) -> bool {
        let attributes = own
            .iter()
            .find(|a| a.kind == STANDARD_INFORMATION)
            .and_then(|a| match &a.value {
                Value::Resident(b) => u32_at(b, 32),
                Value::Clusters { .. } => None,
            })
            .unwrap_or(0);
        if attributes & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0 {
            return false;
        }
        let Value::Clusters { runs, .. } = data else { return true };
        if compressed {
            return true;
        }
        let needed = size.div_ceil(self.cluster_size as u64);
        let mut runs = runs.clone();
        runs.sort_by_key(|r| r.vcn);
        // The clusters from the first that are on the disk, run by run, to the first gap or hole.
        let mut here = 0u64;
        for r in &runs {
            if here >= needed {
                break;
            }
            if r.vcn > here || r.lcn.is_none() {
                return false;
            }
            here = here.max(r.vcn + r.len);
        }
        here >= needed
    }

    /// File `n`'s reparse point, if it has one: its tag, and the point's data (after its 8-byte header).
    pub fn reparse(&self, disk: &mut impl Disk, n: u64) -> Result<Option<(u32, Vec<u8>)>, Error> {
        let base = self.record(disk, n)?;
        self.reparse_in(disk, n, &attributes_of(&base)?)
    }

    fn reparse_in(&self, disk: &mut impl Disk, n: u64, own: &[Attribute]) -> Result<Option<(u32, Vec<u8>)>, Error> {
        let Some(a) = self.attributes_in(disk, n, own, REPARSE_POINT)?.into_iter().next() else { return Ok(None) };
        let value = match a.value {
            Value::Resident(b) => b,
            // (At most 16 KiB, so nearly always in the record; but it can be outside it.)
            Value::Clusters { runs, size, initialized, .. } => {
                let mut b = vec![0u8; size.min(16 * 1024 + 8) as usize];
                self.read_runs(disk, &runs, initialized, 0, &mut b)?;
                b
            }
        };
        Ok(Some(reparse_value(&value)?))
    }

    /// Where link `n` goes, as it says: "C:\Users\Sam\Documents" (a junction's, or a symbolic link's to a path from
    /// a drive), "..\Notes\todo.txt" (a symbolic link's relative to its folder), or for a link made under Linux
    /// (WSL), its path with / between names. None if it isn't a link, or says nothing that can be followed.
    pub fn link_target(&self, disk: &mut impl Disk, n: u64) -> Result<Option<String>, Error> {
        let Some((tag, data)) = self.reparse(disk, n)? else { return Ok(None) };
        if tag == TAG_LX_SYMLINK {
            // A version (2), then the target.
            return Ok(data.get(4..).and_then(|t| core::str::from_utf8(t).ok()).map(String::from));
        }
        // A junction's: where the "substitute" and "print" names are in the path buffer (16 bytes in), as offsets
        // and lengths in bytes; a symbolic link's: the same, then flags (so its buffer is 20 bytes in).
        let buffer = match tag {
            TAG_JUNCTION => 8,
            TAG_SYMLINK => 12,
            _ => return Ok(None),
        };
        let name = |off: usize, len: usize| -> Option<String> {
            let b = data.get(buffer + off..buffer + off + len)?;
            let units = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
            Some(char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect())
        };
        let field = |i: usize| u16_at(&data, i).map(|v| v as usize);
        let (Some(sub_at), Some(sub_len), Some(print_at), Some(print_len)) = (field(0), field(2), field(4), field(6)) else {
            return Err(Error::Damaged);
        };
        // The name for people, or failing that the one Windows follows ("\??\C:\...", in its object namespace).
        let print = name(print_at, print_len).filter(|s| !s.is_empty());
        let target = match print.or_else(|| name(sub_at, sub_len)) {
            Some(t) => t,
            None => return Err(Error::Damaged),
        };
        let target = target.strip_prefix("\\??\\").or_else(|| target.strip_prefix("\\\\?\\")).unwrap_or(&target);
        Ok(Some(String::from(target)))
    }

    /// Reads a file's bytes at `offset` into `buf`: how many there were (fewer at its end).
    pub fn read(&self, disk: &mut impl Disk, file: &File, offset: u64, buf: &mut [u8]) -> Result<usize, Error> {
        if offset >= file.size {
            return Ok(0);
        }
        let n = ((file.size - offset) as usize).min(buf.len());
        let out = &mut buf[..n];
        match &file.data {
            Value::Resident(b) => out.copy_from_slice(&b[offset as usize..offset as usize + n]),
            Value::Clusters { runs, initialized, compression_unit, .. } => {
                if file.compressed && *compression_unit > 0 {
                    self.read_compressed(disk, runs, *compression_unit, offset, out)?;
                    let valid = initialized.saturating_sub(offset);
                    if (n as u64) > valid {
                        out[valid as usize..].fill(0);
                    }
                } else {
                    self.read_runs(disk, runs, *initialized, offset, out)?;
                }
            }
        }
        Ok(n)
    }

    /// A compressed file's bytes: 2^`unit` clusters at a time, each unit stored as it is (all its clusters there),
    /// compressed (fewer clusters, then sparse ones), or not at all (all sparse: zeros).
    fn read_compressed(&self, disk: &mut impl Disk, runs: &[Run], unit: u32, offset: u64, out: &mut [u8]) -> Result<(), Error> {
        if unit > 8 {
            return Err(Error::Damaged);
        }
        let cs = self.cluster_size as u64;
        let unit_clusters = 1u64 << unit;
        let unit_bytes = unit_clusters * cs;
        let mut done = 0usize;
        while done < out.len() {
            let pos = offset + done as u64;
            let first = pos / unit_bytes * unit_clusters;
            // The unit's clusters: those there (in order), counted.
            let mut there: Vec<u64> = Vec::new();
            for vcn in first..first + unit_clusters {
                if let Some(r) = runs.iter().find(|r| vcn >= r.vcn && vcn < r.vcn + r.len)
                    && let Some(l) = r.lcn
                {
                    there.push(l + (vcn - r.vcn));
                }
            }
            let mut plain = vec![0u8; unit_bytes as usize];
            if there.len() as u64 == unit_clusters {
                for (k, &l) in there.iter().enumerate() {
                    self.read_disk(disk, l * cs, &mut plain[k * cs as usize..(k + 1) * cs as usize])?;
                }
            } else if !there.is_empty() {
                let mut packed = vec![0u8; there.len() * cs as usize];
                for (k, &l) in there.iter().enumerate() {
                    if l >= self.clusters {
                        return Err(Error::Damaged);
                    }
                    self.read_disk(disk, l * cs, &mut packed[k * cs as usize..(k + 1) * cs as usize])?;
                }
                lznt1(&packed, &mut plain).ok_or(Error::Damaged)?;
            }
            let from = (pos % unit_bytes) as usize;
            let n = (plain.len() - from).min(out.len() - done);
            out[done..done + n].copy_from_slice(&plain[from..from + n]);
            done += n;
        }
        Ok(())
    }

    /// A folder's files and folders, in its index's order (Windows' sorting of names), each once: by its long name
    /// (not its 8.3 one), NTFS's own files left out. Not a link's (`Error::Link`: what it links to is listed).
    pub fn list(&self, disk: &mut impl Disk, folder: u64) -> Result<Vec<Entry>, Error> {
        let base = self.record(disk, folder)?;
        if u16_at(&base, 0x16).unwrap_or(0) & FOLDER_RECORD == 0 {
            return Err(Error::NotAFolder);
        }
        let own = attributes_of(&base)?;
        if self.reparse_in(disk, folder, &own)?.is_some_and(|r| r.0 & NAME_SURROGATE != 0) {
            return Err(Error::Link);
        }
        let roots = self.attributes_in(disk, folder, &own, INDEX_ROOT)?;
        let root = roots.iter().find(|a| a.name == I30).ok_or(Error::Damaged)?;
        let Value::Resident(root) = &root.value else { return Err(Error::Damaged) };
        let block_size = u32_at(root, 8).ok_or(Error::Damaged)? as usize;
        let alloc = self.attributes_in(disk, folder, &own, INDEX_ALLOCATION)?.into_iter().find(|a| a.name == I30);
        let mut out = Vec::new();
        let mut walk = Walk { nodes: MOST_INDEX_NODES, seen: BTreeSet::new(), bytes: 0 };
        self.walk_node(disk, root, 16, alloc.as_ref(), block_size, 0, &mut out, &mut walk)?;
        // (Windows keeps each reparse point's tag in its folder's index too; were one not there, its record has it.)
        for e in out.iter_mut().filter(|e| e.attributes & REPARSE != 0 && e.tag == 0) {
            if let Ok(Some((tag, _))) = self.reparse(disk, e.record) {
                e.tag = tag;
                e.link = tag & NAME_SURROGATE != 0;
            }
        }
        Ok(out)
    }

    /// The entries of one index node (its header at `header` in `block`), with its sub-nodes' before each of them.
    #[allow(clippy::too_many_arguments)]
    fn walk_node(&self, disk: &mut impl Disk, block: &[u8], header: usize, alloc: Option<&Attribute>, block_size: usize,
                 depth: u32, out: &mut Vec<Entry>, walk: &mut Walk) -> Result<(), Error> {
        if depth > MOST_LEVELS || walk.nodes == 0 {
            return Err(Error::Damaged);
        }
        walk.nodes -= 1;
        let first = header + u32_at(block, header).ok_or(Error::Damaged)? as usize;
        let end = (header + u32_at(block, header + 4).ok_or(Error::Damaged)? as usize).min(block.len());
        let mut at = first;
        while at + 16 <= end {
            let len = u16_at(block, at + 8).ok_or(Error::Damaged)? as usize;
            let key_len = u16_at(block, at + 10).ok_or(Error::Damaged)? as usize;
            let flags = u16_at(block, at + 12).ok_or(Error::Damaged)?;
            if len < 16 || at + len > block.len() {
                return Err(Error::Damaged);
            }
            if flags & SUBNODE != 0 {
                let vcn = u64_at(block, at + len - 8).ok_or(Error::Damaged)?;
                if !walk.seen.insert(vcn) {
                    return Err(Error::Damaged);
                }
                let child = self.index_block(disk, alloc.ok_or(Error::Damaged)?, block_size, vcn)?;
                self.walk_node(disk, &child, 0x18, alloc, block_size, depth + 1, out, walk)?;
            }
            if flags & LAST != 0 {
                break;
            }
            if key_len >= 66
                && let Some(e) = entry(&block[at..at + len], key_len)
            {
                walk.bytes += core::mem::size_of::<Entry>() + e.name.len();
                if walk.bytes > MOST_LISTING_BYTES {
                    return Err(Error::TooMany);
                }
                out.push(e);
            }
            at += len;
        }
        Ok(())
    }

    /// Index block `vcn` of an index's allocation, fixed up.
    fn index_block(&self, disk: &mut impl Disk, alloc: &Attribute, block_size: usize, vcn: u64) -> Result<Vec<u8>, Error> {
        let Value::Clusters { runs, initialized, .. } = &alloc.value else { return Err(Error::Damaged) };
        if !(512..=65536).contains(&block_size) {
            return Err(Error::Damaged);
        }
        // (A block's VCN counts clusters, or, where blocks are smaller than clusters, 512s.)
        let step = if block_size as u32 >= self.cluster_size { self.cluster_size as u64 } else { 512 };
        let offset = vcn.checked_mul(step).ok_or(Error::Damaged)?;
        let mut b = vec![0u8; block_size];
        self.read_runs(disk, runs, *initialized, offset, &mut b)?;
        fix_up(&mut b, b"INDX")?;
        Ok(b)
    }

    /// The file or folder at `path` (its parts split by / or \, matched as Windows matches names: whatever their case),
    /// from the root.
    pub fn find(&self, disk: &mut impl Disk, path: &str) -> Result<Entry, Error> {
        let mut way = self.resolve(disk, path)?;
        Ok(way.pop().unwrap_or_else(root))
    }

    /// Each folder on the way to `path`, and what's at it (none for the root itself): their names as the volume
    /// has them. Not through a link (`Error::Link`).
    pub fn resolve(&self, disk: &mut impl Disk, path: &str) -> Result<Vec<Entry>, Error> {
        let mut way: Vec<Entry> = Vec::new();
        for part in path.split(['/', '\\']).filter(|p| !p.is_empty()) {
            let folder = way.last().map_or(ROOT, |e| e.record);
            if let Some(e) = way.last() {
                if e.link {
                    return Err(Error::Link);
                }
                if !e.folder {
                    return Err(Error::NotAFolder);
                }
            }
            way.push(self.child(disk, folder, part)?);
        }
        Ok(way)
    }

    /// What's called `name` in folder `folder` (whatever its case).
    pub fn child(&self, disk: &mut impl Disk, folder: u64, name: &str) -> Result<Entry, Error> {
        let wanted = self.capitals(name);
        self.list(disk, folder)?.into_iter().find(|e| self.capitals(&e.name) == wanted).ok_or(Error::NotFound)
    }

    /// A name in capitals, as this volume makes them (its $UpCase table).
    fn capitals(&self, name: &str) -> Vec<u16> {
        name.encode_utf16().map(|c| self.upcase.get(c as usize).copied().unwrap_or(c)).collect()
    }
}

/// "$I30", the name of a folder's index.
const I30: [u16; 4] = [b'$' as u16, b'I' as u16, b'3' as u16, b'0' as u16];

impl Volume {
    /// The volume's name, as File Explorer shows it beside the drive's letter ("Data"; "" if it hasn't one).
    pub fn label(&self, disk: &mut impl Disk) -> Result<String, Error> {
        let names = self.attributes(disk, VOLUME, VOLUME_NAME)?;
        Ok(match names.first().map(|a| &a.value) {
            Some(Value::Resident(b)) => {
                let units = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
                char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect()
            }
            _ => String::new(),
        })
    }
}

/// The root folder, as an entry (it isn't in a folder).
pub fn root() -> Entry {
    Entry { name: String::new(), record: ROOT, folder: true, link: false, tag: 0, size: 0, modified: 0, attributes: 0 }
}

/// An index entry's file: its record, and from its $FILE_NAME key, its name, size, time and attributes. None for an 8.3
/// name (the long one has an entry of its own) and NTFS's own files.
fn entry(e: &[u8], key_len: usize) -> Option<Entry> {
    let record = u64_at(e, 0)? & 0xFFFF_FFFF_FFFF;
    let key = e.get(16..16 + key_len)?;
    let name_len = *key.get(64)? as usize;
    let namespace = *key.get(65)?;
    if namespace == DOS_ONLY || record < FIRST_USER_RECORD {
        return None;
    }
    let units: Vec<u16> = (0..name_len).map(|i| u16_at(key, 66 + 2 * i)).collect::<Option<_>>()?;
    let name: String = char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect();
    let attributes = u32_at(key, 56)?;
    // (Where a file with extended attributes keeps their size, one with a reparse point keeps its tag.)
    let tag = if attributes & REPARSE != 0 { u32_at(key, 60)? } else { 0 };
    Some(Entry {
        name,
        record,
        folder: attributes & FOLDER != 0,
        link: tag & NAME_SURROGATE != 0,
        tag,
        size: u64_at(key, 48)?,
        modified: unix_time(u64_at(key, 16)?),
        attributes,
    })
}

/// LZNT1, the compression of NTFS's compressed files: chunks of up to 4 KiB, each stored as it is or as tokens, eight
/// to a flag byte, each a byte as it is or a copy of earlier bytes (where from and how many sharing 16 bits, more for
/// "where from" the further into the chunk). Fills `out` (the rest of a unit after the last chunk is zeros).
fn lznt1(input: &[u8], out: &mut [u8]) -> Option<()> {
    let (mut i, mut o) = (0usize, 0usize);
    while i + 2 <= input.len() && o < out.len() {
        let header = u16::from_le_bytes([input[i], input[i + 1]]);
        if header == 0 {
            break;
        }
        let size = (header & 0x0FFF) as usize + 1;
        let compressed = header & 0x8000 != 0;
        i += 2;
        let chunk = input.get(i..i + size)?;
        i += size;
        let start = o;
        let room = (out.len() - start).min(4096);
        if !compressed {
            let n = chunk.len().min(room);
            out[o..o + n].copy_from_slice(&chunk[..n]);
            o += n;
            continue;
        }
        let mut c = 0usize;
        while c < chunk.len() {
            let flags = chunk[c];
            c += 1;
            for bit in 0..8 {
                if c >= chunk.len() || o - start >= room {
                    break;
                }
                if flags >> bit & 1 == 0 {
                    out[o] = chunk[c];
                    o += 1;
                    c += 1;
                } else {
                    let token = u16::from_le_bytes([chunk[c], *chunk.get(c + 1)?]) as usize;
                    c += 2;
                    // How many of the 16 bits say how many bytes: fewer, the further into the chunk.
                    let pos = o - start;
                    if pos == 0 {
                        return None;
                    }
                    let mut length_bits = 12;
                    let mut p = pos - 1;
                    while p >= 0x10 {
                        p >>= 1;
                        length_bits -= 1;
                    }
                    let back = (token >> length_bits) + 1;
                    let count = (token & ((1 << length_bits) - 1)) + 3;
                    if back > pos {
                        return None;
                    }
                    for _ in 0..count {
                        if o - start >= room {
                            break;
                        }
                        out[o] = out[o - back];
                        o += 1;
                    }
                }
            }
        }
        // (A chunk that came out short is followed by zeros, up to its 4 KiB, before the next.)
        if i + 2 <= input.len() && u16::from_le_bytes([input[i], input[i + 1]]) != 0 {
            o = (start + 4096).min(out.len());
        }
    }
    Some(())
}
