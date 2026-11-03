"""Turns documents, files and pages the model made into something the content layer can show."""
from __future__ import annotations

import contextlib
import hashlib
import html as htmllib
import json
import os
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any

ASSETS = Path(__file__).resolve().parent / "assets"
FONTS = ASSETS / "fonts"    # Lora, and Atkinson Hyperlegible Mono for code (see fonts/README.md)


def asset(name: str) -> str:
    return (ASSETS / name).read_text(encoding="utf-8")


NATIVE = 'data-chatcomputer="native"'  # marks pages laid out by Chat Computer (the screen fills in the size)

# Black on the conversation's warm off-white (theme.css), except that a video plays on black, as in any video player.
DOC_CSS = """
body { overflow-y: auto; }
article { width: var(--co-column, min(57.75rem, calc(100vw - 3rem))); max-width: 100vw; margin: 0 auto; padding: 2rem 0 4rem; }
article > h1.title { font-size: 1.5rem; font-weight: bold; line-height: 1.3; margin: 0 0 1rem; }
.pages img { display: block; width: 100%; margin: 0 0 1rem; border: 1px solid var(--line); }
.media { position: fixed; inset: 0; background: #000; display: flex; align-items: center; justify-content: center; }
.media video { width: 100vw; height: 100vh; object-fit: contain; }
.sound { position: fixed; inset: 0; display: flex; align-items: center; justify-content: center; text-align: center; }
.image { position: fixed; inset: 0; display: flex; align-items: center; justify-content: center; background: var(--ground); }
.image img { max-width: 100vw; max-height: 100vh; object-fit: contain; }
.image img { transition: transform 0.2s; }
.image.zoomed img { transform: scale(2.5); }
.meta { margin: 0 0 1.25rem; }
.meta div { overflow-wrap: anywhere; }
.plain { white-space: pre-wrap; overflow-wrap: anywhere; }
.said { margin: 0 0 1rem; }
.said .who { font-weight: bold; }
.said.mail { padding: 0 0 1rem; margin: 0 0 1.25rem; border-bottom: 2px solid var(--line); }
.said.mail:last-child { border-bottom: 0; }
.said.mail .meta { margin: 0.25rem 0 0.5rem; }
/* On paper (a PDF, a printer): the size of a printed page's text, the page's own width, no screen colours. */
@media print {
  html { font-size: 11pt !important; }
  html, body { background: var(--ground) !important; color: var(--ink) !important; overflow: visible !important; }
  article { width: auto; max-width: none; margin: 0; padding: 0; }
  .media, .sound, .image { position: static; }
}
"""


# A photo: a double tap (or a double click) zooms in where it was, and another zooms out, as on an iPad.
IMAGE_ZOOM = """<script>(function () {
  var box = document.querySelector(".image"), img = box.querySelector("img"), last = 0;
  function toggle(x, y) {
    var r = img.getBoundingClientRect();
    img.style.transformOrigin = (x - r.left) + "px " + (y - r.top) + "px";
    box.classList.toggle("zoomed");
  }
  box.addEventListener("dblclick", function (e) { toggle(e.clientX, e.clientY); });
  box.addEventListener("touchend", function (e) {
    if (e.touches.length || e.changedTouches.length !== 1) return;
    var t = e.changedTouches[0], now = Date.now();
    if (now - last < 350) { e.preventDefault(); toggle(t.clientX, t.clientY); last = 0; } else last = now;
  });
})();</script>"""


def _theme_css() -> str:
    # theme.css finds its fonts next to it (fonts/...). The pages made here are shown from another folder,
    # so they get the fonts' full address.
    return asset("theme.css").replace('url("fonts/', f'url("{FONTS.as_uri()}/')


def _shell(title: str, body: str, settings: Any, script: str = "") -> str:
    # The text size and column width (--co-size, --co-column) are added by the screen when the
    # page is shown, because they depend on the monitor and how far away you sit.
    return (f'<!doctype html><html lang="en" {NATIVE}><head><meta charset="utf-8">'
            f"<title>{htmllib.escape(title)}</title><style>{_theme_css()}\n{DOC_CSS}"
            f"</style></head><body>{body}{script}</body></html>")


