//! Pictures, read: JPEG (baseline and progressive, as cameras, phones and the web make them), PNG (every colour type
//! and depth, interlaced or not), GIF (its first frame), WebP (lossy and lossless, with alpha; an animation's first
//! frame: webp.rs) and SVG (drawn: svg.rs), as RGB pixels on a background (where
//! they're transparent; or with their alpha, `decode_alpha`, to be drawn over what's behind them), made to fit a box: shrunk by averaging (never enlarged), and a photo turned the way its camera says
//! (its EXIF orientation). A big JPEG is decoded at a half, a quarter or an eighth of its size when that's still
//! enough for the box, which takes a fraction of the memory and time. Decoding keeps to the memory it's given.
//!
//! And JPEG, written (encode.rs): a picture made smaller for Claude, written exactly as libjpeg writes it. And pixels
//! resampled as Pillow does it (`resample`), as Qwen's processor resizes a picture for its model.
//!
//! Integer arithmetic (but for the resampler's weights, worked out once a picture). Every number read is checked
//! before it's used, so a damaged picture gives an error, never a crash.

#![no_std]

extern crate alloc;

mod gif;
mod jpeg;
mod png;
mod vp8;
mod vp8l;
mod webp;
mod webp_tables;
mod encode;
pub mod draw;
pub mod svg;

pub use encode::jpeg as encode_jpeg;
pub use encode::png as encode_png;

#[cfg(test)]
mod tests;

use alloc::vec;
use alloc::vec::Vec;

/// A picture: its pixels, RGB, row by row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Not a JPEG, PNG, GIF, WebP or SVG.
    NotAPicture,
    /// Something in it doesn't add up.
    Damaged,
    /// A kind of JPEG this doesn't read (lossless, arithmetic-coded, 12-bit, hierarchical).
    Unsupported(&'static str),
    /// More pixels than it reads at once.
    TooBig,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Jpeg,
    Png,
    Gif,
    Webp,
    /// Drawn (svg.rs) rather than decoded: at twice its own size, if that fits the box (sharper on a screen of more
    /// than one pixel to a CSS pixel).
    Svg,
}

/// What `data` is, from its first bytes.
pub fn format(data: &[u8]) -> Option<Format> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Format::Jpeg)
    } else if data.starts_with(png::SIGNATURE) {
        Some(Format::Png)
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some(Format::Gif)
    } else if webp::is_webp(data) {
        Some(Format::Webp)
    } else if svg::is_svg(data) {
        Some(Format::Svg)
    } else {
        None
    }
}

/// The most pixels decoded at once, before shrinking (a JPEG decoded smaller counts as its smaller size).
const MOST_PIXELS: u64 = 40 << 20;

/// The most bytes a pixel decoded can take while it's decoded, by format: a JPEG's three planes and the result; a PNG's
/// rows as stored (up to 8 bytes a pixel, at 16 bits with transparency) and the result; a GIF's indexes and the result;
/// a WebP's pixels as decoded (lossless: 4 bytes, twice when a palette's undone; lossy: its planes) and the result.
fn per_pixel(f: Format) -> u64 {
    match f {
        Format::Jpeg => 8,
        Format::Png => 12,
        Format::Gif => 5,
        Format::Webp => 12,
        Format::Svg => 24,
    }
}

/// The picture in `data`, turned as its camera says, on `background` where it's transparent, and shrunk (if need be)
/// to fit `width` × `height`, its proportions kept.
pub fn decode(data: &[u8], width: u32, height: u32, background: [u8; 3]) -> Result<Picture, Error> {
    decode_within(data, width, height, background, u64::MAX)
}

/// The same, using at most about `memory` bytes as it's decoded: a picture that would take more is `TooBig` (so a
/// kernel with a heap of its own size can say so, rather than run out).
pub fn decode_within(data: &[u8], width: u32, height: u32, background: [u8; 3], memory: u64) -> Result<Picture, Error> {
    decode_turned(data, width, height, background, memory, true)
}

