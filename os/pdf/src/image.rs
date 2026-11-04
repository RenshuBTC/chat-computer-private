//! A page's pictures, made one picture of the page, for a model to see a page whose words are pictures (a scanned
//! one): the images its contents draw (image XObjects, in forms too, and inline images), each where the page's
//! matrices put it, on white, at the size the caller chooses (from the page's own size at its pictures' resolution).
//! JPEG's pictures, raw samples' (through any of the stream filters; any depth; gray, RGB, CMYK, indexed, ICC-based,
//! calibrated), fax's (CCITT: ccitt.rs) and stencil masks' (in the colour they're filled with), a soft mask's
//! transparency. Each picture is made the size it's drawn at with Pillow's bicubic resampling (picture::resample), as
//! Qwen's processor resizes a picture. Not drawn: the page's text and drawings (a scanned page has none to see), and
//! JPEG 2000's and JBIG2's pictures (said to be missing).

use crate::ccitt;
use crate::file::File;
use crate::filter::Codec;
use crate::lex::{Dict, Lexer, Obj};
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;

/// A page's pictures, made one: its pixels (RGB, row after row), how many pictures were drawn, and the kinds of
/// those that couldn't be ("JPEG 2000", "JBIG2").
pub struct PagePicture {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
    pub drawn: usize,
    pub missing: Vec<&'static str>,
}

/// The most pictures drawn on a page, the most pixels a picture may have, the finest a page is made at (600 dots an
/// inch) and the coarsest (72), how deep forms may be drawn in forms, and the most of a page's operations followed.
const MOST_IMAGES: usize = 1024;
const MOST_PIXELS: usize = 100 << 20;
const MOST_DENSITY: f64 = 600.0 / 72.0;
const LEAST_DENSITY: f64 = 1.0;
const MOST_FORMS: u32 = 8;
const MOST_OPS: usize = 4_000_000;
/// The biggest canvas made (a side), whatever the caller asks.
const MOST_SIDE: usize = 16_384;