def document_html(title: str, markdown: str, settings: Any) -> str:
    body = f'<article><h1 class="title">{htmllib.escape(title)}</h1><div class="md" id="doc"></div></article>'
    # Inside <script>, a "</script>" in the text (a web page's source, a README about HTML) would end the script
    # there and show the rest as raw HTML; "<" written as \u003c is the same string to JavaScript.
    text = json.dumps(markdown).replace("<", "\\u003c")
    script = (f"<script>{asset('md.js')}</script><script>document.getElementById('doc').innerHTML="
              f"window.renderMarkdown({text});</script>")
    return _shell(title, body, settings, script)


def message_html(title: str, meta: list[tuple[str, str]], text: str, settings: Any) -> str:
    """An email (or another message) on the screen: its subject, who and when, then its words as written.
    Everything is escaped: an email's HTML is never run, and its pictures aren't fetched."""
    rows = "".join(f"<div><b>{htmllib.escape(k)}:</b> {htmllib.escape(v)}</div>" for k, v in meta if v)
    body = (f'<article><h1 class="title">{htmllib.escape(title)}</h1><div class="meta">{rows}</div>'
            f'<div class="plain">{htmllib.escape(text)}</div></article>')
    return _shell(title, body, settings)


def thread_html(title: str, meta: list[tuple[str, str]], mails: list[dict[str, Any]], settings: Any) -> str:
    """Emails with one person, oldest first, as a conversation: each one's subject and when, then its words (as
    much as is known: all of the ones read just now). The page starts at the one that was opened ("open")."""
    rows = "".join(f"<div><b>{htmllib.escape(k)}:</b> {htmllib.escape(v)}</div>" for k, v in meta if v)
    blocks = []
    for m in mails:
        att = (f'<div class="meta">Attachments: {htmllib.escape(", ".join(m["attachments"]))}</div>'
               if m.get("attachments") else "")
        blocks.append(f'<section class="said mail"{" id=open" if m.get("open") else ""}>'
                      f'<div><span class="who">{htmllib.escape(m.get("subject") or "(no subject)")}</span> '
                      f'{htmllib.escape(m.get("when") or "")}</div>{att}'
                      f'<div class="plain">{htmllib.escape(m.get("text") or "")}</div></section>')
    body = (f'<article><h1 class="title">{htmllib.escape(title)}</h1><div class="meta">{rows}</div>'
            f'{"".join(blocks)}</article>')
    script = "<script>var o=document.getElementById('open');if(o)o.scrollIntoView({block:'start'});</script>"
    return _shell(title, body, settings, script)


def chat_html(title: str, lines: list[tuple[str, str, str]], settings: Any) -> str:
    """A chat's latest messages (who, when, words), oldest first."""
    body = "".join(f'<div class="said"><span class="who">{htmllib.escape(who)}</span> {htmllib.escape(when)}'
                   f'<div class="plain">{htmllib.escape(text)}</div></div>' for who, when, text in lines)
    return _shell(title, f'<article><h1 class="title">{htmllib.escape(title)}</h1>{body}</article>', settings)


def wrap_page(html: str, title: str, settings: Any) -> str:
    """Pages the model made are shown as written; fragments get the Chat Computer look around them."""
    low = html.lstrip()[:200].lower()
    if low.startswith("<!doctype") or low.startswith("<html"):
        return html
    return _shell(title, f'<article class="md">{html}</article>', settings)


TEXTY = {".txt", ".md", ".markdown", ".log", ".json", ".csv", ".tsv", ".py", ".js", ".ts", ".html", ".css", ".xml",
         ".yaml", ".yml", ".toml", ".ini", ".cfg", ".sh", ".ps1", ".sql", ".c", ".cpp", ".h", ".rs", ".go", ".java"}
IMAGES = {".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg", ".ico"}
VIDEO = {".mp4", ".webm", ".mkv", ".mov", ".m4v", ".avi", ".ogv"}
AUDIO = {".mp3", ".m4a", ".wav", ".flac", ".ogg", ".opus", ".aac"}


def _cell(c: Any) -> str:
    """A table cell as Markdown: one line (a cell's line breaks would end the table's row there), and | as text."""
    return " ".join(str(c).replace("|", "\\|").splitlines())


