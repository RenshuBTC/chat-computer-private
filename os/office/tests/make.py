#!/usr/bin/env python3
"""Makes the documents the office crate's tests read (tests/*.docx, .xlsx, .pptx, .odt): with python-docx, openpyxl
and python-pptx, as programs write them, and the same converted by LibreOffice, which writes them its own way (and
works out the formulas' values, as Excel does when it saves). What's in them is what the tests look for: headings,
lists, a table, a tab and a line break, bold words split into runs, characters XML escapes and others beyond ASCII;
sheets with words, numbers, dates, times, a percentage, true and false and formulas; slides with titles, bullets, a
table and speaker notes. And three written by hand (edge.docx, .xlsx, .pptx), with what programs write less often:
styles based on headings, tracked changes, fields, a text box (twice, as Word writes it), a table in a table,
footnotes; the 1904 date system, shared formulas, errors, phonetic readings, number formats with words, colours and
elapsed time in them, a hidden sheet and a chart sheet; a hidden slide, footers, a group, a picture's description.

    python3 tests/make.py      (needs python-docx, openpyxl, python-pptx, and LibreOffice's soffice)
"""
from __future__ import annotations

import datetime
import shutil
import subprocess
import tempfile
import zipfile
from pathlib import Path

import docx
import openpyxl
import pptx
from docx.enum.text import WD_BREAK
from pptx.util import Inches

HERE = Path(__file__).resolve().parent


def letter(path: Path) -> None:
    d = docx.Document()
    d.add_heading("A letter to the council", 0)
    d.add_heading("Why I'm writing", 1)
    p = d.add_paragraph("Dear Sir or Madam, the bins on ")
    p.add_run("Elm Street").bold = True
    p.add_run(" haven't been emptied since ")
    p.add_run("3 March").italic = True
    p.add_run(". Prices: 5 < 7 & 7 > 5; \"quoted\" and 'single'.")
    d.add_heading("What I'd like", 2)
    d.add_paragraph("Collection every week", style="List Bullet")
    d.add_paragraph("A bigger bin", style="List Bullet")
    d.add_paragraph("First, call me", style="List Number")
    p = d.add_paragraph("Name:\tSam Lee")
    p.add_run().add_break(WD_BREAK.LINE)
    p.add_run("Street: Elm Street 12")
    t = d.add_table(rows=3, cols=3)
    for r, row in enumerate([["Week", "Bins", "Emptied"], ["1", "2", "no"], ["2", "2", "yes"]]):
        for c, value in enumerate(row):
            t.cell(r, c).text = value
    d.add_paragraph("Ünïcödé, 名前, café and 🙂.")
    d.add_page_break()
    d.add_paragraph("Yours faithfully,")
    d.save(path)


def budget(path: Path) -> None:
    wb = openpyxl.Workbook()
    s = wb.active
    s.title = "Budget"
    s.append(["Item", "Cost", "Paid", "When"])
    s.append(["Rent", 1200, True, datetime.date(2026, 10, 1)])
    s.append(["Food, and drink", 412.5, False, datetime.datetime(2026, 10, 3, 18, 30)])
    s.append(["Total", "=SUM(B2:B3)", None, None])
    s["E1"] = "Share"
    s["E2"] = 0.75
    s["E2"].number_format = "0%"
    s["G7"] = "far away"
    t = wb.create_sheet("Notes")
    t["A1"] = "Remember: \"quotes\", commas and new\nlines"
    t["B2"] = datetime.time(7, 45)
    wb.save(path)


def talk(path: Path) -> None:
    p = pptx.Presentation()
    s = p.slides.add_slide(p.slide_layouts[0])
    s.shapes.title.text = "Chat Computer"
    s.placeholders[1].text = "A computer you talk to"
    s = p.slides.add_slide(p.slide_layouts[1])
    s.shapes.title.text = "What it does"
    body = s.placeholders[1].text_frame
    body.text = "Answers questions"
    for words, level in [("Reads your files", 0), ("Word, Excel and PowerPoint", 1), ("Sets timers", 0)]:
        para = body.add_paragraph()
        para.text = words
        para.level = level
    s.notes_slide.notes_text_frame.text = "Say hello first."
    s = p.slides.add_slide(p.slide_layouts[5])
    s.shapes.title.text = "Numbers"
    table = s.shapes.add_table(2, 2, Inches(1), Inches(2), Inches(6), Inches(1)).table
    for r, row in enumerate([["Year", "Users"], ["2026", "1"]]):
        for c, value in enumerate(row):
            table.cell(r, c).text = value
    p.save(path)


