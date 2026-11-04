//! The PC's drives and USB drives, read and never changed: the person's files, for Chat Computer's model (its
//! list_files, read_file, find_files and show_picture tools), as the main Chat Computer can read them, and the model's
//! own file. Every volume in the partitions the drives' GPTs (or old MBRs) list as basic data is found: NTFS (ntfs/),
//! exFAT (exfat/) or FAT (fat/). The one with Windows' own folders ("Windows" and "Users") is C:, and Windows'
//! registry there (SYSTEM's MountedDevices, through registry/) says which letter each other partition has, by its
//! GUID or its disk's signature and where it starts. A drive Windows hasn't seen gets the next letter no drive has, as
//! Windows would give it. Links (a junction such as "My Documents", a symbolic link) are followed, within a drive;
//! OneDrive's files are read when they're on the PC, and said to be online only when they aren't.
//!
//! USB drives (usb/storage.rs: sticks, external disks, the cards in a card reader) are read as they come and go: a
//! drive plugged in, or a card put in, has its volumes found and given letters (Windows' for a USB stick, by its
//! serial number, if Windows' registry has one no drive here has; else the next free one), the other drives keeping
//! theirs; and they're forgotten when it's unplugged or taken out. A stick or card may have no partition table: its
//! first block is then its volume's own.
//!
//! A drive BitLocker encrypts (bitlocker/; a USB drive's "BitLocker To Go" too) is read decrypted: unlocked at once if
//! BitLocker keeps its key in the clear (suspended, or waiting to be switched on, as a new PC's device encryption does
//! until its owner signs in), else once the person types its recovery key in the message box (each locked drive
//! tried). chat.rs hands the key here and never to the model, and it isn't kept: after a restart it's typed again.
//!
//! read_file reads text as it is; Word, Excel and PowerPoint files (and OpenDocument's) as their words (office/); a
//! PDF as its words, page by page (pdf/: the last one read kept, to read on in parts); and a ZIP archive as the list
//! of what's in it. A picture (JPEG, PNG, GIF) is given to the model to see, made the size its vision part reads it
//! at (picture/'s decode_for), as is a PDF's page whose words are pictures (a scanned page: pdf/'s page_picture);
//! show_picture puts a picture on the screen for the person.

use crate::storage::{self, Disk, Drive};
use crate::sync::SpinLock;
use crate::{time, usb};
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// GPT's "Microsoft basic data" partition type, as stored (its first three fields little-endian).
const BASIC_DATA: [u8; 16] = [0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7];
/// The most of a file read_file gives at once, in bytes.
const READ_AT_ONCE: usize = 60_000;
/// The most entries list_files gives, and results find_files gives.
const MOST_LISTED: usize = 300;
const MOST_FOUND: usize = 50;
/// How many folders find_files looks in, and for how long, before it stops (the screen waits meanwhile).
const MOST_FOLDERS: usize = 4_000;
const SEARCH_MS: u64 = 5_000;
/// The most links followed on the way to a file (a link to a link to...), so links in a circle end.
const MOST_LINKS: usize = 8;
/// The biggest Office file, PDF or ZIP archive read.
const MOST_OFFICE: u64 = 64_000_000;
/// How much of a picture is read to find how big it is (its header: a JPEG's may come after a photo's details).
const PICTURE_HEAD: u64 = 65_536;

/// What a file is, from its first bytes (and, for a ZIP, its name's extension).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Picture(&'static str),
    Pdf,
    Office,
    Zip,
    /// Microsoft's "compound file": Office's files before 2007, and Office files a password protects.
    OldOffice,
    /// A picture or video in the ISO media format: what it is ("a HEIC picture").
    Media(&'static str),
    Other,
}

fn kind_of(head: &[u8], name: &str) -> Kind {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Kind::Picture("image/jpeg");
    }
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Kind::Picture("image/png");
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Kind::Picture("image/gif");
    }
    if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        return Kind::Picture("image/webp");
    }
    // (A PDF may have something before its header; a file named .pdf without one is said to be damaged.)
    if head.starts_with(b"%PDF-") || ext == "pdf" {
        return Kind::Pdf;
    }
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        let office = ["docx", "docm", "dotx", "dotm", "xlsx", "xlsm", "xltx", "xltm", "pptx", "pptm", "potx", "ppsx", "odt", "ods", "odp", "ott", "ots", "otp"];
        return if office.contains(&ext.as_str()) { Kind::Office } else { Kind::Zip };
    }
    if head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return Kind::OldOffice;
    }
    if head.get(4..8) == Some(b"ftyp") {
        return match head.get(8..12) {
            Some(b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1") => Kind::Media("a HEIC picture (as iPhones take them)"),
            Some(b"avif" | b"avis") => Kind::Media("an AVIF picture"),
            _ => Kind::Media("a video"),
        };
    }
    Kind::Other
}

/// A picture's width and height in pixels, from its header (None if it doesn't say where it's looked for).
fn dimensions(media: &str, b: &[u8]) -> Option<(u32, u32)> {
    let be16 = |i: usize| Some(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32);
    let le16 = |i: usize| Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32);
    let be32 = |i: usize| Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?));
    let le24 = |i: usize| Some(b.get(i..i + 3)?.iter().rev().fold(0u32, |v, &x| v << 8 | x as u32));
    match media {
        "image/png" => Some((be32(16)?, be32(20)?)),
        "image/gif" => Some((le16(6)?, le16(8)?)),
        "image/webp" => match b.get(12..16)? {
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            b"VP8L" => {
                let v = u32::from_le_bytes(b.get(21..25)?.try_into().ok()?);
                Some(((v & 0x3FFF) + 1, (v >> 14 & 0x3FFF) + 1))
            }
            b"VP8 " => Some((le16(26)? & 0x3FFF, le16(28)? & 0x3FFF)),
            _ => None,
        },
        // JPEG: its segments, to the start of the frame (SOF0 to SOF15, but not DHT, JPG or DAC), which has them.
        _ => {
            let mut at = 2;
            while at + 9 < b.len() {
                if b[at] != 0xFF {
                    return None;
                }
                let marker = b[at + 1];
                if marker == 0xFF {
                    at += 1;
                    continue;
                }
                if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
                    return Some((be16(at + 7)?, be16(at + 5)?));
                }
                at += 2 + be16(at + 2)? as usize;
            }
            None
        }
    }
}

/// A whole file's bytes (at most `most`).
fn whole(fs: &Fs, p: &mut Partition, f: &File, most: u64) -> Result<Vec<u8>, ntfs::Error> {
    let mut data = vec![0u8; f.size.min(most) as usize];
    let n = fs.read(p, f, 0, &mut data)?;
    data.truncate(n);
    Ok(data)
}

/// BitLocker's sector key for a partition it encrypts, once unlocked.
struct Unlocked {
    volume: bitlocker::Volume,
    key: bitlocker::Key,
}

/// A volume on one of the PC's drives, or a USB drive: NTFS, exFAT or FAT, perhaps encrypted with BitLocker.
struct Volume {
    /// Which drive it's on, its partition's first block and size in blocks, the drive's block size, and the
    /// partition as Windows' MountedDevices names it.
    drive: Drive,
    start: u64,
    blocks: u64,
    block_size: u32,
    id: registry::Mounted,
    /// On a USB drive: what's known of it.
    usb: Option<Usb>,
    /// Its letter: Windows' (from Windows' registry: `named`), else one given here.
    letter: char,
    named: bool,
    /// Its name (File Explorer's, beside the letter), once it can be read.
    label: String,
    /// Windows' own drive (it has Windows' folders).
    windows: bool,
    state: State,
}

/// A USB drive a volume is on: its name, serial number and logical unit (Windows names a USB stick's volume by
/// those), and whether the volume is the first on it (the one Windows' name for the stick is for).
struct Usb {
    name: String,
    serial: Option<String>,
    lun: u8,
    first: bool,
}

enum State {
    /// Read: its file system, and BitLocker's key if BitLocker encrypts it (and how it was unlocked).
    Open { fs: Fs, bitlocker: Option<(Unlocked, &'static str)> },
    /// BitLocker encrypts it, locked: its recovery key unlocks it.
    Locked(bitlocker::Volume),
    /// BitLocker encrypts it in a way that isn't read (why).
    Unreadable(String),
}

/// How a volume was unlocked, for the computer's status.
const BY_CLEAR_KEY: &str = "encrypted with BitLocker, which keeps its key in the clear: suspended, or not switched on yet";
const BY_RECOVERY_KEY: &str = "encrypted with BitLocker: unlocked with its recovery key until Chat Computer restarts";

/// Where Windows keeps its registry's SYSTEM hive, which says which drive has which letter.
const SYSTEM_HIVE: &str = "Windows\\System32\\config\\SYSTEM";

static VOLUMES: SpinLock<Vec<Volume>> = SpinLock::new(Vec::new());
/// The letters Windows' registry gives drives, once read (None: there's no Windows drive to read them from).
static LETTERS: SpinLock<Option<Vec<(char, registry::Mounted)>>> = SpinLock::new(None);
/// The USB drives' media looked at (whether or not anything on them could be read), and usb's count of changes to
/// them when they were.
static USB_SEEN: SpinLock<(Vec<Drive>, u64)> = SpinLock::new((Vec::new(), 0));
/// A recovery key typed in the message box, for `work` to unlock with.
static RECOVERY_KEY: SpinLock<Option<String>> = SpinLock::new(None);
/// What was found, for the computer's status.
static SAID: SpinLock<Option<String>> = SpinLock::new(None);

/// The partition's bytes for the file systems' readers, from the drive's blocks, decrypted if BitLocker encrypts it.
/// The last run of blocks read is kept: file systems read small pieces near each other (a FAT's entries, a folder's).
struct Partition<'a> {
    disk: &'a mut dyn Disk,
    start: u64,
    blocks: u64,
    unlocked: Option<&'a Unlocked>,
    /// The first of the blocks kept, and their bytes.
    kept: (u64, Vec<u8>),
}

impl<'a> Partition<'a> {
    fn new(disk: &'a mut dyn Disk, start: u64, blocks: u64, unlocked: Option<&'a Unlocked>) -> Partition<'a> {
        Partition { disk, start, blocks, unlocked, kept: (0, Vec::new()) }
    }

