//! JPEG (ITU T.81, with JFIF's and Adobe's colour conventions and EXIF's orientation): baseline and extended
//! sequential, and progressive, Huffman-coded, 8-bit; any sampling of its components, restart intervals. Decoded as
//! libjpeg decodes it, which most of the world's programs use: its accurate integer inverse DCT ("islow"), its
//! "fancy" upsampling of colour that has half the resolution across (and down), and its YCbCr to RGB arithmetic.
//!
//! A picture made of one scan (as cameras and phones save them) is decoded a row of blocks at a time, each block
//! turned into pixels at once, at a half, a quarter or an eighth of its size when that's enough: a 48-megapixel photo
//! needs a few megabytes. A progressive one is made of several scans that each add to every block's coefficients, so
//! those are kept until the last.

use crate::{Error, Picture};
use alloc::vec;
use alloc::vec::Vec;

/// Where the zigzag order's coefficients go in a block (row by row).
const NATURAL: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55,
    62, 63,
];

/// The most coefficients a progressive picture keeps (2 bytes each): 64 MB.
const MOST_COEFFICIENTS: usize = 32 << 20;

/// A Huffman table: a lookup of every code of up to 9 bits (length << 8 | symbol; 0 for longer ones), and, for
/// longer ones, libjpeg's largest code of each length and where its symbols start.
struct Huffman {
    lookup: Vec<u16>,
    maxcode: [i32; 18],
    offset: [i32; 17],
    values: Vec<u8>,
}

impl Huffman {
    fn new(counts: &[u8; 16], values: &[u8]) -> Result<Huffman, Error> {
        let mut h = Huffman { lookup: vec![0; 512], maxcode: [-1; 18], offset: [0; 17], values: values.to_vec() };
        let (mut code, mut k) = (0i32, 0usize);
        for len in 1..=16 {
            let n = counts[len - 1] as usize;
            h.offset[len] = k as i32 - code;
            for _ in 0..n {
                if code >= 1 << len {
                    return Err(Error::Damaged);
                }
                if len <= 9 {
                    let shift = 9 - len;
                    let entry = (len as u16) << 8 | *values.get(k).ok_or(Error::Damaged)? as u16;
                    for i in (code << shift) as usize..((code + 1) << shift) as usize {
                        h.lookup[i] = entry;
                    }
                }
                code += 1;
                k += 1;
            }
            if n > 0 {
                h.maxcode[len] = code - 1;
            }
            code <<= 1;
        }
        h.maxcode[17] = i32::MAX;
        Ok(h)
    }
}

/// The entropy-coded data's bits, first bit first: bytes 0xFF 0x00 are a 0xFF; a marker ends the data (and zeros
/// follow, as libjpeg gives a scan cut short).
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    buf: u64,
    count: u32,
    marker: Option<u8>,
}

impl Bits<'_> {
    fn fill(&mut self) {
        while self.count <= 56 {
            let mut byte = 0u8;
            if self.marker.is_none() {
                match self.data.get(self.at) {
                    None => self.marker = Some(0xD9),
                    Some(&0xFF) => match self.data.get(self.at + 1) {
                        Some(0x00) => {
                            byte = 0xFF;
                            self.at += 2;
                        }
                        // (Fill bytes before a marker.)
                        Some(0xFF) => {
                            self.at += 1;
                            continue;
                        }
                        Some(&m) => self.marker = Some(m),
                        None => self.marker = Some(0xD9),
                    },
                    Some(&b) => {
                        byte = b;
                        self.at += 1;
                    }
                }
            }
            self.buf |= (byte as u64) << (56 - self.count);
            self.count += 8;
        }
    }

    fn peek(&mut self, n: u32) -> u32 {
        if self.count < n {
            self.fill();
        }
        (self.buf >> (64 - n)) as u32
    }

    fn skip(&mut self, n: u32) {
        self.buf <<= n;
        self.count -= n;
    }

    fn bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.peek(n);
        self.skip(n);
        v
    }

    fn decode(&mut self, h: &Huffman) -> Result<u8, Error> {
        let e = h.lookup[self.peek(9) as usize];
        if e != 0 {
            self.skip((e >> 8) as u32);
            return Ok(e as u8);
        }
        let mut code = 0i32;
        for len in 1..=16 {
            code = code << 1 | self.bits(1) as i32;
            if code <= h.maxcode[len] {
                return h.values.get((code + h.offset[len]) as usize).copied().ok_or(Error::Damaged);
            }
        }
        Err(Error::Damaged)
    }

    /// `n` bits as a signed value (JPEG's "extend": the low half are negative).
    fn signed(&mut self, n: u32) -> i32 {
        if n == 0 {
            return 0;
        }
        let v = self.bits(n) as i32;
        if v < 1 << (n - 1) { v - (1 << n) + 1 } else { v }
    }

    /// At a restart marker: what's left of the byte dropped, the marker passed.
    fn restart(&mut self) {
        self.buf = 0;
        self.count = 0;
        match self.marker {
            Some(m) if (0xD0..=0xD7).contains(&m) => {
                self.at += 2;
                self.marker = None;
            }
            Some(_) => {}
            None => {
                while self.at + 1 < self.data.len() {
                    if self.data[self.at] == 0xFF && (0xD0..=0xD7).contains(&self.data[self.at + 1]) {
                        self.at += 2;
                        return;
                    }
                    self.at += 1;
                }
            }
        }
    }

    /// Where the scan's data ends: the next marker's 0xFF.
    fn end(&mut self) -> usize {
        if self.marker.is_some() {
            return self.at;
        }
        while self.at + 1 < self.data.len() {
            if self.data[self.at] == 0xFF && !matches!(self.data[self.at + 1], 0x00 | 0xFF | 0xD0..=0xD7) {
                return self.at;
            }
            self.at += 1;
        }
        self.data.len()
    }
}

