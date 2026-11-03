//! Against volumes made by Linux's exFAT tools (tests/make.py: mkfs.exfat, filled by exfat-fuse): every folder lists
//! exactly what the tools put there, every file's bytes are exactly theirs (a hash of each is in tests/<volume>.txt),
//! with the same time of change, and names are found whatever their case. One volume has 4 KiB clusters, the other 32
//! KiB. Damaged volumes give errors, never a crash.

extern crate std;

use super::*;
use std::collections::BTreeMap;
use std::format;
use std::process::Command;
use std::string::ToString;
use std::vec::Vec;

struct Image {
    bytes: Vec<u8>,
}

impl Disk for Image {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        match self.bytes.get(offset as usize..offset as usize + buf.len()) {
            Some(b) => {
                buf.copy_from_slice(b);
                true
            }
            None => false,
        }
    }
}

fn image(name: &str) -> Image {
    let path = format!("{}/tests/{name}.img.xz", env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("xz").args(["-dc", &path]).output().expect("xz, to decompress the test volume");
    assert!(out.status.success(), "xz -dc {path}");
    Image { bytes: out.stdout }
}

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01B3))
}

/// Every file and folder under `folder`, as the listing has them: path, kind, size, hash, time.
fn walk(v: &Volume, disk: &mut Image, folder: &Entry, path: &str, out: &mut BTreeMap<String, String>) {
    for e in v.list(disk, folder).unwrap() {
        let p = if path.is_empty() { e.name.clone() } else { format!("{path}/{}", e.name) };
        if e.folder {
            out.insert(p.clone(), format!("dir\t0\t0\t{}", e.modified));
            walk(v, disk, &e, &p, out);
        } else {
            let mut data = std::vec![0u8; e.size as usize];
            assert_eq!(v.read(disk, &e, 0, &mut data).unwrap(), data.len(), "{p}");
            out.insert(p, format!("file\t{}\t{:016x}\t{}", e.size, fnv(&data), e.modified));
        }
    }
}