def _table(rows: list[list[str]]) -> str:
    """Rows (the first is the header) as a Markdown table."""
    width = max(len(r) for r in rows)
    rows = [r + [""] * (width - len(r)) for r in rows]
    lines = ["| " + " | ".join(_cell(c) for c in rows[0]) + " |", "|" + "---|" * width]
    lines += ["| " + " | ".join(_cell(c) for c in r) + " |" for r in rows[1:]]
    return "\n".join(lines)


def _csv_markdown(path: str, limit: int = 200) -> str:
    import csv
    with open(path, newline="", encoding="utf-8", errors="replace") as f:
        sample = f.read(4096)
        f.seek(0)
        try:
            dialect = csv.Sniffer().sniff(sample)
        except csv.Error:
            dialect = csv.excel
        rows = [r for _, r in zip(range(limit + 1), csv.reader(f, dialect))]
    if not rows:
        return "(empty)"
    return _table(rows)


def _xlsx_markdown(path: str) -> str:
    import openpyxl
    wb = openpyxl.load_workbook(path, read_only=True, data_only=True)
    parts = []
    for ws in list(wb.worksheets)[:5]:
        rows = [["" if c is None else str(c) for c in r] for r in ws.iter_rows(max_row=200, values_only=True)]
        rows = [r for r in rows if any(x.strip() for x in r)]
        if not rows:
            continue
        parts.append(f"## {ws.title}\n\n" + _table(rows))
    wb.close()
    return "\n\n".join(parts) or "(empty spreadsheet)"


def _docx_markdown(path: str) -> str:
    import docx
    d = docx.Document(path)
    out = []
    for p in d.paragraphs:
        style = (p.style.name or "").lower() if p.style is not None else ""
        t = p.text.strip()
        if not t:
            continue
        if style.startswith("heading 1") or style == "title":
            out.append("# " + t)
        elif style.startswith("heading 2"):
            out.append("## " + t)
        elif style.startswith("heading"):
            out.append("### " + t)
        elif "list" in style:
            out.append("- " + t)
        else:
            out.append(t)
    for table in d.tables[:10]:
        rows = [[c.text.strip() for c in r.cells] for r in table.rows]
        if rows:
            out.append(_table(rows))
    return "\n\n".join(out)


def _pptx_markdown(path: str) -> str:
    from pptx import Presentation
    prs = Presentation(path)
    out = []
    for i, slide in enumerate(prs.slides, 1):
        texts = [s.text_frame.text.strip() for s in slide.shapes if getattr(s, "has_text_frame", False) and s.text_frame.text.strip()]
        out.append(f"## Slide {i}\n\n" + "\n\n".join(texts))
    return "\n\n".join(out)


PDF_PAGES = 30


def _pdf_pages(path: str) -> int:
    try:
        out = subprocess.run(["pdfinfo", path], capture_output=True, text=True, errors="replace", timeout=20,
                             check=False).stdout
    except (OSError, subprocess.SubprocessError):
        return 0
    for line in out.splitlines():
        if line.startswith("Pages:") and line[6:].strip().isdigit():
            return int(line[6:])
    return 0


def _prune_pdf_pages(root: Path, keep_days: int = 14) -> None:
    """Pages rendered for PDFs not shown for two weeks (each PDF's take a few megabytes)."""
    cutoff = time.time() - keep_days * 86400
    try:
        for d in root.iterdir():
            if d.is_dir() and d.stat().st_mtime < cutoff:
                shutil.rmtree(d, ignore_errors=True)
    except OSError:
        pass


