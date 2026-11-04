//! Streams' filters (ISO 32000-1, 7.4): FlateDecode (with PNG's and TIFF's predictors), LZWDecode, ASCIIHexDecode,
//! ASCII85Decode and RunLengthDecode. Pictures' own compression (DCT, JPX, CCITT, JBIG2) isn't undone here: those
//! streams hold no words, and a page's pictures (image.rs) undo it themselves (`decode_image`).

use crate::lex::{Dict, Obj};
use alloc::vec;
use alloc::vec::Vec;

/// The most a stream decodes to.
pub const MOST: usize = 48 << 20;

/// The filters a stream's dictionary names, in order, each with its parameters (references already followed by
/// `resolve`).
pub fn decode(dict: &Dict, data: &[u8], resolve: &dyn Fn(&Obj) -> Obj) -> Option<Vec<u8>> {
    match decode_image(dict, data, resolve)? {
        (buf, None) => Some(buf),
        _ => None,
    }
}

/// A picture's compression (its stream's last filter): the filter's name and its parameters.
pub type Codec = (Vec<u8>, Option<Dict>);

/// The same for a picture's stream, but for its last filter, if that's a picture's own compression (DCT, JPX, CCITT
/// or JBIG2), which is left for the picture to undo (with its parameters).
pub fn decode_image(dict: &Dict, data: &[u8], resolve: &dyn Fn(&Obj) -> Obj) -> Option<(Vec<u8>, Option<Codec>)> {
    let filters = match dict.get("Filter").map(resolve) {
        None | Some(Obj::Null) => return Some((data.to_vec(), None)),
        Some(Obj::Name(n)) => vec![n],
        Some(Obj::Array(a)) => a.iter().filter_map(|f| resolve(f).name().map(<[u8]>::to_vec)).collect(),
        _ => return None,
    };
    let parms: Vec<Option<Dict>> = match dict.get("DecodeParms").or_else(|| dict.get("DP")).map(resolve) {
        Some(Obj::Dict(d)) => vec![Some(d)],
        Some(Obj::Array(a)) => a.iter().map(|p| resolve(p).dict().cloned()).collect(),
        _ => Vec::new(),
    };
    let mut buf = data.to_vec();
    for (i, f) in filters.iter().enumerate() {
        let p = parms.get(i).cloned().flatten();
        if matches!(f.as_slice(), b"DCTDecode" | b"DCT" | b"JPXDecode" | b"CCITTFaxDecode" | b"CCF" | b"JBIG2Decode") {
            return (i + 1 == filters.len()).then_some((buf, Some((f.clone(), p))));
        }
        let num = |key: &str, default: i64| p.as_ref().and_then(|d| d.get(key)).map(resolve).and_then(|o| o.int()).unwrap_or(default);
        buf = match f.as_slice() {
            b"FlateDecode" | b"Fl" => predict(flate(&buf)?, num("Predictor", 1), num("Columns", 1), num("Colors", 1), num("BitsPerComponent", 8))?,
            b"LZWDecode" | b"LZW" => predict(lzw(&buf, num("EarlyChange", 1) != 0)?, num("Predictor", 1), num("Columns", 1), num("Colors", 1), num("BitsPerComponent", 8))?,
            b"ASCIIHexDecode" | b"AHx" => ascii_hex(&buf),
            b"ASCII85Decode" | b"A85" => ascii85(&buf)?,
            b"RunLengthDecode" | b"RL" => run_length(&buf)?,
            b"Crypt" => buf,
            _ => return None,
        };
    }
    Some((buf, None))
}

/// zlib's DEFLATE; a stream with a broken checksum or no zlib header, as its DEFLATE alone; one cut short or damaged,
/// as far as it goes.
fn flate(data: &[u8]) -> Option<Vec<u8>> {
    if let Ok(out) = zip::zlib(data, MOST) {
        return Some(out);
    }
    if let Some(Ok(out)) = data.get(2..).map(|d| zip::inflate(d, MOST)) {
        return Some(out);
    }
    if let Ok(out) = zip::inflate(data, MOST) {
        return Some(out);
    }
    let part = zip::inflate_partial(data.get(2..)?, MOST);
    (!part.is_empty()).then_some(part)
}

/// PNG's predictors (10 and up: each row's own, in its first byte) and TIFF's (2), undone.
fn predict(data: Vec<u8>, predictor: i64, columns: i64, colors: i64, bits: i64) -> Option<Vec<u8>> {
    if predictor < 2 {
        return Some(data);
    }
    let (columns, colors, bits) = (columns.clamp(1, 1 << 20) as usize, colors.clamp(1, 32) as usize, bits.clamp(1, 16) as usize);
    let bpp = (colors * bits).div_ceil(8);
    // (A row longer than the data is the data's one row.)
    let row = (columns * colors * bits).div_ceil(8).min(data.len().max(1));
    if predictor == 2 {
        if bits != 8 {
            return Some(data);
        }
        let mut out = data;
        for r in out.chunks_mut(row) {
            for i in bpp..r.len() {
                r[i] = r[i].wrapping_add(r[i - bpp]);
            }
        }
        return Some(out);
    }
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row];
    for chunk in data.chunks(row + 1) {
        let (kind, line) = (chunk[0], &chunk[1..]);
        let mut cur = vec![0u8; row];
        for i in 0..line.len() {
            let a = if i >= bpp { cur[i - bpp] } else { 0 } as i32;
            let b = prev[i] as i32;
            let c = if i >= bpp { prev[i - bpp] } else { 0 } as i32;
            let x = line[i] as i32;
            cur[i] = match kind {
                0 => x,
                1 => x + a,
                2 => x + b,
                3 => x + (a + b) / 2,
                4 => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    x + if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
                }
                _ => x,
            } as u8;
        }
        out.extend_from_slice(&cur[..line.len()]);
        prev = cur;
    }
    Some(out)
}