/// The same, but as it's stored: not turned as a camera's EXIF says (a picture inside a PDF is placed by the PDF,
/// whatever its own JPEG says).
pub fn decode_stored(data: &[u8], width: u32, height: u32, background: [u8; 3], memory: u64) -> Result<Picture, Error> {
    decode_turned(data, width, height, background, memory, false)
}

fn decode_turned(data: &[u8], width: u32, height: u32, background: [u8; 3], memory: u64, upright: bool) -> Result<Picture, Error> {
    let (mut p, alpha) = decode_planes(data, width, height, memory, upright)?;
    if let Some(a) = alpha {
        for (px, &a) in p.rgb.chunks_exact_mut(3).zip(&a) {
            let c = blend([px[0], px[1], px[2]], a, background);
            px.copy_from_slice(&c);
        }
    }
    Ok(p)
}

/// A picture with its transparency (for drawing it over what's behind it): its colours, as they are (not mixed with
/// a background), and its alpha (a byte a pixel, 0 clear), None where it's opaque throughout; turned as its camera
/// says, and shrunk (if need be) to fit `width` × `height`, as `decode_within` does.
pub fn decode_alpha(data: &[u8], width: u32, height: u32, memory: u64) -> Result<(Picture, Option<Vec<u8>>), Error> {
    decode_planes(data, width, height, memory, true)
}

