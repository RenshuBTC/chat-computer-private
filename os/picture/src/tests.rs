//! Against pictures written by Pillow (libjpeg-turbo, zlib) and by a PNG writer of the tests' own, and the pixels
//! each should come out as (tests/cases.py makes them all, in a folder of its own for each run): JPEGs as libjpeg
//! decodes them (a sample at most 1 off), PNGs and GIFs exactly, pictures decoded smaller near enough to the whole
//! averaged down. Damaged pictures give errors, never a crash, and pictures that would need more memory than given are
//! too big. JPEGs written here are byte for byte what Pillow (libjpeg-turbo) writes.

extern crate std;

use super::*;
use std::format;
use std::path::PathBuf;
use std::process::Command;
use std::string::String;
use std::sync::OnceLock;
use std::vec::Vec;

const BACKGROUND: [u8; 3] = [250, 249, 246];

fn cases() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("picture-crate-cases-{}", std::process::id()));
        let script = format!("{}/tests/cases.py", env!("CARGO_MANIFEST_DIR"));
        let ok = Command::new("python3").args([&script, dir.to_str().unwrap()]).status().expect("python3 (with Pillow)");
        assert!(ok.success(), "{script}");
        dir
    })
}

struct Case {
    name: String,
    data: Vec<u8>,
    expected: Vec<u8>,
    width: u32,
    height: u32,
    box_w: u32,
    box_h: u32,
    diff: u8,
    psnr: f64,
}

fn all() -> Vec<Case> {
    let dir = cases();
    std::fs::read_to_string(dir.join("cases.txt"))
        .unwrap()
        .lines()
        .map(|l| {
            let v: Vec<&str> = l.split('\t').collect();
            Case {
                name: v[0].into(),
                data: std::fs::read(dir.join(v[1])).unwrap(),
                expected: std::fs::read(dir.join(format!("{}.rgb", v[0]))).unwrap(),
                width: v[2].parse().unwrap(),
                height: v[3].parse().unwrap(),
                box_w: v[4].parse().unwrap(),
                box_h: v[5].parse().unwrap(),
                diff: v[6].parse().unwrap(),
                psnr: v[7].parse().unwrap(),
            }
        })
        .collect()
}

