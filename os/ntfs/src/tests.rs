//! Against volumes made by Linux's NTFS tools (tests/make.py: mkntfs, filled by ntfs-3g): every folder lists exactly
//! what the tools put there, every file's bytes are exactly theirs (a hash of each is in tests/<volume>.txt), and
//! names are found whatever their case. One volume has 512-byte sectors and 4 KiB clusters, the other 4 KiB sectors
//! (so 4 KiB records) and 64 KiB clusters.

extern crate std;

use super::*;
use std::collections::BTreeMap;
use std::format;
use std::process::Command;
use std::string::ToString;
use std::vec::Vec;

struct Image {
    bytes: Vec<u8>,
    reads: usize,
}

impl Disk for Image {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        assert!(offset % 512 == 0 && buf.len() % 512 == 0, "a read of {} at {offset} isn't whole 512s", buf.len());
        self.reads += 1;
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
    Image { bytes: out.stdout, reads: 0 }
}

/// What tests/make.py listed: each path's kind ("file", "dir", "link", "online", "compacted"), size and FNV-1a hash.
fn listing(name: &str) -> BTreeMap<String, (String, u64, u64)> {
    let text = std::fs::read_to_string(format!("{}/tests/{name}.txt", env!("CARGO_MANIFEST_DIR"))).unwrap();
    text.lines()
        .map(|l| {
            let v: Vec<&str> = l.split('\t').collect();
            (v[0].to_string(), (v[1].to_string(), v[2].parse().unwrap(), u64::from_str_radix(v[3], 16).unwrap()))
        })
        .collect()
}

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01B3))
}

/// Every folder from `folder` down: each path, and what's there.
fn walk(v: &Volume, disk: &mut Image, folder: u64, prefix: &str, out: &mut BTreeMap<String, (String, u64, u64)>) {
    for e in v.list(disk, folder).unwrap() {
        let path = if prefix.is_empty() { e.name.clone() } else { format!("{prefix}/{}", e.name) };
        if e.link {
            out.insert(path, ("link".to_string(), 0, 0));
        } else if e.folder {
            out.insert(path.clone(), ("dir".to_string(), 0, 0));
            walk(v, disk, e.record, &path, out);
        } else {
            let f = match v.open_file(disk, e.record) {
                Ok(f) => f,
                Err(Error::Online(_)) => {
                    out.insert(path, ("online".to_string(), 0, 0));
                    continue;
                }
                Err(Error::Compacted) => {
                    out.insert(path, ("compacted".to_string(), 0, 0));
                    continue;
                }
                Err(err) => panic!("{path}: {err:?}"),
            };
            let mut data = std::vec![0u8; f.size as usize];
            // In uneven pieces, so reads start and end mid-cluster.
            let mut at = 0;
            while at < data.len() {
                let n = (data.len() - at).min(7_777);
                assert_eq!(v.read(disk, &f, at as u64, &mut data[at..at + n]).unwrap(), n, "{path} at {at}");
                at += n;
            }
            assert_eq!(v.read(disk, &f, f.size, &mut [0u8; 16]).unwrap(), 0, "{path}: past its end");
            out.insert(path, ("file".to_string(), f.size, fnv(&data)));
        }
    }
}

fn check(name: &str) {
    let mut disk = image(name);
    let v = open(&mut disk).unwrap();
    let mut found = BTreeMap::new();
    walk(&v, &mut disk, ROOT, "", &mut found);
    let want = listing(name);
    let missing: Vec<&String> = want.keys().filter(|k| !found.contains_key(*k)).collect();
    let extra: Vec<&String> = found.keys().filter(|k| !want.contains_key(*k)).collect();
    assert!(missing.is_empty() && extra.is_empty(), "{name}: missing {missing:?}, not made by the tools {extra:?}");
    for (path, w) in &want {
        assert_eq!(&found[path], w, "{name}: {path}");
    }
    std::println!("{name}: {} entries, {} reads", found.len(), disk.reads);
}

#[test]
fn reads_every_file_and_folder() {
    check("small");
    check("big-sectors");
}

