//! Web fonts as sites send them, made back into the TrueType (sfnt) files they were: WOFF (each table compressed with
//! zlib) and WOFF2 (all its tables compressed together with Brotli, its glyphs' outlines and where they are, and its
//! horizontal metrics, transformed to compress better), as W3C's WOFF and WOFF2 recommendations describe them. Every
//! number is checked before it's used and what's made is capped, so a damaged font gives `None`, never a crash.

use alloc::vec::Vec;

/// WOFF2's known tables (its table directory names them by their place here).
const KNOWN_TAGS: [&[u8; 4]; 63] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm", b"glyf", b"loca", b"prep",
    b"CFF ", b"VORG", b"EBDT", b"EBLC", b"gasp", b"hdmx", b"kern", b"LTSH", b"PCLT", b"VDMX", b"vhea", b"vmtx", b"BASE",
    b"GDEF", b"GPOS", b"GSUB", b"EBSC", b"JSTF", b"MATH", b"CBDT", b"CBLC", b"COLR", b"CPAL", b"SVG ", b"sbix", b"acnt",
    b"avar", b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc", b"feat", b"fmtx", b"fvar", b"gvar", b"hsty", b"just", b"lcar",
    b"mort", b"morx", b"opbd", b"prop", b"trak", b"Zapf", b"Silf", b"Glat", b"Gloc", b"Feat", b"Sill",
];

/// A font as a site sent it (TrueType, OpenType, WOFF or WOFF2) as an sfnt file: its own bytes if it's one already;
/// None if it's damaged, of a kind not read, or would make more than `most` bytes.
pub fn sfnt(data: &[u8], most: usize) -> Option<Vec<u8>> {
    match data.get(..4)? {
        b"wOFF" => woff(data, most),
        b"wOF2" => woff2(data, most),
        [0, 1, 0, 0] | b"true" | b"OTTO" => Some(data.to_vec()),
        _ => None,
    }
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(o)?, *d.get(o + 1)?]))
}
fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*d.get(o)?, *d.get(o + 1)?, *d.get(o + 2)?, *d.get(o + 3)?]))
}

/// The sfnt file of some tables (tag, bytes): its header, its table records in their tags' order, each table
/// 4-byte aligned, and each table's checksum.
fn assemble(flavor: u32, mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by(|a, b| a.0.cmp(&b.0));
    let n = tables.len() as u16;
    let mut power = 1u16;
    let mut log = 0u16;
    while power * 2 <= n {
        power *= 2;
        log += 1;
    }
    let mut out = Vec::new();
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&(power * 16).to_be_bytes());
    out.extend_from_slice(&log.to_be_bytes());
    out.extend_from_slice(&(n * 16 - power * 16).to_be_bytes());
    let mut at = 12 + 16 * tables.len();
    for (tag, bytes) in &tables {
        let sum = bytes.chunks(4).fold(0u32, |s, c| {
            let mut w = [0u8; 4];
            w[..c.len()].copy_from_slice(c);
            s.wrapping_add(u32::from_be_bytes(w))
        });
        out.extend_from_slice(tag);
        out.extend_from_slice(&sum.to_be_bytes());
        out.extend_from_slice(&(at as u32).to_be_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        at += (bytes.len() + 3) & !3;
    }
    for (_, bytes) in &tables {
        out.extend_from_slice(bytes);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    out
}

/// WOFF 1.0: a table directory, each table zlib-compressed unless it came out no smaller.
fn woff(d: &[u8], most: usize) -> Option<Vec<u8>> {
    let flavor = u32_at(d, 4)?;
    let n = u16_at(d, 12)? as usize;
    let mut tables = Vec::with_capacity(n);
    let mut total = 0usize;
    for i in 0..n {
        let e = 44 + 20 * i;
        let tag: [u8; 4] = d.get(e..e + 4)?.try_into().ok()?;
        let (offset, comp, orig) = (u32_at(d, e + 4)? as usize, u32_at(d, e + 8)? as usize, u32_at(d, e + 12)? as usize);
        total = total.checked_add(orig)?;
        if total > most || comp > orig {
            return None;
        }
        let raw = d.get(offset..offset.checked_add(comp)?)?;
        let bytes = if comp < orig { zip::zlib(raw, orig).ok()? } else { raw.to_vec() };
        if bytes.len() != orig {
            return None;
        }
        tables.push((tag, bytes));
    }
    Some(assemble(flavor, tables))
}

/// A cursor over bytes, big-endian, each read checked.
struct Read<'a> {
    d: &'a [u8],
    at: usize,
}