#[test]
fn decodes_as_libjpeg_and_pillow_do() {
    let mut failures = Vec::new();
    let all = all();
    assert!(all.len() > 60, "{} cases", all.len());
    for c in &all {
        let p = match decode(&c.data, c.box_w, c.box_h, BACKGROUND) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!("{}: {e:?}", c.name));
                continue;
            }
        };
        if (p.width, p.height) != (c.width, c.height) {
            failures.push(format!("{}: {}×{}, not {}×{}", c.name, p.width, p.height, c.width, c.height));
            continue;
        }
        let worst = p.rgb.iter().zip(&c.expected).map(|(&a, &b)| a.abs_diff(b)).max().unwrap_or(0);
        let squares: f64 = p.rgb.iter().zip(&c.expected).map(|(&a, &b)| (a as f64 - b as f64).powi(2)).sum();
        let mse = squares / p.rgb.len() as f64;
        let psnr = if mse == 0.0 { f64::INFINITY } else { 10.0 * (255.0f64 * 255.0 / mse).log10() };
        let ok = if c.psnr > 0.0 { psnr >= c.psnr } else { worst <= c.diff };
        if !ok {
            failures.push(format!("{}: off by up to {worst}, PSNR {psnr:.1} dB", c.name));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn says_what_it_cant_read() {
    assert_eq!(decode(b"not a picture", 10, 10, BACKGROUND).err(), Some(Error::NotAPicture));
    // A JPEG made lossless or arithmetic-coded (its frame marker changed): said, not guessed at.
    let c = all().into_iter().find(|c| c.name == "jpeg-444").unwrap();
    let sof = c.data.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    for (marker, what) in [(0xC3, "lossless JPEG"), (0xC9, "arithmetic-coded JPEG"), (0xC5, "hierarchical JPEG")] {
        let mut d = c.data.clone();
        d[sof + 1] = marker;
        assert_eq!(decode(&d, 1000, 1000, BACKGROUND).err(), Some(Error::Unsupported(what)));
    }
    // Sizes from headers.
    assert_eq!(size(&c.data), Some((320, 240)));
    let p = all().into_iter().find(|c| c.name == "png-pillow-rgb").unwrap();
    assert_eq!(size(&p.data), Some((45, 31)));
}

#[test]
fn damaged_pictures_give_errors() {
    for c in all().iter().filter(|c| c.data.len() < 40_000).step_by(3) {
        // Each byte changed in turn (every few), and cut short anywhere: an error, or a picture, never a crash.
        for i in (0..c.data.len()).step_by(c.data.len() / 200 + 1) {
            let mut d = c.data.clone();
            d[i] = d[i].wrapping_add(0x5B);
            let _ = decode(&d, 400, 400, BACKGROUND);
            let _ = decode(&c.data[..i], 400, 400, BACKGROUND);
        }
    }
}

#[test]
fn shrinks_and_turns() {
    // Shrinking averages exactly: a 4×2 of two colours to 2×1, and 3 pixels to 2 (the middle one shared).
    let p = Picture { width: 4, height: 2, rgb: [[0u8, 0, 0], [0, 0, 0], [200, 100, 50], [200, 100, 50]].repeat(2).concat() };
    assert_eq!(shrink(&p, 2, 1).rgb, [0, 0, 0, 200, 100, 50]);
    let p = Picture { width: 3, height: 1, rgb: [0, 0, 0, 90, 90, 90, 180, 180, 180].into() };
    assert_eq!(shrink(&p, 2, 1).rgb, [30, 30, 30, 150, 150, 150]);
    assert_eq!(fitted(4000, 3000, 1000, 1000), (1000, 750));
    assert_eq!(fitted(3000, 4000, 1000, 1000), (750, 1000));
    assert_eq!(fitted(10, 10, 1000, 1000), (10, 10));
    assert_eq!(fitted(10_000, 1, 100, 100), (100, 1));
    // A 2×1 picture turned each of EXIF's ways.
    let p = Picture { width: 2, height: 1, rgb: [1, 1, 1, 2, 2, 2].into() };
    assert_eq!(orient(p.clone(), 2).rgb, [2, 2, 2, 1, 1, 1]);
    assert_eq!((orient(p.clone(), 6).width, orient(p.clone(), 6).rgb), (1, [1, 1, 1, 2, 2, 2].into()));
    assert_eq!(orient(p.clone(), 8).rgb, [2, 2, 2, 1, 1, 1]);
}

/// A JPEG's scan: its entropy-coded bytes, from after the start of scan to the end of the picture.
fn scan_of(jpeg: &[u8]) -> &[u8] {
    let sos = jpeg.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    let len = u16::from_be_bytes([jpeg[sos + 2], jpeg[sos + 3]]) as usize;
    &jpeg[sos + 2 + len..jpeg.len() - 2]
}

/// A JPEG's quantization tables, by their number (each 64 bytes, in zigzag order as stored).
fn tables_of(jpeg: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 2;
    while at + 4 <= jpeg.len() && jpeg[at] == 0xFF && jpeg[at + 1] != 0xDA {
        let len = u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]) as usize;
        if jpeg[at + 1] == 0xDB {
            let mut k = at + 4;
            while k < at + 2 + len {
                out.push((jpeg[k] & 15, jpeg[k + 1..k + 65].to_vec()));
                k += 65;
            }
        }
        at += 2 + len;
    }
    out.sort();
    out
}

