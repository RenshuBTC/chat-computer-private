//! Against PDFs written by ReportLab, LibreOffice, pdfTeX, Chrome and qpdf (encrypted seven ways among them), and by
//! hand with what programs write less often (tests/make.py): their words, read back. Damaged ones (offsets wrong, the
//! cross-reference gone, cut short, bytes changed at random, objects that lead back to themselves) give errors or what
//! could be read, never a crash or a hang.

extern crate std;

use super::*;
use std::format;
use std::string::ToString;
use std::vec::Vec;

fn file(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn text_of(name: &str) -> Document {
    read(&file(name)).unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

const LETTER: &str = "## Page 1
A letter to the council
Why I’m writing
Dear Sir or Madam, the bins on Elm Street haven’t been emptied since 3 March. This paragraph is
long enough to be justified across several lines, so the spaces between its words are stretched by the
word spacing operator, which a reader has to follow to see where one word ends and the next
begins. Prices: 5 < 7 & 7 > 5; “quoted” and ‘single’; café, naïve, über and 20 €.

What I’d like
• Collection every week
• A bigger bin
• First, call me

Week Bins Emptied
1 2 no
2 2 yes

## Page 2
Page two
Yours faithfully,
Sam Lee";

#[test]
fn reads_what_reportlab_and_qpdf_write() {
    for name in ["letter-reportlab.pdf", "objects-qpdf.pdf"] {
        let d = text_of(name);
        assert_eq!((d.pages, d.title.as_deref(), d.cut), (2, Some("A letter to the council"), false), "{name}");
        assert_eq!(d.text, LETTER, "{name}");
    }
}

#[test]
fn reads_what_libreoffice_writes() {
    let d = text_of("letter-libreoffice.pdf");
    assert_eq!(d.pages, 2);
    assert_eq!(
        d.text,
        "## Page 1
A letter to the council
Why I'm writing
Dear Sir or Madam, the bins on Elm Street haven't been emptied since 3 March. Prices: 5 < 7 &
7 > 5; \"quoted\" and 'single'.
What I'd like
• Collection every week
• A bigger bin
1. First, call me
Name: Sam Lee
Street: Elm Street 12
Week Bins Emptied
1 2 no
2 2 yes
Ünïcödé, 名前, café and 🙂.

## Page 2
Yours faithfully,"
    );
}

#[test]
fn reads_what_pdftex_writes() {
    let d = text_of("paper-latex.pdf");
    assert_eq!(d.pages, 2);
    assert_eq!(
        d.text,
        "## Page 1
1 Introduction
The office’s first fifty files were difficult to find: “quoted” words, efficient flows—
and an affluent waffle.1 Mathematics: E = mc2 and a1 + b2 ≤ x.
In Helvetica: café, naïve and résumé, the finest “office”.

1A footnote, at the foot of the page.

1

## Page 2
2 Results
The second page’s words.
2"
    );
}

#[test]
fn reads_what_chrome_prints_and_right_to_left_words_in_reading_order() {
    let d = text_of("report-chrome.pdf");
    assert_eq!((d.pages, d.title.as_deref()), (1, Some("Quarterly report")));
    assert_eq!(
        d.text,
        "## Page 1
Quarterly report — Q3 2026
Revenue grew 12% to €4.2 million, while costs fell; the “efficient” offices reported fewer late
shipments.
Regions
Region Sales Change
North 1,200 +5%
South 980 −2%
Notes
Café openings in Zürich and Kraków
名前 and 東京 in the Japanese report
Arabic: مرحبا بالعالم
Hebrew: שלום עולם
السعر 120 دولار في القاهرة
שלום, זה 42 ש\"ח
مرحبا (عالم) جميل
The word (شكرا) means thanks.
אני אוהב Python מאוד

The first column has a long paragraph that keeps
going so that it wraps across several lines, filling
the column before the text continues in the
second column.

The second column's paragraph follows, and it
too has enough words to wrap a few times
across its width."
    );
}

const EDGE: &str = "## Page 1
KernedWords
Spaced by TJ
Wide text
Letter spaced
Next line by apostrophe
And by quote

In a form

After the picture, in the second stream
café, naïve, € and “quoted”

## Page 2
Habc e\u{301}d
名前fi

## Page 3
Abéfi Ab

## Page 4
";

#[test]
fn reads_what_programs_write_less_often() {
    let d = text_of("edge.pdf");
    assert_eq!((d.pages, d.title.as_deref()), (5, Some("Edge cases — ünïcode")));
    let lines: std::string::String =
        (1..=40).map(|i| format!("Line {i} of the filtered contents, through LZW and ASCII85.\n")).collect();
    let expected = format!(
        "{EDGE}{lines}\nThrough ASCIIHex.\n\nChanged by an update.\n\n## Page 5\n(No words on this page.)\n\n## Form\n\
         Your name: Sam Lee\nagree: Yes\naddress.street: Elm Street 12, Zürich"
    );
    assert_eq!(d.text, expected);
}

#[test]
fn opens_encrypted_files_without_a_password_but_not_with_one() {
    for kind in ["rc4-40", "rc4", "aes128", "aes128-metadata", "r5", "aes256", "objects"] {
        let name = format!("encrypted-{kind}.pdf");
        let d = text_of(&name);
        assert_eq!((d.pages, d.title.as_deref()), (2, Some("A letter to the council")), "{name}");
        assert_eq!(d.text, LETTER, "{name}");
    }
    assert_eq!(read(&file("password.pdf")).err(), Some(Error::Encrypted));
}

#[test]
fn a_file_cut_short_gives_what_it_has() {
    let d = text_of("cut.pdf");
    assert_eq!(d.pages, 1);
    let lines: Vec<&str> = d.text.lines().collect();
    assert!(lines.len() > 5 && lines.len() < 60, "{}", d.text);
    for (i, line) in lines[1..lines.len() - 1].iter().enumerate() {
        assert_eq!(*line, format!("Line {}, before the file was cut short.", i + 1));
    }
}

#[test]
fn a_page_of_only_a_picture_has_no_words() {
    let d = text_of("picture.pdf");
    assert_eq!((d.pages, d.text.as_str()), (1, ""));
}

#[test]
fn reads_damaged_files_as_far_as_they_go() {
    assert_eq!(read(b"PK\x03\x04 not a PDF").err(), Some(Error::NotPdf));
    assert_eq!(read(b"%PDF-1.7\nnothing else").err(), Some(Error::Damaged));
    let letter = file("letter-reportlab.pdf");
    // Every offset 7 bytes out (something added after the header, startxref's too): the objects found by looking
    // through the file.
    let mut moved = letter.clone();
    let at = moved.iter().position(|&b| b == b'\n').unwrap() + 1;
    moved.splice(at..at, *b"%extra\n");
    assert_eq!(read(&moved).unwrap().text, LETTER);
    // Each object's offset in the table wrong (but the table's own place right): each found where it is.
    let table = rfind(&letter, b"\nxref\n").unwrap() + 6;
    let mut wrong = letter.clone();
    let mut line = table;
    for _ in 0..2 {
        line += wrong[line..].iter().position(|&b| b == b'\n').unwrap() + 1;
    }
    while wrong[line].is_ascii_digit() {
        let offset: usize = std::str::from_utf8(&wrong[line..line + 10]).unwrap().parse().unwrap();
        wrong[line..line + 10].copy_from_slice(format!("{:010}", offset + 3).as_bytes());
        line += wrong[line..].iter().position(|&b| b == b'\n').unwrap() + 1;
    }
    assert_eq!(read(&wrong).unwrap().text, LETTER);
    // No cross-reference table or trailer at all: the objects and the catalog found by looking through the file.
    assert_eq!(read(&letter[..table - 5]).unwrap().text, LETTER);
    // The same for a file of object streams, its cross-reference stream gone.
    let packed = file("objects-qpdf.pdf");
    let xref = rfind(&packed, b"startxref").unwrap() + 10;
    let xref: usize = std::str::from_utf8(&packed[xref..]).unwrap().split_whitespace().next().unwrap().parse().unwrap();
    assert_eq!(read(&packed[..xref]).unwrap().text, LETTER);
    // Cut short anywhere: an error or the pages that could be read.
    for name in ["letter-reportlab.pdf", "objects-qpdf.pdf", "letter-libreoffice.pdf", "edge.pdf", "encrypted-aes256.pdf"] {
        let data = file(name);
        for k in 1..60 {
            let _ = read(&data[..data.len() * k / 60]);
        }
    }
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    (0..=hay.len() - needle.len()).rev().find(|&i| &hay[i..i + needle.len()] == needle)
}

/// (CHATCOMPUTER_PDF_TRIES: how many changed copies of each file, 150 if not said; CHATCOMPUTER_PDF_SEED: where the
/// random changes start.)
#[test]
fn changed_bytes_never_crash_it() {
    let tries: usize = std::env::var("CHATCOMPUTER_PDF_TRIES").ok().and_then(|t| t.parse().ok()).unwrap_or(150);
    let mut x: u64 = std::env::var("CHATCOMPUTER_PDF_SEED").ok().and_then(|t| t.parse().ok()).unwrap_or(0x9E37_79B9_7F4A_7C15);
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let names = [
        "letter-reportlab.pdf",
        "objects-qpdf.pdf",
        "letter-libreoffice.pdf",
        "paper-latex.pdf",
        "report-chrome.pdf",
        "edge.pdf",
        "encrypted-rc4.pdf",
        "encrypted-aes256.pdf",
        "encrypted-objects.pdf",
        "scan-jpeg.pdf",
        "scan-fax.pdf",
        "scan-kinds.pdf",
    ];
    for name in names {
        let data = file(name);
        let scan = name.starts_with("scan");
        for _ in 0..tries {
            let mut d = data.clone();
            for _ in 0..1 + next() % 8 {
                let at = (next() % d.len() as u64) as usize;
                const BYTES: &[u8] = b"()<>[]{}/%RnobjstreamTjTJ0123456789 \ncmDoBIIDEI";
                d[at] = match next() % 4 {
                    0 => BYTES[(next() % BYTES.len() as u64) as usize],
                    1 => d[at].wrapping_add(1),
                    _ => next() as u8,
                };
            }
            let _ = read(&d);
            // (And a scan's pages as pictures: at a small size, so the tries are quick.)
            if scan {
                for page in 0..3 {
                    let _ = page_picture(&d, page, &|w, h| (w.min(200), h.min(200)), 1 << 30);
                }
            }
        }
    }
}

/// A PDF of `objects` (numbered from 1), its catalog object 1, with a cross-reference table.
fn pdf(objects: &[&[u8]]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{at}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut s = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    s.extend_from_slice(data);
    s.extend_from_slice(b"\nendstream");
    s
}

#[test]
fn objects_that_lead_back_to_themselves_end() {
    let font = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";
    // A page tree whose page lists the tree as its child, a form that draws itself a hundred times, a stream whose
    // length is itself, arrays nested past any limit.
    let deep = format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000));
    let draws: std::string::String = (0..100).map(|_| "/Me Do ").collect();
    let data = pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 2 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /Kids [2 0 R] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> /XObject << /Me 6 0 R >> >> >>",
        &stream("", format!("BT /F1 12 Tf 72 700 Td (Still here) Tj ET {draws} {deep}").as_bytes()),
        font,
        &stream("/Type /XObject /Subtype /Form /Resources << /XObject << /Me 6 0 R >> /Font << /F1 5 0 R >> >>", format!("BT /F1 1 Tf (x) Tj ET {draws}").as_bytes()),
        b"<< /Length 7 0 R >>\nstream\nabc\nendstream",
    ]);
    let d = read(&data).unwrap();
    assert!(d.text.starts_with("## Page 1\nStill here"), "{}", &d.text[..d.text.len().min(200)]);
}

