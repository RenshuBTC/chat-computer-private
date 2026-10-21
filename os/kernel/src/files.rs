//! Windows' drives, read and never changed: the person's files, for Claude (its list_files, read_file, find_files and
//! show_picture tools), as the main Chat Computer can read them. Every NTFS volume in the partitions the PC's drives'
//! GPTs (or old MBRs) list as basic data is found; the one with Windows' own folders ("Windows" and "Users") is C:,
//! and Windows' registry there (SYSTEM's MountedDevices, through registry/) says which letter each other partition
//! has, by its GUID or its disk's signature and where it starts. A drive Windows hasn't seen gets the next letter no
//! drive has, as Windows would give it. The NTFS reader (ntfs/) does the rest. Links (a junction such as "My
//! Documents", a symbolic link) are followed, within a drive; OneDrive's files are read when they're on the PC, and
//! said to be online only when they aren't.
//!
//! A drive BitLocker encrypts (bitlocker/) is read decrypted: unlocked at once if BitLocker keeps its key in the clear
//! (suspended, or waiting to be switched on, as a new PC's device encryption does until its owner signs in), else
//! once the person types its recovery key in the message box (each locked drive tried). chat.rs hands the key here
//! and never to Claude, and it isn't kept: after a restart it's typed again.
//!
//! read_file reads text as it is; Word, Excel and PowerPoint files (and OpenDocument's) as their words (office/); a
//! ZIP archive as the list of what's in it; and pictures (JPEG, PNG, GIF, WebP) and PDFs whole, for Claude to see
//! (the API takes them as they are: Claude reads a PDF's pages as words and as pictures). A picture bigger than the
//! API shows Claude goes made smaller, as a JPEG.

use crate::storage::{self, Disk};
use crate::sync::SpinLock;
use crate::time;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

/// GPT's "Microsoft basic data" partition type, as stored (its first three fields little-endian).
const BASIC_DATA: [u8; 16] = [0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7];
/// An MBR partition's type for NTFS.
const MBR_NTFS: u8 = 0x07;
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
/// The biggest picture sent to Claude as it is (the API takes 10 MB of base64: 7.5 MB), and its most pixels each way.
const MOST_PICTURE: u64 = 7_400_000;
const MOST_PIXELS: u32 = 8_000;
/// What Claude's API shows Claude of a picture (Opus 4.7 and later): at most 2576 pixels on its long edge, and 4784
/// "visual tokens" of 28 × 28 pixels. A picture bigger than that, or than the API takes, is sent the size Claude sees
/// it, as a JPEG written here (picture::encode_jpeg): a phone's photo of several megabytes becomes a few hundred
/// kilobytes, so more of them fit in what's sent with each question.
const SEEN_EDGE: u32 = 2576;
const SEEN_TOKENS: u32 = 4784;
/// The JPEG quality a picture made smaller is written at.
const QUALITY: u8 = 90;
/// The biggest PDF (the API takes a request of 32 MB, the conversation and the PDF's base64 together), and Office
/// file or ZIP archive, read.
const MOST_PDF: u64 = 20_000_000;
const MOST_OFFICE: u64 = 64_000_000;

/// What read_file gives Claude: words, and perhaps a picture or a PDF to see.
pub struct Read {
    pub text: String,
    pub attachment: Option<Attachment>,
}