def libreoffice(src: Path, to: str, out: Path) -> None:
    """`src` opened and saved by LibreOffice, as `to` (its filter's name), at `out`."""
    with tempfile.TemporaryDirectory() as tmp:
        subprocess.run(["soffice", "--headless", f"-env:UserInstallation=file://{tmp}/profile", "--convert-to", to,
                        "--outdir", tmp, str(src)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        made = next(p for p in Path(tmp).iterdir() if p.is_file())
        shutil.move(made, out)


RELS = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
PKG = "http://schemas.openxmlformats.org/package/2006/relationships"


def package(path: Path, parts: dict[str, str]) -> None:
    """A package of these parts, as a ZIP."""
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        for name, text in parts.items():
            z.writestr(name, text.strip())


def rels(*items: tuple[str, str, str]) -> str:
    """A .rels part: each relationship's id, type (its last word) and target."""
    body = "".join(f'<Relationship Id="{i}" Type="{RELS}/{t}" Target="{target}"/>' for i, t, target in items)
    return f'<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{PKG}">{body}</Relationships>'


W = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="' + RELS + '" ' \
    'xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"'


def edge_docx(path: Path) -> None:
    def p(inner: str, props: str = "") -> str:
        return f"<w:p>{f'<w:pPr>{props}</w:pPr>' if props else ''}{inner}</w:p>"

    def r(text: str) -> str:
        return f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>'

    box = '<w:txbxContent>' + p(r("In a box")) + '</w:txbxContent>'
    body = "".join([
        p(r("Custom heading"), '<w:pStyle w:val="MyHeading"/>'),
        p(r("Localized heading"), '<w:pStyle w:val="berschrift2"/>'),
        p(r("No tab here"), '<w:tabs><w:tab w:val="left" w:pos="720"/></w:tabs>'),
        p(r("Kept ") + '<w:del w:id="1"><w:r><w:delText>deleted </w:delText></w:r></w:del>'
          '<w:ins w:id="2"><w:r><w:t>inserted</w:t></w:r></w:ins>'),
        p(r("Page ") + '<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> PAGE </w:instrText></w:r>'
          '<w:r><w:fldChar w:fldCharType="separate"/></w:r>' + r("7") + '<w:r><w:fldChar w:fldCharType="end"/></w:r>'),
        p(r("Before ") + '<w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wps:txbx '
          'xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">' + box +
          '</wps:txbx></w:drawing></mc:Choice><mc:Fallback><w:pict><v:textbox xmlns:v="urn:schemas-microsoft-com:vml">'
          + box + '</v:textbox></w:pict></mc:Fallback></mc:AlternateContent></w:r>' + r(" after")),
        p(r("Not a list"), '<w:pStyle w:val="ListPara"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="0"/></w:numPr>'),
        p(r("A list item"), '<w:pStyle w:val="ListPara"/>'),
        '<w:tbl><w:tr><w:tc>' + p(r("Outer")) + '</w:tc><w:tc><w:tbl>'
        '<w:tr><w:tc>' + p(r("a")) + '</w:tc><w:tc>' + p(r("b")) + '</w:tc></w:tr>'
        '<w:tr><w:tc>' + p(r("c")) + '</w:tc><w:tc>' + p(r("d")) + '</w:tc></w:tr>'
        '</w:tbl>' + p("") + '</w:tc></w:tr></w:tbl>',
        "<w:p/>",
        p(r("Visit ") + '<w:hyperlink r:id="rId9">' + r("a link") + '</w:hyperlink>' + r(" &#233;&#x1F600; &amp;c")),
        p('<w:r><w:t>Two</w:t><w:br/><w:t>lines</w:t><w:noBreakHyphen/><w:t>joined</w:t></w:r>'),
    ])
    document = f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<w:document {W}><w:body>{body}' \
               '<w:sectPr/></w:body></w:document>'
    styles = f"""<?xml version="1.0" encoding="UTF-8"?><w:styles {W}>
<w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="MyHeading"><w:name w:val="My Heading"/><w:basedOn w:val="Heading1"/></w:style>
<w:style w:type="paragraph" w:styleId="berschrift2"><w:name w:val="heading 2"/></w:style>
<w:style w:type="paragraph" w:styleId="ListPara"><w:name w:val="List Para"/><w:pPr><w:numPr><w:numId w:val="3"/></w:numPr></w:pPr></w:style>
<w:style w:type="character" w:styleId="Heading1Char"><w:name w:val="Heading 1 Char"/></w:style>
</w:styles>"""
    footnotes = f"""<?xml version="1.0" encoding="UTF-8"?><w:footnotes {W}>
<w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>
<w:footnote w:id="1"><w:p><w:r><w:t>A footnote.</w:t></w:r></w:p></w:footnote></w:footnotes>"""
    word_rels = rels(("rId1", "styles", "styles.xml"), ("rId2", "footnotes", "footnotes.xml")).replace(
        "</Relationships>", f'<Relationship Id="rId9" Type="{RELS}/hyperlink" Target="https://example.com/" '
        'TargetMode="External"/></Relationships>')
    package(path, {
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>',
        "_rels/.rels": rels(("rId1", "officeDocument", "word/document.xml")),
        "word/_rels/document.xml.rels": word_rels,
        "word/document.xml": document,
        "word/styles.xml": styles,
        "word/footnotes.xml": footnotes,
    })


def edge_xlsx(path: Path) -> None:
    main = 'xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"'
    workbook = f"""<?xml version="1.0" encoding="UTF-8"?><workbook {main} xmlns:x="{RELS}"><workbookPr date1904="1"/>
<sheets><sheet name="Data" sheetId="1" x:id="rId1"/><sheet name="Secret" sheetId="2" state="hidden" x:id="rId2"/>
<sheet name="Chart" sheetId="3" x:id="rId3"/></sheets></workbook>"""
    styles = f"""<?xml version="1.0" encoding="UTF-8"?><styleSheet {main}><numFmts count="5">
<numFmt numFmtId="165" formatCode="[$-409]d\\ mmmm\\ yyyy;@"/><numFmt numFmtId="166" formatCode="&quot;Total: &quot;0.00"/>
<numFmt numFmtId="167" formatCode="[h]:mm"/><numFmt numFmtId="168" formatCode="mmm"/><numFmt numFmtId="169" formatCode="[Red]0.0%"/>
</numFmts><cellXfs count="7"><xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="165"/><xf numFmtId="166"/><xf numFmtId="167"/>
<xf numFmtId="168"/><xf numFmtId="169"/></cellXfs></styleSheet>"""
    shared = f"""<?xml version="1.0" encoding="UTF-8"?><sst {main} count="3" uniqueCount="3"><si/>
<si><r><t xml:space="preserve">Rich </t></r><r><rPr><b/></rPr><t>text</t></r></si><si><t>far</t></si></sst>"""
    data = f"""<?xml version="1.0" encoding="UTF-8"?><worksheet {main}><sheetData>
<row><c t="inlineStr"><is><t>A</t></is></c><c><v>2</v></c><c t="s"><v>1</v></c><c t="s"><v>0</v></c></row>
<row r="2"><c r="A2" s="1"><v>0</v></c><c r="B2"><f t="shared" ref="B2:B3" si="0">B1*2</f><v>4</v></c>
<c r="C2" t="e"><v>#DIV/0!</v></c><c r="D2" t="inlineStr"><is><r><t>東京</t></r><rPh sb="0" eb="2"><t>トウキョウ</t></rPh></is></c></row>
<row r="3"><c r="A3" s="2"><v>366</v></c><c r="B3"><f t="shared" si="0"/><v>6</v></c><c r="C3" s="3"><v>1.5</v></c>
<c r="D3" s="4"><v>1.5</v></c><c r="E3" s="5"><v>31</v></c><c r="F3" s="6"><v>0.123</v></c><c r="G3" t="str"><f>"x"&amp;"y"</f><v>xy</v></c></row>
<row r="1000"><c r="AA1000" t="s"><v>2</v></c></row></sheetData></worksheet>"""
    secret = f"""<?xml version="1.0" encoding="UTF-8"?><worksheet {main}><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>hidden value</t></is></c></row></sheetData></worksheet>"""
    chart = f"""<?xml version="1.0" encoding="UTF-8"?><chartsheet {main}><sheetViews><sheetView workbookViewId="0"/></sheetViews></chartsheet>"""
    package(path, {
        "_rels/.rels": rels(("rId1", "officeDocument", "xl/workbook.xml")),
        "xl/workbook.xml": workbook,
        "xl/_rels/workbook.xml.rels": rels(("rId1", "worksheet", "worksheets/sheet1.xml"),
                                           ("rId2", "worksheet", "/xl/worksheets/sheet2.xml"),
                                           ("rId3", "chartsheet", "chartsheets/sheet1.xml"),
                                           ("rId4", "styles", "styles.xml"), ("rId5", "sharedStrings", "sharedStrings.xml")),
        "xl/styles.xml": styles,
        "xl/sharedStrings.xml": shared,
        "xl/worksheets/sheet1.xml": data,
        "xl/worksheets/sheet2.xml": secret,
        "xl/chartsheets/sheet1.xml": chart,
    })


def edge_pptx(path: Path) -> None:
    ns = ('xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
          'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="' + RELS + '"')

    def sp(text_xml: str, ph: str = "") -> str:
        return (f'<p:sp><p:nvSpPr><p:cNvPr id="2" name="s"/><p:cNvSpPr/><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr/>'
                f'<p:txBody><a:bodyPr/>{text_xml}</p:txBody></p:sp>')

    def para(text: str) -> str:
        return f"<a:p><a:r><a:t>{text}</a:t></a:r></a:p>"

    slide1 = (f'<?xml version="1.0" encoding="UTF-8"?><p:sld {ns} show="0"><p:cSld><p:spTree>'
              + sp(para("Hidden slide"), '<p:ph type="title"/>') + sp(para("Footer text"), '<p:ph type="ftr" idx="11"/>')
              + sp(para("3"), '<p:ph type="sldNum" idx="12"/>')
              + '<p:grpSp><p:nvGrpSpPr><p:cNvPr id="5" name="g"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>'
              + sp(para("In a group")) + '</p:grpSp>'
              + '<p:pic><p:nvPicPr><p:cNvPr id="6" name="Picture 5" descr="A cat on a sofa"/><p:cNvPicPr/><p:nvPr/>'
              '</p:nvPicPr><p:blipFill/><p:spPr/></p:pic></p:spTree></p:cSld></p:sld>')
    slide2 = (f'<?xml version="1.0" encoding="UTF-8"?><p:sld {ns}><p:cSld><p:spTree>'
              + sp('<a:p><a:r><a:t>Line one</a:t></a:r><a:br/><a:r><a:t>line two</a:t></a:r></a:p><a:p/>'
                   + para("&lt;tag&gt; &amp; more"), '<p:ph idx="1"/>')
              + '</p:spTree></p:cSld></p:sld>')
    notes2 = (f'<?xml version="1.0" encoding="UTF-8"?><p:notes {ns}><p:cSld><p:spTree>'
              + sp(para("1"), '<p:ph type="sldNum" idx="5"/>') + sp(para("First") + para("Second"), '<p:ph type="body" idx="1"/>')
              + '</p:spTree></p:cSld></p:notes>')
    presentation = (f'<?xml version="1.0" encoding="UTF-8"?><p:presentation {ns}><p:sldIdLst>'
                    '<p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst></p:presentation>')
    package(path, {
        "_rels/.rels": rels(("rId1", "officeDocument", "ppt/presentation.xml")),
        "ppt/presentation.xml": presentation,
        "ppt/_rels/presentation.xml.rels": rels(("rId2", "slide", "slides/slide1.xml"), ("rId3", "slide", "slides/slide2.xml")),
        "ppt/slides/slide1.xml": slide1,
        "ppt/slides/slide2.xml": slide2,
        "ppt/slides/_rels/slide2.xml.rels": rels(("rId1", "notesSlide", "../notesSlides/notesSlide2.xml")),
        "ppt/notesSlides/notesSlide2.xml": notes2,
    })


if __name__ == "__main__":
    edge_docx(HERE / "edge.docx")
    edge_xlsx(HERE / "edge.xlsx")
    edge_pptx(HERE / "edge.pptx")
    letter(HERE / "letter.docx")
    budget(HERE / "budget.xlsx")
    talk(HERE / "talk.pptx")
    with tempfile.TemporaryDirectory() as tmp:
        for name, to in [("letter.docx", "docx:MS Word 2007 XML"), ("budget.xlsx", "xlsx:Calc MS Excel 2007 XML"),
                         ("talk.pptx", "pptx:Impress MS PowerPoint 2007 XML")]:
            stem, ext = name.split(".")
            copy = Path(tmp) / name
            shutil.copy(HERE / name, copy)
            libreoffice(copy, to, HERE / f"{stem}-libreoffice.{ext}")
        libreoffice(HERE / "letter.docx", "odt", HERE / "letter.odt")
    for p in sorted(HERE.iterdir()):
        if p.suffix in (".docx", ".xlsx", ".pptx", ".odt"):
            print(p.name, p.stat().st_size)
