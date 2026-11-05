//! An Excel workbook from before 2007 (.xls: Excel 97 to 2003's BIFF8, [MS-XLS]; and Excel 5 and 95's BIFF5): each
//! sheet's cells as the .xlsx reader gives them, CSV with each row's number and each column's letter. The Workbook
//! stream is a list of records: the workbook's own (its sheets' names and where they start, the shared strings,
//! number formats, the date system), then each sheet's (its cells: numbers, shared strings, words, true and false,
//! errors, and formulas with the value they last came to, shown with the formula: "1612.5 (=SUM(B2:B3))").
//!
//! A formula is kept as tokens in reverse Polish order; the usual ones (numbers, words, references, ranges, other
//! sheets' cells, operators, the common functions, names) are written out as Excel shows them, and a formula with
//! others shows its value alone.

use crate::cfb::Compound;
use crate::excel::{Format, format_of, number};
use crate::{Error, Out, column_name, sheet_csv};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn f64_at(b: &[u8], i: usize) -> Option<f64> {
    Some(f64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

const BOF: u16 = 0x0809;
const EOF: u16 = 0x000A;
const CONTINUE: u16 = 0x003C;
/// The most cells read from a workbook.
const MOST_CELLS: usize = 2_000_000;

/// A record: its type and its bytes.
struct Record<'a> {
    kind: u16,
    data: &'a [u8],
    /// Where the next one starts.
    next: usize,
}

fn record(stream: &[u8], at: usize) -> Option<Record<'_>> {
    let kind = u16_at(stream, at)?;
    let len = u16_at(stream, at + 2)? as usize;
    let data = stream.get(at + 4..at + 4 + len)?;
    Some(Record { kind, data, next: at + 4 + len })
}

/// A sheet, as the workbook lists it.
struct Sheet {
    name: String,
    at: usize,
    hidden: bool,
    kind: u8,
}

/// What the workbook's own records say.
struct Book {
    biff8: bool,
    shared: Vec<String>,
    /// Each XF's number format.
    formats: Vec<Format>,
    date1904: bool,
    sheets: Vec<Sheet>,
    /// For other sheets' cells in formulas: each XTI's sheet (None: another workbook's).
    xti: Vec<Option<usize>>,
    names: Vec<String>,
}