fn check(name: &str, label: &str, cluster: u32) {
    let mut disk = image(name);
    let v = open(&mut disk).unwrap();
    assert_eq!((v.label.as_str(), v.cluster_size), (label, cluster));
    let mut found = BTreeMap::new();
    walk(&v, &mut disk, &v.root(), "", &mut found);
    let listing = std::fs::read_to_string(format!("{}/tests/{name}.txt", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let want: BTreeMap<String, String> = listing
        .lines()
        .map(|l| {
            let (path, rest) = l.split_once('\t').unwrap();
            (path.to_string(), rest.to_string())
        })
        .collect();
    let missing: Vec<&String> = want.keys().filter(|k| !found.contains_key(*k)).collect();
    let extra: Vec<&String> = found.keys().filter(|k| !want.contains_key(*k)).collect();
    assert!(missing.is_empty() && extra.is_empty(), "{name}: missing {missing:?}, not made by the tools {extra:?}");
    for (path, w) in &want {
        assert_eq!(&found[path], w, "{name}: {path}");
    }
}

#[test]
fn reads_every_file_and_folder() {
    check("usb", "USB STICK", 4096);
    check("sd", "SD CARD", 32768);
}

#[test]
fn finds_names_whatever_their_case() {
    let mut disk = image("usb");
    let v = open(&mut disk).unwrap();
    let e = v.find(&mut disk, "documents/SHOPPING LIST.TXT").unwrap();
    assert!(!e.folder && e.name == "Shopping list.txt" && e.size == 40);
    assert!(v.find(&mut disk, "mixed case\\LOWER.txt").is_ok());
    assert!(v.find(&mut disk, "DOCUMENTS/ünïcödé — 名前 🙂.TXT").is_ok());
    assert_eq!(v.find(&mut disk, "Documents/nothing"), Err(Error::NotFound));
    assert_eq!(v.find(&mut disk, "hello.txt/inside"), Err(Error::NotAFolder));
    // Part of a file, from the middle of a cluster across into the next ones; and from past its end.
    let big = v.find(&mut disk, "Big.bin").unwrap();
    let mut all = std::vec![0u8; big.size as usize];
    v.read(&mut disk, &big, 0, &mut all).unwrap();
    let mut part = std::vec![0u8; 10_000];
    assert_eq!(v.read(&mut disk, &big, 4000, &mut part).unwrap(), 10_000);
    assert!(part == all[4000..14_000]);
    assert_eq!(v.read(&mut disk, &big, big.size - 5, &mut part).unwrap(), 5);
    assert_eq!(v.read(&mut disk, &big, big.size + 5, &mut part).unwrap(), 0);
    let chain = v.find(&mut disk, "Chain A.bin").unwrap();
    assert!(!chain.contiguous, "a file written in pieces has its clusters in the FAT");
    assert!(big.contiguous, "a file written at once has its clusters in one run");
    let many = v.find(&mut disk, "Many").unwrap();
    assert_eq!(v.list(&mut disk, &many).unwrap().len(), 600);
}

#[test]
fn a_folder_is_read_to_its_end_whatever_its_length_says() {
    let good = image("usb");
    let v = open(&mut Image { bytes: good.bytes.clone() }).unwrap();
    let root_at = (v.heap + (v.root as u64 - 2) * v.cluster_size as u64) as usize;
    // "Many" with no length in its stream extension (as the root never has one): still its 600 files, to its
    // end-of-folder entry.
    let mut disk = Image { bytes: good.bytes.clone() };
    let entries = &disk.bytes[root_at..root_at + v.cluster_size as usize];
    let name: Vec<u8> = "Many".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let set = (0..entries.len() / 32).find(|&i| entries[i * 32] == NAME && entries[i * 32 + 2..i * 32 + 10] == name[..]).unwrap();
    let stream = root_at + (set - 1) * 32;
    assert_eq!(disk.bytes[stream], STREAM);
    assert_eq!(disk.bytes[stream + 1] & 2, 0, "Many's clusters are a chain in the FAT");
    disk.bytes[stream + 8..stream + 16].fill(0);
    disk.bytes[stream + 24..stream + 32].fill(0);
    let many = v.find(&mut disk, "Many").unwrap();
    assert_eq!(many.size, 0);
    assert_eq!(v.list(&mut disk, &many).unwrap().len(), 600);
    // A volume that says it has four billion clusters, and a root folder whose chain loops on itself: read as far as a
    // folder's entries go (the loop was only caught after four billion steps, and 16 GB of cluster numbers).
    let mut disk = Image { bytes: good.bytes.clone() };
    disk.bytes[92..96].copy_from_slice(&0xFFFF_FFF5u32.to_le_bytes());
    let fat_entry = v.fat as usize + 4 * v.root as usize;
    disk.bytes[fat_entry..fat_entry + 4].copy_from_slice(&v.root.to_le_bytes());
    let started = std::time::Instant::now();
    let looped = open(&mut disk).unwrap();
    assert_eq!(looped.list(&mut disk, &looped.root()).unwrap().len(), v.list(&mut Image { bytes: good.bytes.clone() }, &v.root()).unwrap().len());
    assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
}

#[test]
fn damaged_volumes_give_errors() {
    let good = image("usb");
    assert_eq!(open(&mut Image { bytes: std::vec![0; 4096] }).err(), Some(Error::NotExfat));
    // Every byte of the boot sector, the FAT's start and the root folder's first cluster changed in turn: whatever
    // comes back, no crash, and no endless loop.
    let v = open(&mut Image { bytes: good.bytes.clone() }).unwrap();
    let root_at = (v.heap + (v.root as u64 - 2) * v.cluster_size as u64) as usize;
    let places: Vec<usize> = (0..512).chain(v.fat as usize..v.fat as usize + 512).chain(root_at..root_at + 1024).collect();
    let mut disk = Image { bytes: good.bytes.clone() };
    for &at in &places {
        for x in [0x01u8, 0x80, 0xFF] {
            disk.bytes[at] ^= x;
            if let Ok(v) = open(&mut disk) {
                let root = v.root();
                if let Ok(entries) = v.list(&mut disk, &root) {
                    for e in entries.iter().take(12) {
                        let mut buf = std::vec![0u8; 8192];
                        let _ = v.read(&mut disk, e, 0, &mut buf);
                        if e.folder {
                            let _ = v.list(&mut disk, e);
                        }
                    }
                }
            }
            disk.bytes[at] ^= x;
        }
    }
}
