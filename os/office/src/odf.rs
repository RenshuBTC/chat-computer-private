//! An OpenDocument file's words (content.xml): a text's headings, paragraphs, lists and tables as Markdown; a
//! spreadsheet's sheets as CSV (`sheet_csv`), each cell as it's shown (OpenDocument keeps that, as a paragraph, with
//! the value); a presentation's pages, with their notes. Spaces (text:s), tabs and line breaks are put back; comments,
//! notes in the text and tracked changes' deleted words are left out.

use crate::xml::{self, Reader, Token};
use crate::{Error, MOST_COLUMNS, Out, markdown_table, part, sheet_csv};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use zip::Archive;

/// The most times a repeated row or cell with something in it is repeated (an empty one only moves the place on).
const MOST_REPEATS: u32 = 100;
/// The most memory a document's table cells take, their words and places, all its tables together: a cell repeated a
/// hundred times in a row repeated a hundred times is ten thousand cells from a hundred bytes of XML, so a small file
/// could otherwise ask for gigabytes. Many times the text made from them (MOST_TEXT): a cell past it is left out.
const MOST_CELL_BYTES: usize = 64 << 20;
/// What a kept cell costs, besides its words.
const CELL_COST: usize = core::mem::size_of::<(usize, String)>();

/// A table being read: its name, and its rows (each its number, and its cells' columns and words).
struct Table {
    name: String,
    rows: Vec<(u32, Vec<(usize, String)>)>,
    row: u32,
    column: usize,
    /// How many times the row being read repeats, and the cell.
    row_repeats: u32,
    cell_repeats: u32,
    cell: Option<String>,
}

pub fn read(archive: &Archive) -> Result<String, Error> {
    let content = part(archive, "content.xml")?.ok_or(Error::Damaged)?;
    let mut out = Out::new();
    let mut spreadsheet = false;
    // Paragraphs being read (a frame's can be inside another's): each its words and what it is.
    let mut paras: Vec<(String, u8, bool)> = Vec::new();
    let mut tables: Vec<Table> = Vec::new();
    // What the document's table cells take so far (MOST_CELL_BYTES).
    let mut spent = 0usize;
    let mut lists = 0usize;
    let mut skip = 0usize;
    let (mut page, mut in_notes) = (0usize, false);
    let mut notes: Vec<String> = Vec::new();
    // Paragraph styles' parents (the automatic styles a document's paragraphs use are based on its named ones): a
    // paragraph whose style is, or is based on, "Title" is the title.
    let mut parents: BTreeMap<String, String> = BTreeMap::new();
    let is_title = |parents: &BTreeMap<String, String>, mut style: Option<String>| {
        for _ in 0..8 {
            match style {
                Some(s) if s == "Title" => return true,
                Some(s) => style = parents.get(&s).cloned(),
                None => return false,
            }
        }
        false
    };
    for token in Reader::new(&content) {
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
                "office:annotation" | "text:note" | "text:tracked-changes" | "office:forms" if !t.empty => skip = 1,
                "office:spreadsheet" => spreadsheet = true,
                "style:style" => {
                    if let (Some(name), Some(parent)) = (t.attr("style:name"), t.attr("style:parent-style-name")) {
                        parents.insert(name, parent);
                    }
                }
                "draw:page" => {
                    page += 1;
                    out.gap();
                    let name = t.attr("draw:name").filter(|n| !n.is_empty() && !n.starts_with("page"));
                    out.line(&match name {
                        Some(n) => format!("## Slide {page}: {n}"),
                        None => format!("## Slide {page}"),
                    });
                }
                "presentation:notes" => in_notes = !t.empty,
                "text:list" => lists += !t.empty as usize,
                "text:h" | "text:p" if !t.empty => {
                    let level = if t.name == "text:h" {
                        t.attr("text:outline-level").and_then(|l| l.parse::<u8>().ok()).unwrap_or(1).clamp(1, 6)
                    } else if is_title(&parents, t.attr("text:style-name")) {
                        1
                    } else {
                        0
                    };
                    paras.push((String::new(), level, lists > 0));
                }
                "text:s" => {
                    let n = t.attr("text:c").and_then(|c| c.parse::<usize>().ok()).unwrap_or(1).min(100);
                    push(&mut paras, &" ".repeat(n));
                }
                "text:tab" => push(&mut paras, "\t"),
                "text:line-break" => push(&mut paras, "\n"),
                "table:table" if !t.empty => tables.push(Table {
                    name: t.attr("table:name").unwrap_or_default(),
                    rows: Vec::new(),
                    row: 0,
                    column: 0,
                    row_repeats: 1,
                    cell_repeats: 1,
                    cell: None,
                }),
                "table:table-row" => {
                    if let Some(table) = tables.last_mut() {
                        table.row += 1;
                        table.column = 0;
                        table.row_repeats = t.attr("table:number-rows-repeated").and_then(|r| r.parse().ok()).unwrap_or(1).max(1);
                        table.rows.push((table.row, Vec::new()));
                        if t.empty {
                            end_row(table, &mut spent);
                        }
                    }
                }
                "table:table-cell" | "table:covered-table-cell" => {
                    if let Some(table) = tables.last_mut() {
                        table.cell_repeats = t.attr("table:number-columns-repeated").and_then(|r| r.parse().ok()).unwrap_or(1).max(1);
                        table.cell = Some(String::new());
                        if t.empty {
                            end_cell(table, &mut spent);
                        }
                    }
                }
                _ => {}
            },
            Token::End(name) => match name {
                "text:list" => lists = lists.saturating_sub(1),
                "presentation:notes" => {
                    in_notes = false;
                    if !notes.is_empty() {
                        out.line(&format!("Notes: {}", notes.join(" / ")));
                        notes.clear();
                    }
                }
                "text:h" | "text:p" => {
                    let Some((words, level, listed)) = paras.pop() else { continue };
                    let words = String::from(words.trim());
                    if let Some(outer) = paras.last_mut() {
                        if !words.is_empty() {
                            if !outer.0.is_empty() {
                                outer.0.push(' ');
                            }
                            outer.0.push_str(&words);
                        }
                    } else if let Some(cell) = tables.last_mut().and_then(|t| t.cell.as_mut()) {
                        if !words.is_empty() {
                            if !cell.is_empty() {
                                cell.push(' ');
                            }
                            cell.push_str(&words);
                        }
                    } else if in_notes {
                        if !words.is_empty() {
                            notes.push(words);
                        }
                    } else if words.is_empty() {
                        out.gap();
                    } else if level > 0 {
                        out.gap();
                        out.line(&format!("{} {}", "#".repeat(level as usize), words.replace('\n', " ")));
                        out.gap();
                    } else if listed {
                        out.line(&format!("{}- {words}", "  ".repeat(lists.saturating_sub(1))));
                    } else {
                        out.line(&words);
                    }
                }
                "table:table-cell" | "table:covered-table-cell" => {
                    if let Some(table) = tables.last_mut() {
                        end_cell(table, &mut spent);
                    }
                }
                "table:table-row" => {
                    if let Some(table) = tables.last_mut() {
                        end_row(table, &mut spent);
                    }
                }
                "table:table" => {
                    let Some(table) = tables.pop() else { continue };
                    if spreadsheet {
                        out.gap();
                        out.line(&format!("## {}", table.name));
                        sheet_csv(&table.rows, &mut out);
                    } else {
                        let rows: Vec<Vec<String>> = table
                            .rows
                            .iter()
                            .filter(|(_, cells)| !cells.is_empty())
                            .map(|(_, cells)| {
                                // A cell repeated 4,000,000,000 times moves the column on that far: don't size a row
                                // by it (and a cell past the cap is left out).
                                let width = cells.iter().map(|(c, _)| c + 1).max().unwrap_or(0).min(MOST_COLUMNS);
                                let mut row = alloc::vec![String::new(); width];
                                for (c, w) in cells {
                                    if *c < width {
                                        row[*c] = w.clone();
                                    }
                                }
                                row
                            })
                            .collect();
                        markdown_table(&rows, &mut out);
                    }
                }
                _ => {}
            },
            Token::Text(s) => {
                // Spaces in a paragraph's text count once (text:s says how many there really are).
                let words = xml::text(s);
                let mut collapsed = String::with_capacity(words.len());
                for (i, part) in words.split([' ', '\t', '\n', '\r']).enumerate() {
                    if i > 0 && !collapsed.ends_with(' ') {
                        collapsed.push(' ');
                    }
                    collapsed.push_str(part);
                }
                push(&mut paras, &collapsed);
            }
            Token::Raw(s) => push(&mut paras, s),
        }
    }
    Ok(out.done())
}