#[test]
fn many_streams_without_their_ends_read_quickly() {
    // (Each stream's end found once for all: looking for it from each stream to the file's end took minutes.)
    let mut data = b"%PDF-1.7\n".to_vec();
    for i in 1..=32_000 {
        data.extend_from_slice(format!("{i} 0 obj\n<< /X {i} >>\nstream\nxxxxxxxxxxxxxxxx\n").as_bytes());
    }
    data.extend_from_slice(b"%%EOF\n");
    let started = std::time::Instant::now();
    assert_eq!(read(&data).err(), Some(Error::Damaged));
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
    // A page whose contents are many such streams, each read to the next object.
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        Vec::new(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    let mut parts = std::string::String::new();
    for i in 0..5_000 {
        objects.push(format!("<< >>\nstream\nBT /F1 9 Tf 72 {} Td (Part {i}) Tj ET\n", 700 - i % 60 * 10).into_bytes());
        parts += &format!("{} 0 R ", objects.len());
    }
    objects[2] = format!("<< /Type /Page /Parent 2 0 R /Contents [{parts}] /Resources << /Font << /F1 4 0 R >> >> >>").into_bytes();
    let refs: Vec<&[u8]> = objects.iter().map(|o| o.as_slice()).collect();
    let started = std::time::Instant::now();
    let d = read(&pdf(&refs)).unwrap();
    assert!(d.text.starts_with("## Page 1\nPart 0\nPart 1\n") && d.text.contains("Part 4999"), "{}", &d.text[..200]);
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
}

#[test]
fn text_strings_and_md5() {
    assert_eq!(text_string(b"\xfe\xff\x00A\x00\xe9\xd8\x3d\xde\x42"), "Aé🙂");
    assert_eq!(text_string(b"\xef\xbb\xbfcaf\xc3\xa9"), "café");
    assert_eq!(text_string(b"\x8dquoted\x8e \x80 \xa0 caf\xe9 \x93"), "“quoted” • € café ﬁ");
    let hex = |b: [u8; 16]| b.iter().map(|x| format!("{x:02x}")).collect::<std::string::String>();
    assert_eq!(hex(crypt::md5_of(b"")), "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(hex(crypt::md5_of(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(hex(crypt::md5_of(&[b'a'; 1000])), "cabe45dcc9ae5b66ba86600cca6b8ba8");
    // RC4's own test (key "Key", "Plaintext").
    assert_eq!(crypt::rc4(b"Key", b"Plaintext"), [0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3]);
}

#[test]
fn many_pages_read_quickly() {
    // 2,000 pages of fifty lines each.
    let mut objects: Vec<Vec<u8>> = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), Vec::new(), b"<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>".to_vec()];
    let mut kids = std::string::String::new();
    for p in 0..2000 {
        let lines: std::string::String = (0..50).map(|l| format!("BT /F1 10 Tf 72 {} Td (Page {p}, line {l}: some words to read.) Tj ET\n", 750 - 7 * l)).collect();
        objects.push(stream("", lines.as_bytes()));
        let content = objects.len();
        objects.push(format!("<< /Type /Page /Parent 2 0 R /Contents {content} 0 R /Resources << /Font << /F1 3 0 R >> >> >>").into_bytes());
        kids += &format!("{} 0 R ", objects.len());
    }
    objects[1] = format!("<< /Type /Pages /Kids [{kids}] /Count 2000 >>").into_bytes();
    let refs: Vec<&[u8]> = objects.iter().map(|o| o.as_slice()).collect();
    let data = pdf(&refs);
    let started = std::time::Instant::now();
    let d = read(&data).unwrap();
    assert_eq!(d.pages, 2000);
    assert!(d.text.contains("## Page 2000\nPage 1999, line 0: some words to read.\nPage 1999, line 1:"));
    assert!(started.elapsed().as_secs() < 20, "{:?}", started.elapsed());
    assert!(!d.cut);
}

