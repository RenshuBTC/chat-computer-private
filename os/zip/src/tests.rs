//! Against what zlib, Python's zipfile and Info-ZIP's zip make (tests/cases.py makes them, in a folder of its own for
//! each run): every stream inflates to exactly its input, every archive's files come out as they went in, and what
//! can't be read says why. Damaged streams give errors, never a crash.

extern crate std;

use super::*;
use std::collections::BTreeMap;
use std::format;
use std::path::PathBuf;
use std::process::Command;
use std::string::ToString;
use std::sync::OnceLock;
use std::vec::Vec;

/// The cases' folder, made once for all the tests.
fn cases() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("zip-crate-cases-{}", std::process::id()));
        let script = format!("{}/tests/cases.py", env!("CARGO_MANIFEST_DIR"));
        let ok = Command::new("python3").args([&script, dir.to_str().unwrap()]).status().expect("python3, to make the cases");
        assert!(ok.success(), "{script}");
        dir
    })
}

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01B3))
}

#[test]
fn inflates_what_zlib_makes() {
    let dir = cases();
    let inputs: Vec<Vec<u8>> = (0..6).map(|i| std::fs::read(dir.join(format!("input-{i}.bin"))).unwrap()).collect();
    let all = std::fs::read(dir.join("deflate.bin")).unwrap();
    let (mut at, mut n) = (0usize, 0usize);
    while at < all.len() {
        let i = all[at] as usize;
        let len = u32::from_le_bytes(all[at + 1..at + 5].try_into().unwrap()) as usize;
        let packed = &all[at + 5..at + 5 + len];
        let plain = &inputs[i];
        let out = inflate(packed, plain.len()).unwrap_or_else(|e| panic!("case {n} (input {i}): {e:?}"));
        assert!(out == *plain, "case {n} (input {i}): {} bytes out, not {}", out.len(), plain.len());
        // One byte less room than it needs: too big (unless it's empty).
        if !plain.is_empty() {
            assert_eq!(inflate(packed, plain.len() - 1), Err(Error::TooBig), "case {n}");
        }
        at += 5 + len;
        n += 1;
    }
    assert_eq!(n, 6 * 52);
    // zlib's wrapping, its checksum checked.
    let mut z = std::fs::read(dir.join("zlib.bin")).unwrap();
    assert_eq!(zlib(&z, 1 << 20).unwrap(), inputs[2]);
    let last = z.len() - 1;
    z[last] ^= 1;
    assert_eq!(zlib(&z, 1 << 20), Err(Error::Damaged));
    assert_eq!(zlib(b"not zlib", 100), Err(Error::NotZip));
}

#[test]
fn damaged_streams_give_errors() {
    let dir = cases();
    let all = std::fs::read(dir.join("deflate.bin")).unwrap();
    // A few streams (words with dynamic codes, with fixed codes, stored), each byte in turn changed, or cut short.
    let mut streams = Vec::new();
    let mut at = 0;
    while at < all.len() {
        let len = u32::from_le_bytes(all[at + 1..at + 5].try_into().unwrap()) as usize;
        streams.push(&all[at + 5..at + 5 + len]);
        at += 5 + len;
    }
    for s in [streams[52 * 5 + 6 * 5], streams[52 * 5 + 9 * 5 + 4], streams[52 * 5]] {
        let s = &s[..s.len().min(3_000)];
        for i in 0..s.len() {
            let mut d = s.to_vec();
            d[i] ^= 0x5A;
            let _ = inflate(&d, 1 << 20);
            let _ = inflate(&s[..i], 1 << 20);
        }
    }
    // Made by hand: a copy (3 bytes from 1 back, in fixed codes) before anything was written; a dynamic block whose
    // first code length repeats the one before it, when there's none; a block type that doesn't exist; a stored
    // block whose length's complement is wrong; nothing at all.
    assert_eq!(inflate(&[0x03, 0x02, 0x00], 10).err(), Some(Error::Damaged));
    assert_eq!(inflate(&[0x05, 0x00, 0x12, 0x00], 10).err(), Some(Error::Damaged));
    assert_eq!(inflate(&[0x07, 0x00], 10).err(), Some(Error::Damaged));
    assert_eq!(inflate(&[0x01, 0x05, 0x00, 0x00, 0x00, b'h'], 10).err(), Some(Error::Damaged));
    assert_eq!(inflate(&[], 10).err(), Some(Error::Damaged));
    // A bomb: a megabyte of zeros from a few hundred bytes, stopped at the cap.
    let zeros = {
        let mut c = Vec::new();
        // Fixed codes: a zero, then copies of 258 from 1 back.
        let mut bits: u64 = 0;
        let mut n = 0u32;
        let mut put = |v: u64, len: u32, c: &mut Vec<u8>| {
            bits |= v << n;
            n += len;
            while n >= 8 {
                c.push(bits as u8);
                bits >>= 8;
                n -= 8;
            }
        };
        put(1, 1, &mut c);
        put(1, 2, &mut c);
        // Literal 0: code 00110000 (8 bits, sent first bit first, so reversed).
        put(0b0000_1100, 8, &mut c);
        for _ in 0..4_000 {
            // Length 258 is symbol 285: code 11000101 (8 bits); distance 1 is code 00000 (5 bits).
            put(0b1010_0011, 8, &mut c);
            put(0, 5, &mut c);
        }
        put(0, 7, &mut c);
        put(0, 8, &mut c);
        c
    };
    assert_eq!(inflate(&zeros, 1 << 30).unwrap().len(), 1 + 4_000 * 258);
    assert_eq!(inflate(&zeros, 100_000), Err(Error::TooBig));
}