pub fn read(cf: &Compound) -> Result<String, Error> {
    let stream = cf.stream("Workbook").or_else(|| cf.stream("Book")).ok_or(Error::Damaged)?;
    let first = record(&stream, 0).ok_or(Error::Damaged)?;
    if first.kind != BOF && first.kind != 0x0409 && first.kind != 0x0209 {
        return Err(Error::Damaged);
    }
    let biff8 = u16_at(first.data, 0) == Some(0x0600);
    let mut book = Book { biff8, shared: Vec::new(), formats: Vec::new(), date1904: false, sheets: Vec::new(), xti: Vec::new(), names: Vec::new() };
    let mut codes: BTreeMap<u16, String> = BTreeMap::new();
    let mut xf_formats: Vec<u16> = Vec::new();
    let mut supbooks: Vec<bool> = Vec::new();
    let mut at = first.next;
    while let Some(r) = record(&stream, at) {
        at = r.next;
        match r.kind {
            EOF => break,
            0x002F => return Err(Error::Encrypted),
            0x0022 => book.date1904 = u16_at(r.data, 0) == Some(1),
            0x041E | 0x001E => {
                if let Some(id) = u16_at(r.data, 0) {
                    let code = if biff8 { unicode_string(r.data, 2).map(|s| s.0) } else { byte_string(r.data, 2) };
                    codes.insert(id, code.unwrap_or_default());
                }
            }
            0x00E0 => xf_formats.push(u16_at(r.data, 2).unwrap_or(0)),
            0x0085 => {
                let at = u32_at(r.data, 0).unwrap_or(0) as usize;
                let hidden = r.data.get(4).copied().unwrap_or(0) != 0;
                let kind = r.data.get(5).copied().unwrap_or(0);
                let name = if biff8 { short_string(r.data, 6) } else { byte_string_u8(r.data, 6) };
                book.sheets.push(Sheet { name: name.unwrap_or_default(), at, hidden, kind });
            }
            0x00FC => {
                // The shared strings: in this record and the CONTINUE records after it.
                let mut parts = alloc::vec![r.data];
                let mut k = r.next;
                while let Some(c) = record(&stream, k) {
                    if c.kind != CONTINUE {
                        break;
                    }
                    parts.push(c.data);
                    k = c.next;
                }
                book.shared = shared_strings(&parts);
            }
            0x01AE => supbooks.push(u16_at(r.data, 2) == Some(0x0401)),
            0x0017 if biff8 => {
                let n = u16_at(r.data, 0).unwrap_or(0) as usize;
                for i in 0..n {
                    let sup = u16_at(r.data, 2 + 6 * i).unwrap_or(0) as usize;
                    let tab = u16_at(r.data, 4 + 6 * i).unwrap_or(0) as usize;
                    book.xti.push(supbooks.get(sup).copied().unwrap_or(false).then_some(tab));
                }
            }
            0x0018 if biff8 => {
                let options = u16_at(r.data, 0).unwrap_or(0);
                let len = r.data.get(3).copied().unwrap_or(0) as usize;
                let name = chars_at(r.data, 14, len).unwrap_or_default();
                // (A built-in name: one character says which.)
                let name = if options & 0x0020 != 0 {
                    String::from(match name.chars().next().map(|c| c as u32) {
                        Some(0x06) => "Print_Area",
                        Some(0x07) => "Print_Titles",
                        Some(0x0D) => "_FilterDatabase",
                        _ => "_builtin",
                    })
                } else {
                    name
                };
                book.names.push(name);
            }
            _ => {}
        }
    }
    book.formats = xf_formats.iter().map(|&id| format_of(id as u32, codes.get(&id).map(|s| s.as_str()))).collect();
    let mut out = Out::new();
    let mut cells = 0usize;
    for (i, sheet) in book.sheets.iter().enumerate() {
        // (Visual Basic's modules aren't sheets.)
        if sheet.kind == 6 {
            continue;
        }
        out.gap();
        out.line(&format!("## {}{}", sheet.name, if sheet.hidden { " (hidden)" } else { "" }));
        if sheet.kind == 2 {
            out.line("(a chart, not cells)");
            continue;
        }
        let rows = sheet_cells(&stream, sheet.at, &book, i, &mut cells);
        sheet_csv(&rows, &mut out);
    }
    Ok(out.done())
}

/// A string with its length in a byte and its options in the next (ShortXLUnicodeString).
fn short_string(b: &[u8], at: usize) -> Option<String> {
    let len = *b.get(at)? as usize;
    chars_at(b, at + 1, len)
}

/// Characters after their options' byte (one byte each, Latin-1, or two, UTF-16).
fn chars_at(b: &[u8], at: usize, len: usize) -> Option<String> {
    let wide = b.get(at)? & 1 != 0;
    let start = at + 1;
    if wide {
        let units: Vec<u16> = (0..len).filter_map(|k| u16_at(b, start + 2 * k)).collect();
        Some(String::from_utf16_lossy(&units))
    } else {
        Some(b.get(start..(start + len).min(b.len()))?.iter().map(|&c| c as char).collect())
    }
}

/// A string with a 16-bit length and an options byte (XLUnicodeString); and where it ends.
fn unicode_string(b: &[u8], at: usize) -> Option<(String, usize)> {
    let len = u16_at(b, at)? as usize;
    let wide = b.get(at + 2)? & 1 != 0;
    let s = chars_at(b, at + 2, len)?;
    Some((s, at + 3 + if wide { 2 * len } else { len }))
}

/// BIFF5's strings: a 16-bit length, then one byte a character (Windows' Latin code page).
fn byte_string(b: &[u8], at: usize) -> Option<String> {
    let len = u16_at(b, at)? as usize;
    Some(b.get(at + 2..(at + 2 + len).min(b.len()))?.iter().map(|&c| crate::doc::cp1252_char(c)).collect())
}

/// BIFF5's short strings: an 8-bit length.
fn byte_string_u8(b: &[u8], at: usize) -> Option<String> {
    let len = *b.get(at)? as usize;
    Some(b.get(at + 1..(at + 1 + len).min(b.len()))?.iter().map(|&c| crate::doc::cp1252_char(c)).collect())
}