/// Each test file's words, written to CHATCOMPUTER_PDF_OUT (a folder) to compare with other readers'.
#[test]
#[ignore]
fn prints() {
    let out = std::env::var("CHATCOMPUTER_PDF_OUT").unwrap();
    for entry in std::fs::read_dir(format!("{}/tests", env!("CARGO_MANIFEST_DIR"))).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "pdf") {
            let name = path.file_name().unwrap().to_str().unwrap().to_string();
            let said = match read(&std::fs::read(&path).unwrap()) {
                Ok(d) => format!("pages: {}\ntitle: {:?}\ncut: {}\n{}", d.pages, d.title, d.cut, d.text),
                Err(e) => format!("error: {e:?}"),
            };
            std::fs::write(format!("{out}/{name}.txt"), said).unwrap();
        }
    }
}

/// A page's pictures as one, at the size `size` gives (and the page's own size it was asked about).
fn page_of(name: &str, page: usize, size: (usize, usize)) -> (PagePicture, (usize, usize)) {
    let asked = core::cell::Cell::new((0, 0));
    let p = page_picture(&file(name), page, &|w, h| {
        asked.set((w, h));
        size
    }, 1 << 30)
    .unwrap()
    .unwrap_or_else(|| panic!("{name}: page {page} has no pictures"));
    (p, asked.get())
}

