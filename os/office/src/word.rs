//! A Word document's words (WordprocessingML): its body's paragraphs in order, headings and lists marked as Markdown
//! marks them (by the paragraph's own properties or its style's, from styles.xml), tables as Markdown tables, tabs and
//! line breaks kept. Text boxes' words go with the paragraph they're in. What Word keeps twice for older readers
//! (mc:Fallback) is read once; deleted text and field codes, which aren't in w:t, aren't read at all. Footnotes and
//! endnotes follow the body.

use crate::xml::{self, Reader, Token};
use crate::{Error, Out, markdown_table, part, relationships};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use zip::Archive;

/// What a paragraph style makes a paragraph: a heading (its level, 1 to 6), a list's item, or neither.
#[derive(Clone, Copy, Default)]
pub(crate) struct Style {
    heading: u8,
    list: bool,
}

pub fn read(archive: &Archive, main: &str) -> Result<String, Error> {
    let rels = relationships(archive, main)?;
    let find = |end: &str| rels.iter().find(|r| r.kind.ends_with(end)).map(|r| r.target.clone());
    let styles = match find("/styles") {
        Some(p) => styles(&part(archive, &p)?.unwrap_or_default()),
        None => BTreeMap::new(),
    };
    let body = part(archive, main)?.ok_or(Error::Damaged)?;
    let mut out = Out::new();
    words(&body, &styles, &mut out);
    for (end, title) in [("/footnotes", "Footnotes"), ("/endnotes", "Endnotes")] {
        if let Some(p) = find(end)
            && let Some(notes) = part(archive, &p)?
        {
            let mut these = Out::new();
            words(&notes, &styles, &mut these);
            let these = these.done();
            if !these.trim().is_empty() {
                out.gap();
                out.line(&format!("{title}:"));
                for l in these.lines() {
                    out.line(l);
                }
            }
        }
    }
    Ok(out.done())
}

/// Each paragraph style's level as a heading, and whether it's a list's: from its name ("heading 2", "Title"), its
/// outline level, its numbering, or the style it's based on.
fn styles(xml: &str) -> BTreeMap<String, Style> {
    struct Raw {
        name: String,
        outline: Option<u8>,
        list: bool,
        based_on: Option<String>,
    }
    let mut raw: BTreeMap<String, Raw> = BTreeMap::new();
    let mut current: Option<(String, Raw)> = None;
    for token in Reader::new(xml) {
        match token {
            Token::Start(t) if t.name == "w:style" => {
                if t.attr("w:type").as_deref().unwrap_or("paragraph") == "paragraph"
                    && let Some(id) = t.attr("w:styleId")
                {
                    let r = Raw { name: String::new(), outline: None, list: false, based_on: None };
                    if t.empty {
                        raw.insert(id, r);
                    } else {
                        current = Some((id, r));
                    }
                }
            }
            Token::Start(t) => {
                if let Some((_, r)) = current.as_mut() {
                    match t.name {
                        "w:name" => r.name = t.attr("w:val").unwrap_or_default().to_lowercase(),
                        "w:outlineLvl" => r.outline = t.attr("w:val").and_then(|v| v.parse().ok()),
                        "w:numPr" => r.list = true,
                        "w:basedOn" => r.based_on = t.attr("w:val"),
                        _ => {}
                    }
                }
            }
            Token::End("w:style") => {
                if let Some((id, r)) = current.take() {
                    raw.insert(id, r);
                }
            }
            _ => {}
        }
    }
    let own = |r: &Raw| -> Option<u8> {
        if r.name == "title" {
            return Some(1);
        }
        if let Some(n) = r.name.strip_prefix("heading ").and_then(|n| n.trim().parse::<u8>().ok()) {
            return Some(n);
        }
        // (Outline level 9 is body text.)
        r.outline.filter(|&o| o < 9).map(|o| o + 1)
    };
    let mut out = BTreeMap::new();
    for (id, r) in &raw {
        // From the style, else the one it's based on (a few steps at most, in case they go round).
        let (mut heading, mut list, mut at) = (own(r), r.list, r);
        for _ in 0..8 {
            if heading.is_some() && list {
                break;
            }
            let Some(base) = at.based_on.as_ref().and_then(|b| raw.get(b)) else { break };
            heading = heading.or(own(base));
            list |= base.list;
            at = base;
        }
        // (A list style is often based on a heading's or the other way: a style that's a heading isn't a list.)
        let heading = heading.unwrap_or(0).min(6);
        out.insert(id.clone(), Style { heading, list: list && heading == 0 });
    }
    out
}

/// A paragraph being read.
#[derive(Default)]
struct Para {
    text: String,
    style: Style,
    /// Set by the paragraph's own properties, over its style's (a list's numbering 0 is none).
    own_heading: Option<u8>,
    own_list: Option<bool>,
}