/// The shared strings table (SST), its strings running on from record to record: where a string's characters
/// continue in the next record, that record starts with a byte saying whether they're one byte each or two.
fn shared_strings(parts: &[&[u8]]) -> Vec<String> {
    struct Reader<'a> {
        parts: &'a [&'a [u8]],
        part: usize,
        at: usize,
    }
    impl Reader<'_> {
        fn byte(&mut self) -> Option<u8> {
            while self.at >= self.parts.get(self.part)?.len() {
                self.part += 1;
                self.at = 0;
            }
            let b = self.parts[self.part][self.at];
            self.at += 1;
            Some(b)
        }
        fn u16(&mut self) -> Option<u16> {
            Some(u16::from_le_bytes([self.byte()?, self.byte()?]))
        }
        fn u32(&mut self) -> Option<u32> {
            Some(u32::from_le_bytes([self.byte()?, self.byte()?, self.byte()?, self.byte()?]))
        }
        fn skip(&mut self, mut n: usize) -> Option<()> {
            while n > 0 {
                let part = self.parts.get(self.part)?;
                if self.at >= part.len() {
                    self.part += 1;
                    self.at = 0;
                    continue;
                }
                let k = n.min(part.len() - self.at);
                self.at += k;
                n -= k;
            }
            Some(())
        }
        /// `n` characters, the options byte at a record's start switching their width.
        fn chars(&mut self, mut n: usize, mut wide: bool) -> Option<String> {
            let mut units: Vec<u16> = Vec::with_capacity(n);
            while n > 0 {
                let part = self.parts.get(self.part)?;
                if self.at >= part.len() {
                    self.part += 1;
                    self.at = 0;
                    wide = self.byte()? & 1 != 0;
                    continue;
                }
                if wide {
                    let lo = self.byte()?;
                    let hi = self.byte()?;
                    units.push(u16::from_le_bytes([lo, hi]));
                } else {
                    units.push(self.byte()? as u16);
                }
                n -= 1;
            }
            Some(String::from_utf16_lossy(&units))
        }
    }
    let mut r = Reader { parts, part: 0, at: 0 };
    let mut out = Vec::new();
    let (Some(_total), Some(unique)) = (r.u32(), r.u32()) else { return out };
    for _ in 0..unique.min(MOST_CELLS as u32) {
        let Some(len) = r.u16() else { break };
        let Some(options) = r.byte() else { break };
        let runs = if options & 0x08 != 0 { r.u16().unwrap_or(0) as usize } else { 0 };
        let ext = if options & 0x04 != 0 { r.u32().unwrap_or(0) as usize } else { 0 };
        let Some(s) = r.chars(len as usize, options & 1 != 0) else { break };
        out.push(s);
        if r.skip(4 * runs + ext).is_none() {
            break;
        }
    }
    out
}

/// A cell's error code as Excel writes it.
fn error_text(code: u8) -> &'static str {
    match code {
        0x00 => "#NULL!",
        0x07 => "#DIV/0!",
        0x0F => "#VALUE!",
        0x17 => "#REF!",
        0x1D => "#NAME?",
        0x24 => "#NUM!",
        0x2A => "#N/A",
        0x2B => "#GETTING_DATA",
        _ => "#ERROR",
    }
}

/// An RK number (a compressed one: a 30-bit integer, or a double's top 30 bits, perhaps a hundredth of it).
fn rk(v: u32) -> f64 {
    let n = if v & 2 != 0 { ((v as i32) >> 2) as f64 } else { f64::from_bits(((v & 0xFFFF_FFFC) as u64) << 32) };
    if v & 1 != 0 { n / 100.0 } else { n }
}

