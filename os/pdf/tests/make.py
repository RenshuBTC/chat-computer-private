#!/usr/bin/env python3
"""Makes the PDFs the pdf crate's tests read (tests/*.pdf): as programs write them, and some written by hand.

- letter-reportlab.pdf: ReportLab's (PDF 1.4, cross-reference table): two pages of justified paragraphs (word spacing),
  a table, a list, Latin-1's letters, curly quotes and the euro sign in the standard fonts (WinAnsi), a title.
- letter-libreoffice.pdf: the office crate's letter (office/tests/letter.docx) as LibreOffice exports it: TrueType
  subsets with ToUnicode CMaps, composite fonts for Chinese and an emoji, glyphs placed with TJ.
- report-chrome.pdf: a web page as Chrome prints it (Skia's PDF): composite TrueType fonts, a table, two columns,
  Chinese and Japanese, and Arabic and Hebrew (right to left, drawn as they're seen: Arabic's letters as their
  presentation forms) among English, with numbers, brackets and an English word in them.
- paper-latex.pdf: pdfTeX's: Computer Modern Type 1 fonts with only their own built-in encoding (OT1: ligatures at
  codes below 32, no ToUnicode), a paragraph in Helvetica with LaTeX's T1 encoding (/Differences), maths, a
  footnote, two pages.
- objects-qpdf.pdf: the ReportLab letter rewritten by qpdf with object streams and a cross-reference stream (PNG
  predictor).
- encrypted-*.pdf: the letter encrypted by qpdf with no password to open it (only an owner's): RC4 40-bit (revision
  2) and 128-bit (3), AES-128 (4; and again with its metadata left unencrypted, which changes the key), AES-256
  (Adobe's revision 5, and 6), and AES-256 with object streams (whose objects' strings aren't encrypted again).
- password.pdf: the letter with a password to open it ("secret").
- edge.pdf, written here by hand: a page tree of two levels with resources inherited; TJ's spacing, Tz, Tc, Tw, ',
  " and T*; a form drawn with Do; an inline picture whose data has "EI" in it; contents in two streams; a composite
  font (Identity-H) with a ToUnicode CMap of bfchar, bfrange and bfrange's arrays (Chinese, a ligature, a combining
  accent); a Type 3 font with /Differences' glyph names; content streams through LZW (early change, codes of 9 to
  11 bits), ASCII85, ASCIIHex and RunLength; a page that's only a picture; filled-in form fields (text, a check box, a
  field under another); a title in UTF-16; and an update appended to the file that changes a page.
- picture.pdf: one page, only a picture (as a scanner's would be).
- cut.pdf: a file cut short half way through its page's (compressed) contents, as a download that stopped would be.
- scan-jpeg.pdf: scanned pages (written here, the scan drawn by Pillow): a colour JPEG filling the page; the same
  on a page turned a quarter (/Rotate 90); and at half its size in a form drawn with Do, with a red stencil mask
  over it. scan-jpeg-1.png, -2.png and -3.png: those pages as Pillow makes them at a fifth of their size (Pillow's
  JPEG decoding and bicubic resampling), for the page pictures to be compared with.
- scan-fax.pdf: a black-and-white scan in fax's compression as libtiff writes it: Group 4, then Group 3's
  one-dimensional lines with their end-of-line codes, then Group 4 with BlackIs1; scan-fax.png: its pixels, which
  they must give exactly.
- scan-kinds.pdf: small pictures of each kind of sample: gray with its Decode turned round, an indexed palette of
  2-bit samples, CMYK, inline images (raw, and in ASCIIHex), a stencil mask filled red over a picture, a picture
  with a soft mask, and a page with only words.

    python3 tests/make.py      (needs ReportLab, LibreOffice's soffice, pdflatex, qpdf, and Playwright's Chromium)
    python3 tests/make.py scans   (only the scanned pages': needs Pillow, with libtiff)
"""
from __future__ import annotations

import base64
import shutil
import subprocess
import tempfile
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent


def reportlab_letter(path: Path) -> None:
    from reportlab.lib.enums import TA_JUSTIFY
    from reportlab.lib.pagesizes import A4
    from reportlab.lib.styles import getSampleStyleSheet, ParagraphStyle
    from reportlab.platypus import ListFlowable, ListItem, PageBreak, Paragraph, SimpleDocTemplate, Spacer, Table

    styles = getSampleStyleSheet()
    body = ParagraphStyle("body", parent=styles["BodyText"], alignment=TA_JUSTIFY, fontName="Times-Roman", fontSize=11)
    doc = SimpleDocTemplate(str(path), pagesize=A4, title="A letter to the council", author="Sam Lee", invariant=1)
    story = [
        Paragraph("A letter to the council", styles["Title"]),
        Paragraph("Why I’m writing", styles["Heading1"]),
        Paragraph(
            "Dear Sir or Madam, the bins on Elm Street haven’t been emptied since 3 March. "
            "This paragraph is long enough to be justified across several lines, so the spaces between its words are "
            "stretched by the word spacing operator, which a reader has to follow to see where one word ends and the "
            "next begins. Prices: 5 < 7 & 7 > 5; “quoted” and ‘single’; café, naïve, "
            "über and 20 €.",
            body,
        ),
        Spacer(1, 12),
        Paragraph("What I’d like", styles["Heading2"]),
        ListFlowable(
            [ListItem(Paragraph(t, body)) for t in ["Collection every week", "A bigger bin", "First, call me"]],
            bulletType="bullet",
        ),
        Spacer(1, 12),
        Table([["Week", "Bins", "Emptied"], ["1", "2", "no"], ["2", "2", "yes"]]),
        PageBreak(),
        Paragraph("Page two", styles["Heading1"]),
        Paragraph("Yours faithfully,", body),
        Paragraph("Sam Lee", body),
    ]
    doc.build(story)


def libreoffice_letter(path: Path, scratch: Path) -> None:
    source = HERE.parent.parent / "office" / "tests" / "letter.docx"
    profile = scratch / "profile"
    subprocess.run(
        [
            "soffice",
            f"-env:UserInstallation=file://{profile}",
            "--headless",
            "--convert-to",
            "pdf",
            "--outdir",
            str(scratch),
            str(source),
        ],
        check=True,
        capture_output=True,
    )
    shutil.move(scratch / "letter.pdf", path)
    shutil.rmtree(profile, ignore_errors=True)


PAPER = r"""\documentclass{article}
\usepackage[T1,OT1]{fontenc}
\begin{document}
\section{Introduction}
The office's first fifty files were difficult to find: ``quoted'' words, efficient flows---and an affluent
waffle.\footnote{A footnote, at the foot of the page.} Mathematics: $E = mc^2$ and $a_1 + b^2 \le x$.

{\fontencoding{T1}\fontfamily{phv}\selectfont In Helvetica: caf\'e, na\"ive and r\'esum\'e, the finest ``office''.\par}
\newpage
\section{Results}
The second page's words.
\end{document}
"""


REPORT = """<!doctype html><html><head><meta charset="utf-8"><title>Quarterly report</title>
<style>body{font-family:serif;margin:2cm} .cols{column-count:2} table{border-collapse:collapse} td,th{border:1px solid #888;padding:4px}</style></head>
<body><h1>Quarterly report — Q3 2026</h1>
<p>Revenue grew <b>12%</b> to €4.2 million, while costs fell; the “efficient” offices reported fewer late shipments.</p>
<h2>Regions</h2>
<table><tr><th>Region</th><th>Sales</th><th>Change</th></tr><tr><td>North</td><td>1,200</td><td>+5%</td></tr><tr><td>South</td><td>980</td><td>−2%</td></tr></table>
<h2>Notes</h2>
<ul><li>Café openings in Zürich and Kraków</li><li>名前 and 東京 in the Japanese report</li><li>Arabic: مرحبا بالعالم</li><li>Hebrew: שלום עולם</li></ul><p dir="rtl">السعر 120 دولار في القاهرة</p><p dir="rtl">שלום, זה 42 ש"ח</p><p dir="rtl">مرحبا (عالم) جميل</p><p>The word (شكرا) means thanks.</p><p dir="rtl">אני אוהב Python מאוד</p><ul></ul>
<div class="cols"><p>The first column has a long paragraph that keeps going so that it wraps across several lines, filling the column before the text continues in the second column.</p><p>The second column's paragraph follows, and it too has enough words to wrap a few times across its width.</p></div>
</body></html>
"""

PRINT = """const { chromium } = require('playwright');
(async () => {
  const b = await chromium.launch();
  const p = await b.newPage();
  await p.goto('file://' + process.argv[2]);
  await p.pdf({ path: process.argv[3], format: 'A4' });
  await b.close();
})();
"""