fn png(name: &str) -> picture::Picture {
    picture::decode(&file(name), u32::MAX, u32::MAX, [255, 255, 255]).unwrap()
}

/// How far two pictures' pixels are apart: the most, and on average.
fn apart(a: &[u8], b: &[u8]) -> (u8, f64) {
    assert_eq!(a.len(), b.len());
    let most = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
    let mean = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as f64).sum::<f64>() / a.len() as f64;
    (most, mean)
}

#[test]
fn a_scanned_page_is_its_picture_as_pillow_makes_it() {
    // Filling the page; on a page turned a quarter; at half its size in a form, with a red stencil over it.
    for (n, size, native) in [(1, (150, 210), (750, 1050)), (2, (210, 150), (1050, 750)), (3, (150, 210), (1500, 2100))] {
        let (p, asked) = page_of("scan-jpeg.pdf", n - 1, size);
        assert_eq!(asked, native, "page {n}'s own size, at its picture's resolution");
        assert_eq!((p.width, p.height, p.drawn), (size.0, size.1, if n == 3 { 2 } else { 1 }));
        let want = png(&format!("scan-jpeg-{n}.png"));
        let (most, mean) = apart(&p.rgb, &want.rgb);
        // (Its JPEG decoded by the picture crate rather than libjpeg, and at a half of its size by the JPEG's own
        // scaling, averaged down to about twice the size it's drawn at, then resampled: within a level on average,
        // more at sharp edges.)
        assert!(most <= 64 && mean < 1.0, "page {n}: {most} at most, {mean:.3} on average");
    }
}