#[test]
fn has_what_windows_drives_have() {
    let mut disk = image("small");
    let v = open(&mut disk).unwrap();
    assert_eq!((v.bytes_per_sector, v.cluster_size), (512, 4096));
    // Its name, as mkntfs was given it.
    assert_eq!(v.label(&mut disk).unwrap(), "small");
    // Found whatever the case, by / or \.
    let a = v.find(&mut disk, "documents\\TAXES/letter FROM iras (march).TXT").unwrap();
    assert!(!a.folder && a.name == "Letter from IRAS (March).txt" && a.size == 6000);
    assert_eq!(v.find(&mut disk, "/").unwrap().record, ROOT);
    assert_eq!(v.find(&mut disk, "Documents/nothing here"), Err(Error::NotFound));
    assert_eq!(v.find(&mut disk, "hello.txt/inside"), Err(Error::NotAFolder));
    assert!(v.find(&mut disk, "ünïcödé — 名前 🙂.TXT").is_ok());
    // A short name isn't listed beside its long one; a hard link is the same file in two folders.
    let programs = v.find(&mut disk, "Program Files").unwrap().record;
    let names: Vec<String> = v.list(&mut disk, programs).unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["Some Program.exe"]);
    let hello = v.find(&mut disk, "hello.txt").unwrap().record;
    assert_eq!(v.find(&mut disk, "Documents/hello again.txt").unwrap().record, hello);
    // A link isn't followed, but says where it goes: a symbolic link's, relative to its folder; a junction's (a
    // folder, hidden, as Windows has "My Documents"), a path from C:. Neither is listed or gone through.
    let link = v.find(&mut disk, "link to hello").unwrap();
    assert!(link.link && link.tag == TAG_SYMLINK && !link.folder);
    assert_eq!(v.open_file(&mut disk, link.record).unwrap_err(), Error::Link);
    assert_eq!(v.link_target(&mut disk, link.record).unwrap().as_deref(), Some("hello.txt"));
    let on_desktop = v.find(&mut disk, "Users/Sam/Desktop/shopping LIST.txt").unwrap();
    assert_eq!(v.link_target(&mut disk, on_desktop.record).unwrap().as_deref(), Some("..\\Documents\\Shopping list.txt"));
    let mine = v.find(&mut disk, "Users/Sam/My Documents").unwrap();
    assert!(mine.link && mine.folder && mine.tag == TAG_JUNCTION && mine.attributes & HIDDEN != 0);
    assert_eq!(v.link_target(&mut disk, mine.record).unwrap().as_deref(), Some("C:\\Users\\Sam\\Documents"));
    assert_eq!(v.list(&mut disk, mine.record).unwrap_err(), Error::Link);
    assert_eq!(v.find(&mut disk, "Users/Sam/My Documents/Old note.txt").unwrap_err(), Error::Link);
    assert_eq!(v.link_target(&mut disk, hello).unwrap(), None);
    // OneDrive's folders are folders (each a placeholder), its files read when their bytes are here; not when
    // they're online only (none of its clusters here, or some not, or its attributes say so). A file Windows
    // compressed itself isn't read yet.
    let onedrive = v.find(&mut disk, "Users/Sam/OneDrive/Documents").unwrap();
    assert!(onedrive.folder && !onedrive.link && is_cloud(onedrive.tag) && onedrive.tag == 0x9000_601A);
    let budget = v.find(&mut disk, "Users/Sam/OneDrive/Documents/Budget.txt").unwrap();
    assert!(is_cloud(budget.tag) && !budget.link);
    let f = v.open_file(&mut disk, budget.record).unwrap();
    let mut b = [0u8; 64];
    let n = v.read(&mut disk, &f, 0, &mut b).unwrap();
    assert_eq!(&b[..n], b"Rent 1200\nFood 400\n");
    for name in ["Plan.docx", "Half.bin", "Notes.txt"] {
        let e = v.find(&mut disk, &format!("Users/Sam/OneDrive/Documents/{name}")).unwrap();
        assert_eq!(v.open_file(&mut disk, e.record).unwrap_err(), Error::Online(0x9000_601A), "{name}");
    }
    let plan = v.find(&mut disk, "Users/Sam/OneDrive/Documents/Plan.docx").unwrap();
    assert_eq!(plan.size, 50_000);
    let notepad = v.find(&mut disk, "Windows/System32/notepad.exe").unwrap();
    assert_eq!((notepad.tag, v.open_file(&mut disk, notepad.record).unwrap_err()), (TAG_WOF, Error::Compacted));
    let documents = v.find(&mut disk, "Documents").unwrap().record;
    assert_eq!(v.open_file(&mut disk, documents).unwrap_err(), Error::NotAFile);
    assert_eq!(v.list(&mut disk, hello).unwrap_err(), Error::NotAFolder);
    // The 600 files in their index's order (Windows': by name), and a time that makes sense.
    let many = v.find(&mut disk, "Many").unwrap().record;
    let many = v.list(&mut disk, many).unwrap();
    assert_eq!(many.len(), 600);
    assert!(many.windows(2).all(|w| w[0].name < w[1].name));
    assert!(many[0].modified > 1_700_000_000);
    // The file in hundreds of pieces (in runs of a few clusters each) needed an attribute list: its runs fill more
    // than one record.
    let frag = v.find(&mut disk, "Fragmented.bin").unwrap();
    let base = attributes_of(&v.record(&mut disk, frag.record).unwrap()).unwrap();
    assert!(base.iter().any(|a| a.kind == ATTRIBUTE_LIST));
    let f = v.open_file(&mut disk, frag.record).unwrap();
    let Value::Clusters { runs, .. } = &f.data else { panic!("resident") };
    assert!(runs.len() > 150, "{} runs", runs.len());
    // Compressed, sparse and named streams are as they should be.
    let words = v.find(&mut disk, "Compressed/words.txt").unwrap().record;
    assert!(v.open_file(&mut disk, words).unwrap().compressed);
    let sparse = v.find(&mut disk, "sparse.bin").unwrap().record;
    let sparse = v.open_file(&mut disk, sparse).unwrap();
    let Value::Clusters { runs, .. } = &sparse.data else { panic!("resident") };
    assert!(runs.iter().any(|r| r.lcn.is_none()));
}