    /// `buf.len()` bytes from byte `offset` of the partition, decrypted if it's encrypted.
    fn bytes(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        match self.unlocked {
            Some(u) => u.volume.read(&u.key, &mut Raw(self), offset, buf),
            None => self.raw(offset, buf),
        }
    }

    /// The bytes as they are on the drive: whole runs of blocks (as many as the drive reads at once) straight in, and
    /// the rest through the blocks kept.
    fn raw(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        let bs = self.disk.block_size() as usize;
        let run = (self.disk.most() / bs).max(1) as u64;
        let mut done = 0usize;
        while done < buf.len() {
            let pos = offset + done as u64;
            let lba = pos / bs as u64;
            if lba >= self.blocks {
                return false;
            }
            let left = buf.len() - done;
            let (first, kept) = (self.kept.0, &self.kept.1);
            if lba >= first && lba < first + (kept.len() / bs) as u64 {
                let at = (pos - first * bs as u64) as usize;
                let n = (kept.len() - at).min(left);
                buf[done..done + n].copy_from_slice(&kept[at..at + n]);
                done += n;
                continue;
            }
            if pos.is_multiple_of(bs as u64) && left >= run as usize * bs {
                let n = (left / bs).min(run as usize).min((self.blocks - lba) as usize);
                if self.disk.read(self.start + lba, &mut buf[done..done + n * bs]).is_err() {
                    return false;
                }
                done += n * bs;
            } else {
                let n = run.min(self.blocks - lba) as usize;
                let mut blocks = vec![0u8; n * bs];
                if self.disk.read(self.start + lba, &mut blocks).is_err() {
                    return false;
                }
                self.kept = (lba, blocks);
            }
        }
        true
    }
}

impl ntfs::Disk for Partition<'_> {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        self.bytes(offset, buf)
    }
}

impl exfat::Disk for Partition<'_> {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        self.bytes(offset, buf)
    }
}

/// The partition's bytes as they are on the drive (BitLocker's reader decrypts them).
struct Raw<'p, 'a>(&'p mut Partition<'a>);

impl bitlocker::Disk for Raw<'_, '_> {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        self.0.raw(offset, buf)
    }
}

/// The partition's sectors for FAT's reader, `size` bytes each.
struct Sectors<'p, 'a> {
    p: &'p mut Partition<'a>,
    size: u64,
}

impl fat::Sectors for Sectors<'_, '_> {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> bool {
        self.p.bytes(sector * self.size, buf)
    }
}

/// A volume's file system.
enum Fs {
    Ntfs(ntfs::Volume),
    Exfat(exfat::Volume),
    Fat(fat::Volume),
}

/// A file or folder, on whichever file system.
#[derive(Clone)]
struct Entry {
    name: String,
    folder: bool,
    size: u64,
    /// When it was last changed, in seconds since 1970.
    modified: i64,
    hidden: bool,
    system: bool,
    /// NTFS's: Windows encrypts it (EFS); it's a link (a junction, a symbolic link); its reparse tag (0: none).
    encrypted: bool,
    link: bool,
    tag: u32,
    at: At,
}

/// Where a file or folder is, in its file system's own terms.
#[derive(Clone)]
enum At {
    Ntfs(u64),
    Exfat(exfat::Entry),
    Fat(fat::Entry),
}

/// A file, opened to read.
struct File {
    size: u64,
    at: FileAt,
}

enum FileAt {
    Ntfs(ntfs::File),
    Exfat(exfat::Entry),
    Fat(fat::Entry),
}

impl Entry {
    fn of_ntfs(e: ntfs::Entry) -> Entry {
        Entry {
            folder: e.folder,
            size: e.size,
            modified: e.modified,
            hidden: e.attributes & ntfs::HIDDEN != 0,
            system: e.attributes & ntfs::SYSTEM != 0,
            encrypted: e.attributes & ntfs::ENCRYPTED != 0,
            link: e.link,
            tag: e.tag,
            at: At::Ntfs(e.record),
            name: e.name,
        }
    }

    fn of_exfat(e: exfat::Entry) -> Entry {
        Entry {
            name: e.name.clone(),
            folder: e.folder,
            size: e.size,
            modified: e.modified,
            hidden: e.attributes & exfat::HIDDEN != 0,
            system: e.attributes & exfat::SYSTEM != 0,
            encrypted: false,
            link: false,
            tag: 0,
            at: At::Exfat(e),
        }
    }

    fn of_fat(i: fat::Item) -> Entry {
        Entry {
            folder: i.entry.folder,
            size: i.entry.size as u64,
            modified: i.modified,
            hidden: i.attributes & fat::HIDDEN != 0,
            system: i.attributes & fat::SYSTEM != 0,
            encrypted: false,
            link: false,
            tag: 0,
            at: At::Fat(i.entry),
            name: i.name,
        }
    }
}

/// FAT's and exFAT's errors, said as NTFS's same ones.
fn fat_error(e: fat::Error) -> ntfs::Error {
    match e {
        fat::Error::Read => ntfs::Error::Read,
        fat::Error::NotFound => ntfs::Error::NotFound,
        fat::Error::NotAFolder => ntfs::Error::NotAFolder,
        fat::Error::NotAFile => ntfs::Error::NotAFile,
        fat::Error::NotFat | fat::Error::Damaged => ntfs::Error::Damaged,
    }
}

fn exfat_error(e: exfat::Error) -> ntfs::Error {
    match e {
        exfat::Error::Read => ntfs::Error::Read,
        exfat::Error::NotFound => ntfs::Error::NotFound,
        exfat::Error::NotAFolder => ntfs::Error::NotAFolder,
        exfat::Error::NotAFile => ntfs::Error::NotAFile,
        exfat::Error::NotExfat | exfat::Error::Damaged => ntfs::Error::Damaged,
    }
}

impl Fs {
    /// The file system on a partition, if it's one that's read.
    fn open(p: &mut Partition) -> Option<Fs> {
        if let Ok(v) = ntfs::open(p) {
            return Some(Fs::Ntfs(v));
        }
        if let Ok(v) = exfat::open(p) {
            return Some(Fs::Exfat(v));
        }
        // (FAT's sector size is in its first sector, which is read as 512 bytes.)
        fat::open(&mut Sectors { p, size: 512 }).ok().map(Fs::Fat)
    }

    /// Its name: "NTFS", "exFAT", "FAT32".
    fn name(&self) -> &'static str {
        match self {
            Fs::Ntfs(_) => "NTFS",
            Fs::Exfat(_) => "exFAT",
            Fs::Fat(v) => match v.kind {
                fat::Kind::Fat12 => "FAT12",
                fat::Kind::Fat16 => "FAT16",
                fat::Kind::Fat32 => "FAT32",
            },
        }
    }

    /// The volume's name (File Explorer's, beside its letter).
    fn label(&self, p: &mut Partition) -> String {
        match self {
            Fs::Ntfs(v) => v.label(p).unwrap_or_default(),
            Fs::Exfat(v) => v.label.clone(),
            Fs::Fat(v) => v.label(&mut Sectors { p, size: v.bytes_per_sector as u64 }).unwrap_or_default(),
        }
    }

    fn root(&self) -> Entry {
        match self {
            Fs::Ntfs(_) => Entry::of_ntfs(ntfs::root()),
            Fs::Exfat(v) => Entry::of_exfat(v.root()),
            Fs::Fat(_) => Entry::of_fat(fat::Item { name: String::new(), short: String::new(), entry: fat::root(), attributes: fat::FOLDER, modified: 0 }),
        }
    }

    /// Whether it's Windows' own drive: NTFS with Windows' folders.
    fn is_windows(&self, p: &mut Partition) -> bool {
        let root = self.root();
        matches!(self, Fs::Ntfs(_)) && ["Windows", "Users"].iter().all(|f| self.child(p, &root, f).is_ok_and(|e| e.folder))
    }

    /// The files and folders in a folder.
    fn list(&self, p: &mut Partition, folder: &Entry) -> Result<Vec<Entry>, ntfs::Error> {
        match (self, &folder.at) {
            (Fs::Ntfs(v), At::Ntfs(record)) => Ok(v.list(p, *record)?.into_iter().map(Entry::of_ntfs).collect()),
            (Fs::Exfat(v), At::Exfat(e)) => Ok(v.list(p, e).map_err(exfat_error)?.into_iter().map(Entry::of_exfat).collect()),
            (Fs::Fat(v), At::Fat(e)) => {
                let items = v.list(&mut Sectors { p, size: v.bytes_per_sector as u64 }, e).map_err(fat_error)?;
                Ok(items.into_iter().map(Entry::of_fat).collect())
            }
            _ => Err(ntfs::Error::Damaged),
        }
    }

    /// The file or folder named `name` in a folder, whatever its case.
    fn child(&self, p: &mut Partition, folder: &Entry, name: &str) -> Result<Entry, ntfs::Error> {
        match (self, &folder.at) {
            (Fs::Ntfs(v), At::Ntfs(record)) => Ok(Entry::of_ntfs(v.child(p, *record, name)?)),
            (Fs::Exfat(v), At::Exfat(e)) => Ok(Entry::of_exfat(v.child(p, e, name).map_err(exfat_error)?)),
            (Fs::Fat(v), At::Fat(e)) => Ok(Entry::of_fat(v.child(&mut Sectors { p, size: v.bytes_per_sector as u64 }, e, name).map_err(fat_error)?)),
            _ => Err(ntfs::Error::Damaged),
        }
    }

    /// A file, to read.
    fn open_file(&self, p: &mut Partition, e: &Entry) -> Result<File, ntfs::Error> {
        if e.folder {
            return Err(ntfs::Error::NotAFile);
        }
        match (self, &e.at) {
            (Fs::Ntfs(v), At::Ntfs(record)) => {
                let f = v.open_file(p, *record)?;
                Ok(File { size: f.size, at: FileAt::Ntfs(f) })
            }
            (Fs::Exfat(_), At::Exfat(x)) => Ok(File { size: e.size, at: FileAt::Exfat(x.clone()) }),
            (Fs::Fat(_), At::Fat(x)) => Ok(File { size: e.size, at: FileAt::Fat(*x) }),
            _ => Err(ntfs::Error::Damaged),
        }
    }

    /// `buf.len()` bytes of a file from byte `offset` (fewer at its end): how many were read.
    fn read(&self, p: &mut Partition, f: &File, offset: u64, buf: &mut [u8]) -> Result<usize, ntfs::Error> {
        match (self, &f.at) {
            (Fs::Ntfs(v), FileAt::Ntfs(file)) => v.read(p, file, offset, buf),
            (Fs::Exfat(v), FileAt::Exfat(e)) => v.read(p, e, offset, buf).map_err(exfat_error),
            (Fs::Fat(v), FileAt::Fat(e)) => v.read(&mut Sectors { p, size: v.bytes_per_sector as u64 }, e, offset, buf).map_err(fat_error),
            _ => Err(ntfs::Error::Damaged),
        }
    }

    /// Where a link goes (NTFS's links; FAT and exFAT have none).
    fn link_target(&self, p: &mut Partition, e: &Entry) -> Result<Option<String>, ntfs::Error> {
        match (self, &e.at) {
            (Fs::Ntfs(v), At::Ntfs(record)) => v.link_target(p, *record),
            _ => Ok(None),
        }
    }
}