/// The JPEG writer, against Pillow's JPEG of the same pixels at the same quality: the same quantization tables, and
/// the same scan, byte for byte (so libjpeg's arithmetic, from its colour conversion to the edges' padding and its
/// dummy blocks, is followed exactly); and it decodes to the same pixels.
#[test]
fn writes_jpeg_as_libjpeg_does() {
    let dir = cases();
    let list = std::fs::read_to_string(dir.join("encode.txt")).unwrap();
    for line in list.lines() {
        let v: Vec<&str> = line.split('\t').collect();
        let (name, w, h, q): (&str, u32, u32, u8) = (v[0], v[1].parse().unwrap(), v[2].parse().unwrap(), v[3].parse().unwrap());
        let p = Picture { width: w, height: h, rgb: std::fs::read(dir.join(format!("{name}.rgb"))).unwrap() };
        let pillow = std::fs::read(dir.join(format!("{name}.jpg"))).unwrap();
        let ours = encode::jpeg(&p, q);
        assert_eq!(tables_of(&ours), tables_of(&pillow), "{name}: quantization tables");
        let (a, b) = (scan_of(&ours), scan_of(&pillow));
        if a != b {
            let first = a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
            panic!("{name}: the scan differs from byte {first} (of {} and {})", a.len(), b.len());
        }
        let (mine, theirs) = (decode(&ours, w, h, BACKGROUND).unwrap(), decode(&pillow, w, h, BACKGROUND).unwrap());
        assert!(mine.rgb == theirs.rgb, "{name}: decodes differently");
    }
}

/// A picture that would take more memory than it's given to decode is too big, not decoded: a PNG by its pixels (12
/// bytes each, at most), a JPEG by its pixels as decoded (8 bytes each), a progressive JPEG by its coefficients too.
#[test]
fn decoding_keeps_to_its_memory() {
    let all = all();
    let case = |name: &str| all.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no case {name}"));
    let png = case("png-shrunk");
    let need = 300 * 200 * 12;
    assert_eq!(decode_within(&png.data, 150, 120, BACKGROUND, need - 1).err(), Some(Error::TooBig));
    assert!(decode_within(&png.data, 150, 120, BACKGROUND, need).is_ok());
    for c in all.iter().filter(|c| c.data.starts_with(&[0xFF, 0xD8])) {
        let (w, h) = size(&c.data).unwrap();
        // Decoded whole (its box as big as it is), it needs 8 bytes a pixel, and a progressive one 3 bytes a pixel or
        // more for its coefficients: with less than a byte a pixel, every one is too big.
        assert_eq!(decode_within(&c.data, w, h, BACKGROUND, (w * h) as u64 / 2).err(), Some(Error::TooBig), "{}", c.name);
        assert!(decode_within(&c.data, w, h, BACKGROUND, 64 * (w * h) as u64 + (1 << 20)).is_ok(), "{}", c.name);
    }
    // Decoded at an eighth of its size, a progressive JPEG's pixels take little, but its coefficients (2 bytes each,
    // a pixel and a half's at 4:2:0) are kept whole.
    let progressive = case("jpeg-progressive");
    let (w, h) = size(&progressive.data).unwrap();
    let wh = (w * h) as u64;
    assert_eq!(decode_within(&progressive.data, w / 8, h / 8, BACKGROUND, 2 * wh).err(), Some(Error::TooBig));
    assert!(decode_within(&progressive.data, w / 8, h / 8, BACKGROUND, 8 * wh).is_ok());
}

#[test]
fn a_frames_size_is_bounded_by_the_memory_budget() {
    // 25 bytes: a 1x1 screen, then a frame that says it is 65535x65535 and holds no pixels. Decoded by the frame's
    // size that would be 4.29 GB, whatever the screen's own budget says.
    let mut gif = std::vec![0u8; 0];
    gif.extend_from_slice(b"GIF89a");
    gif.extend_from_slice(&1u16.to_le_bytes());              // screen width
    gif.extend_from_slice(&1u16.to_le_bytes());              // screen height
    gif.extend_from_slice(&[0, 0, 0]);                       // no global colours, background 0, no aspect ratio
    gif.push(0x2C);                                          // an image descriptor
    gif.extend_from_slice(&0u16.to_le_bytes());              // left
    gif.extend_from_slice(&0u16.to_le_bytes());              // top
    gif.extend_from_slice(&0xFFFFu16.to_le_bytes());         // frame width
    gif.extend_from_slice(&0xFFFFu16.to_le_bytes());         // frame height
    gif.extend_from_slice(&[0, 2, 0]);                       // no local colours, LZW code size 2, no image data
    assert_eq!(gif.len(), 25);
    assert_eq!(decode_within(&gif, 1, 1, BACKGROUND, 1000).err(), Some(Error::TooBig));
}