def _pdf_view(path: str, settings: Any, paths: Any) -> str:
    """Pages rendered as images (pdftoppm) so the PDF looks like itself; text if that's unavailable."""
    title = os.path.basename(path)
    if shutil.which("pdftoppm"):
        st = os.stat(path)
        key = hashlib.sha1(f"{path}:{st.st_mtime}:{st.st_size}".encode()).hexdigest()[:16]
        root = Path(paths.web) / "pdf"
        out_dir = root / key
        where = out_dir
        if not any(out_dir.glob("p-*.jpg")):
            _prune_pdf_pages(root)
            # Rendered into a folder of its own, kept only when complete: one cut short would otherwise show as
            # the whole PDF from then on.
            part = root / (key + ".part")
            shutil.rmtree(part, ignore_errors=True)
            part.mkdir(parents=True, exist_ok=True)
            where = part
            try:
                # JPEG at quality 90: 30 pages took 7.4 s as PNG (compressing them is most of the work) and 0.6 s
                # like this, and look the same (45-51 dB from the PNG).
                subprocess.run(["pdftoppm", "-jpeg", "-jpegopt", "quality=90", "-r", "110", "-l", str(PDF_PAGES), path,
                                str(part / "p")], capture_output=True, timeout=120, check=False)
                if any(part.glob("p-*.jpg")):
                    shutil.rmtree(out_dir, ignore_errors=True)
                    part.rename(out_dir)
                    where = out_dir
            except (OSError, subprocess.SubprocessError):
                pass            # what was rendered in time still shows
        else:
            with contextlib.suppress(OSError):
                os.utime(out_dir)           # shown again: kept another two weeks
        imgs = sorted(where.glob("p-*.jpg"), key=lambda p: int(p.stem.split("-")[-1]))
        if imgs:
            total = _pdf_pages(path)
            more = (f'<p>These are the first {len(imgs)} of its {total} pages.</p>' if total > len(imgs) else "")
            body = (f'<article><h1 class="title">{htmllib.escape(title)}</h1><div class="pages">' +
                    "".join(f'<img src="{i.as_uri()}" alt="page {n}">' for n, i in enumerate(imgs, 1)) +
                    f"</div>{more}</article>")
            return _shell(title, body, settings)
    from ..files_index import extract_text
    return document_html(title, extract_text(path) or "(This PDF has no readable text.)", settings)


def file_view(path: str, settings: Any, paths: Any) -> tuple[str, str]:
    """('url', file_uri) for things the browser shows natively, else ('html', page)."""
    ext = os.path.splitext(path)[1].lower()
    name = os.path.basename(path)
    uri = Path(path).resolve().as_uri()
    if os.path.isdir(path):
        entries = sorted(os.listdir(path))[:400]
        md = "\n".join(f"- {e}{'/' if os.path.isdir(os.path.join(path, e)) else ''}" for e in entries)
        return "html", document_html(name or path, md or "(empty folder)", settings)
    if ext in IMAGES:
        return "html", _shell(name, f'<div class="image"><img src="{uri}" alt="{htmllib.escape(name)}"></div>', settings,
                              IMAGE_ZOOM)
    if ext in VIDEO:
        return "html", _shell(name, f'<div class="media"><video src="{uri}" autoplay playsinline></video></div>', settings)
    if ext in AUDIO:
        return "html", _shell(name, f'<div class="sound">♪ {htmllib.escape(name)}'
                                    f'<audio src="{uri}" autoplay></audio></div>', settings)
    if ext == ".pdf":
        return "html", _pdf_view(path, settings, paths)
    if ext in (".html", ".htm"):
        return "url", uri
    if ext == ".docx":
        return "html", document_html(name, _docx_markdown(path), settings)
    if ext in (".xlsx", ".xlsm"):
        return "html", document_html(name, _xlsx_markdown(path), settings)
    if ext == ".pptx":
        return "html", document_html(name, _pptx_markdown(path), settings)
    if ext in (".csv", ".tsv"):
        return "html", document_html(name, _csv_markdown(path), settings)
    if ext in (".md", ".markdown"):
        with open(path, encoding="utf-8", errors="replace") as f:
            return "html", document_html(name, f.read(200_000), settings)
    if ext in TEXTY or os.path.getsize(path) < 200_000:
        try:
            with open(path, encoding="utf-8") as f:
                text = f.read(200_000)
            return "html", document_html(name, "```\n" + text.replace("```", "ˋˋˋ") + "\n```", settings)
        except (UnicodeDecodeError, OSError):
            pass
    size_mb = os.path.getsize(path) / 1e6
    return "html", document_html(name, f"This is a **{ext or 'binary'}** file ({size_mb:.1f} MB). "
                                       "Chat Computer can't show this kind of file directly.", settings)
