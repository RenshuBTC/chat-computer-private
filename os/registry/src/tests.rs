//! Against a hive hivex wrote, and one written by tests/make.py with data in pieces as Windows keeps it: every key and
//! value, in order, exactly as hivex reads them back (tests/<hive>.txt); MountedDevices' drive letters; keys and
//! values found whatever their case; and damaged hives giving errors, never a crash.

extern crate std;

use super::*;
use std::format;
use std::process::Command;
use std::vec::Vec;

struct File {
    bytes: Vec<u8>,
    reads: usize,
    /// Where it was read (when it's kept).
    log: Option<Vec<(usize, usize)>>,
}

impl Source for File {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        self.reads += 1;
        if let Some(log) = &mut self.log {
            log.push((offset as usize, buf.len()));
        }
        match self.bytes.get(offset as usize..offset as usize + buf.len()) {
            Some(b) => {
                buf.copy_from_slice(b);
                true
            }
            None => false,
        }
    }
}

fn system() -> File {
    hive("system")
}

fn hive(name: &str) -> File {
    let path = format!("{}/tests/{name}.hiv.xz", env!("CARGO_MANIFEST_DIR"));
    let out = Command::new("xz").args(["-dc", &path]).output().expect("xz, to decompress the test hive");
    assert!(out.status.success(), "xz -dc {path}");
    File { bytes: out.stdout, reads: 0, log: None }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn walk(h: &Hive, f: &mut File, key: &Key, path: &str, out: &mut Vec<String>) {
    let subkeys = h.subkeys(f, key).unwrap();
    let values = h.values(f, key).unwrap();
    assert_eq!((key.subkey_count as usize, key.value_count as usize), (subkeys.len(), values.len()), "{path}");
    out.push(format!("K\t{path}\t{}\t{}", subkeys.len(), values.len()));
    for v in &values {
        out.push(format!("V\t{path}\t{}\t{}\t{}", v.name, v.kind, hex(&v.data)));
    }
    for k in &subkeys {
        let p = if path.is_empty() { k.name.clone() } else { format!("{path}\\{}", k.name) };
        walk(h, f, k, &p, out);
    }
}

/// Both hives (hivex's, and the one with data in pieces), key by key and value by value, as hivex reads them.
#[test]
fn reads_what_hivex_reads() {
    for name in ["system", "big"] {
        let mut f = hive(name);
        let h = Hive::open(&mut f).unwrap();
        let root = h.root(&mut f).unwrap();
        assert_eq!(root.name, "ROOT");
        let mut lines = Vec::new();
        walk(&h, &mut f, &root, "", &mut lines);
        let want = std::fs::read_to_string(format!("{}/tests/{name}.txt", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let want: Vec<&str> = want.lines().collect();
        assert_eq!(lines.len(), want.len(), "{name}");
        for (got, want) in lines.iter().zip(&want) {
            assert_eq!(got, want, "{name}");
        }
    }
}

/// The name MountedDevices gives the test hive's USB stick (tests/make.py).
const USB: &str = "_??_USBSTOR#Disk&Ven_SanDisk&Prod_Ultra&Rev_1.00#4C530001234567891234&0#{53f56307-b6bf-11d0-94f2-00a0c91efb8b}";

#[test]
fn finds_keys_values_and_drive_letters() {
    let mut f = system();
    let h = Hive::open(&mut f).unwrap();
    let guid = |s: &str| -> [u8; 16] {
        let p: Vec<&str> = s.split('-').collect();
        let mut g = [0u8; 16];
        g[..4].copy_from_slice(&u32::from_str_radix(p[0], 16).unwrap().to_le_bytes());
        g[4..6].copy_from_slice(&u16::from_str_radix(p[1], 16).unwrap().to_le_bytes());
        g[6..8].copy_from_slice(&u16::from_str_radix(p[2], 16).unwrap().to_le_bytes());
        let rest = format!("{}{}", p[3], p[4]);
        for i in 0..8 {
            g[8 + i] = u8::from_str_radix(&rest[2 * i..2 * i + 2], 16).unwrap();
        }
        g
    };
    f.reads = 0;
    let letters = drive_letters(&h, &mut f).unwrap();
    assert_eq!(
        letters,
        std::vec![
            ('C', Mounted::Gpt(guid("b0e1f3a2-5c4d-4e6f-8a9b-0c1d2e3f4a5b"))),
            ('D', Mounted::Gpt(guid("c1f2a4b3-6d5e-4f70-9bac-1d2e3f4a5b6c"))),
            ('E', Mounted::Mbr { signature: 0x5EED_CAFE, offset: 1 << 20 }),
            ('F', Mounted::Device(std::string::String::from(USB))),
        ]
    );
    // A USB stick's volume, by its drive's serial number and logical unit.
    assert!(letters[3].1.is_usb("4c530001234567891234", 0));
    assert!(!letters[3].1.is_usb("4C530001234567891234", 1) && !letters[3].1.is_usb("4C53000123456789123", 0) && !letters[3].1.is_usb("", 0));
    assert!(!letters[0].1.is_usb("4C530001234567891234", 0));
    // A few cells read, not the whole hive.
    assert!(f.reads < 60, "{} reads", f.reads);
    // Whatever the case.
    let tz = h.key(&mut f, "controlset001\\CONTROL/TimeZoneInformation").unwrap();
    assert_eq!(text(&h.value(&mut f, &tz, "timezonekeyname").unwrap().data), "W. Europe Standard Time");
    assert_eq!(h.value(&mut f, &tz, "Bias").unwrap().data, (-60i32).to_le_bytes());
    let kinds = h.key(&mut f, "ControlSet001\\Control\\Every kind").unwrap();
    assert_eq!(text(&h.value(&mut f, &kinds, "").unwrap().data), "the default value");
    assert_eq!(texts(&h.value(&mut f, &kinds, "MULTI").unwrap().data), ["one", "two", "three"]);
    assert_eq!(text(&h.value(&mut f, &kinds, "ünïcödé ΝΑΜΕ 名前").unwrap().data), "Grüße, κόσμε, 世界");
    assert_eq!(h.value(&mut f, &kinds, "long").unwrap().data.len(), 40_000);
    assert_eq!(h.value(&mut f, &kinds, "tiny").unwrap().data, b"ab");
    assert!(h.key(&mut f, "ControlSet001\\Control\\ελληνικά").is_ok());
    assert!(h.key(&mut f, "ControlSet001\\Control\\mixed case").is_ok());
    assert_eq!(h.key(&mut f, "ControlSet001\\Nothing").err(), Some(Error::NotFound));
    assert_eq!(h.value(&mut f, &tz, "nothing").err(), Some(Error::NotFound));
    let class = h.key(&mut f, "ControlSet001\\Control\\Class\\{4D36E2BB-E325-11CE-BFC1-08002BE10318}").unwrap();
    assert_eq!(class.name, "{4d36e2bb-e325-11ce-bfc1-08002be10318}");
}

#[test]
fn damaged_hives_give_errors() {
    let file = |bytes: Vec<u8>| File { bytes, reads: 0, log: None };
    let mut f = system();
    // Not a hive; cut short; the header's checksum wrong.
    assert_eq!(Hive::open(&mut file(std::vec![0; 8192])).err(), Some(Error::NotAHive));
    assert_eq!(Hive::open(&mut file(f.bytes[..100].to_vec())).err(), Some(Error::Read));
    f.bytes[0x30] ^= 1;
    assert_eq!(Hive::open(&mut f).err(), Some(Error::Damaged));
    f.bytes[0x30] ^= 1;
    // What a look at the hive reads (its drive letters, its first keys and their values).
    let look = |f: &mut File| {
        let Ok(h) = Hive::open(f) else { return };
        let _ = drive_letters(&h, f);
        if let Ok(root) = h.root(f) {
            if let Ok(keys) = h.subkeys(f, &root) {
                for k in keys.iter().take(8) {
                    let _ = h.values(f, k);
                    let _ = h.subkeys(f, k);
                }
            }
        }
        let _ = h.key(f, "ControlSet001\\Control\\Every kind").map(|k| h.values(f, &k));
    };
    f.log = Some(Vec::new());
    look(&mut f);
    let mut places: Vec<usize> = f.log.take().unwrap().iter().filter(|(at, _)| *at >= 4096).flat_map(|&(at, n)| at..at + n.min(256)).collect();
    places.sort();
    places.dedup();
    assert!(places.len() > 1000, "{} places", places.len());
    // Every byte it reads changed in turn, three ways (the header's checksum still right): no crash.
    for &at in &places {
        for x in [0x01u8, 0x80, 0xFF] {
            f.bytes[at] ^= x;
            look(&mut f);
            f.bytes[at] ^= x;
        }
    }
    // Cut short anywhere.
    for len in (4096..f.bytes.len()).step_by(4093) {
        look(&mut file(f.bytes[..len].to_vec()));
    }
}
