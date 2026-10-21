//! Against documents made by python-docx, openpyxl and python-pptx, the same saved by LibreOffice (which works out
//! formulas and writes everything its own way), and three written by hand with what programs write less often
//! (tests/make.py): what each says, read back. Damaged ones give errors or what could be read, never a crash.

extern crate std;

use super::*;
use std::format;
use std::string::ToString;
use std::vec::Vec;

fn file(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn text_of(name: &str) -> (Kind, String) {
    read(&file(name)).unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

/// The letter, as python-docx, LibreOffice (as .docx) and LibreOffice (as .odt) write it.
const LETTER: &str = "# A letter to the council

# Why I'm writing

Dear Sir or Madam, the bins on Elm Street haven't been emptied since 3 March. Prices: 5 < 7 & 7 > 5; \"quoted\" and 'single'.

## What I'd like

- Collection every week
- A bigger bin
- First, call me
Name:\tSam Lee
Street: Elm Street 12

| Week | Bins | Emptied |
| --- | --- | --- |
| 1 | 2 | no |
| 2 | 2 | yes |

Ünïcödé, 名前, café and 🙂.";

#[test]
fn reads_word_documents() {
    for name in ["letter.docx", "letter-libreoffice.docx", "letter.odt"] {
        let (kind, text) = text_of(name);
        assert_eq!(kind, if name.ends_with("odt") { Kind::Text } else { Kind::Word }, "{name}");
        assert!(text.starts_with(LETTER), "{name}:\n{text}");
        assert!(text.trim_end().ends_with("Yours faithfully,"), "{name}:\n{text}");
    }
}

#[test]
fn reads_workbooks() {
    let python = text_of("budget.xlsx");
    let libreoffice = text_of("budget-libreoffice.xlsx");
    assert_eq!((python.0, libreoffice.0), (Kind::Excel, Kind::Excel));
    let rows = |total: &str| {
        format!(
            "## Budget
row,A,B,C,D,E,F,G
1,Item,Cost,Paid,When,Share,,
2,Rent,1200,TRUE,2026-10-01,75%,,
3,\"Food, and drink\",412.5,FALSE,2026-10-03 18:30,,,
4,Total,{total},,,,,
7,,,,,,,far away

## Notes
row,A,B
1,\"Remember: \"\"quotes\"\", commas and new
lines\",
2,,07:45"
        )
    };
    // As written by a program (the formula never worked out), and as LibreOffice saved it (worked out).
    assert_eq!(python.1, rows("=SUM(B2:B3)"));
    assert_eq!(libreoffice.1, rows("1612.5 (=SUM(B2:B3))"));
}

#[test]
fn reads_presentations() {
    for name in ["talk.pptx", "talk-libreoffice.pptx"] {
        let (kind, text) = text_of(name);
        assert_eq!(kind, Kind::PowerPoint);
        assert_eq!(
            text,
            "## Slide 1: Chat Computer
A computer you talk to

## Slide 2: What it does
- Answers questions
- Reads your files
  - Word, Excel and PowerPoint
- Sets timers
Notes: Say hello first.

## Slide 3: Numbers
| Year | Users |
| --- | --- |
| 2026 | 1 |",
            "{name}"
        );
    }
}

#[test]
fn reads_what_programs_write_less_often() {
    assert_eq!(
        text_of("edge.docx").1,
        "# Custom heading

## Localized heading

No tab here
Kept inserted
Page 7
Before In a box after
Not a list
- A list item

| Outer | a, b; c, d |
| --- | --- |

Visit a link é😀 &c
Two
lines-joined

Footnotes:
A footnote."
    );
    let sheet = text_of("edge.xlsx").1;
    let columns: Vec<std::string::String> = (0..27).map(column_name).collect();
    assert_eq!(columns[25..], ["Z".to_string(), "AA".to_string()]);
    let expected = [
        format!("row,{}", columns.join(",")),
        format!("1,A,2,Rich text{}", ",".repeat(24)),
        format!("2,1904-01-01,4 (=B1*2),#DIV/0!,東京{}", ",".repeat(23)),
        format!("3,1905-01-01,6,1.5,36:00,1904-02-01,12.3%,\"xy (=\"\"x\"\"&\"\"y\"\")\"{}", ",".repeat(20)),
        format!("1000{},far", ",".repeat(26)),
    ];
    assert_eq!(
        sheet,
        format!("## Data\n{}\n\n## Secret (hidden)\nrow,A\n1,hidden value\n\n## Chart\n(a chart, not cells)", expected.join("\n"))
    );
    assert_eq!(
        text_of("edge.pptx").1,
        "## Slide 1: Hidden slide (hidden)
In a group
[Picture: A cat on a sofa]

## Slide 2
- Line one line two
- <tag> & more
Notes: First / Second"
    );
}

#[test]
fn dates_in_both_systems() {
    // From 1900: day 1 is its first of January, 59 the last of February, 60 Lotus's 29 February, 61 the first of March;
    // from 1904: day 0 is its first of January.
    assert_eq!(excel::date(1, false), "1900-01-01");
    assert_eq!(excel::date(59, false), "1900-02-28");
    assert_eq!(excel::date(60, false), "1900-02-29");
    assert_eq!(excel::date(61, false), "1900-03-01");
    assert_eq!(excel::date(46_296, false), "2026-10-01");
    assert_eq!(excel::date(0, true), "1904-01-01");
    assert_eq!(excel::date(2_958_465, false), "9999-12-31");
}

#[test]
fn gives_errors_for_what_isnt_office() {
    assert_eq!(read(b"just some words").err(), Some(Error::NotOffice));
    let letter = file("letter.docx");
    assert!(read(&letter[..letter.len() / 2]).is_err());
    // Every byte of the end of the ZIP (its directory) changed in turn: an error or the words, never a crash.
    for i in (letter.len() - 2_000..letter.len()).step_by(7) {
        let mut d = letter.clone();
        d[i] ^= 0x41;
        let _ = read(&d);
    }
}

#[test]
fn damaged_xml_is_read_as_far_as_it_goes() {
    let parts = [("letter.docx", "word/document.xml"), ("budget-libreoffice.xlsx", "xl/worksheets/sheet1.xml"), ("talk.pptx", "ppt/slides/slide2.xml")];
    for (name, part) in parts {
        let data = file(name);
        let archive = zip::Archive::open(&data).unwrap();
        let xml = std::string::String::from_utf8(archive.read(archive.find(part).unwrap(), 1 << 24).unwrap()).unwrap();
        let styles = alloc::collections::BTreeMap::new();
        let read_it = |s: &str| {
            let mut out = Out::new();
            word::words(s, &styles, &mut out);
            excel::cells(s, &[], &[], false);
            slides::shapes(s);
            out.done()
        };
        // Cut short anywhere (on a character's edge), and with its markup's characters swapped about.
        for end in (0..xml.len()).step_by(13).filter(|&e| xml.is_char_boundary(e)) {
            read_it(&xml[..end]);
        }
        let bytes = xml.as_bytes();
        for (i, &b) in bytes.iter().enumerate().step_by(5) {
            if matches!(b, b'<' | b'>' | b'"' | b'/' | b'=' | b'&' | b';') {
                for with in [b'<', b'>', b'"', b' ', b'&'] {
                    let mut d = bytes.to_vec();
                    d[i] = with;
                    read_it(core::str::from_utf8(&d).unwrap());
                }
            }
        }
    }
}

#[test]
fn xml_as_office_writes_it() {
    use xml::{Reader, Token};
    let doc = r#"<?xml version="1.0"?><!-- a comment > with a bracket --><a:b x="1 > 0" y='it&apos;s'>t&lt;&#233;&#x1F600;&bogus;<c/><![CDATA[<raw> & all]]></a:b>"#;
    let tokens: Vec<Token> = Reader::new(doc).collect();
    let Token::Start(b) = tokens[0] else { panic!("{tokens:?}") };
    assert_eq!((b.name, b.local(), b.empty), ("a:b", "b", false));
    assert_eq!(b.attr("x").as_deref(), Some("1 > 0"));
    assert_eq!(b.attr("y").as_deref(), Some("it's"));
    assert_eq!(b.attr("z"), None);
    assert_eq!(tokens[1], Token::Text("t&lt;&#233;&#x1F600;&bogus;"));
    assert_eq!(xml::text("t&lt;&#233;&#x1F600;&bogus;"), "t<é😀&bogus;");
    assert_eq!(xml::text("&abc日本語 &; &#xD800; &#99999999;"), "&abc日本語 &; &#xD800; &#99999999;");
    assert!(matches!(tokens[2], Token::Start(c) if c.name == "c" && c.empty));
    assert_eq!(tokens[3], Token::Raw("<raw> & all"));
    assert_eq!(tokens[4], Token::End("a:b"));
    assert_eq!(tokens.len(), 5);
    // The relationships' prefix whatever it is; an attribute without one isn't it.
    let Some(Token::Start(s)) = Reader::new(r#"<sldId id="256" q:id="rId7"/>"#).next() else { panic!() };
    assert_eq!((s.attr_prefixed("id").as_deref(), s.attr("id").as_deref()), (Some("rId7"), Some("256")));
    // Paths from a part's folder.
    assert_eq!(resolve("ppt/slides", "../notesSlides/notesSlide1.xml"), "ppt/notesSlides/notesSlide1.xml");
    assert_eq!(resolve("xl", "/xl/worksheets/sheet2.xml"), "xl/worksheets/sheet2.xml");
    assert_eq!(resolve("", "word/document.xml"), "word/document.xml");
}