/// A body's (or the notes') paragraphs and tables, as lines.
pub(crate) fn words(xml: &str, styles: &BTreeMap<String, Style>, out: &mut Out) {
    let mut paras: Vec<Para> = Vec::new();
    // Tables being read, each a list of rows of cells.
    let mut tables: Vec<Vec<Vec<String>>> = Vec::new();
    let (mut in_text, mut in_props) = (false, false);
    // How deep inside something that's passed over (mc:Fallback, a deleted run's text).
    let mut skip = 0usize;
    for token in Reader::new(xml) {
        if skip > 0 {
            match token {
                Token::Start(t) if !t.empty => skip += 1,
                Token::End(_) => skip -= 1,
                _ => {}
            }
            continue;
        }
        match token {
            Token::Start(t) => match t.name {
                "mc:Fallback" | "w:delText" | "w:instrText" if !t.empty => skip = 1,
                "w:p" => {
                    paras.push(Para::default());
                    if t.empty {
                        finish(&mut paras, &mut tables, out);
                    }
                }
                "w:pPr" => in_props = !t.empty,
                "w:pStyle" if in_props => {
                    if let (Some(p), Some(id)) = (paras.last_mut(), t.attr("w:val")) {
                        p.style = styles.get(&id).copied().unwrap_or_default();
                    }
                }
                "w:outlineLvl" if in_props => {
                    if let (Some(p), Some(level)) = (paras.last_mut(), t.attr("w:val").and_then(|v| v.parse::<u8>().ok())) {
                        p.own_heading = Some(if level < 9 { (level + 1).min(6) } else { 0 });
                    }
                }
                "w:numPr" if in_props => {
                    if let Some(p) = paras.last_mut() {
                        p.own_list = Some(true);
                    }
                }
                "w:numId" if in_props => {
                    if let Some(p) = paras.last_mut()
                        && t.attr("w:val").as_deref() == Some("0")
                    {
                        p.own_list = Some(false);
                    }
                }
                "w:t" => in_text = !t.empty,
                // (A w:tab in a paragraph's properties is a tab stop, not a tab.)
                "w:tab" if !in_props => push(&mut paras, "\t"),
                "w:br" | "w:cr" if !in_props => push(&mut paras, "\n"),
                "w:noBreakHyphen" => push(&mut paras, "-"),
                "w:tbl" => tables.push(Vec::new()),
                "w:tr" => {
                    if let Some(t) = tables.last_mut() {
                        t.push(Vec::new());
                    }
                }
                "w:tc" => {
                    if let Some(row) = tables.last_mut().and_then(|t| t.last_mut()) {
                        row.push(String::new());
                    }
                }
                _ => {}
            },
            Token::End(name) => match name {
                "w:t" => in_text = false,
                "w:pPr" => in_props = false,
                "w:p" => finish(&mut paras, &mut tables, out),
                "w:tbl" => {
                    let Some(rows) = tables.pop() else { continue };
                    if let Some(cell) = tables.last_mut().and_then(|t| t.last_mut()).and_then(|r| r.last_mut()) {
                        // A table in a table's cell: its rows in the cell, one after another.
                        let inner: Vec<String> = rows.iter().map(|r| r.join(", ")).collect();
                        if !cell.is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(&inner.join("; "));
                    } else {
                        markdown_table(&rows, out);
                    }
                }
                _ => {}
            },
            Token::Text(s) if in_text => push(&mut paras, &xml::text(s)),
            Token::Raw(s) if in_text => push(&mut paras, s),
            _ => {}
        }
    }
    // (A body cut short: what was read so far.)
    while !paras.is_empty() {
        finish(&mut paras, &mut tables, out);
    }
}

fn push(paras: &mut [Para], s: &str) {
    if let Some(p) = paras.last_mut() {
        p.text.push_str(s);
    }
}

/// A paragraph ends: into the paragraph it's in (a text box's), a table's cell, or the text, as a heading, a list's
/// item or as it is.
fn finish(paras: &mut Vec<Para>, tables: &mut [Vec<Vec<String>>], out: &mut Out) {
    let Some(p) = paras.pop() else { return };
    let text = p.text.trim_end();
    if let Some(outer) = paras.last_mut() {
        if !text.is_empty() {
            if !outer.text.is_empty() && !outer.text.ends_with(' ') {
                outer.text.push(' ');
            }
            outer.text.push_str(text.trim_start());
        }
        return;
    }
    if let Some(cell) = tables.last_mut().and_then(|t| t.last_mut()).and_then(|r| r.last_mut()) {
        if !text.is_empty() {
            if !cell.is_empty() {
                cell.push(' ');
            }
            cell.push_str(text.trim_start());
        }
        return;
    }
    let heading = p.own_heading.unwrap_or(p.style.heading);
    if text.is_empty() {
        out.gap();
    } else if heading > 0 {
        out.gap();
        out.line(&format!("{} {}", "#".repeat(heading as usize), text.replace('\n', " ")));
        out.gap();
    } else if p.own_list.unwrap_or(p.style.list) {
        out.line(&format!("- {text}"));
    } else {
        out.line(text);
    }
}