/// MBR partition types read here: NTFS's and exFAT's (07h), FAT12's (01h), FAT16's (04h, 06h, 0Eh), FAT32's (0Bh,
/// 0Ch).
const MBR_VOLUMES: [u8; 7] = [0x07, 0x01, 0x04, 0x06, 0x0E, 0x0B, 0x0C];
/// MBR partition types of an extended partition, which has more ("logical drives") in it.
const MBR_EXTENDED: [u8; 3] = [0x05, 0x0F, 0x85];

/// Whether a drive's first block is a volume's own (a drive with no partition table, as some sticks and memory cards
/// are formatted): NTFS's, exFAT's, BitLocker's, or FAT's (its BIOS parameter block making sense).
fn is_volume(b: &[u8]) -> bool {
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    matches!(&b[3..11], b"NTFS    " | b"EXFAT   " | b"-FVE-FS-")
        || bitlocker::is_bitlocker(b)
        || (matches!(b[0], 0xEB | 0xE9)
            && matches!(u16_at(11), 512 | 1024 | 2048 | 4096)
            && b[13].is_power_of_two()
            && u16_at(14) > 0
            && matches!(b[16], 1 | 2)
            && (b[21] == 0xF0 || b[21] >= 0xF8))
}

/// The volumes' partitions on a drive, as (first block, blocks, the partition as MountedDevices names it): its GPT's
/// basic data partitions (each by its own GUID); or its MBR's, with the logical drives in an extended partition (by
/// the disk's signature and where each starts, in bytes); or, with no partition table, the whole drive.
fn partitions(disk: &mut dyn Disk) -> Vec<(u64, u64, registry::Mounted)> {
    let bs = disk.block_size() as usize;
    let total = disk.blocks();
    let mut out = Vec::new();
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    let u64_at = |b: &[u8], i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
    let mut header = vec![0u8; bs];
    if disk.read(1, &mut header).is_ok() && &header[..8] == b"EFI PART" {
        let (first, count, size) = (u64_at(&header, 72), u32_at(&header, 80) as usize, u32_at(&header, 84) as usize);
        if !(128..=1024).contains(&size) || count > 1024 {
            return out;
        }
        let blocks = (count * size).div_ceil(bs);
        let mut entries = vec![0u8; blocks * bs];
        for k in 0..blocks {
            if disk.read(first + k as u64, &mut entries[k * bs..(k + 1) * bs]).is_err() {
                return out;
            }
        }
        for e in entries.chunks_exact(size).take(count) {
            if e[..16] == BASIC_DATA {
                let (lo, hi) = (u64_at(e, 32), u64_at(e, 40));
                if hi >= lo && hi < total {
                    out.push((lo, hi - lo + 1, registry::Mounted::Gpt(e[16..32].try_into().unwrap())));
                }
            }
        }
        return out;
    }
    let mut mbr = vec![0u8; bs];
    if disk.read(0, &mut mbr).is_err() || mbr[510] != 0x55 || mbr[511] != 0xAA {
        return out;
    }
    if is_volume(&mbr) {
        out.push((0, total, registry::Mounted::Device(String::new())));
        return out;
    }
    let signature = u32_at(&mbr, 0x1B8);
    let mut add = |start: u64, count: u64| {
        if count > 0 && start.checked_add(count).is_some_and(|end| end <= total) {
            out.push((start, count, registry::Mounted::Mbr { signature, offset: start * bs as u64 }));
        }
    };
    let mut extended = None;
    for k in 0..4 {
        let e = &mbr[0x1BE + 16 * k..0x1BE + 16 * (k + 1)];
        let (start, count) = (u32_at(e, 8) as u64, u32_at(e, 12) as u64);
        if MBR_VOLUMES.contains(&e[4]) {
            add(start, count);
        } else if MBR_EXTENDED.contains(&e[4]) && extended.is_none() {
            extended = Some(start);
        }
    }
    // An extended partition's logical drives: a chain of extended boot records, each with a drive (from the record)
    // and where the next record is (from the extended partition's start).
    if let Some(base) = extended {
        let mut at = base;
        let mut ebr = vec![0u8; bs];
        for _ in 0..64 {
            if at >= total || disk.read(at, &mut ebr).is_err() || ebr[510] != 0x55 || ebr[511] != 0xAA {
                break;
            }
            let (this, next) = (&ebr[0x1BE..0x1CE], &ebr[0x1CE..0x1DE]);
            if MBR_VOLUMES.contains(&this[4]) {
                add(at + u32_at(this, 8) as u64, u32_at(this, 12) as u64);
            }
            if !MBR_EXTENDED.contains(&next[4]) || u32_at(next, 8) == 0 {
                break;
            }
            at = base + u32_at(next, 8) as u64;
        }
    }
    out
}

/// What a partition holds: a file system read here (decrypted with `unlocked`, if BitLocker encrypts it), opened,
/// with its name and whether it's Windows' drive; or BitLocker's volume, still locked (or why it can't be read); or
/// nothing to read.
fn open_partition(disk: &mut dyn Disk, start: u64, blocks: u64, unlocked: Option<(Unlocked, &'static str)>) -> Option<(State, String, bool)> {
    let mut boot = [0u8; 512];
    let encrypted = Partition::new(&mut *disk, start, blocks, None).raw(0, &mut boot) && bitlocker::is_bitlocker(&boot);
    if encrypted && unlocked.is_none() {
        let mut p = Partition::new(&mut *disk, start, blocks, None);
        return match bitlocker::open(&mut Raw(&mut p)) {
            // Its key in the clear: BitLocker suspended, or not switched on yet.
            Ok(volume) => match volume.unlock_clear() {
                Ok(key) => open_partition(disk, start, blocks, Some((Unlocked { volume, key }, BY_CLEAR_KEY))),
                Err(_) => Some((State::Locked(volume), String::new(), false)),
            },
            Err(bitlocker::Error::Unsupported(what)) => Some((
                State::Unreadable(format!("encrypted with BitLocker in a way Chat Computer's own system can't read yet ({what})")),
                String::new(),
                false,
            )),
            Err(e) => Some((State::Unreadable(format!("encrypted with BitLocker, whose own records on it couldn't be read ({e:?})")), String::new(), false)),
        };
    }
    let mut p = Partition::new(&mut *disk, start, blocks, unlocked.as_ref().map(|(u, _)| u));
    let fs = Fs::open(&mut p)?;
    let windows = fs.is_windows(&mut p);
    let label = fs.label(&mut p);
    Some((State::Open { fs, bitlocker: unlocked }, label, windows))
}

/// Whether a partition is the one Chat Computer started from (on the start-up drive, or, if that isn't one started
/// here, on a USB drive): its files are Chat Computer's own (its settings and memory file), not a drive's to
/// read.
fn start_up_partition(drive: Drive, start: u64, blocks: u64) -> bool {
    let (s_start, s_blocks, on_start) = storage::start_partition();
    s_blocks != 0 && (start, blocks) == (s_start, s_blocks) && if on_start { drive == Drive::Start } else { matches!(drive, Drive::Usb { .. }) }
}

/// The folder of Chat Computer's own files (its settings and its memory file), which is never read, wherever it
/// is.
const OWN_FOLDER: &str = "efi\\chatcomputer-own";

/// Whether a path from a volume's root is Chat Computer's own folder, or in it.
fn own(rel: &str) -> bool {
    let rel = rel.to_lowercase();
    rel == OWN_FOLDER || rel.starts_with(&format!("{OWN_FOLDER}\\"))
}

/// The volumes on a drive, opened, their letters not given yet.
fn volumes_on(drive: Drive) -> Vec<Volume> {
    let unit = storage::usb_unit(drive);
    let found = storage::with_drive(drive, |disk| {
        let block_size = disk.block_size();
        let mut out = Vec::new();
        for (start, blocks, id) in partitions(disk).into_iter().filter(|&(start, blocks, _)| !start_up_partition(drive, start, blocks)) {
            if let Some((state, label, windows)) = open_partition(&mut *disk, start, blocks, None) {
                let usb = unit.as_ref().map(|u| Usb { name: u.name.clone(), serial: u.serial.clone(), lun: u.lun, first: out.is_empty() });
                out.push(Volume { drive, start, blocks, block_size, id, usb, letter: '?', named: false, label, windows, state });
            }
        }
        out
    });
    found.unwrap_or_default()
}

/// The hive file's bytes, through the NTFS reader.
struct HiveFile<'a, 'b> {
    v: &'a ntfs::Volume,
    p: &'a mut Partition<'b>,
    f: ntfs::File,
}

impl registry::Source for HiveFile<'_, '_> {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        self.v.read(self.p, &self.f, offset, buf).is_ok_and(|n| n == buf.len())
    }
}

