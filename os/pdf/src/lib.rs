//! Chat Computer's own reading of PDFs: a PDF's words, page by page, for its model (read_file). The file's objects are
//! found through its cross-reference sections (or, in a damaged file, by looking through it for them: file.rs), its
//! pages along its page tree, and each page's text read from its content streams by its fonts (text.rs, font.rs); a
//! filled-in form's fields and what's in them follow. Nothing in a file is trusted: everything read is held to a most
//! (sizes, depths, counts), and a part that can't be read is passed over.
//!
//! An encrypted file is read if it opens without a password, as most do (only an owner's password, limiting printing
//! or copying: crypt.rs). What isn't read: files that need a password to open. A page's pictures (a scanned page is a
//! picture of its words, so it has none here) are made one picture of the page, for a model that sees (image.rs).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod ccitt;
mod crypt;
mod file;
mod filter;
mod font;
mod glyphs;
mod image;
mod lex;
mod text;
#[cfg(test)]
mod tests;

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use file::File;
use lex::{Dict, Obj};

pub use image::{PagePicture, page_picture};

/// What a page with no words says, in a document's words.
pub const NO_WORDS: &str = "(No words on this page.)";

/// The most pages read.
const MOST_PAGES: usize = 20_000;
/// The most text kept, in bytes.
pub const MOST_TEXT: usize = 8 << 20;
/// How deep the page tree, or a form's fields, may go.
const MOST_DEPTH: u32 = 64;
/// The most form fields listed.
const MOST_FIELDS: usize = 2_000;
/// The most of a page's contents read (its streams, decoded, together).
const MOST_CONTENTS: usize = 64 << 20;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Error {
    /// It isn't a PDF ("%PDF-" isn't near its start).
    NotPdf,
    /// No pages could be found in it.
    Damaged,
    /// It needs a password to open.
    Encrypted,
}

#[derive(Debug)]
pub struct Document {
    pub pages: usize,
    /// Its title, as its information says.
    pub title: Option<String>,
    /// "## Page 1\n...\n\n## Page 2\n...", then "## Form" with its fields; empty if no page has words and it has no
    /// filled-in form.
    pub text: String,
    /// Whether the text was cut at MOST_TEXT.
    pub cut: bool,
}

/// A PDF's words.
pub fn read(data: &[u8]) -> Result<Document, Error> {
    if file::find(&data[..data.len().min(1024)], b"%PDF-").is_none() {
        return Err(Error::NotPdf);
    }
    let f = File::open(data);
    if f.locked() {
        return Err(Error::Encrypted);
    }
    let catalog = f.get_in(&f.trailer, "Root");
    let catalog = catalog.dict().cloned().unwrap_or_default();
    let pages = pages(&f, &catalog);
    if pages.is_empty() {
        return Err(Error::Damaged);
    }
    let mut reader = text::Reader::new(&f);
    let mut out = String::new();
    let (mut any, mut cut) = (false, false);
    for (n, (page, resources)) in pages.iter().enumerate() {
        let words = reader.page(&contents(&f, page), resources);
        any |= !words.is_empty();
        let part = if words.is_empty() { format!("## Page {}\n{NO_WORDS}", n + 1) } else { format!("## Page {}\n{words}", n + 1) };
        if !add(&mut out, &part) {
            cut = true;
            break;
        }
    }
    let form = form(&f, &catalog);
    if !form.is_empty() {
        any = true;
        if !cut && !add(&mut out, &format!("## Form\n{form}")) {
            cut = true;
        }
    }
    if !any {
        out.clear();
    }
    let info = f.get_in(&f.trailer, "Info");
    let title = info.dict().and_then(|i| f.get_in(i, "Title").string().map(text_string)).map(|t| one_line(&t)).filter(|t| !t.is_empty());
    Ok(Document { pages: pages.len(), title, text: out, cut })
}

/// `part` added to `out` after a blank line, if it fits under MOST_TEXT (else as much as fits, at a character's end):
/// whether all of it did.
fn add(out: &mut String, part: &str) -> bool {
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    let room = MOST_TEXT.saturating_sub(out.len());
    if part.len() <= room {
        out.push_str(part);
        return true;
    }
    let mut end = room;
    while !part.is_char_boundary(end) {
        end -= 1;
    }
    out.push_str(&part[..end]);
    false
}

/// The pages, in order, each with its resources (its own, or the nearest it inherits from the page tree). Where the
/// tree can't be followed, every object that says it's a page, in the order of their numbers.
pub(crate) fn pages(f: &File, catalog: &Dict) -> Vec<(Dict, Dict)> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(root) = catalog.get("Pages") {
        walk(f, root, &Dict::default(), 0, &mut seen, &mut out);
    }
    if out.is_empty() {
        for n in f.numbers() {
            if out.len() >= MOST_PAGES {
                break;
            }
            if let Some(Obj::Dict(d)) = f.get(n)
                && f.get_in(&d, "Type").name() == Some(b"Page")
            {
                // (Its resources its own, or the nearest of its parents'.)
                let mut node = d.clone();
                let mut resources = Dict::default();
                for _ in 0..MOST_DEPTH {
                    if let Obj::Dict(r) = f.get_in(&node, "Resources") {
                        resources = r;
                        break;
                    }
                    let Obj::Dict(parent) = f.get_in(&node, "Parent") else { break };
                    node = parent;
                }
                out.push((d, resources));
            }
        }
    }
    out
}