struct Component {
    id: u8,
    h: usize,
    v: usize,
    table: usize,
    /// Blocks across and down: as the component really is, and padded to whole MCUs.
    width_blocks: usize,
    height_blocks: usize,
    padded_w: usize,
    padded_h: usize,
    /// Its pixels' size (before upsampling), as JPEG says.
    width: usize,
    height: usize,
    dc: i32,
    dc_table: usize,
    ac_table: usize,
    /// Its coefficients, block by block (a progressive picture's, or one in several scans).
    coefficients: Vec<i16>,
    /// Its pixels, at the scale decoded: padded width, rows.
    plane: Vec<u8>,
    plane_w: usize,
}

struct Frame {
    width: usize,
    height: usize,
    progressive: bool,
    components: Vec<Component>,
    hmax: usize,
    vmax: usize,
    mcus_x: usize,
    mcus_y: usize,
}

/// How a JPEG's EXIF says to turn it (1, as it is, if it says nothing): from its markers before its frame header.
pub fn orientation(data: &[u8]) -> u8 {
    let mut at = 2;
    while at + 4 <= data.len() {
        if data[at] != 0xFF {
            return 1;
        }
        let m = data[at + 1];
        if m == 0xFF {
            at += 1;
            continue;
        }
        let len = u16::from_be_bytes([data[at + 2], data[at + 3]]) as usize;
        if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) || m == 0xDA {
            return 1;
        }
        if m == 0xE1
            && let Some(body) = data.get(at + 4..at + 2 + len)
            && body.starts_with(b"Exif\0\0")
        {
            return exif_orientation(&body[6..]).unwrap_or(1);
        }
        at += 2 + len;
    }
    1
}

/// The size a JPEG says it is (from its frame header).
pub fn size(data: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    while at + 9 < data.len() {
        if data[at] != 0xFF {
            return None;
        }
        let m = data[at + 1];
        if m == 0xFF {
            at += 1;
            continue;
        }
        let len = u16::from_be_bytes([data[at + 2], data[at + 3]]) as usize;
        if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            return Some((u16::from_be_bytes([data[at + 7], data[at + 8]]) as u32, u16::from_be_bytes([data[at + 5], data[at + 6]]) as u32));
        }
        at += 2 + len;
    }
    None
}