#[test]
fn a_fax_scan_is_its_pixels_exactly() {
    let want = png("scan-fax.png");
    for page in 0..3 {
        let (p, asked) = page_of("scan-fax.pdf", page, (864, 1100));
        assert_eq!(asked, (864, 1100));
        assert!(p.rgb == want.rgb, "page {}: {:?}", page + 1, apart(&p.rgb, &want.rgb));
    }
}

#[test]
fn each_kind_of_picture_is_drawn() {
    let (w, h) = (40usize, 30usize);
    let page = |n: usize| page_of("scan-kinds.pdf", n, (w, h)).0;
    let at = |p: &PagePicture, x: usize, y: usize| -> [u8; 3] { p.rgb[(y * w + x) * 3..][..3].try_into().unwrap() };
    let gray = |x: usize, y: usize| ((x * 7 + y * 13) % 256) as u8;
    // Gray, its Decode turned round.
    let p = page(0);
    for (x, y) in [(0, 0), (13, 7), (39, 29)] {
        assert_eq!(at(&p, x, y), [255 - gray(x, y); 3]);
    }
    // An indexed palette of 2-bit samples.
    let p = page(1);
    let palette = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [250, 200, 0]];
    for (x, y) in [(0, 0), (15, 3), (25, 12), (39, 29)] {
        assert_eq!(at(&p, x, y), palette[(x / 10 + y / 10) % 4], "{x}, {y}");
    }
    // CMYK.
    let p = page(2);
    let v = |c: usize, k: usize| ((1.0 - c as f64 / 255.0) * (1.0 - k as f64 / 255.0) * 255.0 + 0.5) as u8;
    for (x, y) in [(0, 0), (20, 10), (39, 29)] {
        let k = (x + y) % 64;
        assert_eq!(at(&p, x, y), [v((x * 6) % 256, k), v((y * 8) % 256, k), v(40, k)], "{x}, {y}");
    }
    // Inline images: raw RGB over the page, and a 2 by 2 gray one in ASCIIHex, 10 by 10 points at the bottom left.
    let p = page(3);
    assert_eq!(p.drawn, 2);
    for (x, y) in [(20, 0), (39, 29), (15, 5)] {
        assert_eq!(at(&p, x, y), [((x * 6) % 256) as u8, ((y * 8) % 256) as u8, 100], "{x}, {y}");
    }
    for ((x, y), v) in [((1, 21), 0x00u8), ((8, 21), 0xff), ((1, 28), 0x80), ((8, 28), 0xc0)] {
        assert!(at(&p, x, y)[0].abs_diff(v) <= 24, "{x}, {y}: {:?}", at(&p, x, y));
    }
    // A stencil filled blue over the left half of a gray picture.
    let p = page(4);
    assert_eq!((at(&p, 5, 5), at(&p, 19, 29)), ([0, 0, 255], [0, 0, 255]));
    assert_eq!(at(&p, 30, 10), [gray(30, 10); 3]);
    // A soft mask: opaque on the right, clear on the left, over white.
    let p = page(5);
    for (x, y) in [(0, 3), (20, 9), (39, 29)] {
        let a = (x * 255 / 39) as u32;
        let blend = |s: u32| ((s * a + 255 * (255 - a) + 127) / 255) as u8;
        assert_eq!(at(&p, x, y), [blend(((x * 6) % 256) as u32), blend(((y * 8) % 256) as u32), blend(100)], "{x}, {y}");
    }
    // Only words: nothing to see. And no such page.
    assert!(page_picture(&file("scan-kinds.pdf"), 6, &|w, h| (w, h), 1 << 30).unwrap().is_none());
    assert!(page_picture(&file("scan-kinds.pdf"), 7, &|w, h| (w, h), 1 << 30).unwrap().is_none());
    // A page with no words, where its words say so.
    let d = read(&file("scan-fax.pdf")).unwrap();
    assert!(d.text.is_empty() && d.pages == 3);
}