pub enum Attachment {
    /// A picture's type ("image/jpeg") and bytes.
    Picture(&'static str, Vec<u8>),
    Pdf(Vec<u8>),
}

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
    if head.starts_with(b"%PDF-") {
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
fn whole(v: &ntfs::Volume, p: &mut Partition, f: &ntfs::File, most: u64) -> Result<Vec<u8>, ntfs::Error> {
    let mut data = vec![0u8; f.size.min(most) as usize];
    let n = v.read(p, f, 0, &mut data)?;
    data.truncate(n);
    Ok(data)
}

/// BitLocker's sector key for a partition it encrypts, once unlocked.
struct Unlocked {
    volume: bitlocker::Volume,
    key: bitlocker::Key,
}

/// A volume on one of the PC's drives: NTFS, perhaps encrypted with BitLocker.
struct Volume {
    /// Which drive it's on (storage::with_drive's number), its partition's first block and size in blocks, the
    /// drive's block size, and the partition as Windows' MountedDevices names it.
    drive: usize,
    start: u64,
    blocks: u64,
    block_size: u32,
    id: registry::Mounted,
    /// Its letter: Windows' (from Windows' registry: `named`), else one given here.
    letter: char,
    named: bool,
    /// Its name (File Explorer's, beside the letter), once it can be read.
    label: String,
    /// Windows' own drive (it has Windows' folders).
    windows: bool,
    state: State,
}

enum State {
    /// Read: its NTFS volume, and BitLocker's key if BitLocker encrypts it (and how it was unlocked).
    Open { ntfs: ntfs::Volume, bitlocker: Option<(Unlocked, &'static str)> },
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
/// A recovery key typed in the message box, for `work` to unlock with.
static RECOVERY_KEY: SpinLock<Option<String>> = SpinLock::new(None);
/// What was found, for the computer's status.
static SAID: SpinLock<Option<String>> = SpinLock::new(None);

/// The partition's bytes for the NTFS reader, from the drive's blocks (a page at most at a time), decrypted if
/// BitLocker encrypts it.
struct Partition<'a> {
    disk: &'a mut dyn Disk,
    start: u64,
    blocks: u64,
    unlocked: Option<&'a Unlocked>,
}

impl ntfs::Disk for Partition<'_> {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        let unlocked = self.unlocked;
        match unlocked {
            Some(u) => u.volume.read(&u.key, &mut Raw(self), offset, buf),
            None => self.raw(offset, buf),
        }
    }
}

/// The partition's bytes as they are on the drive (BitLocker's reader decrypts them).
struct Raw<'p, 'a>(&'p mut Partition<'a>);

impl bitlocker::Disk for Raw<'_, '_> {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        self.0.raw(offset, buf)
    }
}

impl Partition<'_> {
    fn raw(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        let bs = self.disk.block_size() as usize;
        let mut done = 0usize;
        let mut block = vec![0u8; bs];
        while done < buf.len() {
            let pos = offset + done as u64;
            let lba = pos / bs as u64;
            let within = (pos % bs as u64) as usize;
            if lba >= self.blocks {
                return false;
            }
            let left = buf.len() - done;
            if within == 0 && left >= bs {
                // Whole blocks straight in, up to a page.
                let n = (left / bs).min((4096 / bs).max(1)).min((self.blocks - lba) as usize);
                if self.disk.read(self.start + lba, &mut buf[done..done + n * bs]).is_err() {
                    return false;
                }
                done += n * bs;
            } else {
                if self.disk.read(self.start + lba, &mut block).is_err() {
                    return false;
                }
                let n = (bs - within).min(left);
                buf[done..done + n].copy_from_slice(&block[within..within + n]);
                done += n;
            }
        }
        true
    }
}

/// The basic data partitions on the drive, as (first block, blocks, the partition as MountedDevices names it): from
/// its GPT (each partition's own GUID), or its MBR on an older PC (the disk's signature and where it starts).
fn partitions(disk: &mut dyn Disk) -> Vec<(u64, u64, registry::Mounted)> {
    let bs = disk.block_size() as usize;
    let mut out = Vec::new();
    let mut header = vec![0u8; bs];
    if disk.read(1, &mut header).is_ok() && &header[..8] == b"EFI PART" {
        let u32_at = |b: &[u8], i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let u64_at = |b: &[u8], i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        let (first, count, size) = (u64_at(&header, 72), u32_at(&header, 80) as usize, u32_at(&header, 84) as usize);
        if !(128..=1024).contains(&size) || count > 1024 {
            return out;
        }
        let total = (count * size).div_ceil(bs);
        let mut entries = vec![0u8; total * bs];
        for k in 0..total {
            if disk.read(first + k as u64, &mut entries[k * bs..(k + 1) * bs]).is_err() {
                return out;
            }
        }
        for e in entries.chunks_exact(size).take(count) {
            if e[..16] == BASIC_DATA {
                let (lo, hi) = (u64_at(e, 32), u64_at(e, 40));
                if hi >= lo && hi < disk.blocks() {
                    out.push((lo, hi - lo + 1, registry::Mounted::Gpt(e[16..32].try_into().unwrap())));
                }
            }
        }
        return out;
    }
    let mut mbr = vec![0u8; bs];
    if disk.read(0, &mut mbr).is_ok() && mbr[510] == 0x55 && mbr[511] == 0xAA {
        let signature = u32::from_le_bytes(mbr[0x1B8..0x1BC].try_into().unwrap());
        for k in 0..4 {
            let e = &mbr[0x1BE + 16 * k..0x1BE + 16 * (k + 1)];
            let (start, count) = (u32::from_le_bytes(e[8..12].try_into().unwrap()), u32::from_le_bytes(e[12..16].try_into().unwrap()));
            if e[4] == MBR_NTFS && count > 0 {
                out.push((start as u64, count as u64, registry::Mounted::Mbr { signature, offset: start as u64 * bs as u64 }));
            }
        }
    }
    out
}