type Matrix = [f64; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `a` then `b`.
fn times(a: &Matrix, b: &Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

/// A picture the page draws: where (the matrix that takes the unit square there), what, the colour a stencil mask is
/// filled with, and the resources of the contents that draw it (a colour space's name is looked up there).
struct Placed {
    at: Matrix,
    source: Source,
    fill: [u8; 3],
    resources: Rc<Dict>,
}

enum Source {
    /// An image XObject (the stream).
    Object(Obj),
    /// An inline image: its dictionary (its keys and names written out in full) and data.
    Inline(Dict, Vec<u8>),
}

#[derive(Clone, Copy)]
struct Graphics {
    ctm: Matrix,
    fill: [u8; 3],
}

/// Page `page`'s pictures (from 0) as one picture, at the size `size` gives for the page's own size in pixels (its
/// pictures' finest resolution, between 72 and 600 dots an inch), using at most about `memory` bytes for a picture as
/// it's decoded. None if the page draws no pictures, or there's no such page.
pub fn page_picture(data: &[u8], page: usize, size: &dyn Fn(usize, usize) -> (usize, usize), memory: u64) -> Result<Option<PagePicture>, crate::Error> {
    if crate::file::find(&data[..data.len().min(1024)], b"%PDF-").is_none() {
        return Err(crate::Error::NotPdf);
    }
    let f = File::open(data);
    if f.locked() {
        return Err(crate::Error::Encrypted);
    }
    let catalog = f.get_in(&f.trailer, "Root");
    let catalog = catalog.dict().cloned().unwrap_or_default();
    let pages = crate::pages(&f, &catalog);
    if pages.is_empty() {
        return Err(crate::Error::Damaged);
    }
    let Some((page_dict, resources)) = pages.get(page) else { return Ok(None) };
    // The page's box (its crop box, else its media box) and how it's turned.
    let boxed = |key: &str| -> Option<[f64; 4]> {
        let a = inherited(&f, page_dict, key);
        let n: Vec<f64> = a.array()?.iter().filter_map(|o| f.resolve(o).num()).collect();
        if n.len() < 4 {
            return None;
        }
        let b = [n[0].min(n[2]), n[1].min(n[3]), n[0].max(n[2]), n[1].max(n[3])];
        (b[2] - b[0] > 1.0 && b[3] - b[1] > 1.0 && b.iter().all(|v| v.is_finite())).then_some(b)
    };
    let [x0, y0, x1, y1] = boxed("CropBox").or_else(|| boxed("MediaBox")).unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let turn = inherited(&f, page_dict, "Rotate").int().unwrap_or(0).rem_euclid(360) / 90 % 4;
    // What it draws.
    let mut placed = Vec::new();
    let mut ops = 0;
    let contents = crate::contents(&f, page_dict);
    collect(&f, &contents, &Rc::new(resources.clone()), IDENTITY, 0, &mut placed, &mut ops);
    if placed.is_empty() {
        return Ok(None);
    }
    // How fine the page is made: its pictures' finest (their pixels over the length they're drawn at).
    let mut density = LEAST_DENSITY;
    for p in &placed {
        if let Some((w, h)) = dimensions(&f, &p.source) {
            let (along, down) = (hypot(p.at[0], p.at[1]), hypot(p.at[2], p.at[3]));
            if along > 0.5 {
                density = density.max(w as f64 / along);
            }
            if down > 0.5 {
                density = density.max(h as f64 / down);
            }
        }
    }
    let density = density.min(MOST_DENSITY);
    let (bw, bh) = if turn % 2 == 1 { (y1 - y0, x1 - x0) } else { (x1 - x0, y1 - y0) };
    let (nw, nh) = (((bw * density) as usize).clamp(1, MOST_SIDE), ((bh * density) as usize).clamp(1, MOST_SIDE));
    let (width, height) = size(nw, nh);
    let (width, height) = (width.clamp(1, MOST_SIDE), height.clamp(1, MOST_SIDE));
    // From the page's space to the picture's pixels (turned as the page is).
    let (sx, sy) = (width as f64 / bw, height as f64 / bh);
    let shown: Matrix = match turn {
        1 => [0.0, sy, sx, 0.0, -y0 * sx, -x0 * sy],
        2 => [-sx, 0.0, 0.0, sy, x1 * sx, -y0 * sy],
        3 => [0.0, -sy, -sx, 0.0, y1 * sx, x1 * sy],
        _ => [sx, 0.0, 0.0, -sy, -x0 * sx, y1 * sy],
    };
    let mut out = PagePicture { width, height, rgb: vec![255u8; width * height * 3], drawn: 0, missing: Vec::new() };
    for p in &placed {
        let t = times(&p.at, &shown);
        let Some((w, h)) = dimensions(&f, &p.source) else { continue };
        // From the picture's own pixels (its first row at the top) to the page picture's.
        let px: Matrix = [t[0] / w as f64, t[1] / w as f64, -t[2] / h as f64, -t[3] / h as f64, t[2] + t[4], t[3] + t[5]];
        if px.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
            continue;
        }
        // The size it's drawn at, along its own rows and down its columns.
        let (along, down) = (ceil(hypot(px[0], px[1]) * w as f64), ceil(hypot(px[2], px[3]) * h as f64));
        let want = (along.min(MOST_SIDE as f64) as usize, down.min(MOST_SIDE as f64) as usize);
        if want.0 == 0 || want.1 == 0 {
            continue;
        }
        match decoded(&f, &p.source, &p.resources, p.fill, want, memory) {
            Ok(d) => {
                // (Decoded perhaps smaller than it is: its pixels are bigger, then.)
                let (fw, fh) = (w as f64 / d.width as f64, h as f64 / d.height as f64);
                draw(&mut out, &d, &[px[0] * fw, px[1] * fw, px[2] * fh, px[3] * fh, px[4], px[5]]);
                out.drawn += 1;
            }
            Err(Some(kind)) => {
                if !out.missing.contains(&kind) {
                    out.missing.push(kind);
                }
            }
            Err(None) => {}
        }
    }
    Ok(Some(out))
}

/// A page's attribute, its own or the nearest it inherits (MediaBox, CropBox, Rotate).
fn inherited(f: &File, page: &Dict, key: &str) -> Obj {
    let mut node = page.clone();
    for _ in 0..64 {
        let v = f.get_in(&node, key);
        if v != Obj::Null {
            return v;
        }
        let Obj::Dict(parent) = f.get_in(&node, "Parent") else { break };
        node = parent;
    }
    Obj::Null
}

fn hypot(a: f64, b: f64) -> f64 {
    let s = a * a + b * b;
    if s <= 0.0 {
        return 0.0;
    }
    let mut r = f64::from_bits((s.to_bits() >> 1) + (1023u64 << 51));
    for _ in 0..6 {
        r = 0.5 * (r + s / r);
    }
    r
}

/// The pictures a content stream draws, where (`ctm`: where it's drawn itself), in order.
fn collect(f: &File, contents: &[u8], resources: &Rc<Dict>, ctm: Matrix, depth: u32, out: &mut Vec<Placed>, ops: &mut usize) {
    let mut g = Graphics { ctm, fill: [0, 0, 0] };
    let mut saved: Vec<Graphics> = Vec::new();
    let mut args: Vec<Obj> = Vec::new();
    let mut lex = Lexer::new(contents, 0);
    loop {
        let at = lex.pos;
        let Some(o) = lex.object() else { break };
        if lex.pos == at {
            break;
        }
        let Obj::Op(op) = o else {
            if args.len() < 64 {
                args.push(o);
            }
            continue;
        };
        *ops += 1;
        if *ops > MOST_OPS || out.len() >= MOST_IMAGES {
            break;
        }
        let num = |i: usize| args.get(i).and_then(Obj::num).unwrap_or(0.0);
        let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        match op.as_slice() {
            b"q" => {
                if saved.len() < 64 {
                    saved.push(g);
                }
            }
            b"Q" => {
                if let Some(s) = saved.pop() {
                    g = s;
                }
            }
            b"cm" if args.len() >= 6 => {
                let m = [num(0), num(1), num(2), num(3), num(4), num(5)];
                if m.iter().all(|v| v.is_finite()) {
                    g.ctm = times(&m, &g.ctm);
                }
            }
            b"g" => g.fill = [byte(num(0)); 3],
            b"rg" => g.fill = [byte(num(0)), byte(num(1)), byte(num(2))],
            b"k" => g.fill = cmyk(num(0), num(1), num(2), num(3)),
            b"sc" | b"scn" => {
                let n: Vec<f64> = args.iter().filter_map(Obj::num).collect();
                match n.len() {
                    1 => g.fill = [byte(n[0]); 3],
                    3 => g.fill = [byte(n[0]), byte(n[1]), byte(n[2])],
                    4 => g.fill = cmyk(n[0], n[1], n[2], n[3]),
                    _ => {}
                }
            }
            b"cs" => g.fill = [0, 0, 0],
            b"Do" => {
                if let Some(name) = args.first().and_then(Obj::name) {
                    let objects = f.get_in(resources, "XObject");
                    let entry = objects.dict().and_then(|d| d.0.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.clone()));
                    if let Some(entry) = entry {
                        let x = f.resolve(&entry);
                        if let Obj::Stream(dict, _) = &x {
                            match f.get_in(dict, "Subtype").name() {
                                Some(b"Image") => out.push(Placed { at: g.ctm, source: Source::Object(entry), fill: g.fill, resources: resources.clone() }),
                                Some(b"Form") if depth < MOST_FORMS => {
                                    let m: Vec<f64> = f.get_in(dict, "Matrix").array().map(|a| a.iter().filter_map(|o| f.resolve(o).num()).collect()).unwrap_or_default();
                                    let m = if m.len() == 6 { [m[0], m[1], m[2], m[3], m[4], m[5]] } else { IDENTITY };
                                    let own = match f.get_in(dict, "Resources") {
                                        Obj::Dict(r) => Rc::new(r),
                                        _ => resources.clone(),
                                    };
                                    if let Some(data) = f.stream(&x) {
                                        collect(f, &data, &own, times(&m, &g.ctm), depth + 1, out, ops);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            b"BI" => {
                if let Some((dict, data)) = inline_image(&mut lex) {
                    out.push(Placed { at: g.ctm, source: Source::Inline(dict, data), fill: g.fill, resources: resources.clone() });
                }
            }
            _ => {}
        }
        args.clear();
    }
}

fn cmyk(c: f64, m: f64, y: f64, k: f64) -> [u8; 3] {
    let k = 1.0 - k.clamp(0.0, 1.0);
    let v = |x: f64| ((1.0 - x.clamp(0.0, 1.0)) * k * 255.0 + 0.5) as u8;
    [v(c), v(m), v(y)]
}

/// An inline image (after its "BI"): its dictionary, its keys and names written out in full, and its data, up to its
/// "EI".
fn inline_image(lex: &mut Lexer) -> Option<(Dict, Vec<u8>)> {
    let mut dict = Dict::default();
    let mut key: Option<Vec<u8>> = None;
    loop {
        let o = lex.object()?;
        if o == Obj::Op(b"ID".to_vec()) {
            break;
        }
        match key.take() {
            None => key = Some(o.name()?.to_vec()),
            Some(k) => {
                let long: &[u8] = match k.as_slice() {
                    b"BPC" => b"BitsPerComponent",
                    b"CS" => b"ColorSpace",
                    b"D" => b"Decode",
                    b"DP" => b"DecodeParms",
                    b"F" => b"Filter",
                    b"H" => b"Height",
                    b"IM" => b"ImageMask",
                    b"I" => b"Interpolate",
                    b"W" => b"Width",
                    b"L" => b"Length",
                    other => other,
                };
                let value = match &o {
                    Obj::Name(n) if long == b"ColorSpace" => Obj::Name(match n.as_slice() {
                        b"G" => b"DeviceGray".to_vec(),
                        b"RGB" => b"DeviceRGB".to_vec(),
                        b"CMYK" => b"DeviceCMYK".to_vec(),
                        b"I" => b"Indexed".to_vec(),
                        other => other.to_vec(),
                    }),
                    Obj::Array(a) if long == b"ColorSpace" && a.first().and_then(Obj::name) == Some(b"I") => {
                        let mut a = a.clone();
                        a[0] = Obj::Name(b"Indexed".to_vec());
                        Obj::Array(a)
                    }
                    _ => o.clone(),
                };
                dict.0.push((long.to_vec(), value));
            }
        }
        if dict.0.len() > 64 {
            return None;
        }
    }
    // The data: from after the one white byte after "ID"; its length known when it isn't filtered, else up to an
    // "EI" on its own.
    let d = lex.data;
    let start = (lex.pos + 1).min(d.len());
    let filtered = dict.get("Filter").is_some();
    let known = (!filtered).then(|| {
        let w = dict.get("Width").and_then(Obj::int).unwrap_or(0).max(0) as usize;
        let h = dict.get("Height").and_then(Obj::int).unwrap_or(0).max(0) as usize;
        let mask = matches!(dict.get("ImageMask"), Some(Obj::Bool(true)));
        let bpc = if mask { 1 } else { dict.get("BitsPerComponent").and_then(Obj::int).unwrap_or(8).clamp(1, 16) as usize };
        let n = match dict.get("ColorSpace") {
            Some(Obj::Name(n)) if n == b"DeviceRGB" => 3,
            Some(Obj::Name(n)) if n == b"DeviceCMYK" => 4,
            _ => 1,
        };
        (w * n * bpc).div_ceil(8).saturating_mul(h)
    });
    let end = match known {
        Some(len) if start + len <= d.len() && d[start + len..].iter().take(8).any(|&b| b == b'E') => start + len,
        _ => {
            let mut i = start;
            loop {
                if i + 2 > d.len() {
                    break d.len();
                }
                if &d[i..i + 2] == b"EI"
                    && i > 0
                    && crate::lex::is_white(d[i - 1])
                    && d.get(i + 2).is_none_or(|&b| crate::lex::is_white(b) || crate::lex::is_delim(b))
                {
                    break i - 1;
                }
                i += 1;
            }
        }
    };
    let data = d[start..end.max(start)].to_vec();
    // Past its "EI".
    let mut i = end;
    while i + 2 <= d.len() && &d[i..i + 2] != b"EI" {
        i += 1;
    }
    lex.pos = (i + 2).min(d.len());
    Some((dict, data))
}

/// A picture's size in pixels, from its dictionary.
fn dimensions(f: &File, s: &Source) -> Option<(usize, usize)> {
    let dict = match s {
        Source::Object(o) => match f.resolve(o) {
            Obj::Stream(d, _) => d,
            _ => return None,
        },
        Source::Inline(d, _) => d.clone(),
    };
    let w = f.get_in(&dict, "Width").int()?;
    let h = f.get_in(&dict, "Height").int()?;
    (w > 0 && h > 0 && (w as usize).saturating_mul(h as usize) <= MOST_PIXELS).then_some((w as usize, h as usize))
}

/// A picture decoded: its pixels (RGB), and how opaque each is, if it isn't all opaque.
struct Decoded {
    width: usize,
    height: usize,
    rgb: Vec<u8>,
    alpha: Option<Vec<u8>>,
}

/// A colour space: how many numbers a pixel has, and how they're made RGB.
#[derive(Clone)]
enum Space {
    Gray,
    Rgb,
    Cmyk,
    /// A palette of RGB colours (from its base space's).
    Indexed(Vec<[u8; 3]>),
    /// One or more inks' tints, shown as gray (the darker, the more ink).
    Tints(usize),
    /// CIE L*a*b*, shown as its lightness.
    Lab,
}

impl Space {
    fn components(&self) -> usize {
        match self {
            Space::Gray | Space::Indexed(_) => 1,
            Space::Rgb | Space::Lab => 3,
            Space::Cmyk => 4,
            Space::Tints(n) => *n,
        }
    }
}

/// The colour space an object names (`resources`' ColorSpace for a name that isn't a device's).
fn space(f: &File, o: &Obj, resources: &Dict, depth: u32) -> Option<Space> {
    if depth > 4 {
        return None;
    }
    let o = f.resolve(o);
    let (name, args): (Vec<u8>, Vec<Obj>) = match &o {
        Obj::Name(n) => (n.clone(), Vec::new()),
        Obj::Array(a) => (f.resolve(a.first()?).name()?.to_vec(), a[1..].to_vec()),
        _ => return None,
    };
    Some(match name.as_slice() {
        b"DeviceGray" | b"CalGray" | b"G" => Space::Gray,
        b"DeviceRGB" | b"CalRGB" | b"RGB" => Space::Rgb,
        b"DeviceCMYK" | b"CMYK" => Space::Cmyk,
        b"Lab" => Space::Lab,
        b"ICCBased" => {
            let n = f.resolve(args.first()?).dict().and_then(|d| f.get_in(d, "N").int()).unwrap_or(3);
            match n {
                1 => Space::Gray,
                4 => Space::Cmyk,
                _ => Space::Rgb,
            }
        }
        b"Indexed" | b"I" => {
            let base = space(f, args.first()?, resources, depth + 1)?;
            let hival = f.resolve(args.get(1)?).int()?.clamp(0, 255) as usize;
            let table = match f.resolve(args.get(2)?) {
                Obj::Str(s) => s,
                s @ Obj::Stream(..) => f.stream(&s)?,
                _ => return None,
            };
            let n = base.components();
            let mut palette = Vec::with_capacity(hival + 1);
            for i in 0..=hival {
                let at = i * n;
                let c: Vec<f64> = (0..n).map(|k| table.get(at + k).copied().unwrap_or(0) as f64 / 255.0).collect();
                palette.push(to_rgb(&base, &c));
            }
            Space::Indexed(palette)
        }
        b"Separation" => Space::Tints(1),
        b"DeviceN" => Space::Tints(f.resolve(args.first()?).array().map_or(1, |a| a.len().clamp(1, 32))),
        // (A name of the page's: its resources say what it is.)
        _ if args.is_empty() => {
            let named = f.get_in(resources, "ColorSpace");
            let entry = named.dict()?.get(core::str::from_utf8(&name).ok()?)?.clone();
            return space(f, &entry, resources, depth + 1);
        }
        _ => return None,
    })
}

/// A pixel's numbers (each 0 to 1, but an index's) as RGB.
fn to_rgb(s: &Space, c: &[f64]) -> [u8; 3] {
    let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    match s {
        Space::Gray => [b(c[0]); 3],
        Space::Rgb => [b(c[0]), b(c[1]), b(c[2])],
        Space::Cmyk => cmyk(c[0], c[1], c[2], c[3]),
        Space::Indexed(p) => p.get(c[0] as usize).copied().unwrap_or([0, 0, 0]),
        Space::Tints(n) => {
            let ink = c[..*n].iter().fold(0.0f64, |a, &v| a.max(v));
            [b(1.0 - ink); 3]
        }
        Space::Lab => [b(c[0]); 3],
    }
}

/// A picture decoded, at about the size it's drawn at (`want`, along its rows and down its columns) or bigger.
/// Err(Some(kind)): a kind it can't decode; Err(None): damaged.
fn decoded(f: &File, s: &Source, resources: &Dict, fill: [u8; 3], want: (usize, usize), memory: u64) -> Result<Decoded, Option<&'static str>> {
    let (dict, data, codec) = match s {
        Source::Object(o) => f.image_stream(o).ok_or(None)?,
        Source::Inline(d, data) => {
            let (out, codec) = crate::filter::decode_image(d, data, &|o| f.resolve(o)).ok_or(None)?;
            (d.clone(), out, codec)
        }
    };
    let mut d = samples(f, &dict, data, codec, resources, fill, want, memory)?;
    // A soft mask: how opaque each pixel is.
    if let Source::Object(_) = s
        && let Some(mask) = dict.get("SMask").cloned()
        && let Obj::Stream(md, _) = f.resolve(&mask)
        && let Some((_, mdata, mcodec)) = f.image_stream(&mask)
        && let Ok(m) = samples(f, &md, mdata, mcodec, resources, [0, 0, 0], (d.width, d.height), memory)
    {
        let gray: Vec<u8> = m.rgb.chunks_exact(3).map(|p| p[0]).collect();
        let gray = if (m.width, m.height) == (d.width, d.height) {
            gray
        } else {
            let spread: Vec<u8> = gray.iter().flat_map(|&v| [v, v, v]).collect();
            picture::resample(&spread, m.width, m.height, d.width, d.height).chunks_exact(3).map(|p| p[0]).collect()
        };
        d.alpha = Some(match d.alpha.take() {
            Some(a) => a.iter().zip(&gray).map(|(&x, &y)| ((x as u32 * y as u32 + 127) / 255) as u8).collect(),
            None => gray,
        });
    }
    Ok(d)
}

#[allow(clippy::too_many_arguments)]
fn samples(
    f: &File,
    dict: &Dict,
    data: Vec<u8>,
    codec: Option<Codec>,
    resources: &Dict,
    fill: [u8; 3],
    want: (usize, usize),
    memory: u64,
) -> Result<Decoded, Option<&'static str>> {
    let get = |k: &str| f.get_in(dict, k);
    let width = get("Width").int().filter(|&v| v > 0).ok_or(None)? as usize;
    let height = get("Height").int().filter(|&v| v > 0).ok_or(None)? as usize;
    if width.saturating_mul(height) > MOST_PIXELS || (width * height * 4) as u64 > memory {
        return Err(None);
    }
    let mask = get("ImageMask") == Obj::Bool(true);
    let mut data = data;
    let mut bpc = if mask { 1 } else { get("BitsPerComponent").int().unwrap_or(8) };
    match codec.as_ref().map(|(name, _)| name.as_slice()) {
        Some(b"DCTDecode" | b"DCT") => {
            // (Decoded perhaps at a fraction of its size, as long as that's at least about twice what's drawn.)
            let (bw, bh) = ((want.0 * 2).clamp(1, width) as u32, (want.1 * 2).clamp(1, height) as u32);
            let p = picture::decode_stored(&data, bw, bh, [255, 255, 255], memory).map_err(|_| None)?;
            return Ok(Decoded { width: p.width as usize, height: p.height as usize, rgb: p.rgb, alpha: None });
        }
        Some(b"JPXDecode") => return Err(Some("JPEG 2000")),
        Some(b"JBIG2Decode") => return Err(Some("JBIG2")),
        Some(b"CCITTFaxDecode" | b"CCF") => {
            let parms = codec.as_ref().and_then(|(_, p)| p.clone()).unwrap_or_default();
            let n = |k: &str, default: i64| f.get_in(&parms, k).int().unwrap_or(default);
            let yes = |k: &str| f.get_in(&parms, k) == Obj::Bool(true);
            let p = ccitt::Params {
                k: n("K", 0),
                columns: n("Columns", 1728).clamp(1, 1 << 16) as usize,
                rows: n("Rows", 0).clamp(0, 1 << 16) as usize,
                byte_align: yes("EncodedByteAlign"),
                black_is_1: yes("BlackIs1"),
            };
            if p.columns.saturating_mul(height) > MOST_PIXELS {
                return Err(None);
            }
            let p = ccitt::Params { rows: if p.rows == 0 { height } else { p.rows.min(height) }, ..p };
            data = ccitt::decode(&data, &p, height).ok_or(None)?;
            // (Its lines are its Columns wide: made the picture's width.)
            if p.columns != width {
                let (from, to) = (p.columns.div_ceil(8), width.div_ceil(8));
                let mut fitted = vec![if p.black_is_1 { 0 } else { 0xFFu8 }; to * height];
                for (row, line) in fitted.chunks_exact_mut(to).zip(data.chunks_exact(from)) {
                    let n = from.min(to);
                    row[..n].copy_from_slice(&line[..n]);
                }
                data = fitted;
            }
            bpc = 1;
        }
        _ => {}
    }
    let bpc = match bpc {
        1 | 2 | 4 | 8 | 16 => bpc as usize,
        _ => return Err(None),
    };
    let space = if mask { Space::Gray } else { space(f, &get("ColorSpace"), resources, 0).unwrap_or(Space::Gray) };
    let n = space.components();
    let stride = (width * n * bpc).div_ceil(8);
    if data.len() < stride * height {
        // (A picture cut short: the rest of it black, as other readers have it.)
        data.resize(stride * height, 0);
    }
    // Each number's range (Decode), as from 0 to 1 (an index's, from 0 to its most).
    let top = ((1u32 << bpc) - 1) as f64;
    let decode: Vec<f64> = get("Decode").array().map(|a| a.iter().filter_map(|o| f.resolve(o).num()).collect()).unwrap_or_default();
    let ranges: Vec<(f64, f64)> = (0..n)
        .map(|i| match (decode.get(2 * i), decode.get(2 * i + 1)) {
            (Some(&lo), Some(&hi)) => (lo, hi),
            _ if matches!(space, Space::Indexed(_)) => (0.0, top),
            // (L*a*b*'s lightness from 0 to 100, its a* and b* from -100 to 100.)
            _ if matches!(space, Space::Lab) => if i == 0 { (0.0, 100.0) } else { (-100.0, 100.0) },
            _ => (0.0, 1.0),
        })
        .collect();
    let sample = |row: &[u8], i: usize| -> u32 {
        match bpc {
            8 => row[i] as u32,
            16 => row[2 * i] as u32,
            _ => {
                let bit = i * bpc;
                ((row[bit / 8] >> (8 - bpc - bit % 8)) as u32) & ((1 << bpc) - 1)
            }
        }
    };
    let scale = if bpc == 16 { 255.0 } else { top };
    // (A stencil's pixels are all the colour it's filled with: where it's painted is how opaque they are.)
    let mut rgb = if mask { fill.repeat(width * height) } else { vec![0u8; width * height * 3] };
    let mut alpha = mask.then(|| vec![0u8; width * height]);
    // (A table for the common case: one number a pixel, 8 bits or fewer.)
    let one: Option<Vec<[u8; 3]>> = (n == 1 && bpc <= 8 && !mask).then(|| {
        (0..=top as u32).map(|v| to_rgb(&space, &[ranges[0].0 + v as f64 * (ranges[0].1 - ranges[0].0) / top])).collect()
    });
    let mut c = vec![0.0f64; n];
    for y in 0..height {
        let row = &data[y * stride..(y + 1) * stride];
        for x in 0..width {
            if let Some(a) = &mut alpha {
                // A stencil: painted where it's 0 (or 1, its Decode turned round), in the colour it's filled with.
                let s = sample(row, x);
                if (ranges[0].0 > ranges[0].1) == (s == 1) {
                    a[y * width + x] = 255;
                }
                continue;
            }
            let at = (y * width + x) * 3;
            if let Some(t) = &one {
                rgb[at..at + 3].copy_from_slice(&t[sample(row, x) as usize]);
                continue;
            }
            for (k, v) in c.iter_mut().enumerate() {
                let s = sample(row, x * n + k) as f64;
                let (lo, hi) = ranges[k];
                *v = lo + s * (hi - lo) / scale;
            }
            if let Space::Lab = space {
                c[0] /= 100.0;
            }
            rgb[at..at + 3].copy_from_slice(&to_rgb(&space, &c));
        }
    }
    Ok(Decoded { width, height, rgb, alpha })
}

/// A decoded picture drawn on the page's picture, where `px` takes its pixels (x along its rows, y down them).
fn draw(out: &mut PagePicture, d: &Decoded, px: &Matrix) {
    let small = |v: f64, of: f64| v.abs() <= 1e-6 * of.abs().max(1e-9);
    let (w, h) = (d.width as f64, d.height as f64);
    // Its corners, on the page's picture.
    let corner = |x: f64, y: f64| (px[0] * x + px[2] * y + px[4], px[1] * x + px[3] * y + px[5]);
    let corners = [corner(0.0, 0.0), corner(w, 0.0), corner(0.0, h), corner(w, h)];
    let (xmin, xmax) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.0), b.max(c.0)));
    let (ymin, ymax) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c.1), b.max(c.1)));
    if xmax <= 0.0 || ymax <= 0.0 || xmin >= out.width as f64 || ymin >= out.height as f64 {
        return;
    }
    let straight = small(px[1], px[0]) && small(px[2], px[3]);
    let across = small(px[0], px[1]) && small(px[3], px[2]);
    if straight || across {
        // Turned and mirrored to lie as it's drawn, then resampled to its size there.
        let (rgb, alpha, (tw, th)) = lie(d, if straight { (px[0] < 0.0, px[3] < 0.0, false) } else { (px[2] < 0.0, px[1] < 0.0, true) });
        let (x0, x1) = (round(xmin), round(xmax));
        let (y0, y1) = (round(ymin), round(ymax));
        let (rw, rh) = (((x1 - x0) as usize).max(1), ((y1 - y0) as usize).max(1));
        let rgb = picture::resample(&rgb, tw, th, rw, rh);
        let alpha = alpha.map(|a| {
            let spread: Vec<u8> = a.iter().flat_map(|&v| [v, v, v]).collect();
            picture::resample(&spread, tw, th, rw, rh).chunks_exact(3).map(|p| p[0]).collect::<Vec<u8>>()
        });
        for y in 0..rh {
            let oy = y0 + y as i64;
            if oy < 0 || oy >= out.height as i64 {
                continue;
            }
            for x in 0..rw {
                let ox = x0 + x as i64;
                if ox < 0 || ox >= out.width as i64 {
                    continue;
                }
                let a = alpha.as_ref().map_or(255, |a| a[y * rw + x]);
                blend(&mut out.rgb[(oy as usize * out.width + ox as usize) * 3..][..3], &rgb[(y * rw + x) * 3..][..3], a);
            }
        }
        return;
    }
    // Slanted or turned other than by quarters: made about the size it's drawn at, then each pixel of the page's
    // picture it covers found in it (between its four nearest pixels).
    let (lw, lh) = (ceil(hypot(px[0], px[1]) * w).max(1.0) as usize, ceil(hypot(px[2], px[3]) * h).max(1.0) as usize);
    let (sw, sh) = (lw.min(d.width), lh.min(d.height));
    let rgb = if (sw, sh) == (d.width, d.height) { d.rgb.clone() } else { picture::resample(&d.rgb, d.width, d.height, sw, sh) };
    let alpha = d.alpha.as_ref().map(|a| {
        if (sw, sh) == (d.width, d.height) {
            a.clone()
        } else {
            let spread: Vec<u8> = a.iter().flat_map(|&v| [v, v, v]).collect();
            picture::resample(&spread, d.width, d.height, sw, sh).chunks_exact(3).map(|p| p[0]).collect()
        }
    });
    let (fx, fy) = (d.width as f64 / sw as f64, d.height as f64 / sh as f64);
    let m = [px[0] * fx, px[1] * fx, px[2] * fy, px[3] * fy, px[4], px[5]];
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-12 {
        return;
    }
    for oy in (ymin.max(0.0) as usize)..(ceil(ymax) as usize).min(out.height) {
        for ox in (xmin.max(0.0) as usize)..(ceil(xmax) as usize).min(out.width) {
            let (dx, dy) = (ox as f64 + 0.5 - m[4], oy as f64 + 0.5 - m[5]);
            let u = (m[3] * dx - m[2] * dy) / det;
            let v = (-m[1] * dx + m[0] * dy) / det;
            if u < 0.0 || v < 0.0 || u >= sw as f64 || v >= sh as f64 {
                continue;
            }
            let (p, a) = between(&rgb, alpha.as_deref(), sw, sh, u - 0.5, v - 0.5);
            blend(&mut out.rgb[(oy * out.width + ox) * 3..][..3], &p, a);
        }
    }
}