fn decode_planes(data: &[u8], width: u32, height: u32, memory: u64, upright: bool) -> Result<(Picture, Option<Vec<u8>>), Error> {
    let kind = format(data).ok_or(Error::NotAPicture)?;
    let most = (memory / per_pixel(kind)).min(MOST_PIXELS);
    let (p, alpha, full) = match kind {
        Format::Png => {
            let (p, a) = png::decode(data, most)?;
            let full = (p.width, p.height);
            (p, a, full)
        }
        Format::Gif => {
            let (p, a) = gif::decode(data, most)?;
            let full = (p.width, p.height);
            (p, a, full)
        }
        Format::Webp => {
            let (p, a) = webp::decode(data, most)?;
            let full = (p.width, p.height);
            (p, a, full)
        }
        Format::Svg => {
            let (w0, h0) = svg::size(data).ok_or(Error::Damaged)?;
            let (mut w, mut h) = fitted(w0.saturating_mul(2), h0.saturating_mul(2), width, height);
            while w as u64 * h as u64 > most && w > 1 && h > 1 {
                (w, h) = ((w / 2).max(1), (h / 2).max(1));
            }
            let (rgba, w, h) = svg::render(data, w, h).ok_or(Error::Damaged)?;
            let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
            let mut a = Vec::with_capacity(w as usize * h as usize);
            for px in rgba.chunks_exact(4) {
                rgb.extend_from_slice(&px[..3]);
                a.push(px[3]);
            }
            (Picture { width: w, height: h, rgb }, Some(a), (w, h))
        }
        Format::Jpeg if upright => {
            // (Decoded perhaps at a fraction of its size: the size it fits is worked out from its whole size.)
            let (p, orientation, (w, h)) = jpeg::decode(data, width, height, most, memory)?;
            (orient(p, orientation), None, if orientation >= 5 { (h, w) } else { (w, h) })
        }
        Format::Jpeg => {
            // (The box as the decoder takes it, for a picture it would turn: it turns the box back.)
            let turns = jpeg::orientation(data) >= 5;
            let (bw, bh) = if turns { (height, width) } else { (width, height) };
            let (p, _, full) = jpeg::decode(data, bw, bh, most, memory)?;
            (p, None, full)
        }
    };
    let (w, h) = fitted(full.0, full.1, width, height);
    let Some(alpha) = alpha else { return Ok((shrink_to(&p, w, h), None)) };
    if (w, h) == (p.width, p.height) {
        return Ok((p, Some(alpha)));
    }
    // Shrunk with its colours weighted by their alpha (a clear pixel's colour counts for nothing), then as they are.
    let mut pre = p.rgb.clone();
    for (px, &a) in pre.chunks_exact_mut(3).zip(&alpha) {
        for c in px.iter_mut() {
            *c = ((*c as u32 * a as u32 + 127) / 255) as u8;
        }
    }
    let shrunk = shrink_to(&Picture { width: p.width, height: p.height, rgb: pre }, w, h);
    let grey: Vec<u8> = alpha.iter().flat_map(|&a| [a, a, a]).collect();
    let a = shrink_to(&Picture { width: p.width, height: p.height, rgb: grey }, w, h);
    let a: Vec<u8> = a.rgb.chunks_exact(3).map(|c| c[0]).collect();
    let mut rgb = shrunk.rgb;
    for (px, &a) in rgb.chunks_exact_mut(3).zip(&a) {
        for c in px.iter_mut() {
            *c = if a == 0 { 0 } else { ((*c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
        }
    }
    Ok((Picture { width: shrunk.width, height: shrunk.height, rgb }, Some(a)))
}

/// A picture as a model that sees it reads it, `size` saying at what size it reads a picture of a given size (width,
/// height): decoded at about twice that (a big JPEG at a fraction of its size), turned as its camera says, on white
/// where it's transparent (as Qwen's processor puts it), then resampled to that size as the processor does it.
pub fn decode_for(data: &[u8], size: &dyn Fn(usize, usize) -> (usize, usize), memory: u64) -> Result<Picture, Error> {
    let (w0, h0) = self::size(data).ok_or(Error::Damaged)?;
    let (tw, th) = size(w0 as usize, h0 as usize);
    let side = (2 * tw.max(th)).clamp(1, u32::MAX as usize) as u32;
    let p = decode_within(data, side, side, [255, 255, 255], memory)?;
    let (tw, th) = size(p.width as usize, p.height as usize);
    let rgb = resample(&p.rgb, p.width as usize, p.height as usize, tw, th);
    Ok(Picture { width: tw as u32, height: th as u32, rgb })
}

/// A colour with an alpha (0 to 255) on the background.
pub(crate) fn blend(c: [u8; 3], alpha: u8, background: [u8; 3]) -> [u8; 3] {
    let a = alpha as u32;
    let mix = |f: u8, b: u8| ((f as u32 * a + b as u32 * (255 - a) + 127) / 255) as u8;
    [mix(c[0], background[0]), mix(c[1], background[1]), mix(c[2], background[2])]
}

/// The size a picture of `w` × `h` is made to fit `max_w` × `max_h`: as it is, or smaller, its proportions kept
/// (each side at least a pixel).
pub fn fitted(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w <= max_w && h <= max_h {
        return (w, h);
    }
    // Whichever side is tighter decides.
    if w as u64 * max_h as u64 >= h as u64 * max_w as u64 {
        (max_w.max(1), ((h as u64 * max_w as u64 / w as u64) as u32).max(1))
    } else {
        (((w as u64 * max_h as u64 / h as u64) as u32).max(1), max_h.max(1))
    }
}

/// `p` shrunk to fit `max_w` × `max_h`: each new pixel the average of the old ones it covers, counted by how much of
/// each it covers (in integers: one side's pixels are cut into the other side's count of parts).
pub fn shrink(p: &Picture, max_w: u32, max_h: u32) -> Picture {
    let (w, h) = fitted(p.width, p.height, max_w, max_h);
    shrink_to(p, w, h)
}

/// `p` shrunk to `w` × `h` (each at most what it is).
fn shrink_to(p: &Picture, w: u32, h: u32) -> Picture {
    let (w, h) = (w.clamp(1, p.width.max(1)), h.clamp(1, p.height.max(1)));
    if (w, h) == (p.width, p.height) {
        return p.clone();
    }
    let (sw, sh) = (p.width as usize, p.height as usize);
    let (dw, dh) = (w as usize, h as usize);
    // Across: each row to `dw` pixels.
    let mut across = vec![0u8; dw * sh * 3];
    let columns = spans(sw, dw);
    for y in 0..sh {
        let src = &p.rgb[y * sw * 3..(y + 1) * sw * 3];
        let dst = &mut across[y * dw * 3..(y + 1) * dw * 3];
        for (x, span) in columns.iter().enumerate() {
            let mut sum = [0u32; 3];
            for &(i, weight) in span {
                for c in 0..3 {
                    sum[c] += src[i * 3 + c] as u32 * weight;
                }
            }
            for c in 0..3 {
                dst[x * 3 + c] = ((sum[c] + sw as u32 / 2) / sw as u32) as u8;
            }
        }
    }
    // Down: each column to `dh` pixels.
    let mut rgb = vec![0u8; dw * dh * 3];
    let rows = spans(sh, dh);
    for (y, span) in rows.iter().enumerate() {
        let mut sums = vec![0u32; dw * 3];
        for &(i, weight) in span {
            let row = &across[i * dw * 3..(i + 1) * dw * 3];
            for (s, &v) in sums.iter_mut().zip(row) {
                *s += v as u32 * weight;
            }
        }
        for (d, s) in rgb[y * dw * 3..(y + 1) * dw * 3].iter_mut().zip(&sums) {
            *d = ((s + sh as u32 / 2) / sh as u32) as u8;
        }
    }
    Picture { width: w, height: h, rgb }
}

/// For each of `to` new pixels, the old ones (of `from`) it covers and by how much, in parts: old pixel i is
/// [i·to, (i+1)·to), new pixel x is [x·from, (x+1)·from), and each new one's weights add up to `from`.
fn spans(from: usize, to: usize) -> Vec<Vec<(usize, u32)>> {
    (0..to)
        .map(|x| {
            let (start, end) = (x * from, (x + 1) * from);
            (start / to..end.div_ceil(to))
                .map(|i| {
                    let (a, b) = (i * to, (i + 1) * to);
                    (i, (b.min(end) - a.max(start)) as u32)
                })
                .filter(|&(_, w)| w > 0)
                .collect()
        })
        .collect()
}

/// A picture turned as EXIF's orientation says (1: as it is; 2: mirrored; 3: turned half round; 4: mirrored top to
/// bottom; 5: mirrored along its diagonal; 6: turned a quarter clockwise; 7: mirrored along its other diagonal; 8: turned
/// a quarter anticlockwise).
pub fn orient(p: Picture, orientation: u8) -> Picture {
    if !(2..=8).contains(&orientation) {
        return p;
    }
    let (w, h) = (p.width as usize, p.height as usize);
    let swap = orientation >= 5;
    let (nw, nh) = if swap { (h, w) } else { (w, h) };
    let mut rgb = vec![0u8; nw * nh * 3];
    for y in 0..nh {
        for x in 0..nw {
            // Where the new pixel comes from.
            let (sx, sy) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                _ => (w - 1 - y, x),
            };
            rgb[(y * nw + x) * 3..][..3].copy_from_slice(&p.rgb[(sy * w + sx) * 3..][..3]);
        }
    }
    Picture { width: nw as u32, height: nh as u32, rgb }
}

/// A picture's own size as CSS sizes it: its width and height where it has them (a decoded picture always; an SVG where
/// they're lengths), and its ratio (width over height) if it has one (an SVG's from its viewBox, without them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Intrinsic {
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub ratio: Option<f32>,
}

impl Intrinsic {
    /// A picture of pixels: its size, and their ratio.
    pub fn sized(w: u32, h: u32) -> Intrinsic {
        Intrinsic { width: Some(w as f32), height: Some(h as f32), ratio: (w > 0 && h > 0).then(|| w as f32 / h as f32) }
    }
}

/// A picture's own size, as CSS sizes it, as it's shown: a photo turned as its camera says (as Chrome shows it).
pub fn intrinsic(data: &[u8]) -> Option<Intrinsic> {
    match format(data)? {
        Format::Svg => svg::intrinsic(data).map(|(width, height, ratio)| Intrinsic { width, height, ratio }),
        Format::Jpeg if jpeg::orientation(data) >= 5 => size(data).map(|(w, h)| Intrinsic::sized(h, w)),
        _ => size(data).map(|(w, h)| Intrinsic::sized(w, h)),
    }
}

/// The size a picture is shown at, from its header: a photo turned as its camera says; an SVG's own (its viewBox's,
/// or 300 by 150, where it gives none).
pub fn shown_size(data: &[u8]) -> Option<(u32, u32)> {
    let (w, h) = size(data)?;
    Some(if format(data) == Some(Format::Jpeg) && jpeg::orientation(data) >= 5 { (h, w) } else { (w, h) })
}

/// The size a picture says it is (before it's turned), from its header: for checking it, without decoding it.
pub fn size(data: &[u8]) -> Option<(u32, u32)> {
    match format(data)? {
        Format::Png => png::size(data),
        Format::Gif => gif::size(data),
        Format::Webp => webp::size(data),
        Format::Jpeg => jpeg::size(data),
        Format::Svg => svg::size(data),
    }
}

/// Pixels resized by bicubic resampling with antialiasing, as Pillow does it (and torchvision for 8-bit pictures):
/// across, then down, each pass's weights (a = -0.5, spread out as much as it shrinks) in whole numbers of 2^-22 and
/// its result rounded to bytes. RGB, row after row. (As Qwen's processor resizes a picture for its model to see:
/// os/model's vision part, and a PDF's scanned page made a picture of, os/pdf.)
pub fn resample(rgb: &[u8], width: usize, height: usize, to_w: usize, to_h: usize) -> Vec<u8> {
    let across = if to_w == width { rgb.to_vec() } else { pass(rgb, width, height, to_w, true) };
    if to_h == height { across } else { pass(&across, to_w, height, to_h, false) }
}

/// One pass of `resample`: across (each row from `size` to `to` pixels) or down (each column).
fn pass(src: &[u8], width: usize, height: usize, to: usize, across: bool) -> Vec<u8> {
    const BITS: u32 = 22;
    let size = if across { width } else { height };
    let scale = size as f64 / to as f64;
    let filter_scale = scale.max(1.0);
    let support = 2.0 * filter_scale;
    let cubic = |x: f64| -> f64 {
        let a = -0.5;
        let x = x.abs();
        if x < 1.0 {
            ((a + 2.0) * x - (a + 3.0)) * x * x + 1.0
        } else if x < 2.0 {
            (((x - 5.0) * x + 8.0) * x - 4.0) * a
        } else {
            0.0
        }
    };
    // Each output pixel's first source pixel and its whole-number weights.
    let mut taps: Vec<(usize, Vec<i32>)> = Vec::with_capacity(to);
    for o in 0..to {
        let center = (o as f64 + 0.5) * scale;
        let lo = ((center - support + 0.5) as isize).max(0) as usize;
        let hi = (((center + support + 0.5) as isize).max(0) as usize).min(size);
        let mut ws: Vec<f64> = (lo..hi).map(|i| cubic((i as f64 - center + 0.5) / filter_scale)).collect();
        let total: f64 = ws.iter().sum();
        if total != 0.0 {
            for w in ws.iter_mut() {
                *w /= total;
            }
        }
        let ws: Vec<i32> = ws.iter().map(|&w| (w * (1u32 << BITS) as f64 + if w < 0.0 { -0.5 } else { 0.5 }) as i32).collect();
        taps.push((lo, ws));
    }
    let (out_w, out_h) = if across { (to, height) } else { (width, to) };
    let mut out = vec![0u8; out_w * out_h * 3];
    let round = 1i64 << (BITS - 1);
    for y in 0..out_h {
        for x in 0..out_w {
            let (o, from_fixed) = if across { (x, y) } else { (y, x) };
            let (lo, ws) = &taps[o];
            for c in 0..3 {
                let mut sum = round;
                for (k, &w) in ws.iter().enumerate() {
                    let i = lo + k;
                    let at = if across { (from_fixed * width + i) * 3 + c } else { (i * width + from_fixed) * 3 + c };
                    sum += src[at] as i64 * w as i64;
                }
                out[(y * out_w + x) * 3 + c] = (sum >> BITS).clamp(0, 255) as u8;
            }
        }
    }
    out
}