impl<'a> Read<'a> {
    fn new(d: &'a [u8]) -> Read<'a> {
        Read { d, at: 0 }
    }
    fn u8(&mut self) -> Option<u8> {
        let v = *self.d.get(self.at)?;
        self.at += 1;
        Some(v)
    }
    fn u16(&mut self) -> Option<u16> {
        let v = u16_at(self.d, self.at)?;
        self.at += 2;
        Some(v)
    }
    fn i16(&mut self) -> Option<i16> {
        self.u16().map(|v| v as i16)
    }
    fn u32(&mut self) -> Option<u32> {
        let v = u32_at(self.d, self.at)?;
        self.at += 4;
        Some(v)
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let v = self.d.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(v)
    }
    /// WOFF2's UIntBase128: up to five bytes, seven bits each, the first bits first.
    fn base128(&mut self) -> Option<u32> {
        let mut v: u32 = 0;
        for i in 0..5 {
            let b = self.u8()?;
            if i == 0 && b == 0x80 {
                return None;
            }
            if v & 0xFE00_0000 != 0 {
                return None;
            }
            v = (v << 7) | (b & 0x7F) as u32;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
    /// WOFF2's 255UInt16: one byte, or a byte after one of the codes 253 (a 16-bit number), 254 or 255.
    fn u255(&mut self) -> Option<u16> {
        Some(match self.u8()? {
            253 => self.u16()?,
            254 => self.u8()? as u16 + 506,
            255 => self.u8()? as u16 + 253,
            c => c as u16,
        })
    }
}

/// WOFF2: the table directory, then all the tables Brotli-compressed together, glyf and loca (and maybe hmtx)
/// transformed.
fn woff2(d: &[u8], most: usize) -> Option<Vec<u8>> {
    let mut r = Read::new(d);
    r.bytes(4)?;
    let flavor = r.u32()?;
    if flavor == u32::from_be_bytes(*b"ttcf") {
        return None;
    }
    r.u32()?; // length
    let n = r.u16()? as usize;
    r.u16()?;
    r.u32()?; // totalSfntSize
    let compressed = r.u32()? as usize;
    r.bytes(4 + 4 * 5)?; // versions, metadata and private data
    // The table directory: each table's tag, transform, length and (transformed) length in the stream.
    struct Entry {
        tag: [u8; 4],
        version: u8,
        orig: usize,
        len: usize,
    }
    let mut entries = Vec::with_capacity(n);
    let mut stream_len = 0usize;
    for _ in 0..n {
        let flags = r.u8()?;
        let tag: [u8; 4] = if flags & 63 == 63 { r.u32()?.to_be_bytes() } else { *KNOWN_TAGS[(flags & 63) as usize] };
        let version = flags >> 6;
        let orig = r.base128()? as usize;
        let transformed = if &tag == b"glyf" || &tag == b"loca" { version == 0 } else { version != 0 };
        let len = if transformed { r.base128()? as usize } else { orig };
        stream_len = stream_len.checked_add(len)?;
        entries.push(Entry { tag, version, orig, len });
    }
    if stream_len > most {
        return None;
    }
    let packed = r.bytes(compressed)?;
    let stream = zip::brotli(packed, stream_len).ok()?;
    if stream.len() < stream_len {
        return None;
    }
    let mut at = 0usize;
    let mut raw: Vec<([u8; 4], u8, usize, &[u8])> = Vec::with_capacity(n);
    for e in &entries {
        raw.push((e.tag, e.version, e.orig, &stream[at..at + e.len]));
        at += e.len;
    }
    let find = |tag: &[u8; 4]| raw.iter().find(|t| &t.0 == tag);
    let mut tables: Vec<([u8; 4], Vec<u8>)> = Vec::with_capacity(n);
    // The glyphs and where they are, made back from glyf's transform.
    let mut x_mins: Vec<i16> = Vec::new();
    if let Some(&(_, 0, _, data)) = find(b"glyf") {
        let (glyf, loca, mins) = glyf_back(data, most)?;
        x_mins = mins;
        tables.push((*b"glyf", glyf));
        tables.push((*b"loca", loca));
    }
    let glyphs = find(b"maxp").and_then(|t| u16_at(t.3, 4)).unwrap_or(0) as usize;
    let h_metrics = find(b"hhea").and_then(|t| u16_at(t.3, 34)).unwrap_or(0) as usize;
    for &(tag, version, orig, data) in &raw {
        if (&tag == b"glyf" || &tag == b"loca") && version == 0 {
            continue;
        }
        if &tag == b"hmtx" && version == 1 {
            tables.push((tag, hmtx_back(data, glyphs, h_metrics, &x_mins)?));
            continue;
        }
        if version != 0 && !(&tag == b"glyf" || &tag == b"loca") {
            return None;
        }
        tables.push((tag, data.get(..orig.min(data.len()))?.to_vec()));
    }
    Some(assemble(flavor, tables))
}

/// glyf (and loca) made back from WOFF2's transformed glyf: its streams of contours' counts, points' counts, flags,
/// coordinates (as triplets) and instructions' lengths, composites, bounding boxes and instructions. Also each glyph's
/// left edge (xMin), for hmtx's transform.
fn glyf_back(d: &[u8], most: usize) -> Option<(Vec<u8>, Vec<u8>, Vec<i16>)> {
    let mut h = Read::new(d);
    h.u16()?;
    let options = h.u16()?;
    let glyphs = h.u16()? as usize;
    let long = h.u16()? != 0;
    let mut sizes = [0usize; 7];
    for s in &mut sizes {
        *s = h.u32()? as usize;
    }
    let mut take = |n: usize| -> Option<&[u8]> { h.bytes(n) };
    let contours_s = take(sizes[0])?;
    let points_s = take(sizes[1])?;
    let flags_s = take(sizes[2])?;
    let glyph_s = take(sizes[3])?;
    let composite_s = take(sizes[4])?;
    let bbox_all = take(sizes[5])?;
    let instr_s = take(sizes[6])?;
    let overlap = if options & 1 != 0 { take(glyphs.div_ceil(8)) } else { None };
    let bitmap_len = 4 * glyphs.div_ceil(32);
    let bitmap = bbox_all.get(..bitmap_len)?;
    let (mut contours, mut points, mut flags, mut glyph, mut composite, mut bbox, mut instr) = (
        Read::new(contours_s),
        Read::new(points_s),
        Read::new(flags_s),
        Read::new(glyph_s),
        Read::new(composite_s),
        Read::new(&bbox_all[bitmap_len..]),
        Read::new(instr_s),
    );
    let mut glyf: Vec<u8> = Vec::new();
    let mut offsets: Vec<u32> = Vec::with_capacity(glyphs + 1);
    let mut mins: Vec<i16> = Vec::with_capacity(glyphs);
    for g in 0..glyphs {
        offsets.push(glyf.len() as u32);
        let explicit = bitmap[g >> 3] & (0x80 >> (g & 7)) != 0;
        let n = contours.i16()?;
        if n == 0 {
            mins.push(0);
            continue;
        }
        let start = glyf.len();
        if n < 0 {
            // A composite: its parts as they are, its box given, its instructions if any part says so.
            let from = composite.at;
            let mut more = true;
            let mut instructions = false;
            while more {
                let f = composite.u16()?;
                composite.u16()?;
                let args = if f & 1 != 0 { 4 } else { 2 };
                let scale = if f & 0x8 != 0 {
                    2
                } else if f & 0x40 != 0 {
                    4
                } else if f & 0x80 != 0 {
                    8
                } else {
                    0
                };
                composite.bytes(args + scale)?;
                instructions |= f & 0x100 != 0;
                more = f & 0x20 != 0;
            }
            if !explicit {
                return None;
            }
            let b = [bbox.i16()?, bbox.i16()?, bbox.i16()?, bbox.i16()?];
            glyf.extend_from_slice(&(-1i16).to_be_bytes());
            for v in b {
                glyf.extend_from_slice(&v.to_be_bytes());
            }
            glyf.extend_from_slice(&composite_s[from..composite.at]);
            if instructions {
                let len = glyph.u255()? as usize;
                glyf.extend_from_slice(&(len as u16).to_be_bytes());
                glyf.extend_from_slice(instr.bytes(len)?);
            }
            mins.push(b[0]);
        } else {
            // A simple glyph: its contours' ends, then each point from its flag and triplet.
            let mut ends: Vec<u16> = Vec::with_capacity(n as usize);
            let mut total: usize = 0;
            for _ in 0..n {
                total = total.checked_add(points.u255()? as usize)?;
                if total == 0 || total > 0xFFFF {
                    return None;
                }
                ends.push((total - 1) as u16);
            }
            let mut pts: Vec<(i32, i32, bool)> = Vec::with_capacity(total);
            let (mut x, mut y) = (0i32, 0i32);
            for _ in 0..total {
                let f = flags.u8()?;
                let on = f & 0x80 == 0;
                let f = f & 0x7F;
                let sign = |flag: u8, v: i32| if flag & 1 != 0 { v } else { -v };
                let (dx, dy) = if f < 10 {
                    (0, sign(f, (((f & 14) as i32) << 7) + glyph.u8()? as i32))
                } else if f < 20 {
                    (sign(f, ((((f - 10) & 14) as i32) << 7) + glyph.u8()? as i32), 0)
                } else if f < 84 {
                    let b0 = (f - 20) as i32;
                    let b1 = glyph.u8()? as i32;
                    (sign(f, 1 + (b0 & 0x30) + (b1 >> 4)), sign(f >> 1, 1 + ((b0 & 0x0C) << 2) + (b1 & 0x0F)))
                } else if f < 120 {
                    let b0 = (f - 84) as i32;
                    let (b1, b2) = (glyph.u8()? as i32, glyph.u8()? as i32);
                    (sign(f, 1 + ((b0 / 12) << 8) + b1), sign(f >> 1, 1 + (((b0 % 12) >> 2) << 8) + b2))
                } else if f < 124 {
                    let (b1, b2, b3) = (glyph.u8()? as i32, glyph.u8()? as i32, glyph.u8()? as i32);
                    (sign(f, (b1 << 4) + (b2 >> 4)), sign(f >> 1, ((b2 & 0x0F) << 8) + b3))
                } else {
                    let (b1, b2, b3, b4) = (glyph.u8()? as i32, glyph.u8()? as i32, glyph.u8()? as i32, glyph.u8()? as i32);
                    (sign(f, (b1 << 8) + b2), sign(f >> 1, (b3 << 8) + b4))
                };
                x += dx;
                y += dy;
                pts.push((x, y, on));
            }
            let len = glyph.u255()? as usize;
            let code = instr.bytes(len)?;
            let b = if explicit {
                [bbox.i16()?, bbox.i16()?, bbox.i16()?, bbox.i16()?]
            } else {
                let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                for &(px, py, _) in &pts {
                    x0 = x0.min(px);
                    y0 = y0.min(py);
                    x1 = x1.max(px);
                    y1 = y1.max(py);
                }
                [x0 as i16, y0 as i16, x1 as i16, y1 as i16]
            };
            glyf.extend_from_slice(&n.to_be_bytes());
            for v in b {
                glyf.extend_from_slice(&v.to_be_bytes());
            }
            for e in &ends {
                glyf.extend_from_slice(&e.to_be_bytes());
            }
            glyf.extend_from_slice(&(len as u16).to_be_bytes());
            glyf.extend_from_slice(code);
            // Flags (on the curve, and each coordinate's size and sign), then the x deltas, then the y deltas.
            let overlaps = overlap.is_some_and(|o| o.get(g >> 3).is_some_and(|b| b & (0x80 >> (g & 7)) != 0));
            let (mut xs, mut ys) = (Vec::new(), Vec::new());
            let (mut px, mut py) = (0i32, 0i32);
            for (i, &(x, y, on)) in pts.iter().enumerate() {
                let (dx, dy) = (x - px, y - py);
                px = x;
                py = y;
                let mut fl: u8 = if on { 1 } else { 0 };
                if i == 0 && overlaps {
                    fl |= 0x40;
                }
                if dx == 0 {
                    fl |= 0x10;
                } else if dx.abs() < 256 {
                    fl |= 0x02 | if dx > 0 { 0x10 } else { 0 };
                    xs.push(dx.unsigned_abs() as u8);
                } else {
                    xs.extend_from_slice(&(dx as i16).to_be_bytes());
                }
                if dy == 0 {
                    fl |= 0x20;
                } else if dy.abs() < 256 {
                    fl |= 0x04 | if dy > 0 { 0x20 } else { 0 };
                    ys.push(dy.unsigned_abs() as u8);
                } else {
                    ys.extend_from_slice(&(dy as i16).to_be_bytes());
                }
                glyf.push(fl);
            }
            glyf.extend_from_slice(&xs);
            glyf.extend_from_slice(&ys);
            mins.push(b[0]);
        }
        while glyf.len() % 4 != 0 {
            glyf.push(0);
        }
        if glyf.len() > most || glyf.len() - start > 0x10_0000 {
            return None;
        }
    }
    offsets.push(glyf.len() as u32);
    let mut loca = Vec::with_capacity(offsets.len() * 4);
    for o in offsets {
        if long {
            loca.extend_from_slice(&o.to_be_bytes());
        } else {
            loca.extend_from_slice(&((o / 2) as u16).to_be_bytes());
        }
    }
    Some((glyf, loca, mins))
}

/// hmtx made back from WOFF2's transform: the advances, and the left side bearings given, or else each glyph's xMin.
fn hmtx_back(d: &[u8], glyphs: usize, h_metrics: usize, x_mins: &[i16]) -> Option<Vec<u8>> {
    if h_metrics == 0 || h_metrics > glyphs {
        return None;
    }
    let mut r = Read::new(d);
    let flags = r.u8()?;
    let mut advances = Vec::with_capacity(h_metrics);
    for _ in 0..h_metrics {
        advances.push(r.u16()?);
    }
    let lsb = |r: &mut Read, i: usize, given: bool| -> Option<i16> { if given { r.i16() } else { Some(x_mins.get(i).copied().unwrap_or(0)) } };
    let mut out = Vec::with_capacity(4 * h_metrics + 2 * (glyphs - h_metrics));
    let mut lsbs = Vec::with_capacity(glyphs);
    for i in 0..h_metrics {
        lsbs.push(lsb(&mut r, i, flags & 1 == 0)?);
    }
    for i in h_metrics..glyphs {
        lsbs.push(lsb(&mut r, i, flags & 2 == 0)?);
    }
    for i in 0..h_metrics {
        out.extend_from_slice(&advances[i].to_be_bytes());
        out.extend_from_slice(&lsbs[i].to_be_bytes());
    }
    for v in &lsbs[h_metrics..] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    Some(out)
}
