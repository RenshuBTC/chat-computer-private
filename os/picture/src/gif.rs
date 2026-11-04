//! GIF (87a and 89a): its first frame, on its logical screen (the rest of which is the background), with the frame's
//! transparent colour (a graphic control extension's) on the background too. LZW-decoded, codes 3 to 12 bits wide;
//! interlaced frames' rows put back in order.

use crate::{Error, Picture};
use alloc::vec;
use alloc::vec::Vec;

/// The size a GIF says it is (its logical screen's).
pub fn size(data: &[u8]) -> Option<(u32, u32)> {
    Some((u16::from_le_bytes(data.get(6..8)?.try_into().ok()?) as u32, u16::from_le_bytes(data.get(8..10)?.try_into().ok()?) as u32))
}

pub fn decode(data: &[u8], background: [u8; 3], most: u64) -> Result<Picture, Error> {
    let (w, h) = size(data).ok_or(Error::Damaged)?;
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 {
        return Err(Error::Damaged);
    }
    if (w * h) as u64 > most {
        return Err(Error::TooBig);
    }
    let flags = *data.get(10).ok_or(Error::Damaged)?;
    let mut at = 13;
    let mut global: &[u8] = &[];
    if flags & 0x80 != 0 {
        let n = 3 << ((flags & 7) + 1);
        global = data.get(at..at + n).ok_or(Error::Damaged)?;
        at += n;
    }
    let mut transparent: Option<u8> = None;
    loop {
        match *data.get(at).ok_or(Error::Damaged)? {
            // An extension: a graphic control one says which colour is transparent; the rest are passed over.
            0x21 => {
                let label = *data.get(at + 1).ok_or(Error::Damaged)?;
                at += 2;
                if label == 0xF9 && data.get(at) == Some(&4) {
                    let packed = *data.get(at + 1).ok_or(Error::Damaged)?;
                    transparent = (packed & 1 != 0).then(|| data.get(at + 4).copied()).flatten();
                }
                at = skip_blocks(data, at)?;
            }
            0x2C => break,
            _ => return Err(Error::Damaged),
        }
    }
    // The frame: where it is on the screen, its size, its own colours (if it has them), and whether it's interlaced.
    let le = |i: usize| -> Result<usize, Error> { Ok(u16::from_le_bytes(data.get(i..i + 2).ok_or(Error::Damaged)?.try_into().unwrap()) as usize) };
    let (left, top, fw, fh) = (le(at + 1)?, le(at + 3)?, le(at + 5)?, le(at + 7)?);
    let frame_flags = *data.get(at + 9).ok_or(Error::Damaged)?;
    at += 10;
    let mut colors = global;
    if frame_flags & 0x80 != 0 {
        let n = 3 << ((frame_flags & 7) + 1);
        colors = data.get(at..at + n).ok_or(Error::Damaged)?;
        at += n;
    }
    let min_code = *data.get(at).ok_or(Error::Damaged)?;
    at += 1;
    // The image data's sub-blocks, joined.
    let mut lzw = Vec::new();
    loop {
        let n = *data.get(at).ok_or(Error::Damaged)? as usize;
        at += 1;
        if n == 0 {
            break;
        }
        lzw.extend_from_slice(data.get(at..at + n).ok_or(Error::Damaged)?);
        at += n;
    }
    // The frame's own size is what's decoded (it can be larger than the screen): it must fit the same budget.
    let frame_pixels = (fw as u64).saturating_mul(fh as u64);
    if frame_pixels > most {
        return Err(Error::TooBig);
    }
    let indexes = decompress(&lzw, min_code, frame_pixels as usize)?;
    let mut rgb = Vec::with_capacity(w * h * 3);
    for _ in 0..w * h {
        rgb.extend_from_slice(&background);
    }
    // Interlaced: rows 0, 8, 16... then 4, 12... then 2, 6... then 1, 3...
    let rows: Vec<usize> = if frame_flags & 0x40 != 0 {
        [(0, 8), (4, 8), (2, 4), (1, 2)].iter().flat_map(|&(start, step)| (start..fh).step_by(step)).collect()
    } else {
        (0..fh).collect()
    };
    for (k, &row) in rows.iter().enumerate() {
        let y = top + row;
        if y >= h {
            continue;
        }
        for i in 0..fw {
            let x = left + i;
            let Some(&index) = indexes.get(k * fw + i) else { continue };
            if x >= w || Some(index) == transparent {
                continue;
            }
            if let Some(c) = colors.get(index as usize * 3..index as usize * 3 + 3) {
                rgb[(y * w + x) * 3..][..3].copy_from_slice(c);
            }
        }
    }
    Ok(Picture { width: w as u32, height: h as u32, rgb })
}

/// Past a run of data sub-blocks (each its length, then its bytes), to after the empty one that ends them.
fn skip_blocks(data: &[u8], mut at: usize) -> Result<usize, Error> {
    loop {
        let n = *data.get(at).ok_or(Error::Damaged)? as usize;
        at += 1 + n;
        if n == 0 {
            return Ok(at);
        }
    }
}

/// GIF's LZW: codes from the lowest bit up, starting `min + 1` bits wide and widening to 12 as the table fills; a clear
/// code starts the table again, an end code ends. At most `most` indexes (a frame's pixels).
fn decompress(data: &[u8], min: u8, most: usize) -> Result<Vec<u8>, Error> {
    if !(2..=11).contains(&min) {
        return Err(Error::Damaged);
    }
    let clear = 1usize << min;
    let end = clear + 1;
    // The table: each code's previous code and last byte (codes below `clear` are their own byte).
    let mut prefix = vec![0u16; 4096];
    let mut suffix = vec![0u8; 4096];
    let mut first = vec![0u8; 4096];
    for i in 0..clear {
        suffix[i] = i as u8;
        first[i] = i as u8;
    }
    let (mut width, mut next) = (min as u32 + 1, end + 1);
    let mut previous: Option<usize> = None;
    let mut out: Vec<u8> = Vec::with_capacity(most);
    let mut stack: Vec<u8> = Vec::with_capacity(4096);
    let (mut bits, mut count, mut at) = (0u32, 0u32, 0usize);
    while out.len() < most {
        while count < width {
            let Some(&b) = data.get(at) else { return Ok(out) };
            bits |= (b as u32) << count;
            count += 8;
            at += 1;
        }
        let code = (bits & ((1 << width) - 1)) as usize;
        bits >>= width;
        count -= width;
        if code == clear {
            width = min as u32 + 1;
            next = end + 1;
            previous = None;
            continue;
        }
        if code == end {
            break;
        }
        let Some(prev) = previous else {
            if code >= clear {
                return Err(Error::Damaged);
            }
            out.push(code as u8);
            previous = Some(code);
            continue;
        };
        // A code not in the table yet can only be the next one: the previous string and its own first byte.
        let (string, extra) = if code < next {
            (code, None)
        } else if code == next {
            (prev, Some(first[prev]))
        } else {
            return Err(Error::Damaged);
        };
        stack.clear();
        let mut c = string;
        while c >= clear {
            stack.push(suffix[c]);
            c = prefix[c] as usize;
        }
        stack.push(c as u8);
        out.extend(stack.iter().rev());
        if let Some(e) = extra {
            out.push(e);
        }
        if next < 4096 {
            prefix[next] = prev as u16;
            suffix[next] = first[string];
            first[next] = first[prev];
            next += 1;
            if next == 1 << width && width < 12 {
                width += 1;
            }
        }
        previous = Some(code);
    }
    out.truncate(most);
    Ok(out)
}