/// The picture, decoded at the smallest of its full size, a half, a quarter or an eighth that's still at least what
/// fits `box_w` × `box_h` (turned, if EXIF's orientation turns it); that orientation; and its whole size. At most
/// `most` pixels decoded, and a progressive picture's coefficients in half of `memory` bytes.
pub fn decode(data: &[u8], box_w: u32, box_h: u32, most: u64, memory: u64) -> Result<(Picture, u8, (u32, u32)), Error> {
    if !data.starts_with(&[0xFF, 0xD8]) {
        return Err(Error::NotAPicture);
    }
    let mut at = 2;
    let mut quant = [[0u16; 64]; 4];
    let mut dc_tables: [Option<Huffman>; 4] = [None, None, None, None];
    let mut ac_tables: [Option<Huffman>; 4] = [None, None, None, None];
    let mut frame: Option<Frame> = None;
    let mut restart_interval = 0usize;
    let mut orientation = 1u8;
    let mut adobe: Option<u8> = None;
    let mut scale = 1usize;
    let mut scans = 0usize;
    // Whether the picture is decoded as each MCU comes (one scan of every component), or kept as coefficients.
    let mut streaming = false;
    loop {
        // The next marker (fill bytes passed over).
        while data.get(at) == Some(&0xFF) && data.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        if data.get(at) != Some(&0xFF) {
            // (Bytes that aren't a marker: a damaged file's, or after a scan cut short. Look for the next one.)
            match data[at.min(data.len())..].windows(2).position(|w| w[0] == 0xFF && w[1] != 0 && w[1] != 0xFF) {
                Some(p) => at += p,
                None => break,
            }
        }
        let Some(&marker) = data.get(at + 1) else { break };
        at += 2;
        if marker == 0xD9 {
            break;
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            continue;
        }
        let len = u16::from_be_bytes(data.get(at..at + 2).ok_or(Error::Damaged)?.try_into().unwrap()) as usize;
        let body = data.get(at + 2..at + len).ok_or(Error::Damaged)?;
        let next = at + len;
        match marker {
            0xC0 | 0xC1 | 0xC2 => {
                if frame.is_some() {
                    return Err(Error::Damaged);
                }
                let f = read_frame(body, marker == 0xC2)?;
                // The scale: as small as still fills what's wanted.
                let (bw, bh) = if orientation >= 5 { (box_h, box_w) } else { (box_w, box_h) };
                let (tw, th) = crate::fitted(f.width as u32, f.height as u32, bw.max(1), bh.max(1));
                scale = [8usize, 4, 2, 1]
                    .into_iter()
                    .find(|&s| f.width.div_ceil(s) >= tw as usize && f.height.div_ceil(s) >= th as usize)
                    .unwrap_or(1);
                if (f.width.div_ceil(scale) * f.height.div_ceil(scale)) as u64 > most {
                    return Err(Error::TooBig);
                }
                frame = Some(f);
            }
            0xC3 => return Err(Error::Unsupported("lossless JPEG")),
            0xC5..=0xC7 | 0xCD..=0xCF => return Err(Error::Unsupported("hierarchical JPEG")),
            0xC9..=0xCB => return Err(Error::Unsupported("arithmetic-coded JPEG")),
            0xC4 => {
                let mut b = body;
                while b.len() >= 17 {
                    let (class, id) = (b[0] >> 4, (b[0] & 15) as usize);
                    let counts: [u8; 16] = b[1..17].try_into().unwrap();
                    let n: usize = counts.iter().map(|&c| c as usize).sum();
                    let values = b.get(17..17 + n).ok_or(Error::Damaged)?;
                    if id > 3 || class > 1 || n > 256 {
                        return Err(Error::Damaged);
                    }
                    let table = Huffman::new(&counts, values)?;
                    if class == 0 {
                        dc_tables[id] = Some(table);
                    } else {
                        ac_tables[id] = Some(table);
                    }
                    b = &b[17 + n..];
                }
            }
            0xDB => {
                let mut b = body;
                while !b.is_empty() {
                    let (wide, id) = (b[0] >> 4, (b[0] & 15) as usize);
                    let n = if wide == 0 { 64 } else { 128 };
                    let values = b.get(1..1 + n).ok_or(Error::Damaged)?;
                    if id > 3 {
                        return Err(Error::Damaged);
                    }
                    for k in 0..64 {
                        quant[id][NATURAL[k]] = if wide == 0 { values[k] as u16 } else { u16::from_be_bytes([values[2 * k], values[2 * k + 1]]) };
                    }
                    b = &b[1 + n..];
                }
            }
            0xDD => restart_interval = u16::from_be_bytes(body.get(0..2).ok_or(Error::Damaged)?.try_into().unwrap()) as usize,
            0xE1 if body.starts_with(b"Exif\0\0") => orientation = exif_orientation(&body[6..]).unwrap_or(1),
            0xEE if body.starts_with(b"Adobe") && body.len() >= 12 => adobe = Some(body[11]),
            0xDA => {
                let f = frame.as_mut().ok_or(Error::Damaged)?;
                let ns = *body.first().ok_or(Error::Damaged)? as usize;
                let spec = body.get(1..1 + 2 * ns + 3).ok_or(Error::Damaged)?;
                let mut members = Vec::new();
                for i in 0..ns {
                    let (id, tables) = (spec[2 * i], spec[2 * i + 1]);
                    let c = f.components.iter().position(|c| c.id == id).ok_or(Error::Damaged)?;
                    f.components[c].dc_table = (tables >> 4) as usize & 3;
                    f.components[c].ac_table = (tables & 15) as usize & 3;
                    members.push(c);
                }
                let (ss, se, ah, al) = (spec[2 * ns] as usize, spec[2 * ns + 1] as usize, spec[2 * ns + 2] >> 4, spec[2 * ns + 2] & 15);
                if scans == 0 {
                    // One scan of every component, not progressive: decoded as it comes.
                    streaming = !f.progressive && ns == f.components.len();
                    let coefficients: usize = f.components.iter().map(|c| c.padded_w * c.padded_h * 64).sum();
                    if !streaming && (coefficients > MOST_COEFFICIENTS || coefficients as u64 * 2 > memory / 2) {
                        return Err(Error::TooBig);
                    }
                    for c in &mut f.components {
                        c.plane_w = c.padded_w * 8 / scale;
                        c.plane = vec![0; c.plane_w * (c.padded_h * 8 / scale)];
                        if !streaming {
                            c.coefficients = vec![0; c.padded_w * c.padded_h * 64];
                        }
                    }
                }
                scans += 1;
                let mut bits = Bits { data, at: next, buf: 0, count: 0, marker: None };
                let scan = Scan { members: &members, ss, se: se.min(63), ah, al, restart_interval, streaming, scale };
                decode_scan(f, &scan, &mut bits, &dc_tables, &ac_tables, &quant)?;
                at = bits.end();
                continue;
            }
            _ => {}
        }
        at = next;
    }
    let f = frame.ok_or(Error::Damaged)?;
    if scans == 0 {
        return Err(Error::Damaged);
    }
    let mut f = f;
    if !streaming {
        for c in &mut f.components {
            let q = quant[c.table];
            for by in 0..c.padded_h {
                for bx in 0..c.padded_w {
                    let k = (by * c.padded_w + bx) * 64;
                    let mut block = [0i32; 64];
                    for (b, &v) in block.iter_mut().zip(&c.coefficients[k..k + 64]) {
                        *b = v as i32;
                    }
                    put_block(c, &block, &q, bx, by, scale);
                }
            }
            c.coefficients = Vec::new();
        }
    }
    Ok((colour(&f, scale, adobe), orientation, (f.width as u32, f.height as u32)))
}