/// The drive letters Windows' registry (SYSTEM's MountedDevices, on Windows' drive) gives volumes.
fn windows_letters(volumes: &[Volume]) -> Option<Vec<(char, registry::Mounted)>> {
    let w = volumes.iter().find(|v| v.windows && matches!(v.state, State::Open { .. }))?;
    let State::Open { fs: Fs::Ntfs(ntfs), bitlocker } = &w.state else { return None };
    storage::with_drive(w.drive, |disk| {
        let mut p = Partition::new(disk, w.start, w.blocks, bitlocker.as_ref().map(|(u, _)| u));
        let e = ntfs.find(&mut p, SYSTEM_HIVE).ok()?;
        let f = ntfs.open_file(&mut p, e.record).ok()?;
        let mut file = HiveFile { v: ntfs, p: &mut p, f };
        let hive = registry::Hive::open(&mut file).ok()?;
        registry::drive_letters(&hive, &mut file).ok()
    })?
}

/// The letter Windows' registry gives a volume: by its partition (a GPT partition's GUID, an MBR disk's signature and
/// where it starts), or, for a USB stick's first volume, by the stick (its serial number and logical unit).
fn registry_letter(v: &Volume, letters: &[(char, registry::Mounted)]) -> Option<char> {
    letters
        .iter()
        .find(|(_, m)| match m {
            registry::Mounted::Device(_) => v.usb.as_ref().is_some_and(|u| u.first && u.serial.as_deref().is_some_and(|s| m.is_usb(s, u.lun))),
            _ => *m == v.id,
        })
        .map(|(l, _)| *l)
}

/// The first letter from D: that no volume here has (`used`) and, if one's left, that Windows hasn't given another
/// drive (`taken`: a USB stick not plugged in, say), as Windows gives a drive it hasn't seen.
fn next_letter(used: &[char], taken: &[char]) -> char {
    ('D'..='Z').find(|c| !used.contains(c) && !taken.contains(c)).or_else(|| ('D'..='Z').find(|c| !used.contains(c))).unwrap_or('?')
}

/// Gives the volumes from `new` on their letters, the others keeping theirs: Windows' where its registry has one
/// (`letters`) that no volume here has; else Windows' own drive C: (if no drive here has it), and the others the
/// next letters, as `next_letter` says. All of them (`new` 0): Windows' letters first, the rest in order.
fn name_volumes(volumes: &mut [Volume], new: usize, letters: &[(char, registry::Mounted)]) {
    let mut used: Vec<char> = volumes[..new].iter().map(|v| v.letter).collect();
    for v in volumes[new..].iter_mut() {
        v.named = false;
        if let Some(l) = registry_letter(v, letters)
            && !used.contains(&l)
        {
            v.letter = l;
            v.named = true;
            used.push(l);
        }
    }
    let taken: Vec<char> = letters.iter().map(|(l, _)| *l).collect();
    // (With no Windows drive read, a locked one on the start-up drive is most likely Windows'.)
    let windows_found = volumes.iter().any(|v| v.windows);
    for v in volumes[new..].iter_mut().filter(|v| !v.named) {
        let likely_windows = v.windows || (!windows_found && v.drive == Drive::Start && matches!(v.state, State::Locked(_)));
        v.letter = if likely_windows && !used.contains(&'C') { 'C' } else { next_letter(&used, &taken) };
        used.push(v.letter);
    }
    // One Windows drive: C: (another with Windows' folders, an old Windows on a second drive say, is just a drive).
    if let Some(c) = volumes.iter().position(|v| v.letter == 'C' && v.windows) {
        for (i, v) in volumes.iter_mut().enumerate() {
            v.windows &= i == c;
        }
    }
    volumes.sort_by_key(|v| v.letter);
}

/// What a volume is, for the computer's status: "C: Windows' drive (NTFS, 512.1 GB)", "E: “MY STICK” (FAT32, 32.0
/// GB, on the USB drive “SanDisk Ultra”)".
fn describe(v: &Volume, registry_read: bool) -> String {
    let what = if v.windows {
        String::from("Windows' drive")
    } else if !v.label.is_empty() {
        format!("\u{201C}{}\u{201D}", v.label)
    } else {
        String::from("a drive")
    };
    let mut more = String::new();
    if let Some(u) = &v.usb {
        more += &format!(", on the USB drive \u{201C}{}\u{201D}", u.name);
    }
    if registry_read && !v.named {
        more += ", its letter given here, as Windows hasn't named it";
    }
    let size = size(v.blocks * v.block_size as u64);
    match &v.state {
        State::Open { fs, bitlocker: Some((_, how)) } => format!("{}: {what} ({}, {size}, {how}{more})", v.letter, fs.name()),
        State::Open { fs, .. } => format!("{}: {what} ({}, {size}{more})", v.letter, fs.name()),
        State::Locked(b) => {
            let ids: Vec<String> = b.recovery_key_ids().iter().map(|id| String::from(&id[..8])).collect();
            format!("{}: encrypted with BitLocker, locked ({size}{more}; its recovery key's ID starts {})", v.letter, ids.join(" or "))
        }
        State::Unreadable(why) => format!("{}: {why} ({size}{more})", v.letter),
    }
}

/// The computer's status's line about files, from the volumes found.
fn said_of(volumes: &[Volume], registry_read: bool) -> String {
    if volumes.is_empty() {
        return String::from("none: no NTFS, exFAT or FAT volume on this PC's drives, or on a USB drive");
    }
    let mut said = volumes.iter().map(|v| describe(v, registry_read)).collect::<Vec<String>>().join("; ");
    said += ", read only";
    if volumes.iter().any(|v| matches!(v.state, State::Locked(_))) {
        said += ". To read a locked drive, the person types its recovery key in the message box (48 digits, from \
                  https://aka.ms/myrecoverykey or wherever they saved it): Chat Computer's own system unlocks the drive \
                  itself, and the key isn't given to the model, or kept";
    }
    said
}

/// Finds the volumes on the PC's drives and the USB drives plugged in, and gives them their letters. Says what it
/// found, for the log.
pub fn init() -> String {
    let changes = usb::storage::changes();
    let drives = storage::drives();
    let mut volumes: Vec<Volume> = Vec::new();
    for &drive in &drives {
        volumes.extend(volumes_on(drive));
    }
    let letters = windows_letters(&volumes);
    name_volumes(&mut volumes, 0, letters.as_deref().unwrap_or(&[]));
    let said = said_of(&volumes, letters.is_some());
    *VOLUMES.lock() = volumes;
    *LETTERS.lock() = letters;
    *USB_SEEN.lock() = (drives.into_iter().filter(|d| matches!(d, Drive::Usb { .. })).collect(), changes);
    *SAID.lock() = Some(said.clone());
    format!("files: {said}")
}

/// Why a drive BitLocker encrypts can't be read yet, and how it can be: the person types its recovery key.
fn locked_why(letter: char, v: &bitlocker::Volume) -> String {
    let ids = v.recovery_key_ids();
    if ids.is_empty() {
        return format!(
            "{letter}: is encrypted with BitLocker, with no recovery key: only this PC's security chip (its TPM) unlocks \
             it, for Windows as it starts, so Chat Computer's own system can't read it"
        );
    }
    let id: Vec<&str> = ids.iter().map(|id| &id[..8]).collect();
    format!(
        "{letter}: is encrypted with BitLocker. To read it here, the person types its recovery key in the message box: \
         48 digits, which Windows keeps in their Microsoft account (https://aka.ms/myrecoverykey) if they signed in with \
         one, or which they saved or printed; its key ID starts {}. Chat Computer's own system unlocks the drive with the \
         key itself: it isn't given to the model, or kept, so it's typed again after a restart",
        id.join(" or ")
    )
}

/// A recovery key typed in the message box (chat.rs): `work` unlocks a drive with it.
pub fn unlock_later(key: String) {
    if let Some(mut old) = RECOVERY_KEY.lock().replace(key) {
        wipe(&mut old);
    }
}

/// Overwrites a key's characters before it's dropped.
pub fn wipe(s: &mut String) {
    // (Zeros are still UTF-8.)
    for b in unsafe { s.as_bytes_mut() } {
        unsafe { core::ptr::write_volatile(b, 0) };
    }
    s.clear();
}

/// From the main loop: USB drives plugged in or out (or cards put in or taken out), their volumes found and given
/// letters, or forgotten; and a drive unlocked with a recovery key typed, if one was. What came of it, for the screen.
/// (Stretching a key takes a second or so: the screen waits.)
pub fn work() -> Vec<String> {
    let mut said = Vec::new();
    if usb::storage::changes() != USB_SEEN.lock().1 {
        said.extend(usb_changed());
    }
    if let Some(mut typed) = RECOVERY_KEY.lock().take() {
        said.push(unlock(&typed));
        wipe(&mut typed);
    }
    said
}

/// What's on a USB drive's medium that came, for the screen: its volumes, or why none can be read.
fn arrived(v: &[&Volume], name: &str, bytes: u64) -> String {
    if v.is_empty() {
        return format!(
            "USB drive \u{201C}{name}\u{201D} ({}): nothing on it can be read here: Chat Computer's own system reads NTFS, \
             exFAT and FAT, as Windows formats drives (not a Mac's or Linux's own).",
            size(bytes)
        );
    }
    let all: Vec<String> = v
        .iter()
        .map(|v| {
            let label = if v.label.is_empty() { String::new() } else { format!(" \u{201C}{}\u{201D}", v.label) };
            match &v.state {
                State::Open { fs, .. } => format!("{}:{label} ({}, {})", v.letter, fs.name(), size(v.blocks * v.block_size as u64)),
                State::Locked(_) => format!("{}: (encrypted with BitLocker: to read it, type its recovery key in the message box)", v.letter),
                State::Unreadable(why) => format!("{}: ({why})", v.letter),
            }
        })
        .collect();
    let readable = v.iter().any(|v| matches!(v.state, State::Open { .. }));
    format!("USB drive \u{201C}{name}\u{201D}: {}.{}", all.join(", "), if readable { " Its files can be read." } else { "" })
}