/// A number as Excel's files write it (the shortest form that reads back the same).
fn number_text(n: f64) -> String {
    if n == (n as i64) as f64 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

/// A sheet's cells, from its BOF to its EOF: rows (numbered from 1), each row's cells by column.
fn sheet_cells(stream: &[u8], at: usize, book: &Book, sheet: usize, cells: &mut usize) -> Vec<(u32, Vec<(usize, String)>)> {
    let mut rows: BTreeMap<u32, BTreeMap<usize, String>> = BTreeMap::new();
    let Some(bof) = record(stream, at) else { return Vec::new() };
    if bof.kind != BOF && bof.kind != 0x0409 && bof.kind != 0x0209 {
        return Vec::new();
    }
    let format = |xf: u16| book.formats.get(xf as usize).copied().unwrap_or(Format::General);
    let shown = |n: f64, xf: u16| number(&number_text(n), format(xf), book.date1904);
    // Shared formulas (by their range), and the formula waiting for its string (in the STRING record after it).
    let mut shared: Vec<(u16, u16, u8, u8, Vec<u8>)> = Vec::new();
    let mut pending: Option<(u32, usize, String)> = None;
    let mut put = |rows: &mut BTreeMap<u32, BTreeMap<usize, String>>, row: u16, col: u16, v: String| {
        if *cells < MOST_CELLS && !v.is_empty() {
            *cells += 1;
            rows.entry(row as u32 + 1).or_default().insert(col as usize, v);
        }
    };
    let mut k = bof.next;
    while let Some(r) = record(stream, k) {
        k = r.next;
        let d = r.data;
        let (row, col, xf) = (u16_at(d, 0).unwrap_or(0), u16_at(d, 2).unwrap_or(0), u16_at(d, 4).unwrap_or(0));
        match r.kind {
            EOF => break,
            BOF => {
                // (A chart inside the sheet: its own records, to its own EOF.)
                let mut depth = 1;
                while depth > 0 {
                    let Some(inner) = record(stream, k) else { break };
                    k = inner.next;
                    match inner.kind {
                        BOF => depth += 1,
                        EOF => depth -= 1,
                        _ => {}
                    }
                }
            }
            0x00FD => {
                let s = u32_at(d, 6).and_then(|i| book.shared.get(i as usize)).cloned().unwrap_or_default();
                put(&mut rows, row, col, s);
            }
            0x0203 => {
                if let Some(n) = f64_at(d, 6) {
                    put(&mut rows, row, col, shown(n, xf));
                }
            }
            0x027E => {
                if let Some(v) = u32_at(d, 6) {
                    put(&mut rows, row, col, shown(rk(v), xf));
                }
            }
            0x00BD => {
                let n = d.len().saturating_sub(6) / 6;
                for i in 0..n {
                    let xf = u16_at(d, 4 + 6 * i).unwrap_or(0);
                    if let Some(v) = u32_at(d, 6 + 6 * i) {
                        put(&mut rows, row, col.saturating_add(i as u16), shown(rk(v), xf));
                    }
                }
            }
            0x0204 | 0x00D6 => {
                let s = if book.biff8 { unicode_string(d, 6).map(|s| s.0) } else { byte_string(d, 6) };
                put(&mut rows, row, col, s.unwrap_or_default());
            }
            0x0205 => {
                let (v, error) = (d.get(6).copied().unwrap_or(0), d.get(7).copied().unwrap_or(0) != 0);
                let s = if error { error_text(v) } else if v != 0 { "TRUE" } else { "FALSE" };
                put(&mut rows, row, col, String::from(s));
            }
            0x04BC if book.biff8 => {
                // A shared formula: the range it's for, and its tokens (references relative to each cell).
                let (r0, r1) = (u16_at(d, 0).unwrap_or(0), u16_at(d, 2).unwrap_or(0));
                let (c0, c1) = (d.get(4).copied().unwrap_or(0), d.get(5).copied().unwrap_or(0));
                let len = u16_at(d, 8).unwrap_or(0) as usize;
                shared.push((r0, r1, c0, c1, d.get(10..10 + len).unwrap_or(&[]).to_vec()));
                // (The formula waiting for it is written out again, now that its tokens are here.)
            }
            0x0006 => {
                let value = d.get(6..14).unwrap_or(&[0; 8]);
                let len = u16_at(d, 20).unwrap_or(0) as usize;
                let tokens = d.get(22..22 + len).unwrap_or(&[]);
                let mut formula = if book.biff8 { decode(tokens, book, sheet, row, col, &shared, stream, k) } else { None };
                // (A true or false written as a formula, as LibreOffice writes them, is just that.)
                if matches!(formula.as_deref(), Some("" | "TRUE" | "FALSE" | "TRUE()" | "FALSE()")) {
                    formula = None;
                }
                let with = |v: String| match &formula {
                    Some(f) if v.is_empty() => format!("={f}"),
                    Some(f) => format!("{v} (={f})"),
                    None => v,
                };
                if value[6..8] == [0xFF, 0xFF] {
                    match value[0] {
                        0 => {
                            pending = Some((row as u32, col as usize, formula.clone().unwrap_or_default()));
                            continue;
                        }
                        1 => put(&mut rows, row, col, with(String::from(if value[2] != 0 { "TRUE" } else { "FALSE" }))),
                        2 => put(&mut rows, row, col, with(String::from(error_text(value[2])))),
                        _ => put(&mut rows, row, col, with(String::new())),
                    }
                } else {
                    let n = f64::from_le_bytes(value.try_into().unwrap_or([0; 8]));
                    put(&mut rows, row, col, with(shown(n, xf)));
                }
            }
            0x0207 => {
                if let Some((row, col, formula)) = pending.take() {
                    let s = if book.biff8 { unicode_string(d, 0).map(|s| s.0) } else { byte_string(d, 0) };
                    let s = s.unwrap_or_default();
                    let v = match (s.is_empty(), formula.is_empty()) {
                        (_, true) => s,
                        (true, false) => format!("={formula}"),
                        (false, false) => format!("{s} (={formula})"),
                    };
                    put(&mut rows, row as u16, col as u16, v);
                }
            }
            _ => {}
        }
    }
    rows.into_iter().map(|(n, cells)| (n, cells.into_iter().collect())).collect()
}

/// A function's name and how many arguments it takes when that's fixed (255: as many as its call says), by its
/// number in Excel's table of functions.
fn function(i: u16) -> Option<(&'static str, u8)> {
    Some(match i {
        0 => ("COUNT", 255),
        1 => ("IF", 255),
        2 => ("ISNA", 1),
        3 => ("ISERROR", 1),
        4 => ("SUM", 255),
        5 => ("AVERAGE", 255),
        6 => ("MIN", 255),
        7 => ("MAX", 255),
        8 => ("ROW", 255),
        9 => ("COLUMN", 255),
        10 => ("NA", 0),
        12 => ("STDEV", 255),
        15 => ("SIN", 1),
        16 => ("COS", 1),
        17 => ("TAN", 1),
        19 => ("PI", 0),
        20 => ("SQRT", 1),
        21 => ("EXP", 1),
        22 => ("LN", 1),
        23 => ("LOG10", 1),
        24 => ("ABS", 1),
        25 => ("INT", 1),
        26 => ("SIGN", 1),
        27 => ("ROUND", 2),
        28 => ("LOOKUP", 255),
        29 => ("INDEX", 255),
        30 => ("REPT", 2),
        31 => ("MID", 3),
        32 => ("LEN", 1),
        33 => ("VALUE", 1),
        34 => ("TRUE", 0),
        35 => ("FALSE", 0),
        36 => ("AND", 255),
        37 => ("OR", 255),
        38 => ("NOT", 1),
        39 => ("MOD", 2),
        46 => ("VAR", 255),
        48 => ("TEXT", 2),
        63 => ("RAND", 0),
        64 => ("MATCH", 255),
        65 => ("DATE", 3),
        66 => ("TIME", 3),
        67 => ("DAY", 1),
        68 => ("MONTH", 1),
        69 => ("YEAR", 1),
        70 => ("WEEKDAY", 255),
        71 => ("HOUR", 1),
        72 => ("MINUTE", 1),
        73 => ("SECOND", 1),
        74 => ("NOW", 0),
        76 => ("ROWS", 1),
        77 => ("COLUMNS", 1),
        78 => ("OFFSET", 255),
        82 => ("SEARCH", 255),
        100 => ("CHOOSE", 255),
        101 => ("HLOOKUP", 255),
        102 => ("VLOOKUP", 255),
        109 => ("LOG", 255),
        111 => ("CHAR", 1),
        112 => ("LOWER", 1),
        113 => ("UPPER", 1),
        114 => ("PROPER", 1),
        115 => ("LEFT", 255),
        116 => ("RIGHT", 255),
        117 => ("EXACT", 2),
        118 => ("TRIM", 1),
        119 => ("REPLACE", 4),
        120 => ("SUBSTITUTE", 255),
        121 => ("CODE", 1),
        124 => ("FIND", 255),
        126 => ("ISERR", 1),
        127 => ("ISTEXT", 1),
        128 => ("ISNUMBER", 1),
        129 => ("ISBLANK", 1),
        148 => ("INDIRECT", 255),
        169 => ("COUNTA", 255),
        183 => ("PRODUCT", 255),
        197 => ("TRUNC", 255),
        212 => ("ROUNDUP", 2),
        213 => ("ROUNDDOWN", 2),
        216 => ("RANK", 255),
        221 => ("TODAY", 0),
        227 => ("MEDIAN", 255),
        228 => ("SUMPRODUCT", 255),
        336 => ("CONCATENATE", 255),
        337 => ("POWER", 2),
        344 => ("SUBTOTAL", 255),
        345 => ("SUMIF", 255),
        346 => ("COUNTIF", 2),
        _ => return None,
    })
}

/// A cell's address as a formula writes it: its column's letters and row's number, "$" before what's absolute.
fn address(row: u32, col: u32, row_rel: bool, col_rel: bool) -> String {
    format!("{}{}{}{}", if col_rel { "" } else { "$" }, column_name(col as usize), if row_rel { "" } else { "$" }, row + 1)
}

/// A sheet's name as a formula writes it (quoted if it has to be).
fn sheet_ref(name: &str) -> String {
    if name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.starts_with(|c: char| c.is_ascii_digit()) {
        String::from(name)
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// A formula's tokens written out (None if it has one this doesn't write), for the cell at (`row`, `col`).
#[allow(clippy::too_many_arguments)]
fn decode(t: &[u8], book: &Book, sheet: usize, row: u16, col: u16, shared: &[(u16, u16, u8, u8, Vec<u8>)], stream: &[u8], next: usize) -> Option<String> {
    // A shared formula's cell: its tokens are the shared formula's, in the SHRFMLA record (after the first cell's
    // FORMULA record, or one already read).
    if t.first() == Some(&0x01) {
        let (r0, c0) = (u16_at(t, 1)?, u16_at(t, 3)?);
        let found = shared.iter().find(|(a, b, c, d, _)| *a <= row && row <= *b && *c as u16 <= col && col <= *d as u16).map(|s| s.4.clone());
        let tokens = match found {
            Some(tokens) => tokens,
            None => {
                let r = record(stream, next)?;
                if r.kind != 0x04BC || u16_at(r.data, 0)? != r0 || r.data.get(4).copied()? as u16 != c0 {
                    return None;
                }
                let len = u16_at(r.data, 8)? as usize;
                r.data.get(10..10 + len)?.to_vec()
            }
        };
        return expression(&tokens, book, sheet, row, col);
    }
    expression(t, book, sheet, row, col)
}

fn expression(t: &[u8], book: &Book, sheet: usize, row: u16, col: u16) -> Option<String> {
    let mut stack: Vec<String> = Vec::new();
    let mut i = 0;
    let reference = |d: &[u8], at: usize, relative_to: Option<(u16, u16)>| -> Option<String> {
        let r = u16_at(d, at)? as i64;
        let c = u16_at(d, at + 2)?;
        let (row_rel, col_rel) = (c & 0x8000 != 0, c & 0x4000 != 0);
        let mut r_abs = r;
        let mut c_abs = (c & 0x3FFF) as i64;
        if let Some((br, bc)) = relative_to {
            if row_rel {
                r_abs = br as i64 + r as i16 as i64;
            }
            if col_rel {
                c_abs = bc as i64 + (c & 0xFF) as u8 as i8 as i64;
            }
        }
        if !(0..1_048_576).contains(&r_abs) || !(0..16_384).contains(&c_abs) {
            return None;
        }
        Some(address(r_abs as u32, c_abs as u32, row_rel, col_rel))
    };
    let area = |d: &[u8], at: usize, relative_to: Option<(u16, u16)>| -> Option<String> {
        let mut first = [0u8; 4];
        let mut last = [0u8; 4];
        first[..2].copy_from_slice(d.get(at..at + 2)?);
        first[2..].copy_from_slice(d.get(at + 4..at + 6)?);
        last[..2].copy_from_slice(d.get(at + 2..at + 4)?);
        last[2..].copy_from_slice(d.get(at + 6..at + 8)?);
        Some(format!("{}:{}", reference(&first, 0, relative_to)?, reference(&last, 0, relative_to)?))
    };
    let other_sheet = |ixti: u16| -> Option<String> {
        let s = (*book.xti.get(ixti as usize)?)?;
        if s == sheet {
            return Some(String::new());
        }
        Some(format!("{}!", sheet_ref(&book.sheets.get(s)?.name)))
    };
    while i < t.len() {
        let ptg = t[i];
        i += 1;
        let base = if ptg >= 0x20 { (ptg & 0x1F) | 0x20 } else { ptg };
        match base {
            0x03..=0x11 => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                let op = ["+", "-", "*", "/", "^", "&", "<", "<=", "=", ">=", ">", "<>", " ", ",", ":"][(base - 3) as usize];
                stack.push(format!("{a}{op}{b}"));
            }
            0x12 => {
                let a = stack.pop()?;
                stack.push(format!("+{a}"));
            }
            0x13 => {
                let a = stack.pop()?;
                stack.push(format!("-{a}"));
            }
            0x14 => {
                let a = stack.pop()?;
                stack.push(format!("{a}%"));
            }
            0x15 => {
                let a = stack.pop()?;
                stack.push(format!("({a})"));
            }
            0x16 => stack.push(String::new()),
            0x17 => {
                let len = *t.get(i)? as usize;
                let wide = t.get(i + 1)? & 1 != 0;
                let s = chars_at(t, i + 1, len)?;
                i += 2 + if wide { 2 * len } else { len };
                stack.push(format!("\"{}\"", s.replace('"', "\"\"")));
            }
            0x19 => {
                let options = *t.get(i)?;
                let data = u16_at(t, i + 1)? as usize;
                i += 3;
                if options & 0x04 != 0 {
                    i += 2 * (data + 1);
                }
                if options & 0x10 != 0 {
                    let a = stack.pop()?;
                    stack.push(format!("SUM({a})"));
                }
            }
            0x1C => {
                stack.push(String::from(error_text(*t.get(i)?)));
                i += 1;
            }
            0x1D => {
                stack.push(String::from(if *t.get(i)? != 0 { "TRUE" } else { "FALSE" }));
                i += 1;
            }
            0x1E => {
                stack.push(u16_at(t, i)?.to_string());
                i += 2;
            }
            0x1F => {
                stack.push(number_text(f64_at(t, i)?));
                i += 8;
            }
            0x21 | 0x22 => {
                let (argc, index) = if base == 0x21 {
                    let index = u16_at(t, i)?;
                    i += 2;
                    let (_, n) = function(index)?;
                    if n == 255 {
                        return None;
                    }
                    (n as usize, index)
                } else {
                    let argc = *t.get(i)? as usize & 0x7F;
                    let index = u16_at(t, i + 1)? & 0x7FFF;
                    i += 3;
                    (argc, index)
                };
                let (name, _) = function(index)?;
                if stack.len() < argc {
                    return None;
                }
                let args = stack.split_off(stack.len() - argc);
                stack.push(format!("{name}({})", args.join(",")));
            }
            0x23 => {
                let index = u16_at(t, i)? as usize;
                i += 4;
                stack.push(book.names.get(index.checked_sub(1)?)?.clone());
            }
            0x24 | 0x2C => {
                stack.push(reference(t, i, (base == 0x2C).then_some((row, col)))?);
                i += 4;
            }
            0x25 | 0x2D => {
                stack.push(area(t, i, (base == 0x2D).then_some((row, col)))?);
                i += 8;
            }
            0x26..=0x28 => i += 6,
            0x29 => i += 2,
            0x2A => {
                stack.push(String::from("#REF!"));
                i += 4;
            }
            0x2B => {
                stack.push(String::from("#REF!"));
                i += 8;
            }
            0x3A => {
                let s = other_sheet(u16_at(t, i)?)?;
                stack.push(format!("{s}{}", reference(t, i + 2, None)?));
                i += 6;
            }
            0x3B => {
                let s = other_sheet(u16_at(t, i)?)?;
                stack.push(format!("{s}{}", area(t, i + 2, None)?));
                i += 10;
            }
            0x3C => {
                stack.push(String::from("#REF!"));
                i += 6;
            }
            0x3D => {
                stack.push(String::from("#REF!"));
                i += 10;
            }
            _ => return None,
        }
    }
    if stack.len() == 1 { stack.pop() } else { None }
}