/// What a partition holds: NTFS (decrypted with `unlocked`, if BitLocker encrypts it), opened, with its name and
/// whether it's Windows' drive; or BitLocker's volume, still locked (or why it can't be read); or nothing to read.
fn open_partition(disk: &mut dyn Disk, start: u64, blocks: u64, unlocked: Option<(Unlocked, &'static str)>) -> Option<(State, String, bool)> {
    let opened = {
        let mut p = Partition { disk: &mut *disk, start, blocks, unlocked: unlocked.as_ref().map(|(u, _)| u) };
        ntfs::open(&mut p).map(|volume| {
            let windows = ["Windows", "Users"].iter().all(|f| volume.find(&mut p, f).is_ok_and(|e| e.folder));
            let label = volume.label(&mut p).unwrap_or_default();
            (volume, label, windows)
        })
    };
    match opened {
        Ok((ntfs, label, windows)) => Some((State::Open { ntfs, bitlocker: unlocked }, label, windows)),
        Err(ntfs::Error::BitLocker) if unlocked.is_none() => {
            let mut p = Partition { disk: &mut *disk, start, blocks, unlocked: None };
            match bitlocker::open(&mut Raw(&mut p)) {
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
            }
        }
        Err(_) => None,
    }
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

/// The drive letters Windows' registry (SYSTEM's MountedDevices, on Windows' drive) gives partitions.
fn windows_letters(volumes: &[Volume]) -> Option<Vec<(char, registry::Mounted)>> {
    let w = volumes.iter().find(|v| v.windows && matches!(v.state, State::Open { .. }))?;
    let State::Open { ntfs, bitlocker } = &w.state else { return None };
    storage::with_drive(w.drive, |disk| {
        let mut p = Partition { disk, start: w.start, blocks: w.blocks, unlocked: bitlocker.as_ref().map(|(u, _)| u) };
        let e = ntfs.find(&mut p, SYSTEM_HIVE).ok()?;
        let f = ntfs.open_file(&mut p, e.record).ok()?;
        let mut file = HiveFile { v: ntfs, p: &mut p, f };
        let hive = registry::Hive::open(&mut file).ok()?;
        registry::drive_letters(&hive, &mut file).ok()
    })?
}

/// Gives each volume its letter: Windows' where its registry has one (`letters`); else Windows' own drive C: (if no
/// drive here has it), and the others the next letters from D: that neither a drive here nor Windows (for another
/// drive, such as a USB stick) has, in order, as Windows gives a drive it hasn't seen.
fn name_volumes(volumes: &mut [Volume], letters: &[(char, registry::Mounted)]) {
    let mut used: Vec<char> = Vec::new();
    for v in volumes.iter_mut() {
        v.named = false;
        if let Some((l, _)) = letters.iter().find(|(_, m)| *m == v.id) {
            v.letter = *l;
            v.named = true;
            used.push(*l);
        }
    }
    let taken: Vec<char> = letters.iter().map(|(l, _)| *l).collect();
    // (With no Windows drive read, a locked one on the start-up drive is most likely Windows'.)
    let windows_found = volumes.iter().any(|v| v.windows);
    for v in volumes.iter_mut().filter(|v| !v.named) {
        let likely_windows = v.windows || (!windows_found && v.drive == 0 && matches!(v.state, State::Locked(_)));
        v.letter = if likely_windows && !used.contains(&'C') {
            'C'
        } else {
            ('D'..='Z').find(|c| !used.contains(c) && !taken.contains(c)).unwrap_or('?')
        };
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

/// What a volume is, for the computer's status: "C: Windows' drive (NTFS, 512.1 GB)".
fn describe(v: &Volume, registry_read: bool) -> String {
    let what = if v.windows {
        String::from("Windows' drive")
    } else if !v.label.is_empty() {
        format!("\u{201C}{}\u{201D}", v.label)
    } else {
        String::from("a drive")
    };
    let size = size(v.blocks * v.block_size as u64);
    let given = if registry_read && !v.named { ", its letter given here, as Windows hasn't named it" } else { "" };
    match &v.state {
        State::Open { bitlocker: Some((_, how)), .. } => format!("{}: {what} (NTFS, {size}, {how}{given})", v.letter),
        State::Open { .. } => format!("{}: {what} (NTFS, {size}{given})", v.letter),
        State::Locked(b) => {
            let ids: Vec<String> = b.recovery_key_ids().iter().map(|id| String::from(&id[..8])).collect();
            format!("{}: encrypted with BitLocker, locked ({size}; its recovery key's ID starts {}{given})", v.letter, ids.join(" or "))
        }
        State::Unreadable(why) => format!("{}: {why} ({size}{given})", v.letter),
    }
}

/// The computer's status's line about files, from the volumes found.
fn said_of(volumes: &[Volume], registry_read: bool) -> String {
    if volumes.is_empty() {
        return String::from("none: no NTFS partition on this PC's drives");
    }
    let mut said = volumes.iter().map(|v| describe(v, registry_read)).collect::<Vec<String>>().join("; ");
    said += ", read only";
    if volumes.iter().any(|v| matches!(v.state, State::Locked(_))) {
        said += ". To read a locked drive, the person types its recovery key in the message box (48 digits, from \
                  https://aka.ms/myrecoverykey or wherever they saved it): Chat Computer's own system unlocks the drive \
                  itself, and the key isn't sent to Claude, or kept";
    }
    said
}

/// Finds the volumes on the PC's drives, and gives them their letters. Says what it found, for the log.
pub fn init() -> String {
    let mut volumes: Vec<Volume> = Vec::new();
    for drive in 0..storage::drives() {
        let _ = storage::with_drive(drive, |disk| {
            let block_size = disk.block_size();
            for (start, blocks, id) in partitions(disk) {
                if let Some((state, label, windows)) = open_partition(&mut *disk, start, blocks, None) {
                    volumes.push(Volume { drive, start, blocks, block_size, id, letter: '?', named: false, label, windows, state });
                }
            }
        });
    }
    let letters = windows_letters(&volumes);
    name_volumes(&mut volumes, letters.as_deref().unwrap_or(&[]));
    let said = said_of(&volumes, letters.is_some());
    *VOLUMES.lock() = volumes;
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
         key itself: it isn't sent to Claude, or kept, so it's typed again after a restart",
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

/// From the main loop: a drive unlocked with a recovery key typed, if one was; what came of it, for the screen.
/// (Stretching the key takes a second or so: the screen waits.)
pub fn work() -> Option<String> {
    let mut typed = RECOVERY_KEY.lock().take()?;
    let said = unlock(&typed);
    wipe(&mut typed);
    Some(said)
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
        let letters = windows_letters(volumes);
        if windows && letters.is_some() {
            name_volumes(volumes, letters.as_deref().unwrap_or(&[]));
        }
        *SAID.lock() = Some(said_of(volumes, letters.is_some()));
        let v = volumes.iter().find(|v| v.drive == drive && v.start == start).unwrap();
        let what = if v.windows { String::from(" (Windows' drive)") } else if v.label.is_empty() { String::new() } else { format!(" (\u{201C}{}\u{201D})", v.label) };
        return format!("{}:{what} is unlocked: Claude can read its files now, until Chat Computer restarts.", v.letter);
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

/// Runs `f` with the volume of drive letter `letter` (Err if there's none to read: the reason, for Claude).
fn with<T>(letter: char, f: impl FnOnce(&ntfs::Volume, &mut Partition) -> T) -> Result<T, String> {
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
            State::Open { ntfs, bitlocker } => storage::with_drive(v.drive, |disk| {
                let mut p = Partition { disk, start: v.start, blocks: v.blocks, unlocked: bitlocker.as_ref().map(|(u, _)| u) };
                f(ntfs, &mut p)
            })
            .ok_or_else(|| String::from("the drive is busy")),
            State::Locked(b) => Err(locked_why(letter, b)),
            State::Unreadable(why) => Err(format!("{letter}: is {why}")),
        },
    };
    *VOLUMES.lock() = volumes;
    result
}

/// A path as Claude gives it ("C:\Users\Ren", "d:/Photos", "/Users/Ren", "Users\Ren"): its drive's letter (Windows'
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
        ntfs::Error::Read => format!("{what}: the drive couldn't be read"),
        _ => format!("{what}: {e:?}"),
    }
}

/// Whether to leave a file or folder out of lists and searches: Windows' own hidden system files (desktop.ini, the
/// recycle bin), as File Explorer does.
fn left_out(e: &ntfs::Entry) -> bool {
    e.attributes & ntfs::HIDDEN != 0 && e.attributes & ntfs::SYSTEM != 0
}

/// The file or folder at `rel` (links on the way followed, to the end), its path with the names as the drive has them,
/// and the first link followed, if one was.
fn found(v: &ntfs::Volume, p: &mut Partition, letter: char, rel: &str) -> Result<(ntfs::Entry, String, Option<String>), String> {
    let shown = |rel: &str| shown(letter, rel);
    let mut parts: VecDeque<String> = rel.split(['\\', '/']).filter(|s| !s.is_empty()).map(String::from).collect();
    let mut way: Vec<ntfs::Entry> = Vec::new();
    let mut links: Vec<String> = Vec::new();
    let path = |way: &[ntfs::Entry]| way.iter().map(|e| e.name.as_str()).collect::<Vec<&str>>().join("\\");
    while let Some(part) = parts.pop_front() {
        if part == "." {
            continue;
        }
        if part == ".." {
            way.pop();
            continue;
        }
        let here = way.last().cloned().unwrap_or_else(ntfs::root);
        if !here.folder {
            return Err(error_text(ntfs::Error::NotAFolder, &shown(&path(&way))));
        }
        let e = v.child(p, here.record, &part).map_err(|err| error_text(err, &shown(&joined(&path(&way), &part))))?;
        if !e.link {
            way.push(e);
            continue;
        }
        let at = shown(&joined(&path(&way), &e.name));
        if links.len() == MOST_LINKS {
            return Err(format!("{at}: a link to a link to... (more than {MOST_LINKS}), which Chat Computer's own system stops following"));
        }
        let target = v.link_target(p, e.record).map_err(|err| error_text(err, &at))?;
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
    Ok((way.pop().unwrap_or_else(ntfs::root), real, links.into_iter().next()))
}

/// After a file's or folder's name in a list: "\\" for a folder, and for a file its size and when it was changed; a
/// link says where it goes, and a file whose bytes aren't here, or can't be read here, says so.
fn details(v: &ntfs::Volume, p: &mut Partition, e: &ntfs::Entry) -> String {
    let slash = if e.folder { "\\" } else { "" };
    if e.link {
        return match v.link_target(p, e.record) {
            Ok(Some(t)) => format!("{slash} (a link to {t})"),
            _ => format!("{slash} (a link)"),
        };
    }
    if e.folder {
        return String::from(slash);
    }
    let mut d = format!("{}, {}", size(e.size), date(e.modified));
    if e.attributes & ntfs::ENCRYPTED != 0 {
        d += ", encrypted by Windows";
    } else if e.tag != 0 {
        match v.open_file(p, e.record) {
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
    with(letter, |v, p| {
        let (here, rel, link) = found(v, p, letter, &rel)?;
        if !here.folder {
            return Err(error_text(ntfs::Error::NotAFolder, &shown(letter, &rel)));
        }
        let all: Vec<ntfs::Entry> = v.list(p, here.record).map_err(|e| error_text(e, &shown(letter, &rel)))?.into_iter().filter(|e| !left_out(e)).collect();
        let (folders, files) = (all.iter().filter(|e| e.folder).count(), all.iter().filter(|e| !e.folder).count());
        let mut out = format!(
            "{}{}: {folders} folder{}, {files} file{}",
            shown(letter, &rel),
            by_way_of(&link),
            if folders == 1 { "" } else { "s" },
            if files == 1 { "" } else { "s" }
        );
        for e in all.iter().take(MOST_LISTED) {
            out += &format!("\n{}{}", e.name, details(v, p, e));
            if e.attributes & ntfs::HIDDEN != 0 {
                out += " (hidden)";
            }
        }
        if all.len() > MOST_LISTED {
            out += &format!("\n...and {} more", all.len() - MOST_LISTED);
        }
        Ok(out)
    })?
}

/// A file (read_file), from byte `from`: text as it is (UTF-8, or UTF-16 as Notepad saves it); an Office file's
/// words (`from` counts in them); a ZIP archive's list; a picture or a PDF whole, for Claude to see. A file of another
/// kind is said to be one.
pub fn read(path: &str, from: u64) -> Result<Read, String> {
    let (letter, rel) = inside(path);
    with(letter, |v, p| {
        let (e, rel, link) = found(v, p, letter, &rel)?;
        let f = v.open_file(p, e.record).map_err(|err| error_text(err, &shown(letter, &rel)))?;
        let said = |text: String| Ok(Read { text, attachment: None });
        let fail = |err: ntfs::Error| error_text(err, &shown(letter, &rel));
        let name = format!("{}{}", shown(letter, &rel), by_way_of(&link));
        let mut head = [0u8; 32];
        let n = v.read(p, &f, 0, &mut head).map_err(fail)?;
        match kind_of(&head[..n], &rel) {
            Kind::Picture(media) => {
                if f.size > MOST_SHOWN {
                    return said(format!("{name} is a picture of {}: too big for Chat Computer's own system to open.", size(f.size)));
                }
                let data = whole(v, p, &f, MOST_SHOWN).map_err(fail)?;
                let pixels = dimensions(media, &data);
                let head = format!(
                    "{name} ({}{}, changed {})",
                    size(f.size),
                    pixels.map(|(w, h)| format!(", {w}×{h} pixels")).unwrap_or_default(),
                    date(e.modified)
                );
                // As it is, if the API takes it and shows it to Claude as it is (a WebP picture, which isn't made
                // smaller here, if the API takes it).
                let takes = f.size <= MOST_PICTURE && pixels.is_some_and(|(w, h)| w <= MOST_PIXELS && h <= MOST_PIXELS);
                let as_it_is = pixels.is_some_and(|(w, h)| seen_edge(w, h) == w.max(h));
                if takes && (as_it_is || media == "image/webp") {
                    return Ok(Read { text: format!("{head}: the picture is below."), attachment: Some(Attachment::Picture(media, data)) });
                }
                if media == "image/webp" {
                    return said(format!("{head} is a WebP picture too big for Claude to see, and Chat Computer's own system can't make WebP pictures smaller yet."));
                }
                let Some((w, h)) = pixels.or_else(|| picture::size(&data)) else {
                    return said(format!("{head}: it seems to be damaged, so it can't be made smaller for Claude to see."));
                };
                let edge = seen_edge(w, h);
                match picture::decode_within(&data, edge, edge, [255, 255, 255], decoding_room()) {
                    Ok(pic) => {
                        let jpeg = picture::encode_jpeg(&pic, QUALITY);
                        let text = format!(
                            "{head}: the picture is below, made smaller ({}×{} pixels, {}) as Claude's API would.",
                            pic.width,
                            pic.height,
                            size(jpeg.len() as u64)
                        );
                        Ok(Read { text, attachment: Some(Attachment::Picture("image/jpeg", jpeg)) })
                    }
                    Err(picture::Error::TooBig) => said(format!("{head}: too many pixels for Chat Computer's own system to make it smaller for Claude.")),
                    Err(picture::Error::Unsupported(what)) => said(format!("{head} is a {what}, which Chat Computer's own system can't make smaller for Claude.")),
                    Err(_) => said(format!("{head}: it seems to be damaged, so it can't be made smaller for Claude to see.")),
                }
            }
            Kind::Pdf => {
                if f.size > MOST_PDF {
                    return said(format!("{name} is a PDF of {}: too big to send to Claude (at most {}).", size(f.size), size(MOST_PDF)));
                }
                let data = whole(v, p, &f, MOST_PDF).map_err(fail)?;
                let text = format!("{name} ({}, changed {}): the PDF is below, its pages' words and pictures.", size(f.size), date(e.modified));
                Ok(Read { text, attachment: Some(Attachment::Pdf(data)) })
            }
            Kind::Office | Kind::Zip if f.size > MOST_OFFICE => {
                said(format!("{name} is {}: too big for Chat Computer's own system to open.", size(f.size)))
            }
            Kind::Office => {
                let data = whole(v, p, &f, MOST_OFFICE).map_err(fail)?;
                match office::read(&data) {
                    Ok((kind, words)) => said(part_of(&words, from, &format!("{name} ({}, {}, changed {}), its words", kind.name(), size(f.size), date(e.modified)))),
                    Err(office::Error::Zip(zip::Error::Encrypted)) => said(format!("{name}: a password protects it, so it can't be read here.")),
                    Err(_) => said(format!("{name} ({}) should be a Word, Excel or PowerPoint file, but Chat Computer's own system can't read it: it may be damaged.", size(f.size))),
                }
            }
            Kind::Zip => {
                let data = whole(v, p, &f, MOST_OFFICE).map_err(fail)?;
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
            Kind::Media(what) => said(format!("{name} ({}) is {what}, which Chat Computer's own system can't show Claude yet.", size(f.size))),
            Kind::Other => {
                if from >= f.size && f.size > 0 {
                    return said(format!("{} is {} long: nothing from byte {from}.", shown(letter, &rel), size(f.size)));
                }
                let mut buf = vec![0u8; (f.size - from.min(f.size)).min(READ_AT_ONCE as u64) as usize];
                let n = v.read(p, &f, from, &mut buf).map_err(fail)?;
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

/// The biggest picture file shown on the screen, or made smaller for Claude.
const MOST_SHOWN: u64 = 64_000_000;

/// The memory a picture may take as it's decoded: half the biggest piece of the kernel's heap that's free (a bigger
/// picture is said to be too big, rather than the heap running out).
fn decoding_room() -> u64 {
    crate::heap::largest_free() as u64 / 2
}

/// The long edge Claude's API shows a `w` × `h` picture to Claude at (its own, if it's no bigger).
fn seen_edge(w: u32, h: u32) -> u32 {
    let mut edge = w.max(h).min(SEEN_EDGE);
    while edge > 28 {
        let (a, b) = picture::fitted(w, h, edge, edge);
        if a.div_ceil(28) * b.div_ceil(28) <= SEEN_TOKENS {
            break;
        }
        edge -= 1;
    }
    edge
}

/// A picture on Windows' drive for the screen (show_picture): what was shown, and its pixels, made to fit `w` × `h`
/// on the screen's background (`ground`, as 0xRRGGBB) where it's transparent.
pub fn picture(path: &str, w: u32, h: u32, ground: u32) -> Result<(String, picture::Picture), String> {
    let (letter, rel) = inside(path);
    let background = [(ground >> 16) as u8, (ground >> 8) as u8, ground as u8];
    with(letter, |v, p| {
        let (e, rel, link) = found(v, p, letter, &rel)?;
        let f = v.open_file(p, e.record).map_err(|err| error_text(err, &shown(letter, &rel)))?;
        let name = format!("{}{}", shown(letter, &rel), by_way_of(&link));
        if f.size > MOST_SHOWN {
            return Err(format!("{name} is {}: too big to show here.", size(f.size)));
        }
        let data = whole(v, p, &f, MOST_SHOWN).map_err(|err| error_text(err, &shown(letter, &rel)))?;
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
    with(letter, |v, p| {
        let (top, rel, _) = found(v, p, letter, &rel)?;
        if !top.folder {
            return Err(error_text(ntfs::Error::NotAFolder, &shown(letter, &rel)));
        }
        let started = time::uptime_ms();
        let mut queue: VecDeque<(u64, String)> = VecDeque::from([(top.record, rel.clone())]);
        let mut found: Vec<String> = Vec::new();
        let mut looked = 0usize;
        let mut cut_short = false;
        while let Some((record, path)) = queue.pop_front() {
            if looked >= MOST_FOLDERS || time::uptime_ms() - started > SEARCH_MS {
                cut_short = true;
                break;
            }
            looked += 1;
            let Ok(entries) = v.list(p, record) else { continue };
            for e in entries.into_iter().filter(|e| !left_out(e)) {
                let full = joined(&path, &e.name);
                let lower = e.name.to_lowercase();
                if wanted.iter().all(|w| lower.contains(w.as_str())) && found.len() < MOST_FOUND {
                    found.push(format!("{}{}", shown(letter, &full), details(v, p, &e)));
                }
                if e.folder && !e.link && e.attributes & ntfs::HIDDEN == 0 {
                    queue.push_back((e.record, full));
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
