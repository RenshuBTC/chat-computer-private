//! Windows' drive, read and never changed: the person's files, for Claude (its list_files, read_file and find_files
//! tools), as the main Chat Computer can read them. Windows' drive is found on the start-up drive, where Windows
//! nearly always is: of the partitions its GPT (or an old MBR) lists as basic data, the NTFS one with Windows' own
//! folders ("Windows" and "Users") is C:. The NTFS reader (ntfs/) does the rest. Links (a junction such as "My
//! Documents", a symbolic link) are followed, on C:; OneDrive's files are read when they're on the PC, and said to be
//! online only when they aren't. A drive BitLocker encrypts can't be read yet, and the status says so.

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

struct Windows {
    /// The partition's first block on the start-up drive, and its size in blocks.
    start: u64,
    blocks: u64,
    volume: ntfs::Volume,
}

static WINDOWS: SpinLock<Option<Windows>> = SpinLock::new(None);
/// What was found, for the computer's status.
static SAID: SpinLock<Option<String>> = SpinLock::new(None);

/// The partition's bytes for the NTFS reader, from the drive's blocks (a page at most at a time).
struct Partition<'a> {
    disk: &'a mut dyn Disk,
    start: u64,
    blocks: u64,
}

impl ntfs::Disk for Partition<'_> {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool {
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

/// The basic data partitions on the drive, as (first block, blocks): from its GPT, or its MBR on an older PC.
fn partitions(disk: &mut dyn Disk) -> Vec<(u64, u64)> {
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
                    out.push((lo, hi - lo + 1));
                }
            }
        }
        return out;
    }
    let mut mbr = vec![0u8; bs];
    if disk.read(0, &mut mbr).is_ok() && mbr[510] == 0x55 && mbr[511] == 0xAA {
        for k in 0..4 {
            let e = &mbr[0x1BE + 16 * k..0x1BE + 16 * (k + 1)];
            let (start, count) = (u32::from_le_bytes(e[8..12].try_into().unwrap()), u32::from_le_bytes(e[12..16].try_into().unwrap()));
            if e[4] == MBR_NTFS && count > 0 {
                out.push((start as u64, count as u64));
            }
        }
    }
    out
}

/// Finds Windows' drive on the start-up drive. Says what it found, for the log.
pub fn init() -> String {
    let found = storage::with_start(|disk| {
        let mut why = String::from("no NTFS partition on the start-up drive");
        for (start, blocks) in partitions(disk) {
            let mut p = Partition { disk: &mut *disk, start, blocks };
            match ntfs::open(&mut p) {
                Ok(volume) => {
                    let windows = ["Windows", "Users"].iter().all(|f| volume.find(&mut p, f).is_ok_and(|e| e.folder));
                    let size = blocks * p.disk.block_size() as u64;
                    if windows {
                        return Ok((Windows { start, blocks, volume }, size));
                    }
                    why = String::from("no NTFS partition with Windows on it");
                }
                Err(ntfs::Error::BitLocker) => {
                    return Err(String::from("Windows' drive is encrypted with BitLocker, which Chat Computer's own system can't read yet"));
                }
                Err(_) => {}
            }
        }
        Err(why)
    });
    let said = match found {
        Some(Ok((w, size))) => {
            let said = format!("Windows' drive (C:, NTFS, {}), read only", self::size(size));
            *WINDOWS.lock() = Some(w);
            said
        }
        Some(Err(why)) => format!("none: {why}"),
        None => String::from("none: the start-up drive isn't started"),
    };
    *SAID.lock() = Some(said.clone());
    format!("files: {said}")
}

/// What files can be read, for the computer's status.
pub fn status() -> String {
    format!("Files: {}", SAID.lock().clone().unwrap_or_else(|| String::from("not looked for")))
}

/// Runs `f` with Windows' drive (None if there's no drive to read: the reason, for Claude).
fn with<T>(f: impl FnOnce(&ntfs::Volume, &mut Partition) -> T) -> Result<T, String> {
    let Some(w) = WINDOWS.lock().take() else {
        return Err(format!("Chat Computer's own system can't read this computer's files: {}", status().trim_start_matches("Files: ")));
    };
    let result = storage::with_start(|disk| {
        let mut p = Partition { disk, start: w.start, blocks: w.blocks };
        f(&w.volume, &mut p)
    });
    *WINDOWS.lock() = Some(w);
    result.ok_or_else(|| String::from("the drive is busy"))
}