/// The USB drives' media now: those gone forgotten, with their volumes; those new looked at, and their volumes given
/// letters (the other drives keep theirs). What came and went, for the screen.
fn usb_changed() -> Vec<String> {
    let changes = usb::storage::changes();
    let now: Vec<Drive> = storage::drives().into_iter().filter(|d| matches!(d, Drive::Usb { .. })).collect();
    let seen = core::mem::replace(&mut *USB_SEEN.lock(), (now.clone(), changes)).0;
    let mut said = Vec::new();
    let mut volumes = core::mem::take(&mut *VOLUMES.lock());
    let (gone, kept): (Vec<Volume>, Vec<Volume>) = volumes.drain(..).partition(|v| matches!(v.drive, Drive::Usb { .. }) && !now.contains(&v.drive));
    volumes = kept;
    for v in &gone {
        let label = if v.label.is_empty() { String::new() } else { format!(" (\u{201C}{}\u{201D})", v.label) };
        said.push(format!("{}:{label} is gone: its USB drive was unplugged, or its card taken out.", v.letter));
    }
    let old = volumes.len();
    let mut new_drives = Vec::new();
    for &drive in now.iter().filter(|d| !seen.contains(d)) {
        let found = volumes_on(drive);
        new_drives.push((drive, found.len()));
        volumes.extend(found);
    }
    if volumes.len() > old {
        let letters = LETTERS.lock().clone();
        name_volumes(&mut volumes, old, letters.as_deref().unwrap_or(&[]));
    }
    for (drive, _) in &new_drives {
        let Some(unit) = storage::usb_unit(*drive) else { continue };
        let on: Vec<&Volume> = volumes.iter().filter(|v| v.drive == *drive).collect();
        said.push(arrived(&on, &unit.name, unit.medium.blocks * unit.medium.block_size as u64));
    }
    let registry_read = LETTERS.lock().is_some();
    *SAID.lock() = Some(said_of(&volumes, registry_read));
    *VOLUMES.lock() = volumes;
    said
}

fn unlock(typed: &str) -> String {
    let mut volumes = core::mem::take(&mut *VOLUMES.lock());
    let said = unlock_in(&mut volumes, typed);
    *VOLUMES.lock() = volumes;
    said
}

fn unlock_in(volumes: &mut Vec<Volume>, typed: &str) -> String {
    let locked: Vec<usize> = (0..volumes.len()).filter(|&i| matches!(volumes[i].state, State::Locked(_))).collect();
    if locked.is_empty() {
        return String::from("No drive here is locked with BitLocker, so that recovery key isn't needed.");
    }
    for i in locked {
        let State::Locked(b) = &volumes[i].state else { continue };
        let key = match b.unlock_recovery(typed) {
            Ok(key) => key,
            Err(bitlocker::Error::WrongKey | bitlocker::Error::NoSuchProtector) => continue,
            Err(e) => return format!("That recovery key can't unlock {}: here ({e:?}).", volumes[i].letter),
        };
        let unlocked = Unlocked { volume: b.clone(), key };
        let (drive, start, blocks) = (volumes[i].drive, volumes[i].start, volumes[i].blocks);
        let opened = storage::with_drive(drive, |disk| open_partition(disk, start, blocks, Some((unlocked, BY_RECOVERY_KEY))));
        let Some(Some((state, label, windows))) = opened else {
            return String::from("That recovery key unlocks the drive, but its files can't be read: it may be damaged.");
        };
        let v = &mut volumes[i];
        (v.state, v.label, v.windows) = (state, label, windows);
        // Windows' drive unlocked: its registry says which drive has which letter.
        if windows {
            let letters = windows_letters(volumes);
            if letters.is_some() {
                name_volumes(volumes, 0, letters.as_deref().unwrap_or(&[]));
                *LETTERS.lock() = letters;
            }
        }
        *SAID.lock() = Some(said_of(volumes, LETTERS.lock().is_some()));
        let v = volumes.iter().find(|v| v.drive == drive && v.start == start).unwrap();
        let what = if v.windows { String::from(" (Windows' drive)") } else if v.label.is_empty() { String::new() } else { format!(" (\u{201C}{}\u{201D})", v.label) };
        return format!("{}:{what} is unlocked: its files can be read now, until Chat Computer restarts.", v.letter);
    }
    let ids: Vec<String> = volumes
        .iter()
        .filter_map(|v| match &v.state {
            State::Locked(b) => Some(b.recovery_key_ids().iter().map(|id| format!("{} ({}:)", &id[..8], v.letter)).collect::<Vec<String>>().join(" or ")),
            _ => None,
        })
        .collect();
    format!(
        "That recovery key doesn't unlock a drive here. The keys that do have IDs starting {} (as \
         https://aka.ms/myrecoverykey lists them).",
        ids.join(", ")
    )
}

/// What files can be read, for the computer's status.
pub fn status() -> String {
    format!("Files: {}", SAID.lock().clone().unwrap_or_else(|| String::from("not looked for")))
}

/// The letter of Windows' own drive (C:, nearly always).
fn windows_letter() -> char {
    VOLUMES.lock().iter().find(|v| v.windows).map_or('C', |v| v.letter)
}

/// Runs `f` with the file system of drive letter `letter` (Err if there's none to read: the reason, for the model).
fn with<T>(letter: char, f: impl FnOnce(&Fs, &mut Partition) -> T) -> Result<T, String> {
    let volumes = core::mem::take(&mut *VOLUMES.lock());
    let result = match volumes.iter().find(|v| v.letter == letter) {
        None if volumes.is_empty() => {
            let why = status();
            let why = why.trim_start_matches("Files: ").trim_start_matches("none: ");
            Err(format!("Chat Computer's own system can't read this computer's files: {why}"))
        }
        None => {
            let all: Vec<String> = volumes.iter().map(|v| format!("{}:", v.letter)).collect();
            Err(format!("There's no drive {letter}: here (its drives are {})", all.join(", ")))
        }
        Some(v) => match &v.state {
            State::Open { fs, bitlocker } => storage::with_drive(v.drive, |disk| {
                let mut p = Partition::new(disk, v.start, v.blocks, bitlocker.as_ref().map(|(u, _)| u));
                f(fs, &mut p)
            })
            .ok_or_else(|| match v.usb {
                Some(_) => format!("{letter}: is gone: its USB drive was unplugged, or its card taken out"),
                None => String::from("the drive is busy"),
            }),
            State::Locked(b) => Err(locked_why(letter, b)),
            State::Unreadable(why) => Err(format!("{letter}: is {why}")),
        },
    };
    *VOLUMES.lock() = volumes;
    result
}

/// A path as the model gives it ("C:\Users\Sam", "d:/Photos", "/Users/Sam", "Users\Sam"): its drive's letter (Windows'
/// drive's if it has none), and the path in that volume from its root.
fn inside(path: &str) -> (char, String) {
    let p = path.trim().trim_matches('"');
    let (letter, p) = match p.as_bytes() {
        [d, b':', ..] if d.is_ascii_alphabetic() => (d.to_ascii_uppercase() as char, &p[2..]),
        _ => (windows_letter(), p),
    };
    (letter, p.replace('/', "\\").trim_matches('\\').to_string())
}

/// The drive's letter and the path, as Windows writes it: "C:\Users".
fn shown(letter: char, rel: &str) -> String {
    format!("{letter}:\\{rel}")
}

fn joined(rel: &str, name: &str) -> String {
    if rel.is_empty() { String::from(name) } else { format!("{rel}\\{name}") }
}

/// A size as it's said: "512 bytes", "12 KB", "3.4 MB", "1.2 GB".
fn size(n: u64) -> String {
    match n {
        0..1_000 => format!("{n} bytes"),
        1_000..1_000_000 => format!("{} KB", n.div_ceil(1_000)),
        1_000_000..1_000_000_000 => format!("{}.{} MB", n / 1_000_000, n / 100_000 % 10),
        _ => format!("{}.{} GB", n / 1_000_000_000, n / 100_000_000 % 10),
    }
}

/// A date: "2026-10-21".
fn date(t: i64) -> String {
    let (y, m, d, _, _) = crate::clock::civil(t);
    format!("{y}-{m:02}-{d:02}")
}

fn error_text(e: ntfs::Error, what: &str) -> String {
    match e {
        ntfs::Error::NotFound => format!("{what}: there's no such file or folder"),
        ntfs::Error::NotAFolder => format!("{what}: that's a file, not a folder (read_file reads it)"),
        ntfs::Error::NotAFile => format!("{what}: that's a folder (list_files lists it)"),
        ntfs::Error::Encrypted => format!("{what}: Windows encrypts this file (EFS): only Windows can read it"),
        ntfs::Error::Link => format!("{what}: a link that Chat Computer's own system can't follow"),
        ntfs::Error::Online(tag) if ntfs::is_cloud(tag) => {
            let service = if what.to_lowercase().contains("onedrive") { "OneDrive" } else { "the cloud service it's synced with" };
            format!("{what}: this file is online only: it's in {service}, not downloaded to this PC (Windows downloads it when it's opened there)")
        }
        ntfs::Error::Online(tag) => format!("{what}: Windows keeps this file's bytes elsewhere, not on this drive (reparse tag {tag:#010X})"),
        ntfs::Error::Compacted => format!("{what}: Windows compressed this file itself (Compact OS), which Chat Computer's own system can't read yet"),
        ntfs::Error::Damaged => format!("{what}: the drive's file system doesn't add up there"),
        ntfs::Error::TooMany => format!("{what}: this folder has more in it than Chat Computer's own system lists (millions of files)"),
        ntfs::Error::Read => format!("{what}: the drive couldn't be read"),
        _ => format!("{what}: {e:?}"),
    }
}

