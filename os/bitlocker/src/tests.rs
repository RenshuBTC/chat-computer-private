//! Against the volumes Windows encrypted that cryptsetup tests its BitLocker support with (its tests/bitlk-images.tar.xz,
//! downloaded from its repository at a fixed commit, or the copy the BITLOCKER_IMAGES variable names, and checked by
//! its SHA-256): each unlocked with its recovery key (and its password, a second recovery key, or its clear key), and
//! the whole volume decrypted, must hash (SHA-256) to what cryptsetup's own decryption hashes to. What isn't read
//! (Windows 7's diffuser, volumes still being encrypted) must say so; wrong keys must be refused.

extern crate std;

use super::*;
use std::collections::BTreeMap;
use std::format;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::string::ToString;
use std::sync::OnceLock;

/// cryptsetup's commit, and the images' archive's SHA-256.
const COMMIT: &str = "bae21790d59897ac50e5471f953f76b3a58aeeb6";
const ARCHIVE_SHA256: &str = "68bf5669f777668112d497234ebe2166b5feaf0e33b206b095b917f82937bd30";

fn archive() -> &'static PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let path = match std::env::var("BITLOCKER_IMAGES") {
            Ok(p) => PathBuf::from(p),
            Err(_) => std::env::temp_dir().join(format!("bitlk-images-{COMMIT}.tar.xz")),
        };
        if !path.exists() {
            let url = format!("https://gitlab.com/cryptsetup/cryptsetup/-/raw/{COMMIT}/tests/bitlk-images.tar.xz");
            let ok = Command::new("curl").args(["-sSfL", "-o", path.to_str().unwrap(), &url]).status().expect("curl");
            assert!(ok.success(), "couldn't download {url}");
        }
        let sum = Command::new("sha256sum").arg(&path).output().expect("sha256sum").stdout;
        assert!(sum.starts_with(ARCHIVE_SHA256.as_bytes()), "{} isn't the archive expected", path.display());
        path
    })
}

/// images.conf: each image's settings.
fn settings() -> BTreeMap<String, BTreeMap<String, String>> {
    let out = Command::new("tar").args(["-xJOf", archive().to_str().unwrap(), "bitlk-images/images.conf"]).output().expect("tar");
    let mut all = BTreeMap::new();
    let mut current = String::new();
    for line in String::from_utf8(out.stdout).unwrap().lines() {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.to_string();
            all.insert(current.clone(), BTreeMap::new());
        } else if let Some((k, v)) = line.split_once('=') {
            all.get_mut(&current).unwrap().insert(k.to_string(), v.to_string());
        }
    }
    all
}

fn octal(field: &[u8]) -> u64 {
    let s = String::from_utf8_lossy(field);
    u64::from_str_radix(s.trim_matches(|c: char| c == '\0' || c == ' '), 8).unwrap_or(0)
}

/// A GNU sparse file's map: where each stored piece goes (offset, length), from `n` entries of 24 bytes.
fn sparse_entries(b: &[u8], n: usize, map: &mut std::vec::Vec<(u64, u64)>) {
    for e in b[..24 * n].chunks(24) {
        if e[0] == 0 {
            break;
        }
        map.push((octal(&e[..12]), octal(&e[12..])));
    }
}

/// Each image in the archive, in turn (read from the archive's tar stream as it's decompressed, one at a time). The
/// images are GNU tar's sparse files: only their pieces that aren't zeros are stored, each where its map says.
fn each_image(mut f: impl FnMut(&str, Vec<u8>)) {
    let mut xz = Command::new("xz").args(["-dc", archive().to_str().unwrap()]).stdout(Stdio::piped()).spawn().expect("xz");
    let mut tar = xz.stdout.take().unwrap();
    let mut header = [0u8; 512];
    loop {
        if tar.read_exact(&mut header).is_err() || header.iter().all(|&b| b == 0) {
            break;
        }
        let name = String::from_utf8_lossy(&header[..100]).trim_end_matches('\0').to_string();
        let stored = octal(&header[124..136]);
        let mut map = std::vec::Vec::new();
        let mut size = stored;
        if header[156] == b'S' {
            sparse_entries(&header[386..], 4, &mut map);
            size = octal(&header[483..495]);
            let mut extended = header[482] != 0;
            while extended {
                let mut more = [0u8; 512];
                tar.read_exact(&mut more).unwrap();
                sparse_entries(&more, 21, &mut map);
                extended = more[504] != 0;
            }
        } else {
            map.push((0, stored));
        }
        let mut data = std::vec![0u8; stored as usize];
        tar.read_exact(&mut data).unwrap();
        let pad = (512 - stored % 512) % 512;
        std::io::copy(&mut (&mut tar).take(pad), &mut std::io::sink()).unwrap();
        if let Some(image) = name.strip_prefix("bitlk-images/").and_then(|n| n.strip_suffix(".img")) {
            let mut whole = std::vec![0u8; size as usize];
            let mut from = 0usize;
            for (offset, len) in map {
                let (offset, len) = (offset as usize, len as usize);
                whole[offset..offset + len].copy_from_slice(&data[from..from + len]);
                from += len;
            }
            assert_eq!(from, data.len(), "{name}'s map");
            f(image, whole);
        }
    }
    // (Its pipe closed first, so it stops if anything's left.)
    drop(tar);
    let _ = xz.wait();
}