/// The whole number at or above v (core has no ceil).
fn ceil(v: f64) -> f64 {
    let t = v as i64 as f64;
    if t < v { t + 1.0 } else { t }
}

fn round(v: f64) -> i64 {
    if v >= 0.0 { (v + 0.5) as i64 } else { -((-v + 0.5) as i64) }
}

/// A picture turned and mirrored to lie as it's drawn: `(mirrored across, mirrored down, its rows down)`.
fn lie(d: &Decoded, (flip_x, flip_y, transpose): (bool, bool, bool)) -> (Vec<u8>, Option<Vec<u8>>, (usize, usize)) {
    if !flip_x && !flip_y && !transpose {
        return (d.rgb.clone(), d.alpha.clone(), (d.width, d.height));
    }
    let (w, h) = (d.width, d.height);
    let (tw, th) = if transpose { (h, w) } else { (w, h) };
    let mut rgb = vec![0u8; w * h * 3];
    let mut alpha = d.alpha.as_ref().map(|_| vec![0u8; w * h]);
    for y in 0..th {
        for x in 0..tw {
            // (Where it comes from: mirrored as it's drawn, then its rows down, if they are.)
            let (mx, my) = (if flip_x { tw - 1 - x } else { x }, if flip_y { th - 1 - y } else { y });
            let (sx, sy) = if transpose { (my, mx) } else { (mx, my) };
            rgb[(y * tw + x) * 3..][..3].copy_from_slice(&d.rgb[(sy * w + sx) * 3..][..3]);
            if let (Some(a), Some(src)) = (&mut alpha, &d.alpha) {
                a[y * tw + x] = src[sy * w + sx];
            }
        }
    }
    (rgb, alpha, (tw, th))
}