fn read_frame(b: &[u8], progressive: bool) -> Result<Frame, Error> {
    if b.len() < 6 {
        return Err(Error::Damaged);
    }
    if b[0] != 8 {
        return Err(Error::Unsupported("12-bit JPEG"));
    }
    let (height, width, n) = (u16::from_be_bytes([b[1], b[2]]) as usize, u16::from_be_bytes([b[3], b[4]]) as usize, b[5] as usize);
    if width == 0 || height == 0 || !matches!(n, 1 | 3 | 4) || b.len() < 6 + 3 * n {
        return Err(if n == 2 { Error::Unsupported("two-component JPEG") } else { Error::Damaged });
    }
    let mut components = Vec::new();
    for i in 0..n {
        let c = &b[6 + 3 * i..9 + 3 * i];
        let (h, v) = ((c[1] >> 4) as usize, (c[1] & 15) as usize);
        if !(1..=4).contains(&h) || !(1..=4).contains(&v) || c[2] > 3 {
            return Err(Error::Damaged);
        }
        components.push(Component {
            id: c[0],
            h,
            v,
            table: c[2] as usize,
            width_blocks: 0,
            height_blocks: 0,
            padded_w: 0,
            padded_h: 0,
            width: 0,
            height: 0,
            dc: 0,
            dc_table: 0,
            ac_table: 0,
            coefficients: Vec::new(),
            plane: Vec::new(),
            plane_w: 0,
        });
    }
    let hmax = components.iter().map(|c| c.h).max().unwrap_or(1);
    let vmax = components.iter().map(|c| c.v).max().unwrap_or(1);
    let (mcus_x, mcus_y) = (width.div_ceil(8 * hmax), height.div_ceil(8 * vmax));
    for c in &mut components {
        c.width = (width * c.h).div_ceil(hmax);
        c.height = (height * c.v).div_ceil(vmax);
        c.width_blocks = c.width.div_ceil(8);
        c.height_blocks = c.height.div_ceil(8);
        c.padded_w = mcus_x * c.h;
        c.padded_h = mcus_y * c.v;
    }
    Ok(Frame { width, height, progressive, components, hmax, vmax, mcus_x, mcus_y })
}

struct Scan<'a> {
    members: &'a [usize],
    ss: usize,
    se: usize,
    ah: u8,
    al: u8,
    restart_interval: usize,
    streaming: bool,
    scale: usize,
}

