//! A PowerPoint presentation from before 2007 (.ppt: PowerPoint 97 to 2003's binary format, [MS-PPT]): each slide's
//! words as the .pptx reader gives them, "## Slide 2: <its title>", its body's paragraphs as a list (indented by
//! their level), other text boxes' words, and its speaker's notes; hidden slides say so.
//!
//! The "PowerPoint Document" stream is records in containers. The Current User stream says where the last edit's
//! UserEditAtom is; each edit's persist directory says where each object (the document, each slide and notes page)
//! is now (later edits' first). The document lists the slides in order (a SlideListWithText), with the words of
//! their placeholders (titles, bodies); each slide's drawing has its shapes, whose client text boxes hold their own
//! words or point at those.

use crate::cfb::Compound;
use crate::{Error, Out, markdown_table};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

const DOCUMENT: u16 = 0x03E8;
const SLIDE: u16 = 0x03EE;
const SLIDE_ATOM: u16 = 0x03EF;
const NOTES: u16 = 0x03F0;
const PERSIST_ATOM: u16 = 0x03F3;
const SHOW_INFO: u16 = 0x03F9;
const TEXT_HEADER: u16 = 0x0F9F;
const TEXT_CHARS: u16 = 0x0FA0;
const TEXT_BYTES: u16 = 0x0FA8;
const STYLE_TEXT: u16 = 0x0FA1;
const OUTLINE_REF: u16 = 0x0F9E;
const SLIDE_LIST: u16 = 0x0FF0;
const USER_EDIT: u16 = 0x0FF5;
const PERSIST_DIRECTORY: u16 = 0x1772;
const CLIENT_TEXTBOX: u16 = 0xF00D;
const PLACEHOLDER: u16 = 0x0BC3;
const SHAPE: u16 = 0xF004;
const GROUP: u16 = 0xF003;
const CHILD_ANCHOR: u16 = 0xF00F;

/// A record: its header's version, instance and type, and its bytes.
struct Record<'a> {
    version: u16,
    instance: u16,
    kind: u16,
    data: &'a [u8],
}

/// The records one after another in `b`.
fn records(b: &[u8]) -> Vec<Record<'_>> {
    let mut out = Vec::new();
    let mut at = 0;
    while let (Some(vi), Some(kind), Some(len)) = (u16_at(b, at), u16_at(b, at + 2), u32_at(b, at + 4)) {
        let start = at + 8;
        let end = start.saturating_add(len as usize).min(b.len());
        out.push(Record { version: vi & 0xF, instance: vi >> 4, kind, data: &b[start..end] });
        if end <= at || out.len() > 1_000_000 {
            break;
        }
        at = end;
    }
    out
}

/// The record at `at` in the stream.
fn record_at(b: &[u8], at: usize) -> Option<Record<'_>> {
    let vi = u16_at(b, at)?;
    let kind = u16_at(b, at + 2)?;
    let len = u32_at(b, at + 4)? as usize;
    let start = at + 8;
    let data = b.get(start..start.checked_add(len)?.min(b.len()))?;
    Some(Record { version: vi & 0xF, instance: vi >> 4, kind, data })
}

/// A piece of text: what it's for (TextHeaderAtom's type: 0 a title, 1 a body, 2 notes, 4 other, 5 a centred body,
/// 6 a centred title, 7 and 8 half and quarter bodies), and its paragraphs.
#[derive(Clone)]
struct Text {
    kind: u32,
    paragraphs: Vec<Para>,
    /// A table's rows (its cells' words), if it's a table.
    table: Vec<Vec<String>>,
}

/// A paragraph: its words, its indent level, and whether it has a bullet if its own properties say.
#[derive(Clone, Default)]
struct Para {
    text: String,
    level: u16,
    bullet: Option<bool>,
}

