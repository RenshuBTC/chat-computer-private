//! A Word document from before 2007 (.doc: Word 97 to 2003's binary format, [MS-DOC]; and Word 6 and 95's, simply):
//! its words in order, as the .docx reader gives them: headings and lists marked as Markdown marks them, tables as
//! Markdown tables, tabs and line breaks kept, a field's result read (not its code); then its footnotes, endnotes,
//! comments and text boxes.
//!
//! The text is in the WordDocument stream, in pieces the piece table (in the table stream's CLX) puts in order: each
//! piece's characters one byte each (Windows' Latin code page) or two (UTF-16). A paragraph's properties (in a table,
//! the end of a table's row, a list's item, its style) are in the FKPs the bin table finds by where its mark is in
//! the stream; its style's (a heading's level) in the style sheet.

use crate::cfb::Compound;
use crate::{Error, Out, markdown_table};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// The most characters read from a document (its stories together: a thousand pages' or so).
const MOST_CHARS: u32 = 2 << 20;

/// A piece of the text: its characters' positions (CPs from `cp` to `end`), and where they are in the WordDocument
/// stream (a byte offset), one byte each or two.
struct Piece {
    cp: u32,
    end: u32,
    fc: u32,
    compressed: bool,
}

/// What a paragraph's properties make it.
#[derive(Default, Clone, Copy)]
struct Props {
    istd: u16,
    in_table: bool,
    row_end: bool,
    list: Option<u8>,
    outline: Option<u8>,
}

/// A style: its built-in identity (sti: 1 to 9 are the headings, 62 the title), name, outline level and whether it
/// makes a list.
struct Style {
    sti: u16,
    name: String,
    outline: Option<u8>,
    list: bool,
}

struct Doc<'a> {
    wd: &'a [u8],
    pieces: Vec<Piece>,
    /// The bin table of paragraphs' properties: where each FKP's paragraphs begin (FCs) and its page.
    bin_fcs: Vec<u32>,
    bin_pages: Vec<u32>,
    styles: Vec<Style>,
}

pub fn read(cf: &Compound) -> Result<String, Error> {
    let wd = cf.stream("WordDocument").ok_or(Error::Damaged)?;
    let ident = u16_at(&wd, 0).ok_or(Error::Damaged)?;
    if ident != 0xA5EC && ident != 0xA5DC {
        return Err(Error::Damaged);
    }
    let nfib = u16_at(&wd, 2).ok_or(Error::Damaged)?;
    let flags = u16_at(&wd, 0x0A).ok_or(Error::Damaged)?;
    // (Encrypted, or obfuscated with a password's XOR.)
    if flags & 0x0100 != 0 || flags & 0x8000 != 0 {
        return Err(Error::Encrypted);
    }
    if nfib < 0x00C1 {
        return Ok(old_word(&wd, flags));
    }
    let table = cf.stream(if flags & 0x0200 != 0 { "1Table" } else { "0Table" }).ok_or(Error::Damaged)?;
    // The FIB: its base (32 bytes), then counts and arrays of 16-bit values, 32-bit ones (the stories' lengths),
    // and pairs of where and how long (the table stream's structures).
    let csw = u16_at(&wd, 0x20).ok_or(Error::Damaged)? as usize;
    let lw = 0x22 + 2 * csw;
    let cslw = u16_at(&wd, lw).ok_or(Error::Damaged)? as usize;
    let ccp = |i: usize| if i < cslw { u32_at(&wd, lw + 2 + 4 * i).unwrap_or(0) } else { 0 };
    let rg = lw + 2 + 4 * cslw;
    let pairs = u16_at(&wd, rg).ok_or(Error::Damaged)? as usize;
    let pair = |i: usize| -> (usize, usize) {
        if i >= pairs {
            return (0, 0);
        }
        (u32_at(&wd, rg + 2 + 8 * i).unwrap_or(0) as usize, u32_at(&wd, rg + 2 + 8 * i + 4).unwrap_or(0) as usize)
    };
    let slice = |(at, len): (usize, usize)| table.get(at..at.saturating_add(len)).unwrap_or(&[]);
    let pieces = pieces(slice(pair(33)), false).ok_or(Error::Damaged)?;
    let (bin_fcs, bin_pages) = bin_table(slice(pair(13)));
    let doc = Doc { wd: &wd, pieces, bin_fcs, bin_pages, styles: styles(slice(pair(1))) };
    // The stories, one after another in the text: the main document's, then footnotes', headers', comments',
    // endnotes' and text boxes'.
    let (text, footnotes, headers, macros, comments, endnotes, boxes) = (ccp(3), ccp(4), ccp(5), ccp(6), ccp(7), ccp(8), ccp(9));
    let mut out = Out::new();
    let mut at = 0u32;
    let mut stories = Vec::new();
    for (len, title) in [(text, ""), (footnotes, "Footnotes"), (headers, ""), (macros, ""), (comments, "Comments"), (endnotes, "Endnotes"), (boxes, "Text boxes")] {
        let end = at.saturating_add(len).min(MOST_CHARS);
        stories.push((at, end, title));
        at = end;
    }
    for (i, (from, to, title)) in stories.into_iter().enumerate() {
        if from >= to || (i > 0 && title.is_empty()) {
            continue;
        }
        let chars = doc.chars(from, to);
        if title.is_empty() {
            doc.story(&chars, &mut out);
            continue;
        }
        let mut these = Out::new();
        doc.story(&chars, &mut these);
        let these = these.done();
        if !these.trim().is_empty() {
            out.gap();
            out.line(&format!("{title}:"));
            for l in these.lines().filter(|l| !l.trim().is_empty()) {
                out.line(l.trim());
            }
        }
    }
    Ok(out.done())
}