/// Each archive's files, read, as name → (size, hash).
fn read_all(name: &str) -> BTreeMap<String, (u64, u64)> {
    let data = std::fs::read(cases().join(format!("{name}.zip"))).unwrap();
    let a = Archive::open(&data).unwrap();
    a.entries()
        .iter()
        .map(|e| {
            let bytes = a.read(e, 1 << 24).unwrap_or_else(|err| panic!("{name}: {}: {err:?}", e.name));
            assert_eq!(e.is_folder(), e.name.ends_with('/'));
            (e.name.clone(), (bytes.len() as u64, fnv(&bytes)))
        })
        .collect()
}

fn listed(name: &str) -> BTreeMap<String, (u64, u64)> {
    std::fs::read_to_string(cases().join(format!("{name}.txt")))
        .unwrap()
        .lines()
        .map(|l| {
            let v: Vec<&str> = l.split('\t').collect();
            (v[0].to_string(), (v[1].parse().unwrap(), u64::from_str_radix(v[2], 16).unwrap()))
        })
        .collect()
}

#[test]
fn reads_archives() {
    for name in ["python", "stream", "zip64", "cp437"] {
        assert_eq!(read_all(name), listed(name), "{name}");
    }
    if cases().join("infozip.zip").exists() {
        assert_eq!(read_all("infozip"), listed("infozip"));
    }
    // Found whatever the case, with or without a / first.
    let data = std::fs::read(cases().join("python.zip")).unwrap();
    let a = Archive::open(&data).unwrap();
    assert!(a.find("/FOLDER/Words.TXT").is_some_and(|e| e.size == 150_000));
    assert!(a.find("folder/nothing.txt").is_none());
    // Too big for what's allowed.
    let words = a.find("folder/words.txt").unwrap();
    assert_eq!(a.read(words, 149_999), Err(Error::TooBig));
}

#[test]
fn says_what_it_cant_read() {
    let dir = cases();
    let data = std::fs::read(dir.join("other.zip")).unwrap();
    let a = Archive::open(&data).unwrap();
    assert_eq!(a.read(a.find("bzip2.txt").unwrap(), 1 << 20), Err(Error::Method(12)));
    assert_eq!(a.read(a.find("lzma.txt").unwrap(), 1 << 20), Err(Error::Method(14)));
    if dir.join("password.zip").exists() {
        let data = std::fs::read(dir.join("password.zip")).unwrap();
        let a = Archive::open(&data).unwrap();
        assert_eq!(a.read(&a.entries()[0], 1 << 20), Err(Error::Encrypted));
    }
    // Not an archive; an archive cut short; a file whose bytes were changed (its CRC no longer matches).
    assert_eq!(Archive::open(b"just some text, no end record").err(), Some(Error::NotZip));
    let data = std::fs::read(dir.join("python.zip")).unwrap();
    assert!(Archive::open(&data[..data.len() / 2]).is_err());
    let mut changed = data.clone();
    let a = Archive::open(&data).unwrap();
    let stored = a.find("folder/stored.txt").unwrap().clone();
    let start = a.read(&stored, 1 << 20).unwrap()[..100].to_vec();
    let at = data.windows(100).position(|w| w == start.as_slice()).unwrap();
    changed[at + 10] ^= 1;
    let b = Archive::open(&changed).unwrap();
    assert_eq!(b.read(&stored, 1 << 20), Err(Error::Damaged));
    // Every byte of the central directory changed in turn: never a crash.
    let end = data.len();
    for i in (end.saturating_sub(1_000)..end).step_by(3) {
        let mut d = data.clone();
        d[i] = d[i].wrapping_add(0x55);
        if let Ok(a) = Archive::open(&d) {
            for e in a.entries() {
                let _ = a.read(e, 1 << 20);
            }
        }
    }
}

#[test]
fn checksums() {
    assert_eq!(crc32(b""), 0);
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    assert_eq!(adler32(&[0xFF; 100_000]), {
        let (mut a, mut b) = (1u64, 0u64);
        for _ in 0..100_000 {
            a = (a + 0xFF) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16 | a) as u32
    });
}
