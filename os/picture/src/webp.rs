//! WebP (RIFF chunks): lossy (VP8, vp8.rs) or lossless (VP8L, vp8l.rs); simple, or extended (VP8X) with its alpha
//! (ALPH: raw or compressed losslessly, filtered or not) and other chunks (colour profile, metadata: passed over);
//! of an animation, its first frame as it's drawn on the canvas (clear around it).

use crate::{vp8, vp8l, Error, Picture};
use alloc::vec;
use alloc::vec::Vec;

/// Whether `data` is a WebP: a RIFF file of the WEBP form.
pub(crate) fn is_webp(data: &[u8]) -> bool {
    data.len() >= 16 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP"
}

/// The chunks from `at` on: each one's tag and data.
fn chunks(data: &[u8], mut at: usize) -> impl Iterator<Item = (&[u8], &[u8])> {
    core::iter::from_fn(move || {
        if at + 8 > data.len() {
            return None;
        }
        let tag = &data[at..at + 4];
        let size = u32::from_le_bytes([data[at + 4], data[at + 5], data[at + 6], data[at + 7]]) as usize;
        let start = at + 8;
        let end = start.saturating_add(size).min(data.len());
        at = start.saturating_add(size).saturating_add(size & 1);
        Some((tag, &data[start..end]))
    })
}

/// The file's chunks, past its RIFF header (as long as its header says it is, if it's shorter than what's there).
fn body(data: &[u8]) -> &[u8] {
    let riff = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    &data[..riff.saturating_add(8).clamp(12, data.len())]
}

fn u24(b: &[u8]) -> usize {
    b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16
}

/// The size the file says it is: its canvas's, or its only frame's.
pub(crate) fn size(data: &[u8]) -> Option<(u32, u32)> {
    let (tag, d) = chunks(body(data), 12).next()?;
    match tag {
        b"VP8 " => vp8::size(d),
        b"VP8L" => vp8l::size(d),
        b"VP8X" if d.len() >= 10 => Some((u24(&d[4..7]) as u32 + 1, u24(&d[7..10]) as u32 + 1)),
        _ => None,
    }
}

/// The picture, and its alpha (None if it's opaque throughout).
pub(crate) fn decode(data: &[u8], most: u64) -> Result<(Picture, Option<Vec<u8>>), Error> {
    let data = body(data);
    let (tag, d) = chunks(data, 12).next().ok_or(Error::Damaged)?;
    match tag {
        b"VP8 " => lossy(d, None, most),
        b"VP8L" => lossless(d, most),
        b"VP8X" => {
            if d.len() < 10 {
                return Err(Error::Damaged);
            }
            let (cw, ch) = (u24(&d[4..7]) + 1, u24(&d[7..10]) + 1);
            if cw as u64 * ch as u64 > most {
                return Err(Error::TooBig);
            }
            let rest = chunks(data, 12).skip(1);
            if d[0] & 0x02 != 0 {
                // An animation: its first frame.
                let (_, f) = rest.into_iter().find(|(t, _)| *t == b"ANMF").ok_or(Error::Damaged)?;
                return first_frame(f, cw, ch, most);
            }
            still(rest, most)
        }
        _ => Err(Error::Damaged),
    }
}

/// A still picture from its chunks: ALPH (perhaps) and VP8, or VP8L.
fn still<'a>(chunks: impl Iterator<Item = (&'a [u8], &'a [u8])>, most: u64) -> Result<(Picture, Option<Vec<u8>>), Error> {
    let mut alpha = None;
    for (tag, d) in chunks {
        match tag {
            b"ALPH" => alpha = Some(d),
            b"VP8 " => return lossy(d, alpha, most),
            b"VP8L" => return lossless(d, most),
            _ => {}
        }
    }
    Err(Error::Damaged)
}