/// A path as Claude gives it ("C:\Users\Ren", "/Users/Ren", "Users\Ren") as the volume's own, from its root.
fn inside(path: &str) -> Result<String, String> {
    let p = path.trim().trim_matches('"');
    let p = match p.get(..2) {
        Some(d) if d.eq_ignore_ascii_case("c:") => &p[2..],
        Some(d) if d.as_bytes()[1] == b':' => return Err(format!("{path}: only C: (Windows' drive) can be read here")),
        _ => p,
    };
    Ok(p.replace('/', "\\").trim_matches('\\').to_string())
}

/// "C:\" and the path, as Windows writes it.
fn shown(rel: &str) -> String {
    format!("C:\\{rel}")
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
fn found(v: &ntfs::Volume, p: &mut Partition, rel: &str) -> Result<(ntfs::Entry, String, Option<String>), String> {
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
        // A path from C: (or from / on C:, or /mnt/c for a link made under Linux) goes from the root; one from
        // another drive, a network's or Linux's own, can't be followed; others go from the link's folder.
        let rest = if target.get(..2).is_some_and(|d| d.eq_ignore_ascii_case("c:")) {
            way.clear();
            &target[2..]
        } else if target.get(..6).is_some_and(|d| d.eq_ignore_ascii_case("/mnt/c")) {
            way.clear();
            &target[6..]
        } else if target.get(1..2) == Some(":") || target.starts_with("UNC\\") || target.starts_with("Volume{") || target.starts_with('/') {
            return Err(format!("{at}: a link to {target}, which isn't on Windows' drive (C:)"));
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
    let rel = inside(path)?;
    with(|v, p| {
        let (here, rel, link) = found(v, p, &rel)?;
        if !here.folder {
            return Err(error_text(ntfs::Error::NotAFolder, &shown(&rel)));
        }
        let all: Vec<ntfs::Entry> = v.list(p, here.record).map_err(|e| error_text(e, &shown(&rel)))?.into_iter().filter(|e| !left_out(e)).collect();
        let (folders, files) = (all.iter().filter(|e| e.folder).count(), all.iter().filter(|e| !e.folder).count());
        let mut out = format!(
            "{}{}: {folders} folder{}, {files} file{}",
            shown(&rel),
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

/// A file's words (read_file), from byte `from`: text as it is (UTF-8, or UTF-16 as Notepad saves it); a file of
/// another kind is said to be one.
pub fn read(path: &str, from: u64) -> Result<String, String> {
    let rel = inside(path)?;
    with(|v, p| {
        let (e, rel, link) = found(v, p, &rel)?;
        let f = v.open_file(p, e.record).map_err(|err| error_text(err, &shown(&rel)))?;
        if from >= f.size && f.size > 0 {
            return Ok(format!("{} is {} long: nothing from byte {from}.", shown(&rel), size(f.size)));
        }
        let mut buf = vec![0u8; (f.size - from.min(f.size)).min(READ_AT_ONCE as u64) as usize];
        let n = v.read(p, &f, from, &mut buf).map_err(|err| error_text(err, &shown(&rel)))?;
        buf.truncate(n);
        let Some((text, used)) = as_text(&buf, from == 0) else {
            let kind = rel.rsplit('.').next().filter(|k| k.len() <= 5 && *k != rel).unwrap_or("binary");
            return Ok(format!(
                "{}{} ({}) isn't text: it's a {kind} file, which Chat Computer's own system can't read inside yet.",
                shown(&rel),
                by_way_of(&link),
                size(f.size)
            ));
        };
        let next = from + used as u64;
        let mut out = format!("{}{} ({}, changed {}):\n{text}", shown(&rel), by_way_of(&link), size(f.size), date(e.modified));
        if next < f.size {
            out += &format!("\n[The file goes on: read_file with offset {next} reads the next part.]");
        }
        Ok(out)
    })?
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
    let rel = inside(if from.trim().is_empty() { "Users" } else { from })?;
    let wanted: Vec<String> = words.split_whitespace().map(|w| w.to_lowercase()).collect();
    if wanted.is_empty() {
        return Err(String::from("Say which words the name has."));
    }
    with(|v, p| {
        let (top, rel, _) = found(v, p, &rel)?;
        if !top.folder {
            return Err(error_text(ntfs::Error::NotAFolder, &shown(&rel)));
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
                    found.push(format!("{}{}", shown(&full), details(v, p, &e)));
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
            format!("Nothing under {} has \"{}\" in its name", shown(&rel), words.trim())
        } else {
            format!("Under {}, {} found:\n{}", shown(&rel), found.len(), found.join("\n"))
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