/// Whether to leave a file or folder out of lists and searches: Windows' own hidden system files (desktop.ini, the
/// recycle bin), as File Explorer does.
fn left_out(e: &Entry) -> bool {
    e.hidden && e.system
}

/// The file or folder at `rel` (links on the way followed, to the end), its path with the names as the drive has them,
/// and the first link followed, if one was.
fn found(fs: &Fs, p: &mut Partition, letter: char, rel: &str) -> Result<(Entry, String, Option<String>), String> {
    let shown = |rel: &str| shown(letter, rel);
    let mut parts: VecDeque<String> = rel.split(['\\', '/']).filter(|s| !s.is_empty()).map(String::from).collect();
    let mut way: Vec<Entry> = Vec::new();
    let mut links: Vec<String> = Vec::new();
    let path = |way: &[Entry]| way.iter().map(|e| e.name.as_str()).collect::<Vec<&str>>().join("\\");
    while let Some(part) = parts.pop_front() {
        if part == "." {
            continue;
        }
        if part == ".." {
            way.pop();
            continue;
        }
        let here = way.last().cloned().unwrap_or_else(|| fs.root());
        if !here.folder {
            return Err(error_text(ntfs::Error::NotAFolder, &shown(&path(&way))));
        }
        let e = fs.child(p, &here, &part).map_err(|err| error_text(err, &shown(&joined(&path(&way), &part))))?;
        if own(&joined(&path(&way), &e.name)) {
            return Err(format!("{}: Chat Computer's own folder (its settings and memory), which isn't read here", shown(&joined(&path(&way), &e.name))));
        }
        if !e.link {
            way.push(e);
            continue;
        }
        let at = shown(&joined(&path(&way), &e.name));
        if links.len() == MOST_LINKS {
            return Err(format!("{at}: a link to a link to... (more than {MOST_LINKS}), which Chat Computer's own system stops following"));
        }
        let target = fs.link_target(p, &e).map_err(|err| error_text(err, &at))?;
        let Some(target) = target else { return Err(error_text(ntfs::Error::Link, &at)) };
        // A path from this drive's letter (or from / on it, or /mnt/c for a link made under Linux on C:) goes from
        // the root; one to another drive, a network's or Linux's own, isn't followed; others go from the link's folder.
        let here = [letter as u8, b':'];
        let linux = [b'/', b'm', b'n', b't', b'/', letter.to_ascii_lowercase() as u8];
        let rest = if target.as_bytes().get(..2).is_some_and(|d| d.eq_ignore_ascii_case(&here)) {
            way.clear();
            &target[2..]
        } else if target.as_bytes().get(..6).is_some_and(|d| d.eq_ignore_ascii_case(&linux)) {
            way.clear();
            &target[6..]
        } else if target.get(1..2) == Some(":") || target.starts_with("UNC\\") || target.starts_with("Volume{") || target.starts_with('/') {
            return Err(format!("{at}: a link to {target}, which isn't on this drive ({letter}:)"));
        } else if target.starts_with('\\') {
            way.clear();
            &target[..]
        } else {
            &target[..]
        };
        for t in rest.split(['\\', '/']).filter(|s| !s.is_empty()).rev() {
            parts.push_front(String::from(t));
        }
        links.push(at);
    }
    let real = path(&way);
    Ok((way.pop().unwrap_or_else(|| fs.root()), real, links.into_iter().next()))
}

/// After a file's or folder's name in a list: "\\" for a folder, and for a file its size and when it was changed; a
/// link says where it goes, and a file whose bytes aren't here, or can't be read here, says so.
fn details(fs: &Fs, p: &mut Partition, e: &Entry) -> String {
    let slash = if e.folder { "\\" } else { "" };
    if e.link {
        return match fs.link_target(p, e) {
            Ok(Some(t)) => format!("{slash} (a link to {t})"),
            _ => format!("{slash} (a link)"),
        };
    }
    if e.folder {
        return String::from(slash);
    }
    let mut d = format!("{}, {}", size(e.size), date(e.modified));
    if e.encrypted {
        d += ", encrypted by Windows";
    } else if e.tag != 0 {
        match fs.open_file(p, e) {
            Err(ntfs::Error::Online(_)) => d += ", online only: not on this PC",
            Err(ntfs::Error::Compacted) => d += ", compressed by Windows",
            _ => {}
        }
    }
    format!(" ({d})")
}

/// "(reached by the link C:\\...)", after a path, if a link was followed to it.
fn by_way_of(link: &Option<String>) -> String {
    link.as_ref().map(|l| format!(" (reached by the link {l})")).unwrap_or_default()
}

/// The files and folders in a folder (list_files).
pub fn list(path: &str) -> Result<String, String> {
    let (letter, rel) = inside(path);
    with(letter, |fs, p| {
        let (here, rel, link) = found(fs, p, letter, &rel)?;
        if !here.folder {
            return Err(error_text(ntfs::Error::NotAFolder, &shown(letter, &rel)));
        }
        let all: Vec<Entry> = fs.list(p, &here).map_err(|e| error_text(e, &shown(letter, &rel)))?.into_iter().filter(|e| !left_out(e)).collect();
        let (folders, files) = (all.iter().filter(|e| e.folder).count(), all.iter().filter(|e| !e.folder).count());
        let mut out = format!(
            "{}{}: {folders} folder{}, {files} file{}",
            shown(letter, &rel),
            by_way_of(&link),
            if folders == 1 { "" } else { "s" },
            if files == 1 { "" } else { "s" }
        );
        for e in all.iter().take(MOST_LISTED) {
            out += &format!("\n{}{}", e.name, details(fs, p, e));
            if e.hidden {
                out += " (hidden)";
            }
        }
        if all.len() > MOST_LISTED {
            out += &format!("\n...and {} more", all.len() - MOST_LISTED);
        }
        Ok(out)
    })?
}

/// The last PDF read (read_file): which (its name, size and when it changed), and its words.
static PDF_READ: SpinLock<Option<(String, Arc<pdf::Document>)>> = SpinLock::new(None);

/// What read_file gives the model: words (or what the file is), and a picture it sees, if it's given one.
pub struct Got {
    pub said: String,
    pub picture: Option<picture::Picture>,
}

/// How the model sees, when it can: the size (width, height) it reads a picture of a given size at.
pub type Sight<'a> = &'a dyn Fn(usize, usize) -> (usize, usize);

