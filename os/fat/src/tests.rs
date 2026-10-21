//! Checked against volumes made by mkfs.vfat and written by mtools (Linux's own FAT tools): every file's sectors, as
//! this crate finds them, must hold exactly the file's bytes; folders list exactly what the tools put in them, with
//! the names, sizes, attributes and times they gave, as a USB stick's are; and damaged volumes give errors, never a
//! crash. (Skipped, with a note, where those tools aren't installed.)

extern crate std;

use super::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::string::ToString;
use std::boxed::Box;
use std::vec::Vec;

struct Image {
    bytes: Vec<u8>,
    sector: usize,
    reads: usize,
}

impl Sectors for Image {
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> bool {
        self.reads += 1;
        let at = sector as usize * self.sector;
        match self.bytes.get(at..at + buf.len()) {
            Some(b) => {
                buf.copy_from_slice(b);
                true
            }
            None => false,
        }
    }
}

fn tools() -> bool {
    let ok = Command::new("mkfs.vfat").arg("--help").output().is_ok() && Command::new("mcopy").arg("-V").output().is_ok();
    if !ok {
        std::println!("mkfs.vfat or mtools aren't installed: skipped");
    }
    ok
}

fn run(cmd: &mut Command) {
    let out = cmd.output().expect("running a FAT tool");
    assert!(out.status.success(), "{:?}: {}", cmd, std::string::String::from_utf8_lossy(&out.stderr));
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(std::format!("chatcomputer-fat-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Bytes that differ everywhere (so a wrong sector shows).
fn pattern(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2654435761) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

/// A volume of `mib` MiB, FAT `bits`, with the files written in the order given. First `holes` small files are
/// written and every other one deleted, so the files after are split around the gaps they leave.
fn make(dir: &Path, mib: u32, bits: u32, extra: &[&str], files: &[(&str, Vec<u8>)], holes: usize) -> Vec<u8> {
    let image = dir.join("fat.img");
    let mut mkfs = Command::new("mkfs.vfat");
    mkfs.args(["-C", "-F", &bits.to_string()]).args(extra).arg(&image).arg((mib * 1024).to_string());
    run(&mut mkfs);
    let img = image.to_str().unwrap();
    for i in 0..holes {
        let local = dir.join(std::format!("hole{i}"));
        fs::write(&local, pattern(3000, 1000 + i as u32)).unwrap();
        run(Command::new("mcopy").args(["-i", img]).arg(&local).arg(std::format!("::/HOLE{i}.BIN")));
    }
    for i in (0..holes).step_by(2) {
        run(Command::new("mdel").args(["-i", img, &std::format!("::/HOLE{i}.BIN")]));
    }
    for (path, bytes) in files {
        let parts: Vec<&str> = path.split('/').collect();
        for i in 1..parts.len() {
            let folder = std::format!("::/{}", parts[..i].join("/"));
            let _ = Command::new("mmd").args(["-i", img, &folder]).output();
        }
        let local = dir.join("file");
        fs::write(&local, bytes).unwrap();
        run(Command::new("mcopy").args(["-o", "-i", img]).arg(&local).arg(std::format!("::/{path}")));
    }
    fs::read(&image).unwrap()
}

fn read_back(v: &Volume, img: &mut Image, sectors: &[u64], size: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = std::vec![0u8; v.bytes_per_sector as usize];
    for &s in sectors {
        assert!(img.read(s, &mut buf));
        out.extend_from_slice(&buf);
    }
    out.truncate(size);
    out
}

fn check(bytes: Vec<u8>, kind: Kind, files: &[(&str, Vec<u8>)]) {
    let mut img = Image { bytes, sector: 512, reads: 0 };
    let v = open(&mut img).unwrap();
    assert_eq!(v.kind, kind);
    for (path, content) in files {
        let parts: Vec<&str> = path.split('/').collect();
        let f = v.find(&mut img, &parts).unwrap_or_else(|e| panic!("{path}: {e:?}"));
        assert!(!f.folder);
        assert_eq!(f.size as usize, content.len(), "{path}");
        let sectors = v.file_sectors(&mut img, &f).unwrap();
        assert_eq!(read_back(&v, &mut img, &sectors, content.len()), *content, "{path}");
        // Any case, and the folders' 8.3 names, find it too.
        let upper: Vec<std::string::String> = parts.iter().map(|p| p.to_uppercase()).collect();
        let upper: Vec<&str> = upper.iter().map(|s| s.as_str()).collect();
        assert_eq!(v.find(&mut img, &upper), Ok(f));
    }
    assert_eq!(v.find(&mut img, &["EFI", "nothing-here.bin"]), Err(Error::NotFound));
}

fn files() -> Vec<(&'static str, Vec<u8>)> {
    std::vec![
        ("EFI/BOOT/BOOTX64.EFI", pattern(70_000, 1)),
        ("EFI/chatcomputer-own/settings.txt", b"claude-api-key = test\r\n".to_vec()),
        ("EFI/chatcomputer-own/memory.bin", pattern(300_000, 2)),
        ("EFI/Microsoft/Boot/a file with a long name.dat", pattern(5_000, 3)),
    ]
}

#[test]
fn fat32_files_are_found_where_they_are_even_split_up() {
    if !tools() {
        return;
    }
    let dir = scratch("32");
    let f = files();
    // 1 KiB clusters, as Windows makes a 100 MB start-up partition.
    let bytes = make(&dir, 80, 32, &["-s", "2"], &f, 12);
    let mut img = Image { bytes: bytes.clone(), sector: 512, reads: 0 };
    let v = open(&mut img).unwrap();
    let memory = v.find(&mut img, &["EFI", "chatcomputer-own", "memory.bin"]).unwrap();
    let chain = v.chain(&mut img, memory.first_cluster).unwrap();
    let split = chain.windows(2).filter(|w| w[1] != w[0] + 1).count();
    std::println!("memory.bin: {} clusters in {} pieces", chain.len(), split + 1);
    check(bytes, Kind::Fat32, &f);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn fat16_and_fat12_too() {
    if !tools() {
        return;
    }
    let dir = scratch("16");
    let f = files();
    check(make(&dir, 16, 16, &[], &f, 6), Kind::Fat16, &f);
    let dir12 = scratch("12");
    check(make(&dir12, 2, 12, &[], &f[..2], 4), Kind::Fat12, &f[..2]);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(dir12);
}

#[test]
fn a_chain_that_loops_is_damage_not_a_hang() {
    if !tools() {
        return;
    }
    let dir = scratch("loop");
    let f = std::vec![("BIG.BIN", pattern(20_000, 9))];
    let mut bytes = make(&dir, 40, 32, &["-s", "1"], &f, 0);
    let mut img = Image { bytes: bytes.clone(), sector: 512, reads: 0 };
    let v = open(&mut img).unwrap();
    let e = v.find(&mut img, &["BIG.BIN"]).unwrap();
    let chain = v.chain(&mut img, e.first_cluster).unwrap();
    assert_eq!(chain.len(), 20_000usize.div_ceil(512));
    // The last cluster made to point back to the first, in the first FAT.
    let last = *chain.last().unwrap() as usize;
    let at = v.fat_start as usize * 512 + last * 4;
    bytes[at..at + 4].copy_from_slice(&e.first_cluster.to_le_bytes());
    let mut img = Image { bytes, sector: 512, reads: 0 };
    assert_eq!(v.chain(&mut img, e.first_cluster), Err(Error::Damaged));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn not_a_fat_volume() {
    let mut img = Image { bytes: std::vec![0u8; 1 << 20], sector: 512, reads: 0 };
    assert_eq!(open(&mut img).err(), Some(Error::NotFat));
    let mut short = Image { bytes: std::vec![0u8; 100], sector: 512, reads: 0 };
    assert_eq!(open(&mut short).err(), Some(Error::Read));
}

#[test]
fn a_file_in_pieces_is_followed_piece_by_piece() {
    if !tools() {
        return;
    }
    // A full volume with every other file deleted: a new file can only go in the gaps.
    let dir = scratch("pieces");
    let image = dir.join("fat.img");
    run(Command::new("mkfs.vfat").args(["-C", "-F", "32", "-s", "1"]).arg(&image).arg((34 * 1024).to_string()));
    let img = image.to_str().unwrap();
    let local = dir.join("hole");
    fs::write(&local, pattern(1 << 20, 5)).unwrap();
    let mut written = 0;
    while Command::new("mcopy").args(["-i", img]).arg(&local).arg(std::format!("::/H{written}.BIN")).output().unwrap().status.success() {
        written += 1;
    }
    for i in (0..written).step_by(2) {
        run(Command::new("mdel").args(["-i", img, &std::format!("::/H{i}.BIN")]));
    }
    let content = pattern(3 << 20, 6);
    fs::write(&local, &content).unwrap();
    run(Command::new("mcopy").args(["-i", img]).arg(&local).arg("::/memory.bin"));
    let mut disk = Image { bytes: fs::read(&image).unwrap(), sector: 512, reads: 0 };
    let v = open(&mut disk).unwrap();
    let f = v.find(&mut disk, &["MEMORY.BIN"]).unwrap();
    let chain = v.chain(&mut disk, f.first_cluster).unwrap();
    let pieces = chain.windows(2).filter(|w| w[1] != w[0] + 1).count() + 1;
    assert!(pieces >= 3, "the file should be in pieces, not {pieces}");
    let sectors = v.file_sectors(&mut disk, &f).unwrap();
    assert_eq!(read_back(&v, &mut disk, &sectors, content.len()), content);
    let _ = fs::remove_dir_all(dir);
}

/// mtools, as a person's PC would write the names: in UTF-8, and times in UTC (FAT keeps local time).
fn mtool(name: &str) -> Command {
    let mut c = Command::new(name);
    c.env("TZ", "UTC").env("LANG", "C.UTF-8").env("LC_ALL", "C.UTF-8");
    c
}

/// A file to put on a stick: its path, bytes, and when it was changed (seconds since 1970, even: FAT keeps two).
struct Put {
    path: &'static str,
    bytes: Vec<u8>,
    when: u64,
}

/// A volume of `mib` MiB, FAT `bits` with `cluster` sectors a cluster, named `label`, as a USB stick's: the files
/// written with their times, then `deleted` deleted and the attributes in `attributes` set ("+h", "+r", "+s").
fn stick(dir: &Path, mib: u32, bits: u32, cluster: u32, label: &str, puts: &[Put], deleted: &[&str], attributes: &[(&str, &str)]) -> Vec<u8> {
    let image = dir.join("stick.img");
    run(mtool("mkfs.vfat").args(["-C", "-F", &bits.to_string(), "-s", &cluster.to_string(), "-n", label]).arg(&image).arg((mib * 1024).to_string()));
    let img = image.to_str().unwrap();
    for p in puts {
        let parts: Vec<&str> = p.path.split('/').collect();
        for i in 1..parts.len() {
            let _ = mtool("mmd").args(["-i", img, &std::format!("::/{}", parts[..i].join("/"))]).output();
        }
        let local = dir.join("file");
        fs::write(&local, &p.bytes).unwrap();
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(p.when);
        fs::File::options().write(true).open(&local).unwrap().set_modified(when).unwrap();
        run(mtool("mcopy").args(["-m", "-o", "-i", img]).arg(&local).arg(std::format!("::/{}", p.path)));
    }
    for d in deleted {
        run(mtool("mdel").args(["-i", img, &std::format!("::/{d}")]));
    }
    for (path, attribute) in attributes {
        run(mtool("mattrib").args(["-i", img, attribute, &std::format!("::/{path}")]));
    }
    fs::read(&image).unwrap()
}

fn puts() -> Vec<Put> {
    let mut p = std::vec![
        Put { path: "lower.txt", bytes: b"small letters, kept as Windows keeps them\r\n".to_vec(), when: 1_709_618_828 },
        Put { path: "MiXed Case.TXT", bytes: b"mixed\n".to_vec(), when: 1_700_000_000 },
        // (Not a character past U+FFFF: mtools keeps those wrongly, in one UTF-16 unit.)
        Put { path: "Ünïcödé — 名前.txt", bytes: "Ünïcödé — 名前\n".as_bytes().to_vec(), when: 1_750_000_000 },
        Put { path: "Documents/A name longer than fifteen characters, by far (several entries).txt", bytes: pattern(3000, 11), when: 1_600_000_000 },
        Put { path: "Documents/Gone.txt", bytes: pattern(100, 12), when: 1_600_000_002 },
        Put { path: "Documents/Kept.txt", bytes: pattern(100, 13), when: 1_600_000_004 },
        Put { path: "Photos/2024/Beach.jpg", bytes: pattern(250_000, 14), when: 1_720_000_000 },
        Put { path: "Empty.txt", bytes: Vec::new(), when: 1_710_000_000 },
        Put { path: "Hidden.dat", bytes: pattern(10, 15), when: 1_710_000_002 },
    ];
    for i in 0..600 {
        p.push(Put { path: Box::leak(std::format!("Many/file {i:03}.txt").into_boxed_str()), bytes: std::format!("{i}\n").into_bytes(), when: 1_690_000_000 + 2 * i });
    }
    p
}

/// Every file and folder under `folder`, by path: what it is, its size, and its attributes and time for a file.
fn walk(v: &Volume, img: &mut Image, folder: &Entry, path: &str, out: &mut std::collections::BTreeMap<std::string::String, Item>) {
    for item in v.list(img, folder).unwrap() {
        let p = if path.is_empty() { item.name.clone() } else { std::format!("{path}/{}", item.name) };
        if item.entry.folder {
            walk(v, img, &item.entry, &p, out);
        }
        out.insert(p, item);
    }
}

fn check_stick(bytes: Vec<u8>, kind: Kind, label: &str) {
    let mut img = Image { bytes, sector: 512, reads: 0 };
    let v = open(&mut img).unwrap();
    assert_eq!(v.kind, kind);
    assert_eq!(v.label(&mut img).unwrap(), label);
    let mut found = std::collections::BTreeMap::new();
    walk(&v, &mut img, &root(), "", &mut found);
    let all = puts();
    // Every file put there, and the folders on the way, and nothing else (the deleted file is gone).
    let mut wanted: std::collections::BTreeSet<std::string::String> = std::collections::BTreeSet::new();
    for p in all.iter().filter(|p| p.path != "Documents/Gone.txt") {
        let parts: Vec<&str> = p.path.split('/').collect();
        for i in 1..=parts.len() {
            wanted.insert(parts[..i].join("/"));
        }
    }
    let missing: Vec<&std::string::String> = wanted.iter().filter(|p| !found.contains_key(*p)).collect();
    let extra: Vec<&std::string::String> = found.keys().filter(|p| !wanted.contains(*p)).collect();
    assert!(missing.is_empty() && extra.is_empty(), "missing {missing:?}, not put there {extra:?}");
    for p in all.iter().filter(|p| p.path != "Documents/Gone.txt") {
        let item = &found[p.path];
        assert!(!item.entry.folder && item.entry.size as usize == p.bytes.len(), "{}", p.path);
        assert_eq!(item.modified, p.when as i64, "{}", p.path);
        let mut data = std::vec![0u8; p.bytes.len()];
        assert_eq!(v.read(&mut img, &item.entry, 0, &mut data).unwrap(), data.len(), "{}", p.path);
        assert!(data == p.bytes, "{}", p.path);
    }
    assert_eq!(found["lower.txt"].short, "lower.txt", "Windows' small-letter flags");
    assert_eq!(found["MiXed Case.TXT"].attributes & (HIDDEN | READ_ONLY | SYSTEM), 0);
    assert_eq!(found["Hidden.dat"].attributes & (HIDDEN | SYSTEM), HIDDEN | SYSTEM);
    assert_eq!(found["Documents/Kept.txt"].attributes & READ_ONLY, READ_ONLY);
    assert!(found["Photos"].entry.folder && found["Photos"].attributes & FOLDER != 0);
    // Found by name whatever its case, or by its 8.3 name.
    let docs = v.child(&mut img, &root(), "DOCUMENTS").unwrap();
    let long = v.child(&mut img, &docs.entry, "a name longer than fifteen characters, BY FAR (several entries).TXT").unwrap();
    assert_eq!(v.child(&mut img, &docs.entry, &long.short).unwrap(), long);
    assert!(v.child(&mut img, &root(), "ÜNÏCÖDÉ — 名前.TXT").is_ok());
    assert_eq!(v.child(&mut img, &docs.entry, "Gone.txt"), Err(Error::NotFound));
    assert_eq!(v.list(&mut img, &long.entry), Err(Error::NotAFolder));
    assert_eq!(v.read(&mut img, &docs.entry, 0, &mut [0u8; 4]), Err(Error::NotAFile));
    // Parts of a file: across clusters, at its end, and past it.
    let beach = found["Photos/2024/Beach.jpg"].entry;
    let all_of = &all.iter().find(|p| p.path == "Photos/2024/Beach.jpg").unwrap().bytes;
    for (from, len) in [(0usize, 1usize), (1000, 10_000), (4095, 2), (65_535, 70_000), (249_990, 10)] {
        let mut part = std::vec![0u8; len];
        assert_eq!(v.read(&mut img, &beach, from as u64, &mut part).unwrap(), len);
        assert!(part == all_of[from..from + len], "from {from}, {len} bytes");
    }
    let mut part = std::vec![0u8; 100];
    assert_eq!(v.read(&mut img, &beach, 249_950, &mut part).unwrap(), 50);
    assert_eq!(v.read(&mut img, &beach, 250_000, &mut part).unwrap(), 0);
    assert_eq!(v.list(&mut img, &found["Many"].entry).unwrap().len(), 600);
}

#[test]
fn folders_list_as_windows_shows_them() {
    if !tools() {
        return;
    }
    for (bits, mib, cluster, kind) in [(32, 64, 1, Kind::Fat32), (16, 32, 4, Kind::Fat16), (12, 8, 8, Kind::Fat12)] {
        let dir = scratch(&std::format!("stick{bits}"));
        let attributes = [("Hidden.dat", "+h"), ("Hidden.dat", "+s"), ("Documents/Kept.txt", "+r")];
        let bytes = stick(&dir, mib, bits, cluster, "MY STICK", &puts(), &["Documents/Gone.txt"], &attributes);
        check_stick(bytes, kind, "MY STICK");
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn damaged_volumes_give_errors() {
    if !tools() {
        return;
    }
    let dir = scratch("damaged");
    let few: Vec<Put> = puts().into_iter().take(9).collect();
    let good = stick(&dir, 34, 32, 1, "DAMAGED", &few, &[], &[]);
    let v = open(&mut Image { bytes: good.clone(), sector: 512, reads: 0 }).unwrap();
    let root_at = v.cluster_sector(v.root_cluster) as usize * 512;
    let fat_at = v.fat_start as usize * 512;
    // Every byte of the first sector, the FAT's start and the root folder's first sector changed in turn: whatever
    // comes back, no crash, and no endless loop.
    let places: Vec<usize> = (0..512).chain(fat_at..fat_at + 512).chain(root_at..root_at + 512).collect();
    let mut img = Image { bytes: good, sector: 512, reads: 0 };
    for &at in &places {
        for x in [0x01u8, 0x80, 0xFF] {
            img.bytes[at] ^= x;
            if let Ok(v) = open(&mut img) {
                let _ = v.label(&mut img);
                if let Ok(items) = v.list(&mut img, &root()) {
                    for item in items.iter().take(12) {
                        let mut buf = std::vec![0u8; 8192];
                        let _ = v.read(&mut img, &item.entry, 0, &mut buf);
                        if item.entry.folder {
                            let _ = v.list(&mut img, &item.entry);
                        }
                    }
                }
            }
            img.bytes[at] ^= x;
        }
    }
    let _ = fs::remove_dir_all(dir);
}
