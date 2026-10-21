//! An Excel workbook's cells (SpreadsheetML): each sheet in the workbook's order, its rows as CSV with each row's
//! number and each column's letter (`sheet_csv`). A cell's value as the file keeps it: words from the shared strings
//! or the cell itself, TRUE and FALSE, errors, and numbers, shown as dates, times or percentages where the cell's
//! number format makes them so (styles.xml), in either of Excel's date systems (from 1900, or 1904's). A formula comes
//! after its value: "1612.5 (=SUM(B2:B3))"; one never worked out by a spreadsheet program shows as the formula alone.

use crate::xml::{self, Reader, Token};
use crate::{Error, Out, part, relationships, sheet_csv};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use zip::Archive;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Format {
    General,
    Date,
    Time,
    DateTime,
    /// With this many decimals.
    Percent(u8),
}

pub fn read(archive: &Archive, main: &str) -> Result<String, Error> {
    let rels = relationships(archive, main)?;
    let find = |end: &str| rels.iter().find(|r| r.kind.ends_with(end)).map(|r| r.target.clone());
    let shared = match find("/sharedStrings") {
        Some(p) => shared_strings(&part(archive, &p)?.unwrap_or_default()),
        None => Vec::new(),
    };
    let formats = match find("/styles") {
        Some(p) => formats(&part(archive, &p)?.unwrap_or_default()),
        None => Vec::new(),
    };
    let book = part(archive, main)?.ok_or(Error::Damaged)?;
    let mut sheets: Vec<(String, Option<String>, bool)> = Vec::new();
    let mut date1904 = false;
    for token in Reader::new(&book) {
        let Token::Start(t) = token else { continue };
        match t.local() {
            "sheet" => {
                let hidden = matches!(t.attr("state").as_deref(), Some("hidden" | "veryHidden"));
                sheets.push((t.attr("name").unwrap_or_default(), t.attr_prefixed("id"), hidden));
            }
            "workbookPr" => date1904 = matches!(t.attr("date1904").as_deref(), Some("1" | "true")),
            _ => {}
        }
    }
    let mut out = Out::new();
    for (name, id, hidden) in sheets {
        out.gap();
        out.line(&format!("## {name}{}", if hidden { " (hidden)" } else { "" }));
        let target = id.and_then(|id| rels.iter().find(|r| r.id == id)).map(|r| r.target.clone());
        match target {
            Some(t) => match part(archive, &t)? {
                Some(xml) if t.contains("worksheets/") || xml.contains("<sheetData") => {
                    sheet_csv(&cells(&xml, &shared, &formats, date1904), &mut out)
                }
                Some(_) => out.line("(a chart, not cells)"),
                None => out.line("(not in the file)"),
            },
            None => out.line("(not in the file)"),
        }
    }
    Ok(out.done())
}

/// The shared strings, in order: each one's words (a rich one's runs together; the readings of Japanese words, kept
/// apart in rPh, left out).
fn shared_strings(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let (mut in_t, mut phonetic) = (false, false);
    for token in Reader::new(xml) {
        match token {
            Token::Start(t) => match t.local() {
                "si" if t.empty => out.push(String::new()),
                "si" => current.clear(),
                "rPh" => phonetic = !t.empty,
                "t" => in_t = !t.empty && !phonetic,
                _ => {}
            },
            Token::End(name) => match xml::local(name) {
                "si" => out.push(core::mem::take(&mut current)),
                "rPh" => phonetic = false,
                "t" => in_t = false,
                _ => {}
            },
            Token::Text(s) if in_t => current.push_str(&xml::text(s)),
            Token::Raw(s) if in_t => current.push_str(s),
            _ => {}
        }
    }
    out
}

/// Each cell style's number format (a cell's "s" is its index here).
fn formats(xml: &str) -> Vec<Format> {
    let mut codes: BTreeMap<u32, String> = BTreeMap::new();
    let mut ids: Vec<u32> = Vec::new();
    let mut in_cells = false;
    for token in Reader::new(xml) {
        match token {
            Token::Start(t) => match t.local() {
                "numFmt" => {
                    if let (Some(id), Some(code)) = (t.attr("numFmtId").and_then(|v| v.parse().ok()), t.attr("formatCode")) {
                        codes.insert(id, code);
                    }
                }
                "cellXfs" => in_cells = !t.empty,
                "xf" if in_cells => ids.push(t.attr("numFmtId").and_then(|v| v.parse().ok()).unwrap_or(0)),
                _ => {}
            },
            Token::End(name) if xml::local(name) == "cellXfs" => in_cells = false,
            _ => {}
        }
    }
    ids.into_iter().map(|id| format_of(id, codes.get(&id).map(|s| s.as_str()))).collect()
}