/// The piece table, from the CLX: its property lists (passed over), then the pieces. Word 6 and 95's pieces are
/// one byte a character, where they are.
fn pieces(clx: &[u8], old: bool) -> Option<Vec<Piece>> {
    let mut i = 0;
    while i < clx.len() {
        match clx[i] {
            1 => i += 3 + u16_at(clx, i + 1)? as usize,
            2 => {
                let len = u32_at(clx, i + 1)? as usize;
                let plc = clx.get(i + 5..i + 5 + len)?;
                let n = len.checked_sub(4)? / 12;
                let mut out = Vec::with_capacity(n);
                for k in 0..n {
                    let (cp, end) = (u32_at(plc, 4 * k)?, u32_at(plc, 4 * (k + 1))?);
                    let raw = u32_at(plc, 4 * (n + 1) + 8 * k + 2)?;
                    let compressed = old || raw & 0x4000_0000 != 0;
                    let fc = if old {
                        raw
                    } else if compressed {
                        (raw & 0x3FFF_FFFF) / 2
                    } else {
                        raw & 0x3FFF_FFFF
                    };
                    if end > cp {
                        out.push(Piece { cp, end, fc, compressed });
                    }
                }
                return Some(out);
            }
            _ => return None,
        }
    }
    None
}

/// The bin table of paragraphs' properties (PlcBtePapx): FCs, then each FKP's page number.
fn bin_table(plc: &[u8]) -> (Vec<u32>, Vec<u32>) {
    if plc.len() < 4 {
        return (Vec::new(), Vec::new());
    }
    let n = (plc.len() - 4) / 8;
    let fcs = (0..=n).filter_map(|k| u32_at(plc, 4 * k)).collect();
    let pages = (0..n).filter_map(|k| u32_at(plc, 4 * (n + 1) + 4 * k)).map(|p| p & 0x003F_FFFF).collect();
    (fcs, pages)
}