/// A file (read_file), from byte `from`: text as it is (UTF-8, or UTF-16 as Notepad saves it); an Office file's or a
/// PDF's words (`from` counts in them); a ZIP archive's list. A picture (JPEG, PNG or GIF) is given to the model to see
/// if it can (`sight`), as is a PDF's page whose words are pictures (a scanned one: `page`, from 1); else, as a file
/// of another kind, it's said to be one (and why the model can't see it: `blind`).
pub fn read(path: &str, from: u64, page: Option<u64>, sight: Option<Sight>, blind: &str) -> Result<Got, String> {
    let (letter, rel) = inside(path);
    with(letter, |fs, p| {
        let (e, rel, link) = found(fs, p, letter, &rel)?;
        let f = fs.open_file(p, &e).map_err(|err| error_text(err, &shown(letter, &rel)))?;
        let said = |text: String| Ok(Got { said: text, picture: None });
        let fail = |err: ntfs::Error| error_text(err, &shown(letter, &rel));
        let name = format!("{}{}", shown(letter, &rel), by_way_of(&link));
        let mut head = [0u8; 32];
        let n = fs.read(p, &f, 0, &mut head).map_err(fail)?;
        match kind_of(&head[..n], &rel) {
            Kind::Picture(media) => {
                let mut head = vec![0u8; f.size.min(PICTURE_HEAD) as usize];
                let n = fs.read(p, &f, 0, &mut head).map_err(fail)?;
                let pixels = dimensions(media, &head[..n]).map(|(w, h)| format!(", {w}×{h} pixels")).unwrap_or_default();
                let about = format!("{name} ({}{pixels}, changed {})", size(f.size), date(e.modified));
                let Some(sight) = sight else {
                    return said(format!(
                        "{about} is a picture, which Chat Computer's model here can't see: {blind} show_picture shows it \
                         to the person."
                    ));
                };
                if media == "image/webp" {
                    return said(format!("{about} is a WebP picture, which Chat Computer's own system can't open yet."));
                }
                if f.size > MOST_SHOWN {
                    return said(format!("{about} is a picture too big for Chat Computer's own system to open."));
                }
                let data = whole(fs, p, &f, MOST_SHOWN).map_err(fail)?;
                let started = time::uptime_ms();
                match picture::decode_for(&data, sight, decoding_room()) {
                    Ok(pic) => {
                        crate::log!(
                            "Chat Computer kernel: files: a picture made ready to be seen ({}×{}) in {} ms",
                            pic.width,
                            pic.height,
                            time::uptime_ms().saturating_sub(started)
                        );
                        Ok(Got {
                            said: format!("{about}: the picture is here (as you see it, {}×{} pixels).", pic.width, pic.height),
                            picture: Some(pic),
                        })
                    }
                    Err(picture::Error::Unsupported(what)) => said(format!("{about} is a {what}, which Chat Computer's own system can't open yet.")),
                    Err(picture::Error::TooBig) => said(format!("{about} has too many pixels for Chat Computer's own system to open.")),
                    Err(_) => said(format!("{about} should be a picture, but it seems to be damaged: it can't be opened.")),
                }
            }
            Kind::Office | Kind::Zip | Kind::Pdf if f.size > MOST_OFFICE => {
                said(format!("{name} is {}: too big for Chat Computer's own system to open.", size(f.size)))
            }
            Kind::Pdf => {
                // (Read once: its next parts come from what was read, if it hasn't changed.)
                let key = format!("{name}\n{}\n{}", f.size, e.modified);
                let known = PDF_READ.lock().as_ref().filter(|(k, _)| *k == key).map(|(_, d)| d.clone());
                let d = match known {
                    Some(d) => d,
                    None => {
                        let data = whole(fs, p, &f, MOST_OFFICE).map_err(fail)?;
                        let started = time::uptime_ms();
                        match pdf::read(&data) {
                            Ok(d) => {
                                crate::log!(
                                    "Chat Computer kernel: files: read a PDF ({} page{}, {} bytes of words) in {} ms",
                                    d.pages,
                                    if d.pages == 1 { "" } else { "s" },
                                    d.text.len(),
                                    time::uptime_ms().saturating_sub(started)
                                );
                                let d = Arc::new(d);
                                *PDF_READ.lock() = Some((key, d.clone()));
                                d
                            }
                            Err(pdf::Error::Encrypted) => return said(format!("{name}: a password protects it, so it can't be read here.")),
                            Err(_) => {
                                return said(format!(
                                    "{name} ({}) should be a PDF, but Chat Computer's own system can't read it: it may be damaged.",
                                    size(f.size)
                                ));
                            }
                        }
                    }
                };
                let about = format!(
                    "{name} (a PDF, {} page{}, {}, changed {}){}",
                    d.pages,
                    if d.pages == 1 { "" } else { "s" },
                    size(f.size),
                    date(e.modified),
                    d.title.as_ref().map(|t| format!(", titled \u{201C}{t}\u{201D}")).unwrap_or_default()
                );
                // A page to see: the one asked for, or the first, if no page has words.
                let look = page.or(d.text.is_empty().then_some(1));
                if let Some(k) = look {
                    let wordless = if d.text.is_empty() { " has no words that can be read: its pages are pictures (scanned, perhaps)" } else { "" };
                    if k == 0 || k > d.pages as u64 {
                        return said(format!("{about}{wordless}: there's no page {k}."));
                    }
                    let Some(sight) = sight else {
                        return said(format!("{about}{wordless}, which Chat Computer's model here can't see as pictures: {blind}"));
                    };
                    let data = whole(fs, p, &f, MOST_OFFICE).map_err(fail)?;
                    let started = time::uptime_ms();
                    let next = if k < d.pages as u64 { format!(" read_file with page {} shows the next.", k + 1) } else { String::new() };
                    return match pdf::page_picture(&data, k as usize - 1, sight, decoding_room()) {
                        Ok(Some(pic)) if pic.drawn == 0 => {
                            let kinds = if pic.missing.is_empty() { String::from("too big, or damaged") } else { format!("in {}", pic.missing.join(" and ")) };
                            said(format!(
                                "{about}{wordless}: page {k}'s pictures can't be opened by Chat Computer's own system \
                                 ({kinds}).{next}"
                            ))
                        }
                        Ok(Some(pic)) => {
                            crate::log!(
                                "Chat Computer kernel: files: a PDF's page {k} made a picture to be seen ({}×{}, {} of its \
                                 pictures) in {} ms",
                                pic.width,
                                pic.height,
                                pic.drawn,
                                time::uptime_ms().saturating_sub(started)
                            );
                            let missing = if pic.missing.is_empty() {
                                String::new()
                            } else {
                                format!(
                                    " (some of its pictures, in {}, Chat Computer's own system can't open yet: they're left white)",
                                    pic.missing.join(" and ")
                                )
                            };
                            let (width, height) = (pic.width as u32, pic.height as u32);
                            Ok(Got {
                                said: format!(
                                    "{about}{wordless}. Page {k}'s pictures are here, where they are on the page (as you see \
                                     it, {width}×{height} pixels){missing}.{next}"
                                ),
                                picture: Some(picture::Picture { width, height, rgb: pic.rgb }),
                            })
                        }
                        Ok(None) if pdf_page_has_words(&d.text, k) => said(format!("{about}: page {k} has no pictures, only words (read_file reads them).")),
                        Ok(None) => said(format!("{about}{wordless}: page {k} has nothing to see on it.{next}")),
                        Err(pdf::Error::Encrypted) => said(format!("{name}: a password protects it, so it can't be read here.")),
                        Err(_) => said(format!("{about}: page {k} can't be read: the file may be damaged.")),
                    };
                }
                let mut out = part_of(&d.text, from, &format!("{about}, its words"));
                if d.cut && (from as usize).saturating_add(READ_AT_ONCE) >= d.text.len() {
                    out += "\n[That's as far as it's read here: its words go on past 8 MB.]";
                }
                if sight.is_some() && d.text.contains(pdf::NO_WORDS) {
                    out += "\n[A page with no words may be a picture (scanned): read_file with page shows you its pictures.]";
                }
                said(out)
            }
            Kind::Office => {
                let data = whole(fs, p, &f, MOST_OFFICE).map_err(fail)?;
                match office::read(&data) {
                    Ok((kind, words)) => said(part_of(&words, from, &format!("{name} ({}, {}, changed {}), its words", kind.name(), size(f.size), date(e.modified)))),
                    Err(office::Error::Zip(zip::Error::Encrypted)) => said(format!("{name}: a password protects it, so it can't be read here.")),
                    Err(_) => said(format!("{name} ({}) should be a Word, Excel or PowerPoint file, but Chat Computer's own system can't read it: it may be damaged.", size(f.size))),
                }
            }
            Kind::Zip => {
                let data = whole(fs, p, &f, MOST_OFFICE).map_err(fail)?;
                match zip::Archive::open(&data) {
                    Ok(archive) => {
                        let files: Vec<&zip::Entry> = archive.entries().iter().filter(|e| !e.is_folder()).collect();
                        let mut out = format!("{name} is a ZIP archive ({}) of {} file{}:", size(f.size), files.len(), if files.len() == 1 { "" } else { "s" });
                        for entry in files.iter().take(MOST_LISTED) {
                            out += &format!("\n{} ({})", entry.name, size(entry.size));
                        }
                        if files.len() > MOST_LISTED {
                            out += &format!("\n...and {} more", files.len() - MOST_LISTED);
                        }
                        out += "\n[What's in it isn't opened here.]";
                        said(out)
                    }
                    Err(_) => said(format!("{name} ({}) isn't a ZIP archive Chat Computer's own system can read: it may be damaged.", size(f.size))),
                }
            }
            Kind::OldOffice => {
                let ext = rel.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                said(if ext.ends_with('x') || ext.ends_with('m') {
                    format!("{name}: a password protects it, so it can't be read here.")
                } else {
                    format!("{name} ({}) is in Office's format from before 2007 (.doc, .xls, .ppt), which Chat Computer's own system can't read inside yet.", size(f.size))
                })
            }
            Kind::Media(what) => said(format!("{name} ({}) is {what}, which Chat Computer's own system can't open yet.", size(f.size))),
            Kind::Other => {
                if from >= f.size && f.size > 0 {
                    return said(format!("{} is {} long: nothing from byte {from}.", shown(letter, &rel), size(f.size)));
                }
                let mut buf = vec![0u8; (f.size - from.min(f.size)).min(READ_AT_ONCE as u64) as usize];
                let n = fs.read(p, &f, from, &mut buf).map_err(fail)?;
                buf.truncate(n);
                let Some((text, used)) = as_text(&buf, from == 0) else {
                    let kind = rel.rsplit('.').next().filter(|k| k.len() <= 5 && *k != rel).unwrap_or("binary");
                    return said(format!(
                        "{name} ({}) isn't text: it's a {kind} file, which Chat Computer's own system can't read inside yet.",
                        size(f.size)
                    ));
                };
                let next = from + used as u64;
                let mut out = format!("{name} ({}, changed {}):\n{text}", size(f.size), date(e.modified));
                if next < f.size {
                    out += &format!("\n[The file goes on: read_file with offset {next} reads the next part.]");
                }
                said(out)
            }
        }
    })?
}

/// Whether a PDF's words (pdf::read's) have words on page `k` (from 1).
fn pdf_page_has_words(text: &str, k: u64) -> bool {
    let head = format!("## Page {k}\n");
    text.find(&head).is_some_and(|at| !text[at + head.len()..].starts_with(pdf::NO_WORDS))
}

/// The biggest picture file shown on the screen.
const MOST_SHOWN: u64 = 64_000_000;

/// The memory a picture may take as it's decoded: half the biggest piece of the kernel's heap that's free (a bigger
/// picture is said to be too big, rather than the heap running out).
fn decoding_room() -> u64 {
    crate::heap::largest_free() as u64 / 2
}

/// A picture on Windows' drive for the screen (show_picture): what was shown, and its pixels, made to fit `w` × `h`
/// on the screen's background (`ground`, as 0xRRGGBB) where it's transparent.
pub fn picture(path: &str, w: u32, h: u32, ground: u32) -> Result<(String, picture::Picture), String> {
    let (letter, rel) = inside(path);
    let background = [(ground >> 16) as u8, (ground >> 8) as u8, ground as u8];
    with(letter, |fs, p| {
        let (e, rel, link) = found(fs, p, letter, &rel)?;
        let f = fs.open_file(p, &e).map_err(|err| error_text(err, &shown(letter, &rel)))?;
        let name = format!("{}{}", shown(letter, &rel), by_way_of(&link));
        if f.size > MOST_SHOWN {
            return Err(format!("{name} is {}: too big to show here.", size(f.size)));
        }
        let data = whole(fs, p, &f, MOST_SHOWN).map_err(|err| error_text(err, &shown(letter, &rel)))?;
        match picture::decode_within(&data, w, h, background, decoding_room()) {
            Ok(pic) => {
                let full = picture::size(&data).map(|(w, h)| format!(", {w}×{h} pixels")).unwrap_or_default();
                Ok((format!("Shown on the screen: {name} ({}{full}).", size(f.size)), pic))
            }
            Err(picture::Error::NotAPicture) => Err(match kind_of(&data[..data.len().min(32)], &rel) {
                Kind::Picture("image/webp") => format!("{name} is a WebP picture, which this screen can't show yet (read_file shows it to you)."),
                Kind::Media(what) => format!("{name} is {what}, which Chat Computer's own system can't show yet."),
                _ => format!("{name} isn't a picture this screen shows (JPEG, PNG or GIF)."),
            }),
            Err(picture::Error::Unsupported(what)) => Err(format!("{name} is a {what}, which Chat Computer's own system can't show yet.")),
            Err(picture::Error::TooBig) => Err(format!("{name} has too many pixels to show here.")),
            Err(picture::Error::Damaged) => Err(format!("{name} seems to be damaged: it can't be shown.")),
        }
    })?
}