/// What a number format shows: Excel's built-in ones by number (ECMA-376, 18.8.30), others by their code.
fn format_of(id: u32, code: Option<&str>) -> Format {
    match id {
        9 => return Format::Percent(0),
        10 => return Format::Percent(2),
        14..=17 | 27..=31 | 34..=36 | 50..=58 => return Format::Date,
        18..=21 | 32 | 33 | 45..=47 => return Format::Time,
        22 => return Format::DateTime,
        _ => {}
    }
    let Some(code) = code else { return Format::General };
    // The format for positive numbers (before the first ;), without quoted words, escaped characters, padding,
    // colours and locales ([Red], [$-409]); [h] and the like are elapsed time.
    let mut plain = String::new();
    let mut elapsed = false;
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            ';' => break,
            '"' => {
                for d in chars.by_ref() {
                    if d == '"' {
                        break;
                    }
                }
            }
            '\\' | '_' | '*' => {
                chars.next();
            }
            '[' => {
                let inside: String = chars.by_ref().take_while(|&d| d != ']').collect();
                elapsed |= matches!(inside.to_ascii_lowercase().as_str(), "h" | "hh" | "m" | "mm" | "s" | "ss");
            }
            c => plain.push(c.to_ascii_lowercase()),
        }
    }
    // (An m is minutes beside hours or seconds, else months.)
    let digits = plain.contains(['0', '#', '?']);
    let time = plain.contains(['h', 's']) || elapsed;
    let date = plain.contains(['y', 'd']) || (plain.contains('m') && !time && !digits);
    match (date, time) {
        (true, true) => Format::DateTime,
        (true, false) => Format::Date,
        (false, true) => Format::Time,
        _ if plain.contains('%') => {
            let decimals = plain.split('.').nth(1).map_or(0, |d| d.chars().take_while(|&c| c == '0').count());
            Format::Percent(decimals.min(10) as u8)
        }
        _ => Format::General,
    }
}

/// A cell being read.
#[derive(Default)]
struct Cell {
    column: usize,
    kind: String,
    style: usize,
    value: String,
    formula: String,
    words: String,
}

/// Which of a cell's parts text goes into.
#[derive(Clone, Copy, PartialEq)]
enum Collecting {
    Nothing,
    Value,
    Formula,
    Words,
}

/// A sheet's rows: each row's number, and its cells' columns (0 for A) and values.
pub(crate) fn cells(xml: &str, shared: &[String], formats: &[Format], date1904: bool) -> Vec<(u32, Vec<(usize, String)>)> {
    let mut rows: Vec<(u32, Vec<(usize, String)>)> = Vec::new();
    let (mut row, mut next_column) = (0u32, 0usize);
    let mut cell: Option<Cell> = None;
    let mut into = Collecting::Nothing;
    let (mut inline, mut phonetic) = (false, false);
    for token in Reader::new(xml) {
        match token {
            Token::Start(t) => match t.local() {
                "row" => {
                    row = t.attr("r").and_then(|r| r.parse().ok()).unwrap_or(row + 1);
                    next_column = 0;
                    rows.push((row, Vec::new()));
                }
                "c" => {
                    let column = t.attr("r").and_then(|r| column_of(&r)).unwrap_or(next_column);
                    next_column = column + 1;
                    let c = Cell {
                        column,
                        kind: t.attr("t").unwrap_or_default(),
                        style: t.attr("s").and_then(|s| s.parse().ok()).unwrap_or(0),
                        ..Cell::default()
                    };
                    if t.empty {
                        continue;
                    }
                    cell = Some(c);
                }
                "v" if !t.empty => into = Collecting::Value,
                "f" if !t.empty => into = Collecting::Formula,
                "is" => inline = !t.empty,
                "rPh" => phonetic = !t.empty,
                "t" if inline && !phonetic && !t.empty => into = Collecting::Words,
                _ => {}
            },
            Token::End(name) => match xml::local(name) {
                "v" | "f" | "t" => into = Collecting::Nothing,
                "is" => inline = false,
                "rPh" => phonetic = false,
                "c" => {
                    if let (Some(c), Some((_, cells))) = (cell.take(), rows.last_mut()) {
                        let shown = value(&c, shared, formats, date1904);
                        if !shown.is_empty() {
                            cells.push((c.column, shown));
                        }
                    }
                }
                _ => {}
            },
            Token::Text(s) | Token::Raw(s) => {
                let s = if matches!(token, Token::Text(_)) { xml::text(s) } else { String::from(s) };
                if let Some(c) = cell.as_mut() {
                    match into {
                        Collecting::Value => c.value.push_str(&s),
                        Collecting::Formula => c.formula.push_str(&s),
                        Collecting::Words => c.words.push_str(&s),
                        Collecting::Nothing => {}
                    }
                }
            }
        }
    }
    rows
}