/// The style sheet's styles, by number (istd).
fn styles(stsh: &[u8]) -> Vec<Style> {
    let mut out = Vec::new();
    let Some(cb_info) = u16_at(stsh, 0).map(|v| v as usize) else { return out };
    let Some(info) = stsh.get(2..2 + cb_info) else { return out };
    let count = u16_at(info, 0).unwrap_or(0) as usize;
    let base_len = u16_at(info, 2).unwrap_or(10) as usize;
    let mut at = 2 + cb_info;
    for _ in 0..count.min(4096) {
        let Some(cb) = u16_at(stsh, at).map(|v| v as usize) else { break };
        at += 2;
        let Some(std) = stsh.get(at..at + cb) else { break };
        at += cb;
        let mut style = Style { sti: 0x0FFE, name: String::new(), outline: None, list: false };
        if cb >= base_len {
            style.sti = u16_at(std, 0).unwrap_or(0x0FFE) & 0x0FFF;
            let paragraph = u16_at(std, 2).unwrap_or(0) & 0x000F == 1;
            // Its name (UTF-16, with its length before it and a zero after), then its properties: a paragraph
            // style's paragraph properties first (their length, the style's number, then the sprms).
            if let Some(cch) = u16_at(std, base_len).map(|v| v as usize) {
                let units: Vec<u16> = (0..cch.min(255)).filter_map(|k| u16_at(std, base_len + 2 + 2 * k)).collect();
                style.name = String::from_utf16_lossy(&units);
                let mut upx = base_len + 2 + 2 * cch + 2;
                upx += upx % 2;
                if paragraph && let Some(len) = u16_at(std, upx).map(|v| v as usize) {
                    if let Some(grpprl) = std.get(upx + 4..upx + 2 + len) {
                        for (sprm, op) in sprms(grpprl) {
                            match sprm {
                                0x460B => style.list = u16_at(op, 0).unwrap_or(0) != 0,
                                0x2640 => style.outline = op.first().copied().filter(|&l| l < 9),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        out.push(style);
    }
    out
}

/// A list of sprms (properties: each a 16-bit name, whose top three bits say how long its operand is), with their
/// operands.
fn sprms(g: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 <= g.len() {
        let sprm = u16_at(g, i).unwrap_or(0);
        i += 2;
        let len = match sprm >> 13 {
            0 | 1 => 1,
            2 | 4 | 5 => 2,
            3 => 4,
            7 => 3,
            _ => match sprm {
                // (sprmTDefTable and sprmTDefTable10: a 16-bit length, one more than what follows it.)
                0xD608 | 0xD606 => 2 + (u16_at(g, i).unwrap_or(0) as usize).saturating_sub(1),
                0xC615 if g.get(i) == Some(&255) => break,
                _ => 1 + *g.get(i).unwrap_or(&0) as usize,
            },
        };
        let Some(op) = g.get(i..i + len) else { break };
        out.push((sprm, op));
        i += len;
    }
    out
}

/// A byte of 8-bit text as a character, in Windows' Latin code page (1252).
pub(crate) fn cp1252_char(b: u8) -> char {
    char::from_u32(cp1252(b) as u32).unwrap_or('\u{FFFD}')
}

/// A byte of 8-bit text as Windows' Latin code page (1252) has it.
fn cp1252(b: u8) -> u16 {
    const HIGH: [u16; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F,
        0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    if (0x80..0xA0).contains(&b) { HIGH[b as usize - 0x80] } else { b as u16 }
}

impl Doc<'_> {
    /// The characters from CP `from` to `to` (each a UTF-16 unit, with where it is in the WordDocument stream).
    fn chars(&self, from: u32, to: u32) -> Vec<(u16, u32)> {
        let mut out = Vec::new();
        for p in &self.pieces {
            let (a, b) = (p.cp.max(from), p.end.min(to));
            for cp in a..b {
                let k = cp - p.cp;
                if p.compressed {
                    let fc = p.fc.saturating_add(k);
                    let Some(&byte) = self.wd.get(fc as usize) else { break };
                    out.push((cp1252(byte), fc));
                } else {
                    let fc = p.fc.saturating_add(2 * k);
                    let Some(u) = u16_at(self.wd, fc as usize) else { break };
                    out.push((u, fc));
                }
            }
        }
        out
    }

    /// The properties of the paragraph whose mark is at `fc` in the stream.
    fn props(&self, fc: u32) -> Props {
        let mut p = Props::default();
        let Some(k) = self.bin_fcs.windows(2).position(|w| w[0] <= fc && fc < w[1]) else { return p };
        let Some(&page) = self.bin_pages.get(k) else { return p };
        let at = page as usize * 512;
        let Some(fkp) = self.wd.get(at..at + 512) else { return p };
        let runs = fkp[511] as usize;
        for j in 0..runs {
            let (Some(a), Some(b)) = (u32_at(fkp, 4 * j), u32_at(fkp, 4 * (j + 1))) else { break };
            if !(a <= fc && fc < b) {
                continue;
            }
            let off = *fkp.get(4 * (runs + 1) + 13 * j).unwrap_or(&0) as usize * 2;
            if off == 0 || off >= 511 {
                return p;
            }
            let cb = fkp[off] as usize;
            let (start, len) = if cb != 0 { (off + 1, 2 * cb - 1) } else { (off + 2, 2 * *fkp.get(off + 1).unwrap_or(&0) as usize) };
            let Some(papx) = fkp.get(start..(start + len).min(511)) else { return p };
            p.istd = u16_at(papx, 0).unwrap_or(0);
            for (sprm, op) in sprms(papx.get(2..).unwrap_or(&[])) {
                let v = op.first().copied().unwrap_or(0);
                match sprm {
                    0x2416 => p.in_table = v != 0,
                    0x2417 => p.row_end = v != 0,
                    0x6649 => p.in_table |= u32_at(op, 0).unwrap_or(0) > 0,
                    0x460B => {
                        if u16_at(op, 0).unwrap_or(0) == 0 {
                            p.list = None;
                        } else if p.list.is_none() {
                            p.list = Some(0);
                        }
                    }
                    0x260A => p.list = p.list.map(|_| v.min(8)),
                    0x2640 => p.outline = Some(v).filter(|&l| l < 9),
                    _ => {}
                }
            }
            return p;
        }
        p
    }

    /// A paragraph's level as a heading (1 to 6), from its own outline level, its style's, or its style's being a
    /// heading or the title.
    fn heading(&self, p: &Props) -> u8 {
        let style = self.styles.get(p.istd as usize);
        let level = p.outline.or_else(|| style.and_then(|s| s.outline)).map(|l| l + 1);
        let level = level.or_else(|| {
            let s = style?;
            match s.sti {
                1..=9 => Some(s.sti as u8),
                62 => Some(1),
                _ => {
                    let name = s.name.to_ascii_lowercase();
                    let n = name.strip_prefix("heading ")?.parse::<u8>().ok()?;
                    (1..=9).contains(&n).then_some(n)
                }
            }
        });
        level.unwrap_or(0).min(6)
    }

    /// A story's paragraphs, as lines (and tables) of the text.
    fn story(&self, chars: &[(u16, u32)], out: &mut Out) {
        let mut para: Vec<u16> = Vec::new();
        // (Fields: whether each one open is showing its result yet, past its code.)
        let mut fields: Vec<bool> = Vec::new();
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut row: Vec<String> = Vec::new();
        let mut cell = String::new();
        for &(u, fc) in chars {
            match u {
                0x13 => {
                    fields.push(false);
                    continue;
                }
                0x14 => {
                    if let Some(f) = fields.last_mut() {
                        *f = true;
                    }
                    continue;
                }
                0x15 => {
                    fields.pop();
                    continue;
                }
                _ if fields.iter().any(|showing| !showing) => continue,
                0x0D | 0x07 | 0x0C => {}
                0x0B | 0x0E => {
                    para.push(b'\n' as u16);
                    continue;
                }
                0x09 => {
                    para.push(u);
                    continue;
                }
                0x1E => {
                    para.push(b'-' as u16);
                    continue;
                }
                0xA0 => {
                    para.push(b' ' as u16);
                    continue;
                }
                0x1F | 0xFEFF => continue,
                u if u < 0x20 => continue,
                u => {
                    para.push(u);
                    continue;
                }
            }
            // A paragraph ends (or a table's cell, or its row).
            let text = String::from_utf16_lossy(&para);
            para.clear();
            let p = self.props(fc);
            if p.in_table || p.row_end {
                let text = text.replace('\n', " ");
                let text = text.trim();
                if p.row_end {
                    if !cell.is_empty() {
                        row.push(core::mem::take(&mut cell));
                    }
                    rows.push(core::mem::take(&mut row));
                } else {
                    if !text.is_empty() {
                        if !cell.is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(text);
                    }
                    if u == 0x07 {
                        row.push(core::mem::take(&mut cell));
                    }
                }
                continue;
            }
            if !rows.is_empty() || !row.is_empty() {
                if !row.is_empty() {
                    rows.push(core::mem::take(&mut row));
                }
                markdown_table(&rows, out);
                rows.clear();
            }
            self.paragraph(&text, &p, out);
        }
        if !para.is_empty() {
            let text = String::from_utf16_lossy(&para);
            self.paragraph(&text, &Props::default(), out);
        }
        if !row.is_empty() {
            rows.push(row);
        }
        if !rows.is_empty() {
            markdown_table(&rows, out);
        }
    }

    fn paragraph(&self, text: &str, p: &Props, out: &mut Out) {
        let text = text.trim_end();
        let heading = self.heading(p);
        let list = p.list.or_else(|| self.styles.get(p.istd as usize).filter(|s| s.list).map(|_| 0));
        if text.trim().is_empty() {
            out.gap();
        } else if heading > 0 {
            out.gap();
            out.line(&format!("{} {}", "#".repeat(heading as usize), text.trim().replace('\n', " ")));
            out.gap();
        } else if let Some(level) = list {
            out.line(&format!("{}- {}", "  ".repeat(level as usize), text.trim_start()));
        } else {
            out.line(text);
        }
    }
}

/// Word 6 and 95's documents: their text one byte a character, from fcMin to fcMac (the main document's first, as
/// long as ccpText says), or in pieces where it was saved quickly (fComplex).
fn old_word(wd: &[u8], flags: u16) -> String {
    let fc_min = u32_at(wd, 0x18).unwrap_or(0);
    let fc_mac = u32_at(wd, 0x1C).unwrap_or(0);
    let ccp_text = u32_at(wd, 0x34).unwrap_or(0);
    let mut chars: Vec<(u16, u32)> = Vec::new();
    let complex = flags & 0x0004 != 0;
    let pieces = if complex {
        let (at, len) = (u32_at(wd, 0x160).unwrap_or(0) as usize, u32_at(wd, 0x164).unwrap_or(0) as usize);
        wd.get(at..at.saturating_add(len)).and_then(|clx| pieces(clx, true))
    } else {
        None
    };
    match pieces {
        Some(pieces) => {
            for p in pieces {
                for cp in p.cp..p.end.min(ccp_text.max(1)).min(MOST_CHARS) {
                    let fc = p.fc + (cp - p.cp);
                    if let Some(&b) = wd.get(fc as usize) {
                        chars.push((cp1252(b), fc));
                    }
                }
            }
        }
        None => {
            let end = if ccp_text > 0 { fc_min.saturating_add(ccp_text).min(fc_mac) } else { fc_mac };
            for fc in fc_min..end.min(fc_min.saturating_add(MOST_CHARS)) {
                if let Some(&b) = wd.get(fc as usize) {
                    chars.push((cp1252(b), fc));
                }
            }
        }
    }
    // (Paragraphs' properties aren't read: tables' cells are separated, each row a line.)
    let mut out = Out::new();
    let mut line = String::new();
    let mut fields: Vec<bool> = Vec::new();
    for (u, _) in chars {
        match u {
            0x13 => fields.push(false),
            0x14 => {
                if let Some(f) = fields.last_mut() {
                    *f = true;
                }
            }
            0x15 => {
                fields.pop();
            }
            _ if fields.iter().any(|showing| !showing) => {}
            0x0D | 0x0C => {
                let t = core::mem::take(&mut line);
                if t.trim().is_empty() { out.gap() } else { out.line(t.trim_end()) }
            }
            0x07 => line.push_str(" | "),
            0x0B => line.push('\n'),
            0x09 => line.push('\t'),
            0xA0 => line.push(' '),
            u if u < 0x20 => {}
            u => line.push(char::from_u32(u as u32).unwrap_or('\u{FFFD}')),
        }
    }
    if !line.trim().is_empty() {
        out.line(line.trim_end());
    }
    out.done()
}