pub fn read(cf: &Compound) -> Result<String, Error> {
    let doc = cf.stream("PowerPoint Document").ok_or(Error::Damaged)?;
    let user = cf.stream("Current User").ok_or(Error::Damaged)?;
    if u32_at(&user, 12) == Some(0xF3D1_C4DF) || cf.has("EncryptedSummary") {
        return Err(Error::Encrypted);
    }
    // Where everything is: the persist directories of every edit, the last one's first.
    let mut persist: BTreeMap<u32, u32> = BTreeMap::new();
    let mut edit = u32_at(&user, 16).ok_or(Error::Damaged)? as usize;
    let mut document = None;
    let mut seen = 0;
    while seen < 1000 {
        seen += 1;
        let Some(r) = record_at(&doc, edit).filter(|r| r.kind == USER_EDIT) else { break };
        if document.is_none() {
            document = u32_at(r.data, 16);
            if r.data.len() > 28 && u32_at(r.data, 28).unwrap_or(0) != 0 {
                return Err(Error::Encrypted);
            }
        }
        let directory = u32_at(r.data, 12).unwrap_or(0) as usize;
        if let Some(dir) = record_at(&doc, directory).filter(|d| d.kind == PERSIST_DIRECTORY) {
            let mut i = 0;
            while let Some(head) = u32_at(dir.data, i) {
                let (first, count) = (head & 0xF_FFFF, head >> 20);
                for k in 0..count {
                    let Some(offset) = u32_at(dir.data, i + 4 + 4 * k as usize) else { break };
                    persist.entry(first + k).or_insert(offset);
                }
                i += 4 + 4 * count as usize;
            }
        }
        let last = u32_at(r.data, 8).unwrap_or(0) as usize;
        if last == 0 || last == edit {
            break;
        }
        edit = last;
    }
    let document = document.and_then(|d| persist.get(&d)).ok_or(Error::Damaged)?;
    let document = record_at(&doc, *document as usize).filter(|r| r.kind == DOCUMENT).ok_or(Error::Damaged)?;
    // The slides in order, each with its placeholders' words; and the notes pages, by their ids.
    let mut slides: Vec<(u32, Vec<Text>)> = Vec::new();
    let mut notes_pages: BTreeMap<u32, u32> = BTreeMap::new();
    for list in records(document.data).into_iter().filter(|r| r.kind == SLIDE_LIST && r.version == 0xF) {
        let mut current: Option<usize> = None;
        let mut kind = 4;
        for r in records(list.data) {
            match (list.instance, r.kind) {
                (0, PERSIST_ATOM) => {
                    slides.push((u32_at(r.data, 0).unwrap_or(0), Vec::new()));
                    current = Some(slides.len() - 1);
                }
                (2, PERSIST_ATOM) => {
                    if let (Some(p), Some(id)) = (u32_at(r.data, 0), u32_at(r.data, 12)) {
                        notes_pages.insert(id, p);
                    }
                }
                (0, TEXT_HEADER) => kind = u32_at(r.data, 0).unwrap_or(4),
                (0, TEXT_CHARS | TEXT_BYTES) => {
                    if let Some(c) = current {
                        slides[c].1.push(Text { kind, paragraphs: paragraphs(&r), table: Vec::new() });
                    }
                }
                (0, STYLE_TEXT) => {
                    if let Some(t) = current.and_then(|c| slides[c].1.last_mut()) {
                        levels(r.data, t);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Out::new();
    for (n, (persist_id, outline)) in slides.iter().enumerate() {
        let Some(slide) = persist.get(persist_id).and_then(|&at| record_at(&doc, at as usize)).filter(|r| r.kind == SLIDE) else { continue };
        let mut hidden = false;
        let mut notes_id = 0;
        for r in records(slide.data) {
            match r.kind {
                SLIDE_ATOM => notes_id = u32_at(r.data, 16).unwrap_or(0),
                SHOW_INFO => hidden = u16_at(r.data, 10).unwrap_or(0) & 0x0004 != 0,
                _ => {}
            }
        }
        let mut texts = Vec::new();
        let mut used = Vec::new();
        shapes(slide.data, outline, &mut texts, &mut used);
        // (Placeholders' words no shape pointed at: as they are in the list.)
        for (i, t) in outline.iter().enumerate() {
            if !used.contains(&i) {
                texts.push(t.clone());
            }
        }
        let title = texts.iter().position(|t| matches!(t.kind, 0 | 6));
        out.gap();
        let mut heading = format!("## Slide {}", n + 1);
        if let Some(i) = title {
            let words: Vec<&str> = texts[i].paragraphs.iter().map(|p| p.text.trim()).filter(|p| !p.is_empty()).collect();
            if !words.is_empty() {
                heading.push_str(": ");
                heading.push_str(&words.join(" "));
            }
        }
        if hidden {
            heading.push_str(" (hidden)");
        }
        out.line(&heading);
        for (i, t) in texts.iter().enumerate() {
            if Some(i) == title || t.kind == 2 {
                continue;
            }
            if !t.table.is_empty() {
                let mut lines = Out::new();
                markdown_table(&t.table, &mut lines);
                for l in lines.done().lines().filter(|l| !l.is_empty()) {
                    out.line(l);
                }
                continue;
            }
            // (A body's paragraphs are a list unless they say they've no bullet; others' if they say they have.)
            let body = matches!(t.kind, 1 | 7 | 8);
            for p in &t.paragraphs {
                if p.text.trim().is_empty() {
                    continue;
                }
                if p.bullet.unwrap_or(body) {
                    out.line(&format!("{}- {}", "  ".repeat(p.level as usize), p.text.trim()));
                } else {
                    out.line(p.text.trim());
                }
            }
        }
        // Its notes: the notes page's notes placeholder.
        if let Some(page) = notes_pages.get(&notes_id).and_then(|p| persist.get(p)).and_then(|&at| record_at(&doc, at as usize)).filter(|r| r.kind == NOTES) {
            let mut texts = Vec::new();
            shapes(page.data, &[], &mut texts, &mut Vec::new());
            let words: Vec<String> = texts.iter().filter(|t| t.kind == 2).flat_map(|t| t.paragraphs.iter().map(|p| String::from(p.text.trim()))).filter(|p| !p.is_empty()).collect();
            if !words.is_empty() {
                out.line(&format!("Notes: {}", words.join(" / ")));
            }
        }
    }
    Ok(out.done())
}

/// A text atom's paragraphs (split at carriage returns; a line break within one is a space).
fn paragraphs(r: &Record) -> Vec<Para> {
    let text: String = if r.kind == TEXT_CHARS {
        let units: Vec<u16> = r.data.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        r.data.iter().map(|&b| b as char).collect()
    };
    text.split('\r')
        .map(|p| {
            let text = p.chars().map(|c| if c == '\u{0B}' { ' ' } else { c }).filter(|&c| c == '\t' || c >= ' ').collect();
            Para { text, ..Para::default() }
        })
        .collect()
}

/// The paragraphs' indent levels and bullets, from the style atom after their text: its paragraph runs (each how
/// many characters it covers, its level, then the properties its mask says it has, a bullet's flags first).
fn levels(b: &[u8], t: &mut Text) {
    let lengths: Vec<usize> = t.paragraphs.iter().map(|p| p.text.encode_utf16().count() + 1).collect();
    let mut i = 0;
    let mut para = 0;
    let mut covered = 0usize;
    let total: usize = lengths.iter().sum();
    while covered < total {
        let (Some(count), Some(level), Some(mask)) = (u32_at(b, i), u16_at(b, i + 4), u32_at(b, i + 6)) else { return };
        i += 10;
        let bullet = if mask & 1 != 0 { u16_at(b, i).map(|f| f & 1 != 0) } else { None };
        // The properties present, in the order they come.
        let sizes: [(u32, usize); 16] = [
            (0x0000_000F, 2),
            (0x0000_0080, 2),
            (0x0000_0010, 2),
            (0x0000_0040, 2),
            (0x0000_0020, 4),
            (0x0000_0800, 2),
            (0x0000_1000, 2),
            (0x0000_2000, 2),
            (0x0000_4000, 2),
            (0x0000_0100, 2),
            (0x0000_0400, 2),
            (0x0000_8000, 2),
            (0x0010_0000, 0),
            (0x0001_0000, 2),
            (0x000E_0000, 2),
            (0x0020_0000, 2),
        ];
        for (bits, size) in sizes {
            if mask & bits == 0 {
                continue;
            }
            if bits == 0x0010_0000 {
                // Tab stops: how many, then 4 bytes each.
                let Some(n) = u16_at(b, i) else { return };
                i += 2 + 4 * n as usize;
            } else {
                i += size;
            }
        }
        // The paragraphs this run covers take its level.
        let end = covered + count as usize;
        let mut at = covered;
        while para < lengths.len() && at < end {
            t.paragraphs[para].level = level.min(8);
            t.paragraphs[para].bullet = bullet;
            at += lengths[para];
            para += 1;
        }
        covered = end.max(covered + 1);
        if i > b.len() {
            return;
        }
    }
}

/// The words of a slide's (or notes page's) shapes, in the order they're drawn: each client text box's own text, or
/// the placeholder's words in the slide list it points at (whose numbers go in `used`). The date, footer and slide
/// number placeholders are passed over.
fn shapes(b: &[u8], outline: &[Text], texts: &mut Vec<Text>, used: &mut Vec<usize>) {
    for r in records(b) {
        if r.kind == GROUP && r.version == 0xF {
            let children = records(r.data);
            if children.first().is_some_and(|g| g.kind == SHAPE && is_table(g.data)) {
                texts.push(Text { kind: 4, paragraphs: Vec::new(), table: table(&children[1..]) });
                continue;
            }
        }
        if r.kind == SHAPE && r.version == 0xF {
            // A shape: its placeholder's type, if it's one, decides whether it's read.
            let inner = records(r.data);
            let footer = inner.iter().any(|c| {
                c.kind == 0xF011 && records(c.data).iter().any(|p| p.kind == PLACEHOLDER && matches!(p.data.get(4), Some(0x07..=0x0A)))
            });
            if footer {
                continue;
            }
            shapes(r.data, outline, texts, used);
            continue;
        }
        if r.kind == CLIENT_TEXTBOX {
            let mut kind = 4;
            for c in records(r.data) {
                match c.kind {
                    TEXT_HEADER => kind = u32_at(c.data, 0).unwrap_or(4),
                    TEXT_CHARS | TEXT_BYTES => texts.push(Text { kind, paragraphs: paragraphs(&c), table: Vec::new() }),
                    STYLE_TEXT => {
                        if let Some(t) = texts.last_mut() {
                            levels(c.data, t);
                        }
                    }
                    OUTLINE_REF => {
                        let i = u32_at(c.data, 0).unwrap_or(u32::MAX) as usize;
                        if let Some(t) = outline.get(i) {
                            texts.push(t.clone());
                            used.push(i);
                        }
                    }
                    _ => {}
                }
            }
            continue;
        }
        if r.version == 0xF {
            shapes(r.data, outline, texts, used);
        }
    }
}

/// Whether a group's own shape says the group is a table (its tableProperties, property 0x039F, in its property
/// tables).
fn is_table(shape: &[u8]) -> bool {
    records(shape).iter().any(|r| {
        matches!(r.kind, 0xF00B | 0xF121 | 0xF122)
            && (0..r.instance as usize).any(|k| u16_at(r.data, 6 * k).unwrap_or(0) & 0x3FFF == 0x039F && u32_at(r.data, 6 * k + 2).unwrap_or(0) & 1 != 0)
    })
}

/// A table's rows: its cells (the shapes with text boxes) put in rows and columns by where they are in the group.
fn table(shapes: &[Record]) -> Vec<Vec<String>> {
    let mut cells: Vec<(i32, i32, String)> = Vec::new();
    for s in shapes.iter().filter(|s| s.kind == SHAPE) {
        let inner = records(s.data);
        let Some(anchor) = inner.iter().find(|r| r.kind == CHILD_ANCHOR) else { continue };
        let Some(textbox) = inner.iter().find(|r| r.kind == CLIENT_TEXTBOX) else { continue };
        let (Some(left), Some(top)) = (u32_at(anchor.data, 0), u32_at(anchor.data, 4)) else { continue };
        let words: Vec<String> = records(textbox.data)
            .iter()
            .filter(|r| matches!(r.kind, TEXT_CHARS | TEXT_BYTES))
            .flat_map(|r| paragraphs(r).into_iter().map(|p| p.text))
            .filter(|p| !p.trim().is_empty())
            .collect();
        cells.push((top as i32, left as i32, words.join(" ")));
    }
    let mut tops: Vec<i32> = cells.iter().map(|c| c.0).collect();
    let mut lefts: Vec<i32> = cells.iter().map(|c| c.1).collect();
    tops.sort_unstable();
    tops.dedup();
    lefts.sort_unstable();
    lefts.dedup();
    tops.iter()
        .map(|&t| lefts.iter().map(|&l| cells.iter().find(|c| c.0 == t && c.1 == l).map(|c| c.2.clone()).unwrap_or_default()).collect())
        .collect()
}
