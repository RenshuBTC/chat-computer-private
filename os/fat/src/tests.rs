//! Checked against volumes made by mkfs.vfat and written by mtools (Linux's own FAT tools): every file's sectors, as
//! this crate finds them, must hold exactly the file's bytes. (Skipped, with a note, where those tools aren't installed.)

extern crate std;

use super::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::string::ToString;
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
