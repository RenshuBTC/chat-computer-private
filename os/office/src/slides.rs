//! A PowerPoint presentation's words (PresentationML): each slide in the presentation's order, "## Slide 2: <its
//! title>", then the words of its other shapes (a content placeholder's paragraphs as a list, indented by their
//! level), its tables in Markdown, its pictures' descriptions, and its speaker's notes. Hidden slides say so. The
//! date, footer and slide number shapes are left out.

use crate::xml::{self, Reader, Token};
use crate::{Error, Out, markdown_table, part, relationships};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use zip::Archive;

pub fn read(archive: &Archive, main: &str) -> Result<String, Error> {
    let rels = relationships(archive, main)?;
    let presentation = part(archive, main)?.ok_or(Error::Damaged)?;
    let order: Vec<String> = Reader::new(&presentation)
        .filter_map(|token| match token {
            Token::Start(t) if t.local() == "sldId" => t.attr_prefixed("id"),
            _ => None,
        })
        .collect();
    let mut out = Out::new();
    for (n, id) in order.iter().enumerate() {
        let Some(target) = rels.iter().find(|r| &r.id == id).map(|r| r.target.clone()) else { continue };
        let Some(xml) = part(archive, &target)? else { continue };
        let slide = shapes(&xml);
        out.gap();
        let mut heading = format!("## Slide {}", n + 1);
        if let Some(title) = &slide.title {
            heading.push_str(": ");
            heading.push_str(title);
        }
        if slide.hidden {
            heading.push_str(" (hidden)");
        }
        out.line(&heading);
        for l in &slide.lines {
            out.line(l);
        }
        // Its notes: the body of the notes page that goes with it.
        let notes = relationships(archive, &target)?.into_iter().find(|r| r.kind.ends_with("/notesSlide"));
        if let Some(notes) = notes
            && let Some(xml) = part(archive, &notes.target)?
        {
            let words: Vec<String> = shapes(&xml).notes;
            if !words.is_empty() {
                out.line(&format!("Notes: {}", words.join(" / ")));
            }
        }
    }
    Ok(out.done())
}

/// A slide's (or notes page's) words.
#[derive(Default)]
pub(crate) struct Slide {
    title: Option<String>,
    lines: Vec<String>,
    /// A notes page's notes (its body placeholder's paragraphs).
    notes: Vec<String>,
    hidden: bool,
}

/// A shape being read: its placeholder's type ("title", "body"...), if it's one, and its paragraphs (with levels).
struct Shape {
    placeholder: Option<String>,
    paragraphs: Vec<(u8, String)>,
}

pub(crate) fn shapes(xml: &str) -> Slide {
    let mut slide = Slide::default();
    let mut shapes: Vec<Shape> = Vec::new();
    let mut paragraph: Option<(u8, String)> = None;
    let mut tables: Vec<Vec<Vec<String>>> = Vec::new();
    let (mut in_text, mut in_picture) = (false, false);
    for token in Reader::new(xml) {
        match token {
            Token::Start(t) => match t.name {
                "p:sld" => slide.hidden = t.attr("show").as_deref() == Some("0"),
                "p:sp" if !t.empty => shapes.push(Shape { placeholder: None, paragraphs: Vec::new() }),
                "p:ph" => {
                    if let Some(s) = shapes.last_mut() {
                        // (A placeholder without a type is the body's, for content.)
                        s.placeholder = Some(t.attr("type").unwrap_or_else(|| String::from("body")));
                    }
                }
                "p:pic" => in_picture = !t.empty,
                "p:cNvPr" if in_picture => {
                    if let Some(d) = t.attr("descr").filter(|d| !d.trim().is_empty()) {
                        slide.lines.push(format!("[Picture: {}]", d.trim()));
                    }
                }
                "a:tbl" => tables.push(Vec::new()),
                "a:tr" => {
                    if let Some(t) = tables.last_mut() {
                        t.push(Vec::new());
                    }
                }
                "a:tc" => {
                    if let Some(row) = tables.last_mut().and_then(|t| t.last_mut()) {
                        row.push(String::new());
                    }
                }
                "a:p" if !t.empty => paragraph = Some((0, String::new())),
                "a:pPr" => {
                    if let (Some(p), Some(level)) = (paragraph.as_mut(), t.attr("lvl").and_then(|l| l.parse::<u8>().ok())) {
                        p.0 = level.min(8);
                    }
                }
                "a:t" => in_text = !t.empty,
                "a:br" => {
                    if let Some(p) = paragraph.as_mut() {
                        p.1.push(' ');
                    }
                }
                _ => {}
            },
            Token::End(name) => match name {
                "a:t" => in_text = false,
                "p:pic" => in_picture = false,
                "a:p" => {
                    let Some((level, text)) = paragraph.take() else { continue };
                    let text = String::from(text.trim());
                    if text.is_empty() {
                        continue;
                    }
                    if let Some(cell) = tables.last_mut().and_then(|t| t.last_mut()).and_then(|r| r.last_mut()) {
                        if !cell.is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(&text);
                    } else if let Some(s) = shapes.last_mut() {
                        s.paragraphs.push((level, text));
                    } else {
                        slide.lines.push(text);
                    }
                }
                "a:tbl" => {
                    if let Some(rows) = tables.pop() {
                        let mut t = Out::new();
                        markdown_table(&rows, &mut t);
                        slide.lines.extend(t.done().lines().filter(|l| !l.is_empty()).map(String::from));
                    }
                }
                "p:sp" => {
                    let Some(s) = shapes.pop() else { continue };
                    let words: Vec<&str> = s.paragraphs.iter().map(|(_, t)| t.as_str()).collect();
                    match s.placeholder.as_deref() {
                        Some("title" | "ctrTitle") if slide.title.is_none() && !words.is_empty() => {
                            slide.title = Some(words.join(" "));
                        }
                        Some("dt" | "ftr" | "sldNum" | "hdr" | "sldImg") => {}
                        Some("body") => {
                            slide.notes.extend(words.iter().map(|w| String::from(*w)));
                            for (level, text) in &s.paragraphs {
                                slide.lines.push(format!("{}- {text}", "  ".repeat(*level as usize)));
                            }
                        }
                        Some("obj") => {
                            for (level, text) in &s.paragraphs {
                                slide.lines.push(format!("{}- {text}", "  ".repeat(*level as usize)));
                            }
                        }
                        _ => slide.lines.extend(words.iter().map(|w| String::from(*w))),
                    }
                }
                _ => {}
            },
            Token::Text(s) if in_text => {
                if let Some(p) = paragraph.as_mut() {
                    p.1.push_str(&xml::text(s));
                }
            }
            _ => {}
        }
    }
    slide
}
