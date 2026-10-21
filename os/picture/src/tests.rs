//! Against pictures written by Pillow (libjpeg-turbo, zlib) and by a PNG writer of the tests' own, and the pixels
//! each should come out as (tests/cases.py makes them all, in a folder of its own for each run): JPEGs as libjpeg
//! decodes them (a sample at most 1 off), PNGs and GIFs exactly, pictures decoded smaller near enough to the whole
//! averaged down. Damaged pictures give errors, never a crash.

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
