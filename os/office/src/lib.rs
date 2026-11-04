//! The words in Word, Excel and PowerPoint files (Office Open XML: .docx, .xlsx and .pptx, as Microsoft Office,
//! LibreOffice, Google Docs and others save them) and in OpenDocument's (.odt, .ods, .odp), as text for Claude to
//! read: a document's paragraphs, with Markdown's headings, lists and tables (`word`); each sheet's cells as CSV, with
//! each row's number and each column's letter, dates as dates and percentages as percentages (`excel`); each slide's
//! words, its title first, and the speaker's notes (`slides`).
//!
//! Each file is a ZIP of XML files ("parts"). An Office Open XML package says which part is the document, and each
//! part which others go with it (its styles, a workbook's sheets and their shared strings, a presentation's slides
//! and their notes), in "relationships" (a part's `_rels/<name>.rels`). OpenDocument keeps the words in content.xml.
//!
//! As ECMA-376 and OASIS's OpenDocument standard describe them. What isn't words (pictures, charts, fonts, formatting)
//! is passed over, except pictures' descriptions on slides. Damaged files give an error, never a crash, and what
//! comes out is capped.

#![no_std]

extern crate alloc;

mod excel;
mod odf;
mod slides;
mod word;
mod xml;

#[cfg(test)]
mod tests;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use zip::Archive;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Not a Word, Excel, PowerPoint or OpenDocument file (or a password protects it: then it isn't a ZIP).
    NotOffice,
    /// The ZIP couldn't be read.
    Zip(zip::Error),
    /// A part that has to be there isn't, or isn't XML.
    Damaged,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Word,
    Excel,
    PowerPoint,
    /// OpenDocument's text, spreadsheet and presentation.
    Text,
    Spreadsheet,
    Presentation,
}

impl Kind {
    /// What it is, for people: "a Word document".
    pub fn name(self) -> &'static str {
        match self {
            Kind::Word => "a Word document",
            Kind::Excel => "an Excel workbook",
            Kind::PowerPoint => "a PowerPoint presentation",
            Kind::Text => "an OpenDocument text",
            Kind::Spreadsheet => "an OpenDocument spreadsheet",
            Kind::Presentation => "an OpenDocument presentation",
        }
    }
}

/// The most of a part read (uncompressed), and the most text made.
const PART_BYTES: usize = 64 << 20;
const MOST_TEXT: usize = 8 << 20;

/// What `data` is, and its words.
pub fn read(data: &[u8]) -> Result<(Kind, String), Error> {
    let archive = Archive::open(data).map_err(|_| Error::NotOffice)?;
    // OpenDocument's first file, "mimetype", says what it is.
    if let Some(e) = archive.find("mimetype") {
        let kind = match archive.read(e, 100).map_err(Error::Zip)?.as_slice() {
            b"application/vnd.oasis.opendocument.text" => Kind::Text,
            b"application/vnd.oasis.opendocument.spreadsheet" => Kind::Spreadsheet,
            b"application/vnd.oasis.opendocument.presentation" => Kind::Presentation,
            _ => return Err(Error::NotOffice),
        };
        return Ok((kind, odf::read(&archive)?));
    }
    // Office Open XML: the package's relationships name its main part, whose folder says what it is.
    let main = relationships(&archive, "")?
        .into_iter()
        .find(|r| r.kind.ends_with("/officeDocument"))
        .ok_or(Error::NotOffice)?
        .target;
    let (kind, text) = match main.split('/').next() {
        Some("word") => (Kind::Word, word::read(&archive, &main)?),
        Some("xl") => (Kind::Excel, excel::read(&archive, &main)?),
        Some("ppt") => (Kind::PowerPoint, slides::read(&archive, &main)?),
        _ => return Err(Error::NotOffice),
    };
    Ok((kind, text))
}

/// A part, as text (None if the package hasn't got it).
fn part(archive: &Archive, name: &str) -> Result<Option<String>, Error> {
    let Some(e) = archive.find(name) else { return Ok(None) };
    let bytes = archive.read(e, PART_BYTES).map_err(Error::Zip)?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    Ok(Some(String::from_utf8_lossy(bytes).into_owned()))
}

/// A relationship: its id, its type (a URI, whose end says what it is: ".../styles"), and its target, a part's
/// name.
struct Relationship {
    id: String,
    kind: String,
    target: String,
}