struct Image(Vec<u8>);

impl Disk for Image {
    fn read(&mut self, offset: u64, buf: &mut [u8]) -> bool {
        assert!(offset % 512 == 0 && buf.len() % 512 == 0);
        match self.0.get(offset as usize..offset as usize + buf.len()) {
            Some(b) => {
                buf.copy_from_slice(b);
                true
            }
            None => false,
        }
    }
}

/// The SHA-256 of the whole volume decrypted, a megabyte at a time.
fn decrypted_sha256(v: &Volume, key: &Key, disk: &mut Image) -> String {
    let size = disk.0.len() as u64;
    let mut h = Sha256::new();
    let mut buf = std::vec![0u8; 1 << 20];
    let mut at = 0u64;
    while at < size {
        let n = ((size - at) as usize).min(buf.len());
        assert!(v.read(key, disk, at, &mut buf[..n]), "read at {at}");
        h.update(&buf[..n]);
        at += n as u64;
    }
    let mut out = [0u8; 32];
    h.finish_into(&mut out);
    out.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn decrypts_as_cryptsetup_does() {
    let settings = settings();
    let mut checked = 0;
    let mut problems = std::vec::Vec::new();
    each_image(|name, data| {
        let s = &settings[name];
        let mut disk = Image(data);
        let opened = open(&mut disk);
        if name.contains("eow") || name.contains("partially") {
            if !matches!(opened, Err(Error::Unsupported(_))) {
                problems.push(format!("{name}: {:?}, not unsupported", opened.as_ref().err()));
            }
            return;
        }
        let v = match opened {
            Ok(v) => v,
            Err(e) => {
                problems.push(format!("{name}: {e:?}"));
                return;
            }
        };
        let unlocked = if name.contains("clearkey") { v.unlock_clear() } else { v.unlock_recovery(&s["RP"]) };
        if name.contains("elephant") {
            if !matches!(unlocked, Err(Error::Unsupported(_))) {
                problems.push(format!("{name}: {:?}, not unsupported", unlocked.as_ref().err()));
            }
            return;
        }
        let key = match unlocked {
            Ok(k) => k,
            Err(e) => {
                problems.push(format!("{name}: unlocking: {e:?}"));
                return;
            }
        };
        // Its recovery keys' IDs, as Windows shows them.
        for id in ["RP_VMK_GUID", "RP2_VMK_GUID"].iter().filter_map(|k| s.get(*k)).filter(|g| !g.is_empty()) {
            if !v.recovery_key_ids().contains(&id.to_uppercase()) {
                problems.push(format!("{name}: no recovery key {id} in {:?}", v.recovery_key_ids()));
            }
        }
        let want = &s["SHA256SUM"];
        if !want.is_empty() {
            let got = decrypted_sha256(&v, &key, &mut disk);
            if &got != want {
                problems.push(format!("{name}: decrypted to {got}, not {want}"));
            }
        }
        // Its password, and a second recovery key, unlock it too: the same key (the same first megabyte).
        let first = |k: &Key, disk: &mut Image| {
            let mut b = std::vec![0u8; 1 << 20];
            assert!(v.read(k, disk, 0, &mut b));
            b
        };
        let reference = first(&key, &mut disk);
        for (what, other) in [("PW", s.get("PW").filter(|p| !p.is_empty()).map(|p| v.unlock_password(p))), ("RP2", s.get("RP2").map(|r| v.unlock_recovery(r)))] {
            if let Some(result) = other {
                match result {
                    Ok(k) if first(&k, &mut disk) == reference => {}
                    Ok(_) => problems.push(format!("{name}: {what} unlocked it to something else")),
                    Err(e) => problems.push(format!("{name}: {what}: {e:?}")),
                }
            }
        }
        checked += 1;
    });
    assert!(problems.is_empty(), "{problems:#?}");
    assert_eq!(checked, 17);
}

#[test]
fn refuses_wrong_keys() {
    let settings = settings();
    let mut seen = false;
    each_image(|name, data| {
        if name != "bitlk-aes-xts-128" {
            return;
        }
        seen = true;
        let mut disk = Image(data);
        let v = open(&mut disk).unwrap();
        assert!(v.protections().contains(&Protection::RecoveryKey) && v.protections().contains(&Protection::Password));
        assert_eq!(v.description, "DESKTOP-NPM7RCA H: 7/4/2019");
        assert_eq!((v.sector_size, v.size, v.method, v.to_go), (512, 104857600, 0x8004, false));
        // A recovery key with one group changed (still a multiple of 11), and ones that aren't recovery keys.
        let right = &settings[name]["RP"];
        let wrong = format!("{}{}", if right.starts_with("000011") { "000022" } else { "000011" }, &right[6..]);
        assert_eq!(v.unlock_recovery(&wrong).err(), Some(Error::WrongKey));
        assert_eq!(v.unlock_password("not the password").err(), Some(Error::WrongKey));
        assert_eq!(v.unlock_clear().err(), Some(Error::NoSuchProtector));
        for bad in ["123", "000012-000000-000000-000000-000000-000000-000000-000000", &"9".repeat(48)] {
            assert_eq!(v.unlock_recovery(bad).err(), Some(Error::NotARecoveryKey), "{bad}");
        }
        // Spaces instead of dashes, or none: the same digits.
        assert!(v.unlock_recovery(&right.replace('-', " ")).is_ok());
        // Damaged metadata, copy by copy: the next good copy is read; all three damaged, an error.
        let good = disk.0.clone();
        let offsets: std::vec::Vec<u64> = (0..3).map(|i| u64::from_le_bytes(good[176 + 8 * i..184 + 8 * i].try_into().unwrap())).collect();
        disk.0[offsets[0] as usize + 200] ^= 1;
        assert!(open(&mut disk).is_ok());
        for &o in &offsets {
            disk.0[o as usize + 200] ^= 1;
        }
        disk.0[offsets[0] as usize + 200] ^= 1;
        assert_eq!(open(&mut disk).err(), Some(Error::Damaged));
        disk.0 = good.clone();
        // The metadata changed (the volume's description), its CRCs made right again: it opens, but the VMK shows the
        // metadata isn't what Windows wrote.
        for &o in &offsets {
            let m = &mut disk.0[o as usize..o as usize + METADATA_SIZE as usize];
            let size = (u16::from_le_bytes([m[8], m[9]]) as usize) << 4;
            let at = m[..size].windows(14).position(|w| w == "DESKTOP".encode_utf16().flat_map(|u| u.to_le_bytes()).collect::<std::vec::Vec<u8>>()).unwrap();
            m[at] = b'L';
            let crc = crc32(&m[..size]);
            m[size + 4..size + 8].copy_from_slice(&crc.to_le_bytes());
        }
        let changed = open(&mut disk).unwrap();
        assert_eq!(changed.description, "LESKTOP-NPM7RCA H: 7/4/2019");
        assert_eq!(changed.unlock_recovery(right).err(), Some(Error::Damaged));
        disk.0 = good;
        // Not BitLocker at all.
        disk.0[3..11].copy_from_slice(b"NTFS    ");
        assert_eq!(open(&mut disk).err(), Some(Error::NotBitLocker));
    });
    assert!(seen);
}

#[test]
fn recovery_keys() {
    // A recovery key's digits: 8 groups, each 11 times two bytes.
    assert_eq!(recovery_key_bytes("000011-000022-000033-000044-000055-000066-000077-720885").unwrap()[..4], [1, 0, 2, 0]);
    assert_eq!(recovery_key_bytes("000011-000022-000033-000044-000055-000066-000077-720885").unwrap()[14..], [0xFF, 0xFF]);
    assert_eq!(recovery_key_bytes("000011-000022-000033-000044-000055-000066-000077-720896").err(), Some(Error::NotARecoveryKey));
    // Found in a message: with dashes, spaces or nothing between the groups; not in a longer number, or one whose
    // groups aren't a recovery key's.
    let key = "123453-234564-345675-456786-567897-678909-111111-222222";
    assert_eq!(find_recovery_key(key), Some(key));
    assert_eq!(find_recovery_key(&format!("My key is {key}.")), Some(key));
    assert_eq!(find_recovery_key(&format!("key:{}\n", key.replace('-', " "))), Some(key.replace('-', " ").as_str()));
    assert_eq!(find_recovery_key(&key.replace('-', "")), Some(key.replace('-', "").as_str()));
    assert_eq!(find_recovery_key(&format!("1{key}")), None);
    assert_eq!(find_recovery_key(&format!("{key}0")), None);
    assert_eq!(find_recovery_key(&key.replace("222222", "222223")), None);
    assert_eq!(find_recovery_key(&key[..48]), None);
    assert_eq!(find_recovery_key("Call 555-123456 at 9"), None);
    assert_eq!(find_recovery_key("£€ 123453-234564-345675-456786-567897-678909-111111-222222 ünïcode"), Some(key));
}


#[test]
fn reads_any_part() {
    let settings = settings();
    let mut seen = 0;
    each_image(|name, data| {
        if !["bitlk-aes-xts-128", "bitlk-aes-xts-128-4k", "bitlk-togo-aes-cbc-128"].contains(&name) {
            return;
        }
        seen += 1;
        let mut disk = Image(data);
        let v = open(&mut disk).unwrap();
        let key = v.unlock_recovery(&settings[name]["RP"]).unwrap();
        // The whole volume, a megabyte at a time (as checked against cryptsetup's), then pieces of it, anywhere, any
        // size: most across where the places kept elsewhere or as zeros begin and end.
        let size = disk.0.len();
        let mut all = std::vec![0u8; size];
        for (i, chunk) in all.chunks_mut(1 << 20).enumerate() {
            assert!(v.read(&key, &mut disk, (i << 20) as u64, chunk));
        }
        let mut edges: std::vec::Vec<u64> = std::vec![0, v.header_size, v.header_offset, v.header_offset + v.header_size, size as u64];
        for m in v.metadata {
            edges.extend([m, m + METADATA_SIZE]);
        }
        let mut x = 7u64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut checked = 0;
        for edge in edges {
            for _ in 0..40 {
                let len = [1, 7, 511, 512, 513, 4095, 4096, 4097, 9000, 70000][(next() % 10) as usize];
                let near = edge as i64 + (next() % 20_000) as i64 - 10_000;
                let at = near.clamp(0, (size - len) as i64) as usize;
                let mut got = std::vec![0u8; len];
                assert!(v.read(&key, &mut disk, at as u64, &mut got), "{name}: {at} + {len}");
                assert!(got == all[at..at + len], "{name}: {at} + {len}");
                checked += 1;
            }
        }
        assert!(checked > 300);
    });
    assert_eq!(seen, 3);
}

/// tests/encrypt.py's volume (as check.py starts Chat Computer with): unlocked with its recovery key, or its clear key
/// when it has one, it reads as the volume it encrypted.
#[test]
fn reads_what_encrypt_py_makes() {
    let dir = std::env::temp_dir().join(format!("bitlocker-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut x = 99u64;
    let mut volume: std::vec::Vec<u8> = (0..3 << 20)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    volume[3..11].copy_from_slice(b"NTFS    ");
    std::fs::write(dir.join("ntfs.img"), &volume).unwrap();
    for clear in [false, true] {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/encrypt.py");
        let mut command = Command::new("python3");
        command.arg(&script).arg(dir.join("ntfs.img")).arg(dir.join("bitlocker.img"));
        if clear {
            command.arg("--clear");
        }
        let out = command.output().expect("python3");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let info = String::from_utf8(out.stdout).unwrap();
        let field = |name: &str| info.split(&format!("\"{name}\": \"")).nth(1).unwrap().split('"').next().unwrap().to_string();
        let mut disk = Image(std::fs::read(dir.join("bitlocker.img")).unwrap());
        assert_eq!(disk.0.len(), volume.len() + (1 << 20));
        let v = open(&mut disk).unwrap();
        let mut kinds = std::vec![Protection::Tpm, Protection::RecoveryKey];
        if clear {
            kinds.push(Protection::ClearKey);
        }
        assert_eq!(v.protections(), kinds);
        assert_eq!(v.recovery_key_ids(), std::vec![field("recovery_id")]);
        assert_eq!(v.description, "DESKTOP-CHAT C: 21/10/2026");
        let keys = if clear { std::vec![v.unlock_clear().unwrap()] } else { std::vec![] };
        for key in keys.into_iter().chain([v.unlock_recovery(&field("recovery_key")).unwrap()]) {
            let mut back = std::vec![0u8; volume.len()];
            assert!(v.read(&key, &mut disk, 0, &mut back));
            assert!(back == volume);
            // After the NTFS volume, where the metadata is kept: zeros.
            let mut after = std::vec![1u8; 1 << 20];
            assert!(v.read(&key, &mut disk, volume.len() as u64, &mut after));
            assert!(after[..8192 + 3 * 65536].iter().all(|&b| b == 0));
        }
        assert_eq!(v.unlock_clear().err(), if clear { None } else { Some(Error::NoSuchProtector) });
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
