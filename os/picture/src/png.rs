//! PNG (ISO/IEC 15948): its chunks (each one's CRC checked), the header's size, colour type and depth, the palette
//! and transparency; the image data inflated (zlib, by the zip crate), each row unfiltered (none, sub, up, average,
//! Paeth), interlaced images' seven passes put in place (Adam7); every colour type and depth, 16-bit samples by their
//! high byte, onto the background where they're transparent.

use crate::{Error, Picture, blend};
use alloc::vec;
use alloc::vec::Vec;

pub const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The passes of Adam7: where each starts, and how far apart its pixels are, across and down.
const ADAM7: [(usize, usize, usize, usize); 7] = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];

struct Header {
    width: usize,
    height: usize,
    depth: u8,
    color: u8,
    interlaced: bool,
}

/// The size a PNG says it is, from its header.
pub fn size(data: &[u8]) -> Option<(u32, u32)> {
    if !data.starts_with(SIGNATURE) || data.get(12..16) != Some(b"IHDR") {
        return None;
    }
    Some((u32::from_be_bytes(data.get(16..20)?.try_into().ok()?), u32::from_be_bytes(data.get(20..24)?.try_into().ok()?)))
}

pub fn decode(data: &[u8], background: [u8; 3], most: u64) -> Result<Picture, Error> {
    if !data.starts_with(SIGNATURE) {
        return Err(Error::NotAPicture);
    }
    let mut at = SIGNATURE.len();
    let mut header: Option<Header> = None;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut alphas: Vec<u8> = Vec::new();
    let mut transparent: Option<[u16; 3]> = None;
    let mut idat: Vec<u8> = Vec::new();
    loop {
        let len = u32::from_be_bytes(data.get(at..at + 4).ok_or(Error::Damaged)?.try_into().unwrap()) as usize;
        let kind = data.get(at + 4..at + 8).ok_or(Error::Damaged)?;
        let body = data.get(at + 8..at + 8 + len).ok_or(Error::Damaged)?;
        let crc = data.get(at + 8 + len..at + 12 + len).ok_or(Error::Damaged)?;
        if zip::crc32(&data[at + 4..at + 8 + len]) != u32::from_be_bytes(crc.try_into().unwrap()) {
            return Err(Error::Damaged);
        }
        at += 12 + len;
        match kind {
            b"IHDR" if body.len() == 13 => {
                let be = |i: usize| u32::from_be_bytes(body[i..i + 4].try_into().unwrap()) as usize;
                let (width, height, depth, color) = (be(0), be(4), body[8], body[9]);
                let ok = matches!(
                    (color, depth),
                    (0, 1 | 2 | 4 | 8 | 16) | (2, 8 | 16) | (3, 1 | 2 | 4 | 8) | (4, 8 | 16) | (6, 8 | 16)
                );
                if !ok || width == 0 || height == 0 || body[10] != 0 || body[11] != 0 || body[12] > 1 {
                    return Err(Error::Damaged);
                }
                if width as u64 * height as u64 > most {
                    return Err(Error::TooBig);
                }
                header = Some(Header { width, height, depth, color, interlaced: body[12] == 1 });
            }
            b"PLTE" if body.len() % 3 == 0 && body.len() <= 768 => {
                palette = body.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            }
            b"tRNS" => match header.as_ref().map(|h| h.color) {
                Some(3) => alphas = body.to_vec(),
                Some(0) if body.len() >= 2 => {
                    let g = u16::from_be_bytes([body[0], body[1]]);
                    transparent = Some([g, g, g]);
                }
                Some(2) if body.len() >= 6 => {
                    let v = |i: usize| u16::from_be_bytes([body[i], body[i + 1]]);
                    transparent = Some([v(0), v(2), v(4)]);
                }
                _ => {}
            },
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
    }
    let h = header.ok_or(Error::Damaged)?;
    if h.color == 3 && palette.is_empty() {
        return Err(Error::Damaged);
    }
    let channels = match h.color {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        _ => 4,
    };
    let bits = channels * h.depth as usize;
    let step = bits.div_ceil(8);
    let passes: Vec<(usize, usize, usize, usize)> = if h.interlaced { ADAM7.to_vec() } else { vec![(0, 0, 1, 1)] };
    // Each pass's size, and all of them together, filter bytes and all.
    let sizes: Vec<(usize, usize)> = passes
        .iter()
        .map(|&(x0, y0, dx, dy)| (h.width.saturating_sub(x0).div_ceil(dx), h.height.saturating_sub(y0).div_ceil(dy)))
        .collect();
    let total: usize = sizes.iter().map(|&(w, ht)| if w == 0 || ht == 0 { 0 } else { ht * (1 + (w * bits).div_ceil(8)) }).sum();
    let raw = zip::zlib(&idat, total).map_err(|_| Error::Damaged)?;
    if raw.len() < total {
        return Err(Error::Damaged);
    }
    let mut rgb = vec![0u8; h.width * h.height * 3];
    let mut at = 0;
    for (&(x0, y0, dx, dy), &(pw, ph)) in passes.iter().zip(&sizes) {
        if pw == 0 || ph == 0 {
            continue;
        }
        let row_len = (pw * bits).div_ceil(8);
        let mut prev = vec![0u8; row_len];
        let mut row = vec![0u8; row_len];
        for j in 0..ph {
            let filter = raw[at];
            row.copy_from_slice(&raw[at + 1..at + 1 + row_len]);
            at += 1 + row_len;
            unfilter(filter, &mut row, &prev, step)?;
            for i in 0..pw {
                let pixel = sample(&h, &row, i, &palette, &alphas, transparent, background);
                let (x, y) = (x0 + i * dx, y0 + j * dy);
                rgb[(y * h.width + x) * 3..][..3].copy_from_slice(&pixel);
            }
            core::mem::swap(&mut prev, &mut row);
        }
    }
    Ok(Picture { width: h.width as u32, height: h.height as u32, rgb })
}

/// A row's bytes, unfiltered in place, from the row above's (`step`: bytes a pixel, at least 1).
fn unfilter(filter: u8, row: &mut [u8], prev: &[u8], step: usize) -> Result<(), Error> {
    match filter {
        0 => {}
        1 => {
            for i in step..row.len() {
                row[i] = row[i].wrapping_add(row[i - step]);
            }
        }
        2 => {
            for i in 0..row.len() {
                row[i] = row[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            for i in 0..row.len() {
                let left = if i >= step { row[i - step] as u16 } else { 0 };
                row[i] = row[i].wrapping_add(((left + prev[i] as u16) / 2) as u8);
            }
        }
        4 => {
            for i in 0..row.len() {
                let (a, b, c) = if i >= step { (row[i - step], prev[i], prev[i - step]) } else { (0, prev[i], 0) };
                row[i] = row[i].wrapping_add(paeth(a, b, c));
            }
        }
        _ => return Err(Error::Damaged),
    }
    Ok(())
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Pixel `i` of an unfiltered row, as RGB on the background.
fn sample(h: &Header, row: &[u8], i: usize, palette: &[[u8; 3]], alphas: &[u8], transparent: Option<[u16; 3]>, background: [u8; 3]) -> [u8; 3] {
    let d = h.depth as usize;
    // A sample's full value (for comparing with tRNS), and as 8 bits.
    let get = |k: usize| -> (u16, u8) {
        match d {
            16 => {
                let v = u16::from_be_bytes([row[2 * k], row[2 * k + 1]]);
                (v, (v >> 8) as u8)
            }
            8 => (row[k] as u16, row[k]),
            _ => {
                let bit = k * d;
                let v = (row[bit / 8] >> (8 - d - bit % 8)) & ((1 << d) - 1) as u8;
                (v as u16, (v as u16 * 255 / ((1 << d) - 1)) as u8)
            }
        }
    };
    match h.color {
        0 => {
            let (full, g) = get(i);
            if transparent.is_some_and(|t| t[0] == full) { background } else { [g, g, g] }
        }
        2 => {
            let (r, g, b) = (get(3 * i), get(3 * i + 1), get(3 * i + 2));
            if transparent.is_some_and(|t| t == [r.0, g.0, b.0]) { background } else { [r.1, g.1, b.1] }
        }
        3 => {
            let index = get(i).0 as usize;
            let c = palette.get(index).copied().unwrap_or([0, 0, 0]);
            blend(c, alphas.get(index).copied().unwrap_or(255), background)
        }
        4 => {
            let (g, a) = (get(2 * i).1, get(2 * i + 1).1);
            blend([g, g, g], a, background)
        }
        _ => blend([get(4 * i).1, get(4 * i + 1).1, get(4 * i + 2).1], get(4 * i + 3).1, background),
    }
}