def chrome_report(path: Path, scratch: Path) -> None:
    (scratch / "report.html").write_text(REPORT, encoding="utf-8")
    (scratch / "print.js").write_text(PRINT)
    subprocess.run(["node", str(scratch / "print.js"), str(scratch / "report.html"), str(path)], check=True)


def latex_paper(path: Path, scratch: Path) -> None:
    (scratch / "paper.tex").write_text(PAPER)
    for _ in range(2):
        subprocess.run(["pdflatex", "-interaction=nonstopmode", "paper.tex"], cwd=scratch, check=True, capture_output=True)
    shutil.move(scratch / "paper.pdf", path)


def qpdf(args: list[str]) -> None:
    subprocess.run(["qpdf", *args], check=True)


# A PDF by hand.


class Pdf:
    def __init__(self) -> None:
        self.objects: dict[int, bytes] = {}

    def obj(self, n: int, body: bytes | str) -> None:
        self.objects[n] = body.encode("latin-1") if isinstance(body, str) else body

    def stream(self, n: int, entries: str, data: bytes) -> None:
        self.obj(n, b"<< " + entries.encode() + b" /Length %d >>\nstream\n" % len(data) + data + b"\nendstream")

    def write(self, root: int, info: int | None = None) -> bytes:
        out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n"
        offsets = {}
        for n in sorted(self.objects):
            offsets[n] = len(out)
            out += b"%d 0 obj\n" % n + self.objects[n] + b"\nendobj\n"
        return out + xref(offsets, len(out), root, info)


def xref(offsets: dict[int, int], at: int, root: int, info: int | None, prev: int | None = None, size: int | None = None) -> bytes:
    """A cross-reference table at `at` for `offsets` (in sections of consecutive numbers), and the trailer."""
    size = size or max(offsets) + 1
    out = b"xref\n"
    numbers = sorted(offsets)
    if prev is None:
        out += b"0 1\n0000000000 65535 f \n"
    run: list[int] = []
    for n in numbers + [None]:
        if run and (n is None or n != run[-1] + 1):
            out += b"%d %d\n" % (run[0], len(run))
            for m in run:
                out += b"%010d 00000 n \n" % offsets[m]
            run = []
        if n is not None:
            run.append(n)
    trailer = b"/Size %d /Root %d 0 R" % (size, root)
    if info:
        trailer += b" /Info %d 0 R" % info
    if prev is not None:
        trailer += b" /Prev %d" % prev
    return out + b"trailer\n<< " + trailer + b" >>\nstartxref\n%d\n%%%%EOF\n" % at


def lzw(data: bytes) -> bytes:
    """LZW as PDF has it, early change: a clear code first, codes 9 to 12 bits wide, 257 at the end."""
    table = {bytes([i]): i for i in range(256)}
    next_code, width = 258, 9
    codes: list[tuple[int, int]] = [(256, 9)]

    def width_for(n: int) -> int:
        return 9 if n < 512 else 10 if n < 1024 else 11 if n < 2048 else 12

    w = b""
    for c in data:
        wc = w + bytes([c])
        if wc in table:
            w = wc
            continue
        codes.append((table[w], width))
        table[wc] = next_code
        next_code += 1
        width = width_for(next_code)
        w = bytes([c])
    if w:
        codes.append((table[w], width))
        # (The reader adds an entry for that last code before it reads the end's.)
        width = width_for(next_code + 1)
    codes.append((257, width))
    acc, bits, out = 0, 0, bytearray()
    for code, n in codes:
        acc = acc << n | code
        bits += n
        while bits >= 8:
            bits -= 8
            out.append(acc >> bits & 0xFF)
    if bits:
        out.append(acc << (8 - bits) & 0xFF)
    return bytes(out)


def run_length(data: bytes) -> bytes:
    out = bytearray()
    for i in range(0, len(data), 100):
        chunk = data[i : i + 100]
        out += bytes([len(chunk) - 1]) + chunk
    out += bytes([256 - 3 + 1, ord("\n")])  # (three line breaks: a repeated run)
    return bytes(out + b"\x80")


def utf16(s: str) -> str:
    return "<FEFF" + s.encode("utf-16-be").hex().upper() + ">"