/// LZW, as PDF has it (and TIFF): 9- to 12-bit codes, 256 clears, 257 ends; `early`: codes grow a code sooner.
fn lzw(data: &[u8], early: bool) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    let mut table: Vec<(u16, u8, u16)> = Vec::with_capacity(4096); // (prefix, last byte, length)
    let reset = |t: &mut Vec<(u16, u8, u16)>| {
        t.clear();
        for i in 0..256u16 {
            t.push((u16::MAX, i as u8, 1));
        }
        t.push((0, 0, 0));
        t.push((0, 0, 0));
    };
    reset(&mut table);
    let (mut bits, mut acc, mut have) = (9u32, 0u32, 0u32);
    let mut prev: Option<u16> = None;
    let mut i = 0;
    let entry = |t: &[(u16, u8, u16)], code: u16, out: &mut Vec<u8>| -> Option<u8> {
        let len = t.get(code as usize)?.2 as usize;
        let start = out.len();
        out.resize(start + len, 0);
        let mut c = code;
        for k in (0..len).rev() {
            let (p, b, _) = t[c as usize];
            out[start + k] = b;
            c = p;
        }
        Some(out[start])
    };
    loop {
        while have < bits {
            let Some(&b) = data.get(i) else { return Some(out) };
            acc = acc << 8 | b as u32;
            have += 8;
            i += 1;
        }
        let code = ((acc >> (have - bits)) & ((1 << bits) - 1)) as u16;
        have -= bits;
        match code {
            256 => {
                reset(&mut table);
                bits = 9;
                prev = None;
                continue;
            }
            257 => return Some(out),
            _ => {}
        }
        if out.len() > MOST {
            return None;
        }
        match prev {
            None => {
                entry(&table, code, &mut out)?;
            }
            Some(p) => {
                if code as usize > table.len() || p as usize >= table.len() {
                    return Some(out);
                }
                let first = if (code as usize) < table.len() {
                    entry(&table, code, &mut out)?
                } else {
                    // (The code being made: the last string and its own first byte.)
                    let f = entry(&table, p, &mut out)?;
                    out.push(f);
                    f
                };
                if table.len() < 4096 {
                    let len = table[p as usize].2 + 1;
                    table.push((p, first, len));
                }
            }
        }
        prev = Some(code);
        let grow = table.len() + if early { 1 } else { 0 };
        bits = match grow {
            n if n >= 2048 => 12,
            n if n >= 1024 => 11,
            n if n >= 512 => 10,
            _ => 9,
        };
    }
}

fn ascii_hex(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2);
    let mut high: Option<u8> = None;
    for &b in data {
        if b == b'>' {
            break;
        }
        let Some(d) = (b as char).to_digit(16) else { continue };
        match high.take() {
            Some(h) => out.push(h * 16 + d as u8),
            None => high = Some(d as u8),
        }
    }
    if let Some(h) = high {
        out.push(h * 16);
    }
    out
}

fn ascii85(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() * 4 / 5);
    let (mut group, mut n) = (0u64, 0usize);
    let data = data.strip_prefix(b"<~").unwrap_or(data);
    for &b in data {
        match b {
            b'~' => break,
            b'z' if n == 0 => {
                if out.len() > MOST {
                    return None;
                }
                out.extend_from_slice(&[0, 0, 0, 0]);
            }
            b'!'..=b'u' => {
                group = group * 85 + (b - b'!') as u64;
                n += 1;
                if n == 5 {
                    out.extend_from_slice(&(group as u32).to_be_bytes());
                    group = 0;
                    n = 0;
                }
            }
            _ => {}
        }
    }
    if n > 1 {
        for _ in n..5 {
            group = group * 85 + 84;
        }
        out.extend_from_slice(&(group as u32).to_be_bytes()[..n - 1]);
    }
    Some(out)
}

fn run_length(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&len) = data.get(i) {
        i += 1;
        match len {
            128 => break,
            0..=127 => {
                let n = len as usize + 1;
                out.extend_from_slice(data.get(i..i + n)?);
                i += n;
            }
            _ => {
                let b = *data.get(i)?;
                i += 1;
                out.extend(core::iter::repeat_n(b, 257 - len as usize));
            }
        }
        if out.len() > MOST {
            return None;
        }
    }
    Some(out)
}