/// A cell reference's column ("AB12": 27).
fn column_of(r: &str) -> Option<usize> {
    let letters: Vec<u8> = r.bytes().take_while(|b| b.is_ascii_alphabetic()).collect();
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let n = letters.iter().fold(0usize, |n, &b| n * 26 + (b.to_ascii_uppercase() - b'A') as usize + 1);
    Some(n - 1)
}

/// What a cell shows.
fn value(c: &Cell, shared: &[String], formats: &[Format], date1904: bool) -> String {
    let v = c.value.trim();
    let shown = match c.kind.as_str() {
        "s" => v.parse::<usize>().ok().and_then(|i| shared.get(i)).cloned().unwrap_or_default(),
        "inlineStr" => c.words.clone(),
        "b" => String::from(if v == "1" { "TRUE" } else { "FALSE" }),
        "e" | "str" | "d" => String::from(v),
        _ => number(v, formats.get(c.style).copied().unwrap_or(Format::General), date1904),
    };
    // (A true or false written as a formula, as LibreOffice writes them, is just that.)
    let formula = c.formula.trim();
    let formula = if matches!(formula, "TRUE()" | "FALSE()") { "" } else { formula };
    match (shown.is_empty(), formula.is_empty()) {
        (_, true) => shown,
        (true, false) => format!("={formula}"),
        (false, false) => format!("{shown} (={formula})"),
    }
}

/// A number as its format shows it: a date ("2026-10-01"), a time ("18:30"), both, a percentage, or as it is.
fn number(v: &str, format: Format, date1904: bool) -> String {
    let Ok(n) = v.parse::<f64>() else { return String::from(v) };
    match format {
        Format::Percent(d) => format!("{:.*}%", d as usize, n * 100.0),
        // (Dates from year 0 to 9999.)
        Format::Date | Format::Time | Format::DateTime if (0.0..2_958_466.0).contains(&n) => {
            let mut days = n as i64;
            let mut seconds = ((n - days as f64) * 86_400.0 + 0.5) as i64;
            if seconds >= 86_400 {
                days += 1;
                seconds -= 86_400;
            }
            let time = if seconds % 60 == 0 {
                format!("{:02}:{:02}", seconds / 3600, seconds / 60 % 60)
            } else {
                format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
            };
            match format {
                Format::Time if days > 0 => {
                    // A duration of a day or more: in hours.
                    let hours = days * 24 + seconds / 3600;
                    format!("{hours}{}", &time[2..])
                }
                Format::Time => time,
                Format::Date => date(days, date1904),
                _ => format!("{} {time}", date(days, date1904)),
            }
        }
        _ => String::from(v),
    }
}

/// Day `n` of a workbook's date system as "YYYY-MM-DD": from 1900 (day 1 is 1900-01-01, and day 60 the 29 February
/// 1900 that wasn't, kept from Lotus 1-2-3), or 1904's (day 0 is 1904-01-01).
pub(crate) fn date(n: i64, date1904: bool) -> String {
    let unix_day = if date1904 {
        n - 24_107
    } else if n == 60 {
        return String::from("1900-02-29");
    } else if n < 60 {
        n - 25_568
    } else {
        n - 25_569
    };
    let (y, m, d) = civil(unix_day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 as a year, month and day (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe as i64 + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}