fn decode_scan(
    f: &mut Frame,
    scan: &Scan,
    bits: &mut Bits,
    dc_tables: &[Option<Huffman>; 4],
    ac_tables: &[Option<Huffman>; 4],
    quant: &[[u16; 64]; 4],
) -> Result<(), Error> {
    for &c in scan.members {
        f.components[c].dc = 0;
    }
    let mut eobrun = 0u32;
    // An interleaved scan goes MCU by MCU; one of a single component, block by block (only its real blocks).
    let single = scan.members.len() == 1;
    let (units_x, units_y) = if single {
        let c = &f.components[scan.members[0]];
        (c.width_blocks, c.height_blocks)
    } else {
        (f.mcus_x, f.mcus_y)
    };
    let mut done = 0usize;
    for uy in 0..units_y {
        for ux in 0..units_x {
            if scan.restart_interval > 0 && done > 0 && done % scan.restart_interval == 0 {
                bits.restart();
                eobrun = 0;
                for &c in scan.members {
                    f.components[c].dc = 0;
                }
            }
            done += 1;
            for &ci in scan.members {
                let (h, v) = if single { (1, 1) } else { (f.components[ci].h, f.components[ci].v) };
                for by in 0..v {
                    for bx in 0..h {
                        let (x, y) = if single { (ux, uy) } else { (ux * h + bx, uy * v + by) };
                        let c = &mut f.components[ci];
                        if scan.streaming {
                            let mut block = [0i32; 64];
                            let dc = dc_tables[c.dc_table].as_ref().ok_or(Error::Damaged)?;
                            let ac = ac_tables[c.ac_table].as_ref().ok_or(Error::Damaged)?;
                            sequential(bits, dc, ac, &mut c.dc, &mut block)?;
                            put_block(c, &block, &quant[c.table], x, y, scan.scale);
                        } else {
                            let k = (y * c.padded_w + x) * 64;
                            let Some(coefficients) = c.coefficients.get_mut(k..k + 64) else { continue };
                            let block: &mut [i16; 64] = coefficients.try_into().unwrap();
                            if !f.progressive {
                                let mut whole = [0i32; 64];
                                let dc = dc_tables[c.dc_table].as_ref().ok_or(Error::Damaged)?;
                                let ac = ac_tables[c.ac_table].as_ref().ok_or(Error::Damaged)?;
                                sequential(bits, dc, ac, &mut c.dc, &mut whole)?;
                                for (b, w) in block.iter_mut().zip(whole) {
                                    *b = w.clamp(-32768, 32767) as i16;
                                }
                            } else if scan.ss == 0 {
                                if scan.ah == 0 {
                                    let dc = dc_tables[c.dc_table].as_ref().ok_or(Error::Damaged)?;
                                    let s = bits.decode(dc)? as u32;
                                    if s > 15 {
                                        return Err(Error::Damaged);
                                    }
                                    c.dc = c.dc.wrapping_add(bits.signed(s));
                                    block[0] = c.dc.wrapping_shl(scan.al as u32) as i16;
                                } else if bits.bits(1) == 1 {
                                    block[0] |= 1 << scan.al;
                                }
                            } else {
                                let ac = ac_tables[c.ac_table].as_ref().ok_or(Error::Damaged)?;
                                if scan.ah == 0 {
                                    ac_first(bits, ac, block, scan, &mut eobrun)?;
                                } else {
                                    ac_refine(bits, ac, block, scan, &mut eobrun)?;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// A sequential scan's block: its DC's difference from the one before, then each AC coefficient after its run of
/// zeros, up to the end of the block (EOB).
fn sequential(bits: &mut Bits, dc: &Huffman, ac: &Huffman, pred: &mut i32, block: &mut [i32; 64]) -> Result<(), Error> {
    let s = bits.decode(dc)? as u32;
    if s > 15 {
        return Err(Error::Damaged);
    }
    *pred = pred.wrapping_add(bits.signed(s));
    block[0] = *pred;
    let mut k = 1;
    while k < 64 {
        let rs = bits.decode(ac)?;
        let (r, s) = ((rs >> 4) as usize, (rs & 15) as u32);
        if s == 0 {
            if r == 15 {
                k += 16;
                continue;
            }
            break;
        }
        k += r;
        if k > 63 {
            return Err(Error::Damaged);
        }
        block[NATURAL[k]] = bits.signed(s);
        k += 1;
    }
    Ok(())
}

/// A progressive scan's first pass over a band of AC coefficients (a single component's): runs of zeros, values
/// (shifted up by Al), and end-of-band runs over whole blocks.
fn ac_first(bits: &mut Bits, ac: &Huffman, block: &mut [i16; 64], scan: &Scan, eobrun: &mut u32) -> Result<(), Error> {
    if *eobrun > 0 {
        *eobrun -= 1;
        return Ok(());
    }
    let mut k = scan.ss;
    while k <= scan.se {
        let rs = bits.decode(ac)?;
        let (r, s) = ((rs >> 4) as u32, (rs & 15) as u32);
        if s == 0 {
            if r < 15 {
                *eobrun = (1 << r) - 1 + bits.bits(r);
                break;
            }
            k += 16;
            continue;
        }
        k += r as usize;
        if k > 63 {
            return Err(Error::Damaged);
        }
        block[NATURAL[k]] = (bits.signed(s) << scan.al) as i16;
        k += 1;
    }
    Ok(())
}

/// A progressive scan's later pass over a band: a bit more of each coefficient already there, and coefficients new
/// in this pass (each ±1, shifted up by Al), as libjpeg's decode_mcu_AC_refine reads them.
fn ac_refine(bits: &mut Bits, ac: &Huffman, block: &mut [i16; 64], scan: &Scan, eobrun: &mut u32) -> Result<(), Error> {
    let p1 = 1i16 << scan.al;
    let m1 = -1i16 << scan.al;
    let mut k = scan.ss;
    let refine = |bits: &mut Bits, coefficient: &mut i16| {
        if bits.bits(1) == 1 && *coefficient & p1 == 0 {
            *coefficient += if *coefficient >= 0 { p1 } else { m1 };
        }
    };
    if *eobrun == 0 {
        while k <= scan.se {
            let rs = bits.decode(ac)?;
            let (mut r, s) = ((rs >> 4) as i32, rs & 15);
            let mut value = 0i16;
            if s != 0 {
                value = if bits.bits(1) == 1 { p1 } else { m1 };
            } else if r != 15 {
                *eobrun = (1 << r) + bits.bits(r as u32);
                break;
            }
            // Past the coefficients already there (each refined) and `r` that aren't, to where the new one goes.
            while k <= scan.se {
                let z = NATURAL[k];
                if block[z] != 0 {
                    refine(bits, &mut block[z]);
                } else {
                    r -= 1;
                    if r < 0 {
                        break;
                    }
                }
                k += 1;
            }
            if value != 0 && k <= scan.se {
                block[NATURAL[k]] = value;
            }
            k += 1;
        }
    }
    if *eobrun > 0 {
        // The rest of the band in an end-of-band run: only the coefficients already there are refined.
        while k <= scan.se {
            let z = NATURAL[k];
            if block[z] != 0 {
                refine(bits, &mut block[z]);
            }
            k += 1;
        }
        *eobrun -= 1;
    }
    Ok(())
}

/// A block's coefficients dequantized and turned into pixels, put in its component's plane at `scale`.
fn put_block(c: &mut Component, block: &[i32; 64], q: &[u16; 64], bx: usize, by: usize, scale: usize) {
    let size = 8 / scale;
    let (x0, y0) = (bx * size, by * size);
    if (y0 + size) * c.plane_w > c.plane.len() || x0 + size > c.plane_w {
        return;
    }
    if scale == 8 {
        // The block's average: its DC.
        let v = ((block[0] as i64 * q[0] as i64 + 4) >> 3) + 128;
        c.plane[y0 * c.plane_w + x0] = v.clamp(0, 255) as u8;
        return;
    }
    let pixels = idct(block, q);
    for y in 0..size {
        for x in 0..size {
            let mut sum = 0u32;
            for dy in 0..scale {
                for dx in 0..scale {
                    sum += pixels[(y * scale + dy) * 8 + x * scale + dx] as u32;
                }
            }
            let n = (scale * scale) as u32;
            c.plane[(y0 + y) * c.plane_w + x0 + x] = ((sum + n / 2) / n) as u8;
        }
    }
}

// libjpeg's accurate integer inverse DCT (jidctint.c): 13-bit constants, two passes.
// (In 64 bits, where libjpeg uses 32: the same for any picture, and no overflow for a damaged one.)
const CONST_BITS: i64 = 13;
const PASS1_BITS: i64 = 2;
const FIX_0_298631336: i64 = 2446;
const FIX_0_390180644: i64 = 3196;
const FIX_0_541196100: i64 = 4433;
const FIX_0_765366865: i64 = 6270;
const FIX_0_899976223: i64 = 7373;
const FIX_1_175875602: i64 = 9633;
const FIX_1_501321110: i64 = 12299;
const FIX_1_847759065: i64 = 15137;
const FIX_1_961570560: i64 = 16069;
const FIX_2_053119869: i64 = 16819;
const FIX_2_562915447: i64 = 20995;
const FIX_3_072711026: i64 = 25172;

fn descale(x: i64, n: i64) -> i64 {
    (x + (1 << (n - 1))) >> n
}

/// One pass of the inverse DCT over 8 values: the even part, the odd part, and the 8 sums and differences of the two.
fn idct_1d(v: [i64; 8]) -> [i64; 8] {
    let (z2, z3) = (v[2], v[6]);
    let z1 = (z2 + z3) * FIX_0_541196100;
    let tmp2 = z1 + z3 * -FIX_1_847759065;
    let tmp3 = z1 + z2 * FIX_0_765366865;
    let tmp0 = (v[0] + v[4]) << CONST_BITS;
    let tmp1 = (v[0] - v[4]) << CONST_BITS;
    let (tmp10, tmp13, tmp11, tmp12) = (tmp0 + tmp3, tmp0 - tmp3, tmp1 + tmp2, tmp1 - tmp2);
    let (mut t0, mut t1, mut t2, mut t3) = (v[7], v[5], v[3], v[1]);
    let (mut z1, mut z2, mut z3, mut z4) = (t0 + t3, t1 + t2, t0 + t2, t1 + t3);
    let z5 = (z3 + z4) * FIX_1_175875602;
    t0 *= FIX_0_298631336;
    t1 *= FIX_2_053119869;
    t2 *= FIX_3_072711026;
    t3 *= FIX_1_501321110;
    z1 *= -FIX_0_899976223;
    z2 *= -FIX_2_562915447;
    z3 *= -FIX_1_961570560;
    z4 *= -FIX_0_390180644;
    z3 += z5;
    z4 += z5;
    t0 += z1 + z3;
    t1 += z2 + z4;
    t2 += z2 + z3;
    t3 += z1 + z4;
    [tmp10 + t3, tmp11 + t2, tmp12 + t1, tmp13 + t0, tmp13 - t0, tmp12 - t1, tmp11 - t2, tmp10 - t3]
}

/// A block's pixels, from its coefficients (natural order) and quantization table.
fn idct(block: &[i32; 64], q: &[u16; 64]) -> [u8; 64] {
    let mut work = [0i64; 64];
    // Columns: dequantized, and kept with PASS1_BITS more precision.
    for col in 0..8 {
        let v: [i64; 8] = core::array::from_fn(|r| block[r * 8 + col] as i64 * q[r * 8 + col] as i64);
        if v[1..].iter().all(|&x| x == 0) {
            let dc = v[0] << PASS1_BITS;
            for r in 0..8 {
                work[r * 8 + col] = dc;
            }
            continue;
        }
        let out = idct_1d(v);
        for r in 0..8 {
            work[r * 8 + col] = descale(out[r], CONST_BITS - PASS1_BITS);
        }
    }
    // Rows: to pixels, shifted up by 128 and kept to 0..255.
    let mut pixels = [0u8; 64];
    for row in 0..8 {
        let v: [i64; 8] = core::array::from_fn(|c| work[row * 8 + c]);
        let out: [i64; 8] = if v[1..].iter().all(|&x| x == 0) {
            [descale(v[0], PASS1_BITS + 3); 8]
        } else {
            let o = idct_1d(v);
            core::array::from_fn(|c| descale(o[c], CONST_BITS + PASS1_BITS + 3))
        };
        for c in 0..8 {
            pixels[row * 8 + c] = (out[c] + 128).clamp(0, 255) as u8;
        }
    }
    pixels
}

/// The components' planes as RGB: upsampled to the picture's size (libjpeg's "fancy" way where colour has half the
/// resolution across, or across and down; else each sample repeated), then YCbCr (or Adobe's RGB, CMYK or YCCK, or
/// grey) to RGB.
fn colour(f: &Frame, scale: usize, adobe: Option<u8>) -> Picture {
    let (w, h) = (f.width.div_ceil(scale), f.height.div_ceil(scale));
    let full: Vec<Vec<u8>> = f
        .components
        .iter()
        .map(|c| {
            let (cw, ch) = (c.width.div_ceil(scale).max(1), c.height.div_ceil(scale).max(1));
            let (hs, vs) = (f.hmax / c.h, f.vmax / c.v);
            upsample(&c.plane, c.plane_w, cw, ch, hs, vs, w, h)
        })
        .collect();
    let mut rgb = vec![0u8; w * h * 3];
    let n = f.components.len();
    // Adobe's marker says whether three or four components are YCbCr / YCCK (1, 2) or as they are (0); without it, three
    // whose ids are R, G and B are RGB.
    let transform = adobe.unwrap_or(if n == 3 && f.components.iter().map(|c| c.id).eq([b'R', b'G', b'B']) { 0 } else { 1 });
    for i in 0..w * h {
        let px = match n {
            1 => [full[0][i]; 3],
            3 if transform == 0 => [full[0][i], full[1][i], full[2][i]],
            3 => ycc(full[0][i], full[1][i], full[2][i]),
            _ => {
                // Adobe's CMYK is stored inverted (255 is none of the ink); YCCK is YCbCr of that, and K.
                let cmy = if transform == 2 { ycc(full[0][i], full[1][i], full[2][i]) } else { [full[0][i], full[1][i], full[2][i]] };
                let k = full[3][i] as u32;
                [(cmy[0] as u32 * k / 255) as u8, (cmy[1] as u32 * k / 255) as u8, (cmy[2] as u32 * k / 255) as u8]
            }
        };
        rgb[i * 3..i * 3 + 3].copy_from_slice(&px);
    }
    Picture { width: w as u32, height: h as u32, rgb }
}

/// JFIF's YCbCr to RGB, as libjpeg works it out (16-bit fixed point).
fn ycc(y: u8, cb: u8, cr: u8) -> [u8; 3] {
    const ONE_HALF: i32 = 1 << 15;
    let (y, cb, cr) = (y as i32, cb as i32 - 128, cr as i32 - 128);
    let r = y + ((91_881 * cr + ONE_HALF) >> 16);
    let g = y + ((-22_554 * cb + ONE_HALF - 46_802 * cr) >> 16);
    let b = y + ((116_130 * cb + ONE_HALF) >> 16);
    [r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8]
}

/// A plane (`pw` wide; its real samples `cw` × `ch`) at the picture's size `w` × `h`, from `hs` × `vs` times smaller.
#[allow(clippy::too_many_arguments)]
fn upsample(plane: &[u8], pw: usize, cw: usize, ch: usize, hs: usize, vs: usize, w: usize, h: usize) -> Vec<u8> {
    let at = |x: usize, y: usize| plane.get(y.min(ch - 1) * pw + x.min(cw - 1)).copied().unwrap_or(0) as i32;
    let mut out = vec![0u8; w * h];
    match (hs, vs) {
        (1, 1) => {
            for y in 0..h {
                for x in 0..w {
                    out[y * w + x] = at(x, y) as u8;
                }
            }
        }
        (2, 1) | (2, 2) => {
            // Each output sample from the nearest input sample (3/4) and its neighbour across (1/4), and for (2, 2)
            // down too: libjpeg's h2v1 and h2v2 "fancy" upsampling, the picture's edges repeated.
            for y in 0..h {
                let (row, other) = if vs == 2 {
                    let r = y / 2;
                    (r, if y % 2 == 0 { r.saturating_sub(1) } else { (r + 1).min(ch - 1) })
                } else {
                    (y, y)
                };
                let colsum = |x: usize| -> i32 { if vs == 2 { at(x, row) * 3 + at(x, other) } else { at(x, row) } };
                for x in 0..w {
                    let i = x / 2;
                    let this = colsum(i);
                    let v = if vs == 2 {
                        if x % 2 == 0 {
                            if i == 0 { (this * 4 + 8) >> 4 } else { (this * 3 + colsum(i - 1) + 8) >> 4 }
                        } else if i + 1 >= cw {
                            (this * 4 + 7) >> 4
                        } else {
                            (this * 3 + colsum(i + 1) + 7) >> 4
                        }
                    } else if x % 2 == 0 {
                        if i == 0 { this } else { (this * 3 + colsum(i - 1) + 1) >> 2 }
                    } else if i + 1 >= cw {
                        this
                    } else {
                        (this * 3 + colsum(i + 1) + 2) >> 2
                    };
                    out[y * w + x] = v.clamp(0, 255) as u8;
                }
            }
        }
        _ => {
            for y in 0..h {
                for x in 0..w {
                    out[y * w + x] = at(x / hs, y / vs) as u8;
                }
            }
        }
    }
    out
}

/// EXIF's orientation (its first image's tag 0x0112), from the TIFF structure after "Exif\0\0".
fn exif_orientation(t: &[u8]) -> Option<u8> {
    let le = match t.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |i: usize| -> Option<u16> {
        let b: [u8; 2] = t.get(i..i + 2)?.try_into().ok()?;
        Some(if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |i: usize| -> Option<u32> {
        let b: [u8; 4] = t.get(i..i + 4)?.try_into().ok()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let ifd = u32_at(4)? as usize;
    let n = u16_at(ifd)? as usize;
    for i in 0..n.min(500) {
        let e = ifd + 2 + 12 * i;
        if u16_at(e)? == 0x0112 {
            let v = u16_at(e + 8)?;
            return (1..=8).contains(&v).then_some(v as u8);
        }
    }
    None
}