/// A picture's colour at (x, y), between its four nearest pixels, and how opaque.
fn between(rgb: &[u8], alpha: Option<&[u8]>, w: usize, h: usize, x: f64, y: f64) -> ([u8; 3], u8) {
    let (x, y) = (x.clamp(0.0, (w - 1) as f64), y.clamp(0.0, (h - 1) as f64));
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f64, y - y0 as f64);
    let mix = |a: f64, b: f64, c: f64, d: f64| a * (1.0 - fx) * (1.0 - fy) + b * fx * (1.0 - fy) + c * (1.0 - fx) * fy + d * fx * fy;
    let mut p = [0u8; 3];
    for (k, v) in p.iter_mut().enumerate() {
        let at = |x: usize, y: usize| rgb[(y * w + x) * 3 + k] as f64;
        *v = (mix(at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1)) + 0.5) as u8;
    }
    let a = alpha.map_or(255, |a| {
        let at = |x: usize, y: usize| a[y * w + x] as f64;
        (mix(at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1)) + 0.5) as u8
    });
    (p, a)
}

/// A pixel put over what's there, as opaque as `a` says.
fn blend(dst: &mut [u8], src: &[u8], a: u8) {
    if a == 255 {
        dst.copy_from_slice(src);
        return;
    }
    let a = a as u32;
    for (d, &s) in dst.iter_mut().zip(src) {
        *d = ((s as u32 * a + *d as u32 * (255 - a) + 127) / 255) as u8;
    }
}