#[test]
fn refuses_what_isnt_ntfs_or_is_damaged() {
    let mut disk = image("small");
    let good = disk.bytes.clone();
    // BitLocker's signature; another file system; a bad cluster size.
    disk.bytes[3..11].copy_from_slice(b"-FVE-FS-");
    assert_eq!(open(&mut disk).err(), Some(Error::BitLocker));
    disk.bytes[3..11].copy_from_slice(b"MSDOS5.0");
    assert_eq!(open(&mut disk).err(), Some(Error::NotNtfs));
    disk.bytes = good.clone();
    disk.bytes[0x0D] = 3;
    assert_eq!(open(&mut disk).err(), Some(Error::Damaged));
    // A record whose update sequence doesn't match (half written): an error, not a wrong file.
    disk.bytes = good.clone();
    let v = open(&mut disk).unwrap();
    let rec = v.find(&mut disk, "hello.txt").unwrap().record;
    let at = (rec as usize) * 1024;
    let mft = disk.bytes.windows(4).position(|w| w == b"FILE").unwrap();
    disk.bytes[mft + at + 510] ^= 0xFF;
    assert_eq!(v.open_file(&mut disk, rec).unwrap_err(), Error::Damaged);
    // Every byte of the boot sector and the first records changed in turn: never a crash.
    for i in (0..4096).step_by(7) {
        let mut d = Image { bytes: good.clone(), reads: 0 };
        let spot = if i < 512 { i } else { mft + i };
        d.bytes[spot] = d.bytes[spot].wrapping_add(0x55);
        if let Ok(v) = open(&mut d) {
            let _ = v.list(&mut d, ROOT);
        }
    }
}

#[test]
fn a_reparse_point_shorter_than_its_header_is_damaged_not_a_crash() {
    // Its header is a tag (4 bytes) and the length of what follows (2): a 6- or 7-byte value once indexed past its
    // end. Every value too short to slice is an error.
    for len in 0..8 {
        assert_eq!(reparse_value(&std::vec![0u8; len]).unwrap_err(), Error::Damaged, "{len} bytes");
    }
    // A well-formed one: the tag, its data's length, then the data (of which only what's there is kept).
    let mut v = std::vec![0u8; 13];
    v[0..4].copy_from_slice(&0xA000_000Cu32.to_le_bytes());
    v[4..6].copy_from_slice(&3u16.to_le_bytes());
    v[8..11].copy_from_slice(b"abc");
    assert_eq!(reparse_value(&v).unwrap(), (0xA000_000C, b"abc".to_vec()));
    // A length that reaches past the value is cut to it.
    v[4..6].copy_from_slice(&999u16.to_le_bytes());
    assert_eq!(reparse_value(&v).unwrap().1.len(), 5);
}