TOUNICODE = b"""/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def
/CMapName /Test-UCS def
/CMapType 2 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
3 beginbfchar
<0001> <0048>
<0002> <00650301>
<0003> <0020>
endbfchar
2 beginbfrange
<0010> <0015> <0061>
<0020> <0022> [<540D> <524D> <FB01>]
endbfrange
endcmap
CMapName currentdict /CMap defineresource pop
end
end
"""


def edge(path: Path) -> None:
    p = Pdf()
    p.obj(1, "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [40 0 R 41 0 R 42 0 R 44 0 R] >> >>")
    p.obj(2, "<< /Type /Pages /Kids [3 0 R 20 0 R] /Count 5 >>")
    # Page 1: Helvetica (WinAnsi), spacing operators, a form, an inline picture, contents in two streams.
    p.obj(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents [4 0 R 5 0 R] "
             "/Resources << /Font << /F1 6 0 R >> /XObject << /Fm1 7 0 R >> >> >>")
    p.stream(4, "", b"BT /F1 12 Tf 72 720 Td (Kerned) Tj [(W) 120 (ord) -30 (s)] TJ ET\n"
                    b"BT /F1 12 Tf 72 700 Td [(Spaced) -600 (by) -600 (TJ)] TJ ET\n"
                    b"BT /F1 12 Tf 14 TL 72 680 Td 150 Tz (Wide text) Tj T* 100 Tz 3 Tc (Letter spaced) Tj 0 Tc\n"
                    b"(Next line by apostrophe) ' 5 0 (And by quote) \" ET\n"
                    b"q 1 0 0 1 72 600 cm /Fm1 Do Q\n"
                    b"BI /W 5 /H 1 /BPC 8 /CS /G ID \x01EI\x02\x03 EI\n"
                    b"BT /F1 12 Tf 72 560 Td (After the picture,) Tj")
    p.stream(5, "", b"( in the second stream) Tj 0 -14 Td (caf\351, na\357ve, \200 and \223quoted\224) Tj ET")
    p.obj(6, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    p.stream(7, "/Type /XObject /Subtype /Form /BBox [0 0 300 50] /Matrix [1 0 0 1 0 10] "
                "/Resources << /Font << /F9 6 0 R >> >>",
             b"BT /F9 12 Tf 0 0 Td (In a form) Tj ET")
    # Pages 2 to 5, under a second level of the tree, with their resources from it.
    p.obj(20, "<< /Type /Pages /Parent 2 0 R /Kids [21 0 R 22 0 R 23 0 R 26 0 R] /Count 4 "
              "/Resources << /Font << /F1 6 0 R /F2 30 0 R /F3 35 0 R >> /XObject << /Im1 27 0 R >> >> >>")
    # Page 2: a composite font.
    p.obj(21, "<< /Type /Page /Parent 20 0 R /MediaBox [0 0 612 792] /Contents 24 0 R >>")
    p.stream(24, "", b"BT /F2 12 Tf 72 720 Td <0001001000110012> Tj <0003> Tj <00020013> Tj ET\n"
                     b"BT /F2 12 Tf 72 700 Td <002000210022> Tj ET")
    p.obj(30, "<< /Type /Font /Subtype /Type0 /BaseFont /Test /Encoding /Identity-H /DescendantFonts [31 0 R] "
              "/ToUnicode 32 0 R >>")
    p.obj(31, "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Test "
              "/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 1000 "
              "/W [1 [700 500 250] 16 21 500 32 34 1000] /CIDToGIDMap /Identity >>")
    p.stream(32, "", TOUNICODE)
    # Page 3: a Type 3 font.
    p.obj(22, "<< /Type /Page /Parent 20 0 R /MediaBox [0 0 612 792] /Contents 25 0 R >>")
    p.stream(25, "", b"BT /F3 12 Tf 72 720 Td (ABCD) Tj (E) Tj (AB) Tj ET")
    p.obj(35, "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.01 0 0 0.01 0 0] "
              "/CharProcs << /A 36 0 R /b 36 0 R /uni00E9 36 0 R /f_i 36 0 R /space 36 0 R >> "
              "/Encoding << /Type /Encoding /Differences [65 /A /b /uni00E9 /f_i /space] >> "
              "/FirstChar 65 /LastChar 69 /Widths [60 50 50 60 25] /Resources << >> >>")
    p.stream(36, "", b"60 0 d0")
    # Page 4: contents through filters (an update at the end changes them: see below).
    p.obj(23, "<< /Type /Page /Parent 20 0 R /MediaBox [0 0 612 792] /Contents [50 0 R 51 0 R 52 0 R] >>")
    long = b"".join(b"BT /F1 9 Tf 72 %d Td (Line %d of the filtered contents, through LZW and ASCII85.) Tj ET\n" % (700 - 11 * i, i + 1) for i in range(40))
    p.stream(50, "/Filter [/ASCII85Decode /LZWDecode]", base64.a85encode(lzw(long), adobe=True)[2:])
    p.stream(51, "/Filter /ASCIIHexDecode", b"BT /F1 9 Tf 72 200 Td (Through ASCIIHex.) Tj ET\n".hex().encode() + b">")
    p.stream(52, "/Filter /RunLengthDecode", run_length(b"BT /F1 9 Tf 72 180 Td (Through RunLength.) Tj ET"))
    # Page 5: only a picture.
    p.obj(26, "<< /Type /Page /Parent 20 0 R /MediaBox [0 0 612 792] /Contents 28 0 R >>")
    p.stream(28, "", b"q 200 0 0 100 72 600 cm /Im1 Do Q")
    p.stream(27, "/Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8", b"\x00\xff")
    # A form's fields.
    p.obj(40, "<< /FT /Tx /T (name) /TU (Your name) /V (Sam Lee) >>")
    p.obj(41, "<< /FT /Btn /T (agree) /V /Yes >>")
    p.obj(42, "<< /T (address) /Kids [43 0 R] >>")
    p.obj(43, "<< /FT /Tx /Parent 42 0 R /T (street) /V " + utf16("Elm Street 12, Zürich") + " >>")
    p.obj(44, "<< /FT /Btn /T (newsletter) /V /Off >>")
    p.obj(60, "<< /Title " + utf16("Edge cases — ünïcode") + " /Producer (make.py) >>")
    first = p.write(1, 60)
    # The update: page 4's last stream replaced.
    at = len(first)
    data = b"BT /F1 9 Tf 72 180 Td (Changed by an update.) Tj ET"
    new = b"52 0 obj\n<< /Length %d >>\nstream\n" % len(data) + data + b"\nendstream\nendobj\n"
    prev = int(first.rsplit(b"startxref\n", 1)[1].split(b"\n")[0])
    update = new
    path.write_bytes(first + update + xref({52: at}, at + len(update), 1, 60, prev=prev, size=max(p.objects) + 1))


def picture(path: Path) -> None:
    p = Pdf()
    p.obj(1, "<< /Type /Catalog /Pages 2 0 R >>")
    p.obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    p.obj(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /XObject << /Im1 5 0 R >> >> >>")
    p.stream(4, "/Filter /FlateDecode", zlib.compress(b"q 612 0 0 792 0 0 cm /Im1 Do Q"))
    pixels = bytes(range(256)) * 4
    p.stream(5, "/Type /XObject /Subtype /Image /Width 32 /Height 32 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode", zlib.compress(pixels))
    path.write_bytes(p.write(1))


def cut(path: Path) -> None:
    p = Pdf()
    p.obj(1, "<< /Type /Catalog /Pages 2 0 R >>")
    p.obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    p.obj(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R /Resources << /Font << /F1 4 0 R >> >> >>")
    p.obj(4, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    lines = b"".join(b"BT /F1 10 Tf 72 %d Td (Line %d, before the file was cut short.) Tj ET\n" % (750 - 12 * i, i + 1) for i in range(60))
    packed = zlib.compress(lines, 9)
    p.stream(5, "/Filter /FlateDecode", packed)
    data = p.write(1)
    at = data.index(b"stream\n", data.index(b"5 0 obj")) + 7
    path.write_bytes(data[: at + len(packed) // 2])


def scan_page_image():
    """A scanned page, as Pillow draws one: words, a box, a photo-like gradient, 750 by 1050 pixels."""
    from PIL import Image, ImageDraw, ImageFont
    im = Image.new("RGB", (750, 1050), (248, 247, 242))
    d = ImageDraw.Draw(im)
    font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf", 26)
    big = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf", 40)
    d.text((60, 60), "Tenancy agreement", font=big, fill=(20, 20, 30))
    for i, line in enumerate(["This agreement is made on 2 May 2026", "between Ana Ruiz (the landlord)",
                              "and Tom Byrne (the tenant), for the flat", "at 4 Quay Street, Galway.",
                              "Rent: EUR 1,250 a month, due on the 1st."]):
        d.text((60, 160 + 52 * i, ), line, font=font, fill=(30, 30, 30))
    for x in range(300):
        for y in range(200):
            im.putpixel((400 + x, 600 + y), (x * 255 // 299, y * 255 // 199, 128))
    d.rectangle((60, 600, 340, 800), outline=(200, 30, 30), width=6)
    d.ellipse((120, 860, 300, 1000), fill=(40, 90, 200))
    return im


def scan_expected(jpeg: bytes, size, rotate: int, half: bool, stencil) -> "Image":
    """A scan page as one picture, at `size`, as Pillow makes it (its JPEG decoder, its bicubic resampling)."""
    import io
    from PIL import Image
    w, h = size
    im = Image.open(io.BytesIO(jpeg)).convert("RGB")
    page = Image.new("RGB", (w, h), (255, 255, 255))
    if half:
        # (Drawn at 300 by 420 points from 152, 212, on a page 600 by 840: a quarter of its area, upright; its
        # edges on whole pixels at a fifth of the page's size.)
        sx, sy = w / 600, h / 840
        x0, x1 = round(152 * sx), round(452 * sx)
        y0, y1 = round((840 - 632) * sy), round((840 - 212) * sy)
        page.paste(im.resize((x1 - x0, y1 - y0), Image.BICUBIC, reducing_gap=None), (x0, y0))
        mask, (mw, mh), (mx, my) = stencil
        sm = Image.frombytes("1", (mw, mh), mask).convert("L")
        # (Painted where the stencil's 0: from 100, 100 points, 200 by 100.)
        X0, X1 = round(100 * sx), round(300 * sx)
        Y0, Y1 = round((840 - 200) * sy), round((840 - 100) * sy)
        alpha = Image.eval(sm, lambda v: 255 - v).resize((X1 - X0, Y1 - Y0), Image.BICUBIC, reducing_gap=None)
        page.paste(Image.new("RGB", (X1 - X0, Y1 - Y0), (255, 0, 0)), (X0, Y0), alpha)
        return page
    if rotate:
        im = im.transpose(Image.ROTATE_270)
    return im.resize((w, h), Image.BICUBIC, reducing_gap=None)


def scan_jpeg(path: Path) -> None:
    import io
    im = scan_page_image()
    buf = io.BytesIO()
    im.save(buf, "JPEG", quality=85, subsampling=2)
    jpeg = buf.getvalue()
    # A stencil: 1 where it's left alone, 0 where it's painted (a chequer of 10-pixel squares), 80 by 40.
    mw, mh = 80, 40
    bits = bytearray()
    for y in range(mh):
        row = 0
        for x in range(mw):
            row = (row << 1) | (0 if ((x // 10) + (y // 10)) % 2 == 0 else 1)
        bits += row.to_bytes(mw // 8, "big")
    p = Pdf()
    p.obj(1, "<< /Type /Catalog /Pages 2 0 R >>")
    p.obj(2, "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 600 840] >>")
    res = "/Resources << /XObject << /Scan 10 0 R /Half 11 0 R >> >>"
    p.obj(3, f"<< /Type /Page /Parent 2 0 R /Contents 6 0 R {res} >>")
    p.obj(4, f"<< /Type /Page /Parent 2 0 R /Rotate 90 /Contents 6 0 R {res} >>")
    p.obj(5, f"<< /Type /Page /Parent 2 0 R /Contents 7 0 R {res} >>")
    p.stream(6, "", b"q 600 0 0 840 0 0 cm /Scan Do Q")
    p.stream(7, "", b"q 1 0 0 1 152 212 cm /Half Do Q q 1 0 0 rg 200 0 0 100 100 100 cm BI /W 80 /H 40 /IM true /BPC 1 ID\n" + bytes(bits) + b"\nEI Q")
    p.stream(10, f"/Type /XObject /Subtype /Image /Width 750 /Height 1050 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode", jpeg)
    p.stream(11, "/Type /XObject /Subtype /Form /BBox [0 0 300 420] /Matrix [0.5 0 0 0.5 0 0] /Resources << /XObject << /Scan 10 0 R >> >>",
             b"q 600 0 0 840 0 0 cm /Scan Do Q")
    path.write_bytes(p.write(1))
    for n, (rotate, half) in enumerate([(False, False), (True, False), (False, True)], 1):
        size = (210, 150) if rotate else (150, 210)
        scan_expected(jpeg, size, rotate, half, (bytes(bits), (mw, mh), (100, 100))).save(HERE / f"scan-jpeg-{n}.png", optimize=True)


def scan_fax(path: Path) -> None:
    """A black-and-white scan, 864 by 1100 pixels on a page of 432 by 550 points, in Group 4 and Group 3."""
    import io
    from PIL import Image, ImageDraw, ImageFont
    im = Image.new("1", (864, 1100), 255)
    d = ImageDraw.Draw(im)
    font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 30)
    for i in range(14):
        d.text((40, 40 + 70 * i), f"Line {i + 1}: the quick brown fox jumps over {i * 7} lazy dogs.", font=font, fill=0)
    d.rectangle((600, 900, 820, 1060), fill=0)
    d.line((0, 1099, 863, 0), fill=0, width=3)
    im.save(HERE / "scan-fax.png", optimize=True)

    def strip(compression: str, picture) -> bytes:
        buf = io.BytesIO()
        picture.save(buf, "TIFF", compression=compression, tiffinfo={278: 1100})
        t = Image.open(io.BytesIO(buf.getvalue()))
        # (Black is zero, as Pillow writes it, and libtiff codes zeros as fax's white runs.)
        assert t.tag_v2.get(262) == 1
        (offset,), (count,) = t.tag_v2[273], t.tag_v2[279]
        return buf.getvalue()[offset : offset + count]

    # (So the scan's white is coded as fax's white: its colours turned round first. And as it is, for BlackIs1.)
    turned = Image.eval(im.convert("L"), lambda v: 255 - v).convert("1", dither=Image.Dither.NONE)
    g4, g3, g4_black_is_1 = strip("group4", turned), strip("group3", turned), strip("group4", im)
    p = Pdf()
    p.obj(1, "<< /Type /Catalog /Pages 2 0 R >>")
    p.obj(2, "<< /Type /Pages /Kids [3 0 R 4 0 R 8 0 R] /Count 3 /MediaBox [0 0 432 550] >>")
    p.obj(3, "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /XObject << /Im 6 0 R >> >> >>")
    p.obj(4, "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /XObject << /Im 7 0 R >> >> >>")
    p.obj(8, "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /XObject << /Im 9 0 R >> >> >>")
    p.stream(5, "", b"q 432 0 0 550 0 0 cm /Im Do Q")
    common = "/Type /XObject /Subtype /Image /Width 864 /Height 1100 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode"
    p.stream(6, common + " /DecodeParms << /K -1 /Columns 864 /Rows 1100 >>", g4)
    p.stream(7, common + " /DecodeParms << /K 0 /Columns 864 /Rows 1100 /EndOfLine true >>", g3)
    p.stream(9, common + " /DecodeParms << /K -1 /Columns 864 /Rows 1100 /BlackIs1 true >>", g4_black_is_1)
    path.write_bytes(p.write(1))


def scan_kinds(path: Path) -> None:
    """Small pictures of each kind of sample, on pages their own size (a pixel a point), so their pixels are exact."""
    w, h = 40, 30
    gray = bytes((x * 7 + y * 13) % 256 for y in range(h) for x in range(w))
    # 2-bit indexes into a palette of four colours.
    idx = bytearray()
    for y in range(h):
        row = 0
        for x in range(w):
            row = (row << 2) | ((x // 10 + y // 10) % 4)
        idx += row.to_bytes(w // 4, "big")
    palette = bytes([255, 0, 0, 0, 255, 0, 0, 0, 255, 250, 200, 0])
    cmyk = bytes(v for y in range(h) for x in range(w) for v in ((x * 6) % 256, (y * 8) % 256, 40, (x + y) % 64))
    rgb = bytes(v for y in range(h) for x in range(w) for v in ((x * 6) % 256, (y * 8) % 256, 100))
    smask = bytes(((x * 255) // (w - 1)) for y in range(h) for x in range(w))
    stencil = bytearray()
    for y in range(h):
        row = 0
        for x in range(w):
            row = (row << 1) | (0 if x < 20 else 1)
        stencil += row.to_bytes(w // 8, "big")
    p = Pdf()
    kids = " ".join(f"{n} 0 R" for n in range(3, 10))
    p.obj(1, "<< /Type /Catalog /Pages 2 0 R >>")
    p.obj(2, f"<< /Type /Pages /Kids [{kids}] /Count 7 /MediaBox [0 0 {w} {h}] >>")
    full = f"q {w} 0 0 {h} 0 0 cm /Im Do Q".encode()
    img = f"/Type /XObject /Subtype /Image /Width {w} /Height {h}"
    pages = [
        (f"{img} /ColorSpace /DeviceGray /BitsPerComponent 8 /Decode [1 0] /Filter /FlateDecode", zlib.compress(gray), full),
        (f"{img} /ColorSpace [/Indexed /DeviceRGB 3 <{palette.hex()}>] /BitsPerComponent 2", bytes(idx), full),
        (f"{img} /ColorSpace /DeviceCMYK /BitsPerComponent 8 /Filter /FlateDecode", zlib.compress(cmyk), full),
        (None, None, f"q {w} 0 0 {h} 0 0 cm BI /W {w} /H {h} /CS /RGB /BPC 8 ID\n".encode() + rgb + b"\nEI Q q 10 0 0 10 0 0 cm BI /W 2 /H 2 /CS /G /BPC 8 /F /AHx ID\n00ff80c0>\nEI Q"),
        (f"{img} /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode", zlib.compress(gray),
         full + f" q 0 0 1 rg {w} 0 0 {h} 0 0 cm /Mask Do Q".encode()),
        (f"{img} /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask 30 0 R", rgb, full),
        (None, None, b"BT /F1 12 Tf 2 10 Td (Words) Tj ET"),
    ]
    for i, (entries, data, contents) in enumerate(pages):
        n = 3 + i
        p.obj(n, f"<< /Type /Page /Parent 2 0 R /Contents {n + 10} 0 R /Resources << /XObject << /Im {n + 20} 0 R /Mask 31 0 R >> /Font << /F1 32 0 R >> >> >>")
        p.stream(n + 10, "", contents)
        if entries:
            p.stream(n + 20, entries, data)
    p.stream(30, f"/Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceGray /BitsPerComponent 8", smask)
    p.stream(31, f"/Type /XObject /Subtype /Image /Width {w} /Height {h} /ImageMask true", bytes(stencil))
    p.obj(32, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    path.write_bytes(p.write(1))


def scans() -> None:
    scan_jpeg(HERE / "scan-jpeg.pdf")
    scan_fax(HERE / "scan-fax.pdf")
    scan_kinds(HERE / "scan-kinds.pdf")


def main() -> None:
    import sys
    if sys.argv[1:] == ["scans"]:
        scans()
        return
    with tempfile.TemporaryDirectory() as t:
        scratch = Path(t)
        reportlab_letter(HERE / "letter-reportlab.pdf")
        libreoffice_letter(HERE / "letter-libreoffice.pdf", scratch)
        latex_paper(HERE / "paper-latex.pdf", scratch)
        chrome_report(HERE / "report-chrome.pdf", scratch)
    letter = str(HERE / "letter-reportlab.pdf")
    qpdf(["--object-streams=generate", "--compress-streams=y", letter, str(HERE / "objects-qpdf.pdf")])
    qpdf(["--allow-weak-crypto", "--encrypt", "", "owner", "40", "--", letter, str(HERE / "encrypted-rc4-40.pdf")])
    qpdf(["--allow-weak-crypto", "--encrypt", "", "owner", "128", "--use-aes=n", "--", letter, str(HERE / "encrypted-rc4.pdf")])
    qpdf(["--encrypt", "", "owner", "128", "--use-aes=y", "--", letter, str(HERE / "encrypted-aes128.pdf")])
    qpdf(["--encrypt", "", "owner", "128", "--use-aes=y", "--cleartext-metadata", "--", letter, str(HERE / "encrypted-aes128-metadata.pdf")])
    qpdf(["--allow-weak-crypto", "--encrypt", "", "owner", "256", "--force-R5", "--", letter, str(HERE / "encrypted-r5.pdf")])
    qpdf(["--encrypt", "", "owner", "256", "--", letter, str(HERE / "encrypted-aes256.pdf")])
    qpdf(["--object-streams=generate", "--encrypt", "", "owner", "256", "--", letter, str(HERE / "encrypted-objects.pdf")])
    qpdf(["--encrypt", "secret", "owner", "256", "--", letter, str(HERE / "password.pdf")])
    edge(HERE / "edge.pdf")
    picture(HERE / "picture.pdf")
    cut(HERE / "cut.pdf")
    scans()


if __name__ == "__main__":
    main()