/// Up to `READ_AT_ONCE` bytes of `words`, from byte `from` (moved back to a character's start), after `header`; and,
/// if there's more, where to go on from.
fn part_of(words: &str, from: u64, header: &str) -> String {
    let mut start = (from as usize).min(words.len());
    while !words.is_char_boundary(start) {
        start -= 1;
    }
    if start >= words.len() && !words.is_empty() {
        return format!("{header}: {} bytes of them, so nothing from {from}.", words.len());
    }
    let mut end = (start + READ_AT_ONCE).min(words.len());
    while !words.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = format!("{header}{}:\n{}", if start > 0 { format!(", from byte {start}") } else { String::new() }, &words[start..end]);
    if end < words.len() {
        out += &format!("\n[The words go on: read_file with offset {end} reads the next part.]");
    }
    out
}

/// Bytes as text, if they are text: UTF-16 with its byte order mark, or UTF-8 (a character cut off at the end left
/// for the next part); how many bytes that was. None for anything else (control characters, NULs among them, or
/// bytes UTF-8 can't be).
fn as_text(b: &[u8], start: bool) -> Option<(String, usize)> {
    if start && b.len() >= 2 && (b[..2] == [0xFF, 0xFE] || b[..2] == [0xFE, 0xFF]) {
        let le = b[0] == 0xFF;
        let units = b[2..].chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) });
        let text: String = char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect();
        return Some((text, 2 + (b.len() - 2) / 2 * 2));
    }
    let bom = if start && b.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
    let b = &b[bom..];
    let (text, used) = match core::str::from_utf8(b) {
        Ok(t) => (t, b.len()),
        Err(e) if e.error_len().is_none() && e.valid_up_to() + 4 > b.len() => {
            (core::str::from_utf8(&b[..e.valid_up_to()]).ok()?, e.valid_up_to())
        }
        Err(_) => return None,
    };
    // (Control characters other than tabs, line breaks and form feeds don't come in text.)
    if text.chars().any(|c| c < ' ' && !matches!(c, '\t' | '\n' | '\r' | '\x0c')) {
        return None;
    }
    Some((String::from(text), bom + used))
}

/// Files and folders whose names have all of `words` in them, from folder `from` down (find_files); hidden folders
/// (AppData and the like) left out unless the search starts in one.
pub fn find(words: &str, from: &str) -> Result<String, String> {
    let (letter, rel) = inside(if from.trim().is_empty() { "Users" } else { from });
    let wanted: Vec<String> = words.split_whitespace().map(|w| w.to_lowercase()).collect();
    if wanted.is_empty() {
        return Err(String::from("Say which words the name has."));
    }
    with(letter, |fs, p| {
        let (top, rel, _) = found(fs, p, letter, &rel)?;
        if !top.folder {
            return Err(error_text(ntfs::Error::NotAFolder, &shown(letter, &rel)));
        }
        let started = time::uptime_ms();
        let mut queue: VecDeque<(Entry, String)> = VecDeque::from([(top, rel.clone())]);
        let mut found: Vec<String> = Vec::new();
        let mut looked = 0usize;
        let mut cut_short = false;
        while let Some((folder, path)) = queue.pop_front() {
            if looked >= MOST_FOLDERS || time::uptime_ms() - started > SEARCH_MS {
                cut_short = true;
                break;
            }
            looked += 1;
            let Ok(entries) = fs.list(p, &folder) else { continue };
            for e in entries.into_iter().filter(|e| !left_out(e)) {
                let full = joined(&path, &e.name);
                let lower = e.name.to_lowercase();
                if wanted.iter().all(|w| lower.contains(w.as_str())) && found.len() < MOST_FOUND {
                    found.push(format!("{}{}", shown(letter, &full), details(fs, p, &e)));
                }
                if e.folder && !e.link && !e.hidden && !own(&full) {
                    queue.push_back((e, full));
                }
            }
            if found.len() >= MOST_FOUND {
                cut_short = !queue.is_empty();
                break;
            }
        }
        let mut out = if found.is_empty() {
            format!("Nothing under {} has \"{}\" in its name", shown(letter, &rel), words.trim())
        } else {
            format!("Under {}, {} found:\n{}", shown(letter, &rel), found.len(), found.join("\n"))
        };
        if cut_short {
            out += &format!(
                "\n[Stopped after looking in {looked} folders: search a folder further down (in) to look further.]"
            );
        } else {
            out += &format!("\n[Looked in {looked} folders: all of them.]");
        }
        Ok(out)
    })?
}

// ------------------------------------------------------------------------------------------------ the model's file

/// A file opened to be read a part at a time (the model's file, which is gigabytes): where it is, and its size.
pub struct Opened {
    letter: char,
    /// The volume it's on (so a drive given the same letter since isn't read instead).
    drive: Drive,
    start: u64,
    file: File,
    pub size: u64,
    /// Its path, as Windows writes it.
    pub path: String,
}

/// Opens a file to read a part at a time (`read_at`). Err: why it can't be, for the person.
pub fn open(path: &str) -> Result<Opened, String> {
    let (letter, rel) = inside(path);
    let (drive, start) = {
        let volumes = VOLUMES.lock();
        match volumes.iter().find(|v| v.letter == letter) {
            Some(v) => (v.drive, v.start),
            None => (Drive::Start, 0), // (`with` says why there's nothing to read)
        }
    };
    with(letter, |fs, p| {
        let (e, rel, _) = found(fs, p, letter, &rel)?;
        let file = fs.open_file(p, &e).map_err(|err| error_text(err, &shown(letter, &rel)))?;
        Ok(Opened { letter, drive, start, size: file.size, file, path: shown(letter, &rel) })
    })?
}

/// `buf.len()` bytes of an opened file from byte `offset` (fewer at its end): how many were read.
pub fn read_at(o: &Opened, offset: u64, buf: &mut [u8]) -> Result<usize, String> {
    let same = VOLUMES.lock().iter().any(|v| v.letter == o.letter && v.drive == o.drive && v.start == o.start);
    if !same {
        return Err(format!("{}: its drive is gone", o.path));
    }
    with(o.letter, |fs, p| fs.read(p, &o.file, offset, buf).map_err(|err| error_text(err, &o.path)))?
}

/// Whether Windows' drive is locked by BitLocker (its recovery key unlocks it): a drive is locked, and none that's
/// read is Windows'.
pub fn windows_locked() -> bool {
    let volumes = VOLUMES.lock();
    !volumes.iter().any(|v| v.windows) && volumes.iter().any(|v| matches!(v.state, State::Locked(_)))
}

/// Where Chat Computer's app keeps the models it gets ready on Windows, under each person's folder.
const APP_MODELS: [&str; 4] = ["AppData", "Local", "ai.chatcomputer.desktop", "models"];

/// The models Chat Computer's app has got ready on this PC (Users\NAME\AppData\Local\ai.chatcomputer.desktop\models\
/// MODEL.ccm, for each person, on Windows' drive and any other with people's folders): their paths and sizes. Where
/// the own system looks for a model when its settings name none. Err: why no drive can be read.
pub fn app_models() -> Result<Vec<(String, u64)>, String> {
    app_files(&[])
}

/// The speech models Chat Computer's app has got ready beside them (the models folder's speech folder: speech
/// recognition's, whisper-small.en.ccm, say): their paths and sizes.
pub fn app_speech_models() -> Result<Vec<(String, u64)>, String> {
    app_files(&["speech"])
}

/// The model files in each person's folder for Chat Computer's app (in `sub`, under its models folder).
fn app_files(sub: &[&str]) -> Result<Vec<(String, u64)>, String> {
    // Windows' drive first.
    let mut letters: Vec<(bool, char)> = VOLUMES
        .lock()
        .iter()
        .filter(|v| matches!(v.state, State::Open { .. }))
        .map(|v| (!v.windows, v.letter))
        .collect();
    if letters.is_empty() {
        // (What there is to say about why: `with` says it.)
        return with(windows_letter(), |_, _| Vec::new());
    }
    letters.sort();
    let mut out = Vec::new();
    for (_, letter) in letters {
        if let Ok(found) = with(letter, |fs, p| models_on(fs, p, letter, sub)) {
            out.extend(found);
        }
    }
    Ok(out)
}

/// The models in each person's folder for Chat Computer's app on one volume (in `sub`, under its models folder).
fn models_on(fs: &Fs, p: &mut Partition, letter: char, sub: &[&str]) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let root = fs.root();
    let Ok(users) = fs.child(p, &root, "Users") else { return out };
    if !users.folder {
        return out;
    }
    let Ok(people) = fs.list(p, &users) else { return out };
    for person in people.iter().filter(|e| e.folder && !e.link) {
        let mut at = person.clone();
        let mut names = Vec::new();
        for part in APP_MODELS.iter().chain(sub) {
            match fs.child(p, &at, part) {
                Ok(e) if e.folder && !e.link => {
                    names.push(e.name.clone());
                    at = e;
                }
                _ => break,
            }
        }
        if names.len() < APP_MODELS.len() + sub.len() {
            continue;
        }
        let Ok(files) = fs.list(p, &at) else { continue };
        for f in files.iter().filter(|f| !f.folder && f.name.to_ascii_lowercase().ends_with(".ccm")) {
            out.push((shown(letter, &format!("{}\\{}\\{}\\{}", users.name, person.name, names.join("\\"), f.name)), f.size));
        }
    }
    out
}