#[test]
fn an_index_that_reaches_a_node_twice_is_damaged() {
    // A big folder's index is a tree: each entry with a sub-node leads to a node of its own. Two that lead to the same
    // node would list its files twice, and an index made so at every level multiplies the reads, and the entries kept,
    // level by level. "Many" (600 files) has nodes with sub-nodes: in the first, the second such entry is made to lead
    // where the first does.
    let mut disk = image("small");
    let v = open(&mut disk).unwrap();
    let many = v.find(&mut disk, "Many").unwrap().record;
    assert_eq!(v.list(&mut disk, many).unwrap().len(), 600);
    let base = v.record(&mut disk, many).unwrap();
    let own = attributes_of(&base).unwrap();
    let alloc = v.attributes_in(&mut disk, many, &own, INDEX_ALLOCATION).unwrap().into_iter().find(|a| a.name == I30).unwrap();
    let Value::Clusters { runs, .. } = &alloc.value else { panic!("a resident index allocation") };
    let cs = v.cluster_size as u64;
    let blocks: Vec<u64> = runs
        .iter()
        .filter_map(|r| r.lcn.map(|lcn| (lcn, r.len)))
        .flat_map(|(lcn, len)| (0..len * cs / 4096).map(move |i| lcn * cs + i * 4096))
        .collect();
    let mut patched = false;
    'blocks: for at in blocks {
        let mut b = disk.bytes[at as usize..at as usize + 4096].to_vec();
        if fix_up(&mut b, b"INDX").is_err() {
            continue;
        }
        let (first, end) = (0x18 + u32_at(&b, 0x18).unwrap() as usize, 0x18 + u32_at(&b, 0x1C).unwrap() as usize);
        let mut leads = Vec::new(); // where each sub-node's VCN is
        let mut e = first;
        while e + 16 <= end {
            let len = u16_at(&b, e + 8).unwrap() as usize;
            if u16_at(&b, e + 12).unwrap() & SUBNODE != 0 {
                leads.push(e + len - 8);
            }
            if len == 0 {
                break;
            }
            e += len;
        }
        // (Not over a sector's last two bytes, which on the disk hold the update sequence number, not the VCN's.)
        for pair in leads.windows(2) {
            if (pair[1]..pair[1] + 8).any(|i| i % 512 >= 510) {
                continue;
            }
            let vcn = b[pair[0]..pair[0] + 8].to_vec();
            disk.bytes[at as usize + pair[1]..at as usize + pair[1] + 8].copy_from_slice(&vcn);
            patched = true;
            break 'blocks;
        }
    }
    assert!(patched, "no index node with two sub-nodes in Many");
    assert_eq!(v.list(&mut disk, many).unwrap_err(), Error::Damaged);
}

#[test]
fn decompresses_lznt1() {
    // A chunk with a literal run, then copies near and far (as Windows writes them).
    let plain: Vec<u8> = b"abcabcabcabcabcabc hello hello hello".to_vec();
    let mut packed = Vec::new();
    // Built by hand: 3 literals, a copy of 15 from 3 back, then literals and a copy.
    let mut body = Vec::new();
    body.push(0b0000_1000u8);
    body.extend_from_slice(b"abc");
    // pos = 3: 12 length bits; back 3 → (3-1) << 12; length 15 → 15-3.
    body.extend_from_slice(&(((3 - 1) << 12 | (15 - 3)) as u16).to_le_bytes());
    body.extend_from_slice(b" hel");
    body.push(0b0000_0100u8);
    body.extend_from_slice(b"lo");
    // pos = 24: back 6, length 12; bits for length at pos 24: 12 − 1 (24-1 = 23 ≥ 16 once) = 11.
    body.extend_from_slice(&(((6 - 1) << 11 | (12 - 3)) as u16).to_le_bytes());
    let header = 0xB000u16 | (body.len() as u16 - 1);
    packed.extend_from_slice(&header.to_le_bytes());
    packed.extend_from_slice(&body);
    let mut out = std::vec![0u8; plain.len()];
    lznt1(&packed, &mut out).unwrap();
    assert_eq!(out, plain);
    // A back-reference before anything was written is refused.
    assert!(lznt1(&[0x02, 0xB0, 0x01, 0x00, 0x00], &mut [0u8; 8]).is_none());
}