fn walk(f: &File, node: &Obj, inherited: &Dict, depth: u32, seen: &mut BTreeSet<u32>, out: &mut Vec<(Dict, Dict)>) {
    if depth > MOST_DEPTH || out.len() >= MOST_PAGES {
        return;
    }
    if let Obj::Ref(n, _) = node
        && !seen.insert(*n)
    {
        return;
    }
    let Obj::Dict(d) = f.resolve(node) else { return };
    let resources = match f.get_in(&d, "Resources") {
        Obj::Dict(r) => r,
        _ => inherited.clone(),
    };
    let kind = f.get_in(&d, "Type");
    match f.get_in(&d, "Kids") {
        Obj::Array(kids) if kind.name() != Some(b"Page") => {
            for kid in &kids {
                walk(f, kid, &resources, depth + 1, seen, out);
            }
        }
        _ if kind.name() == Some(b"Page") || d.get("Contents").is_some() => out.push((d, resources)),
        _ => {}
    }
}

/// A page's content streams, decoded, one after another.
pub(crate) fn contents(f: &File, page: &Dict) -> Vec<u8> {
    match f.get_in(page, "Contents") {
        Obj::Array(parts) => {
            let mut all = Vec::new();
            for part in &parts {
                if all.len() >= MOST_CONTENTS {
                    break;
                }
                if let Some(data) = f.stream(part) {
                    all.extend_from_slice(&data[..data.len().min(MOST_CONTENTS - all.len())]);
                    all.push(b'\n');
                }
            }
            all
        }
        s @ Obj::Stream(..) => f.stream(&s).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// A form's fields that are filled in (12.7): each one's full name (its names down the field tree, with dots) and what's
/// in it, a line each.
fn form(f: &File, catalog: &Dict) -> String {
    let mut out = String::new();
    let acro = f.get_in(catalog, "AcroForm");
    let Some(fields) = acro.dict().map(|a| f.get_in(a, "Fields")) else { return out };
    let Some(fields) = fields.array() else { return out };
    let mut seen = BTreeSet::new();
    let mut count = 0;
    for field in fields {
        field_lines(f, field, "", 0, &mut seen, &mut count, &mut out);
    }
    out
}

fn field_lines(f: &File, node: &Obj, parent: &str, depth: u32, seen: &mut BTreeSet<u32>, count: &mut usize, out: &mut String) {
    if depth > MOST_DEPTH || *count >= MOST_FIELDS {
        return;
    }
    if let Obj::Ref(n, _) = node
        && !seen.insert(*n)
    {
        return;
    }
    let Obj::Dict(d) = f.resolve(node) else { return };
    let own = f.get_in(&d, "T").string().map(text_string).map(|t| one_line(&t)).unwrap_or_default();
    let name = match (parent.is_empty(), own.is_empty()) {
        (_, true) => String::from(parent),
        (true, false) => own,
        (false, false) => format!("{parent}.{own}"),
    };
    if let Obj::Array(kids) = f.get_in(&d, "Kids") {
        for kid in &kids {
            field_lines(f, kid, &name, depth + 1, seen, count, out);
        }
    }
    let value = match f.get_in(&d, "V") {
        Obj::Str(s) => one_line(&text_string(&s)),
        // (A check box's or a radio button's: its state's name, "Off" for none.)
        Obj::Name(n) if n != b"Off" => String::from_utf8_lossy(&n).into_owned(),
        Obj::Array(items) => {
            let chosen: Vec<String> = items.iter().filter_map(|i| f.resolve(i).string().map(text_string)).map(|t| one_line(&t)).collect();
            chosen.join("; ")
        }
        _ => String::new(),
    };
    if value.is_empty() {
        return;
    }
    // (Shown by the name it's given for people to see, if it has one.)
    let label = f.get_in(&d, "TU").string().map(text_string).map(|t| one_line(&t)).filter(|t| !t.is_empty()).unwrap_or(name);
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!("{label}: {value}"));
    *count += 1;
}

/// A text string (7.9.2.2): UTF-16BE after its byte order mark, UTF-8 after its own (PDF 2.0), else PDFDocEncoding.
pub(crate) fn text_string(s: &[u8]) -> String {
    if let Some(rest) = s.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = s.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    s.iter().map(|&b| pdf_doc(b)).collect()
}

/// PDFDocEncoding (Annex D.2): Latin-1 but for a few accents below 32, typographic marks from 0x80, and the euro.
fn pdf_doc(b: u8) -> char {
    const LOW: [u16; 8] = [0x02D8, 0x02C7, 0x02C6, 0x02D9, 0x02DD, 0x02DB, 0x02DA, 0x02DC];
    const HIGH: [u16; 32] = [
        0x2022, 0x2020, 0x2021, 0x2026, 0x2014, 0x2013, 0x0192, 0x2044, 0x2039, 0x203A, 0x2212, 0x2030, 0x201E, 0x201C,
        0x201D, 0x2018, 0x2019, 0x201A, 0x2122, 0xFB01, 0xFB02, 0x0141, 0x0152, 0x0160, 0x0178, 0x017D, 0x0131, 0x0142,
        0x0153, 0x0161, 0x017E, 0xFFFD,
    ];
    let c = match b {
        0x18..=0x1F => LOW[(b - 0x18) as usize] as u32,
        0x80..=0x9F => HIGH[(b - 0x80) as usize] as u32,
        0xA0 => 0x20AC,
        _ => b as u32,
    };
    char::from_u32(c).unwrap_or('\u{FFFD}')
}

/// Text on one line: line breaks and tabs as spaces, other control characters left out, spaces run together.
fn one_line(s: &str) -> String {
    let cleaned: String = s.chars().filter(|&c| c >= ' ' || matches!(c, '\n' | '\r' | '\t')).map(|c| if c < ' ' { ' ' } else { c }).collect();
    let words: Vec<&str> = cleaned.split(' ').filter(|w| !w.is_empty()).collect();
    words.join(" ")
}