/// Part `name`'s relationships ("" for the package's own), from its .rels part, their targets resolved from its
/// folder. Ones to outside the package (a hyperlink's) are left out.
fn relationships(archive: &Archive, name: &str) -> Result<Vec<Relationship>, Error> {
    let (folder, file) = name.rsplit_once('/').unwrap_or(("", name));
    let rels = match (folder, file) {
        ("", "") => String::from("_rels/.rels"),
        ("", f) => format!("_rels/{f}.rels"),
        (d, f) => format!("{d}/_rels/{f}.rels"),
    };
    let Some(text) = part(archive, &rels)? else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for token in xml::Reader::new(&text) {
        let xml::Token::Start(tag) = token else { continue };
        if tag.local() != "Relationship" || tag.attr("TargetMode").as_deref() == Some("External") {
            continue;
        }
        let (Some(id), Some(kind), Some(target)) = (tag.attr("Id"), tag.attr("Type"), tag.attr("Target")) else { continue };
        out.push(Relationship { id, kind, target: resolve(folder, &target) });
    }
    Ok(out)
}

/// A target ("../notesSlides/notesSlide1.xml", "/xl/worksheets/sheet1.xml") as a part's name, from `folder`.
fn resolve(folder: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') { Vec::new() } else { folder.split('/').filter(|p| !p.is_empty()).collect() };
    for p in target.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// The text being made: lines, at most `MOST_TEXT` of them.
struct Out {
    text: String,
    full: bool,
}

impl Out {
    fn new() -> Out {
        Out { text: String::new(), full: false }
    }

    /// A line (without its line break).
    fn line(&mut self, s: &str) {
        if self.full {
            return;
        }
        if self.text.len() + s.len() > MOST_TEXT {
            self.text.push_str("[...and more, not read: the file's text is longer than Chat Computer reads.]\n");
            self.full = true;
            return;
        }
        self.text.push_str(s);
        self.text.push('\n');
    }

    /// A blank line, unless there's one already (or nothing yet).
    fn gap(&mut self) {
        if !self.text.is_empty() && !self.text.ends_with("\n\n") {
            self.line("");
        }
    }

    fn done(mut self) -> String {
        while self.text.ends_with('\n') {
            self.text.pop();
        }
        self.text
    }
}

/// A table in Markdown: a header row (the first), its rule, then the rest, every row as wide as the widest.
fn markdown_table(rows: &[Vec<String>], out: &mut Out) {
    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if width == 0 {
        return;
    }
    out.gap();
    for (i, row) in rows.iter().enumerate() {
        let mut line = String::from("|");
        for c in 0..width {
            let cell = row.get(c).map(|s| s.as_str()).unwrap_or("");
            line.push(' ');
            line.push_str(&cell.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', "\\|"));
            line.push_str(" |");
        }
        out.line(&line);
        if i == 0 {
            out.line(&format!("|{}", " --- |".repeat(width)));
        }
    }
    out.gap();
}

/// A CSV field: as it is, or in quotes (its own quotes doubled) if it has a comma, a quote or a line break.
fn csv(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        String::from(field)
    }
}

/// A spreadsheet column's letters: 0 is A, 25 Z, 26 AA.
fn column_name(mut c: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// The most columns a sheet or a text's table is shown with.
pub(crate) const MOST_COLUMNS: usize = 200;

/// A sheet's rows as CSV: a first line naming the columns (A, B, C...), then each row with something in it, its
/// number first ("row,A,B" / "4,Total,1612.5").
fn sheet_csv(rows: &[(u32, Vec<(usize, String)>)], out: &mut Out) {
    let width = rows.iter().flat_map(|(_, cells)| cells.iter().map(|(c, _)| c + 1)).max().unwrap_or(0).min(MOST_COLUMNS);
    if width == 0 {
        out.line("(empty)");
        return;
    }
    let mut header = String::from("row");
    for c in 0..width {
        header.push(',');
        header.push_str(&column_name(c));
    }
    out.line(&header);
    for (n, cells) in rows {
        if cells.iter().all(|(_, v)| v.is_empty()) {
            continue;
        }
        let mut fields = alloc::vec![String::new(); width];
        for (c, v) in cells {
            if *c < width {
                fields[*c] = csv(v);
            }
        }
        out.line(&format!("{n},{}", fields.join(",")));
    }
}