fn push(paras: &mut [(String, u8, bool)], s: &str) {
    if let Some(p) = paras.last_mut() {
        p.0.push_str(s);
    }
}

/// A cell ends: its words in its row, as many times as it repeats (an empty one just moves the column on), while the
/// document's cells are within MOST_CELL_BYTES (`spent`).
fn end_cell(table: &mut Table, spent: &mut usize) {
    let words = table.cell.take().unwrap_or_default();
    let repeats = table.cell_repeats as usize;
    if !words.is_empty()
        && let Some((_, cells)) = table.rows.last_mut()
    {
        let cost = words.len() + CELL_COST;
        for k in 0..repeats.min(MOST_REPEATS as usize) {
            if *spent + cost > MOST_CELL_BYTES {
                break;
            }
            *spent += cost;
            cells.push((table.column + k, words.clone()));
        }
    }
    table.column = table.column.saturating_add(repeats);
}

/// A row ends: repeated as many times as it says, if it has anything in it (and while the document's cells are within
/// MOST_CELL_BYTES); a row with nothing in it isn't kept, only its number moves on.
fn end_row(table: &mut Table, spent: &mut usize) {
    let repeats = table.row_repeats;
    table.row_repeats = 1;
    if table.rows.last().is_some_and(|(_, cells)| cells.is_empty()) {
        table.rows.pop();
        table.row = table.row.saturating_add(repeats - 1);
        return;
    }
    if repeats <= 1 {
        return;
    }
    let Some((n, cells)) = table.rows.last().cloned() else { return };
    let cost: usize = cells.iter().map(|(_, w)| w.len() + CELL_COST).sum();
    for k in 1..repeats.min(MOST_REPEATS) {
        if *spent + cost > MOST_CELL_BYTES {
            break;
        }
        *spent += cost;
        table.rows.push((n + k, cells.clone()));
    }
    table.row = n.saturating_add(repeats - 1);
}