/// An animation's first frame (ANMF: where it's put and its size, then its chunks), on a clear canvas.
fn first_frame(f: &[u8], cw: usize, ch: usize, most: u64) -> Result<(Picture, Option<Vec<u8>>), Error> {
    if f.len() < 16 {
        return Err(Error::Damaged);
    }
    let (fx, fy) = (2 * u24(&f[0..3]), 2 * u24(&f[3..6]));
    let (p, alpha) = still(chunks(f, 16), most)?;
    let (fw, fh) = (p.width as usize, p.height as usize);
    if (fx, fy, fw, fh) == (0, 0, cw, ch) {
        return Ok((p, alpha));
    }
    let mut rgb = vec![0u8; cw * ch * 3];
    let mut a = vec![0u8; cw * ch];
    for y in 0..fh.min(ch.saturating_sub(fy)) {
        for x in 0..fw.min(cw.saturating_sub(fx)) {
            let (s, t) = (y * fw + x, (fy + y) * cw + fx + x);
            rgb[t * 3..t * 3 + 3].copy_from_slice(&p.rgb[s * 3..s * 3 + 3]);
            a[t] = alpha.as_ref().map_or(255, |al| al[s]);
        }
    }
    Ok((Picture { width: cw as u32, height: ch as u32, rgb }, Some(a)))
}

fn lossy(d: &[u8], alph: Option<&[u8]>, most: u64) -> Result<(Picture, Option<Vec<u8>>), Error> {
    let (w, h, rgb) = vp8::decode(d, most)?;
    let alpha = match alph {
        Some(a) => opaque_none(alpha_plane(a, w, h, most)?),
        None => None,
    };
    Ok((Picture { width: w as u32, height: h as u32, rgb }, alpha))
}

fn lossless(d: &[u8], most: u64) -> Result<(Picture, Option<Vec<u8>>), Error> {
    let (w, h, argb) = vp8l::decode(d, most)?;
    let mut rgb = Vec::with_capacity(w * h * 3);
    let mut alpha = Vec::with_capacity(w * h);
    for p in argb {
        rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        alpha.push((p >> 24) as u8);
    }
    Ok((Picture { width: w as u32, height: h as u32, rgb }, opaque_none(alpha)))
}

fn opaque_none(alpha: Vec<u8>) -> Option<Vec<u8>> {
    if alpha.iter().all(|&a| a == 255) { None } else { Some(alpha) }
}

/// An ALPH chunk's alpha, `w` × `h`: its header (compression, filter, pre-processing), then its values, raw or as
/// the green of a lossless image with no header of its own; then the filter undone.
fn alpha_plane(d: &[u8], w: usize, h: usize, most: u64) -> Result<Vec<u8>, Error> {
    let (&head, rest) = d.split_first().ok_or(Error::Damaged)?;
    let (method, filter, pre) = (head & 3, (head >> 2) & 3, (head >> 4) & 3);
    if method > 1 || pre > 1 || head >> 6 != 0 {
        return Err(Error::Damaged);
    }
    let mut a = if method == 0 {
        rest.get(..w * h).ok_or(Error::Damaged)?.to_vec()
    } else {
        vp8l::decode_headless(rest, w, h, most)?.iter().map(|&p| (p >> 8) as u8).collect()
    };
    // The filter undone: each value was stored as its difference from its prediction (from the left, from above, or
    // their gradient; the first row from the left, and each row's first value from above).
    if filter > 0 {
        for x in 1..w {
            a[x] = a[x].wrapping_add(a[x - 1]);
        }
        for y in 1..h {
            let row = y * w;
            a[row] = a[row].wrapping_add(a[row - w]);
            for x in 1..w {
                let i = row + x;
                let pred = match filter {
                    1 => a[i - 1],
                    2 => a[i - w],
                    _ => (a[i - 1] as i32 + a[i - w] as i32 - a[i - w - 1] as i32).clamp(0, 255) as u8,
                };
                a[i] = a[i].wrapping_add(pred);
            }
        }
    }
    Ok(a)
}
