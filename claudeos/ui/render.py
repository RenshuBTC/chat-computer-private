"""Turns documents, files and Claude-made pages into something the content layer can show."""
from __future__ import annotations

import hashlib
import html as htmllib
import json
import os
import shutil
import subprocess
from pathlib import Path
from typing import Any

ASSETS = Path(__file__).resolve().parent / "assets"


def asset(name: str) -> str:
    return (ASSETS / name).read_text(encoding="utf-8")


NATIVE = 'data-claudeos="native"'   # marks pages laid out by ClaudeOS (the screen fills in the size)

DOC_CSS = """
body { overflow-y: auto; cursor: none; }
article { width: var(--co-column, min(48rem, calc(100vw - 3rem))); max-width: 100vw; margin: 0 auto; padding: 2rem 0 4rem; }
article > h1.title { font-size: 1.5rem; font-weight: bold; line-height: 1.3; margin: 0 0 1rem; }
.pages img { display: block; width: 100%; margin: 0 0 1rem; border: 1px solid var(--rule); }
.media { position: fixed; inset: 0; background: #000; display: flex; align-items: center; justify-content: center; }
.media video { width: 100vw; height: 100vh; object-fit: contain; }
.media .audio { text-align: center; color: #fff; }
.image { position: fixed; inset: 0; display: flex; align-items: center; justify-content: center; background: var(--ground); }
.image img { max-width: 100vw; max-height: 100vh; object-fit: contain; }
"""


def _theme_attr(settings: Any) -> str:
    return "dark" if getattr(settings, "theme", "light") == "dark" else "light"


def _theme_css() -> str:
    return asset("theme.css")


def _shell(title: str, body: str, settings: Any, script: str = "") -> str:
    # The text size and column width (--co-size, --co-column) are added by the screen when the
    # page is shown, because they depend on the monitor and how far away you sit.
    return (f'<!doctype html><html lang="en" data-theme="{_theme_attr(settings)}" {NATIVE}><head><meta charset="utf-8">'
            f"<title>{htmllib.escape(title)}</title><style>{_theme_css()}\n{DOC_CSS}"
            f"</style></head><body>{body}{script}</body></html>")


def document_html(title: str, markdown: str, settings: Any) -> str:
    body = f'<article><h1 class="title">{htmllib.escape(title)}</h1><div class="md" id="doc"></div></article>'
    script = (f"<script>{asset('md.js')}</script><script>document.getElementById('doc').innerHTML="
              f"window.renderMarkdown({json.dumps(markdown)});</script>")
    return _shell(title, body, settings, script)


def wrap_page(html: str, title: str, settings: Any) -> str:
    """Claude-made pages are shown as written; fragments get the ClaudeOS look around them."""
    low = html.lstrip()[:200].lower()
    if low.startswith("<!doctype") or low.startswith("<html"):
        return html
    return _shell(title, f'<article class="md">{html}</article>', settings)


TEXTY = {".txt", ".md", ".markdown", ".log", ".json", ".csv", ".tsv", ".py", ".js", ".ts", ".html", ".css", ".xml",
         ".yaml", ".yml", ".toml", ".ini", ".cfg", ".sh", ".ps1", ".sql", ".c", ".cpp", ".h", ".rs", ".go", ".java"}
IMAGES = {".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg", ".ico"}
VIDEO = {".mp4", ".webm", ".mkv", ".mov", ".m4v", ".avi", ".ogv"}
AUDIO = {".mp3", ".m4a", ".wav", ".flac", ".ogg", ".opus", ".aac"}


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
    width = max(len(r) for r in rows)
    rows = [r + [""] * (width - len(r)) for r in rows]
    esc = lambda c: c.replace("|", "\\|").replace("\n", " ")
    lines = ["| " + " | ".join(esc(c) for c in rows[0]) + " |", "|" + "---|" * width]
    lines += ["| " + " | ".join(esc(c) for c in r) + " |" for r in rows[1:]]
    return "\n".join(lines)


def _xlsx_markdown(path: str) -> str:
    import openpyxl
    wb = openpyxl.load_workbook(path, read_only=True, data_only=True)
    parts = []
    for ws in list(wb.worksheets)[:5]:
        rows = [["" if c is None else str(c) for c in r] for r in ws.iter_rows(max_row=200, values_only=True)]
        rows = [r for r in rows if any(x.strip() for x in r)]
        if not rows:
            continue
        width = max(len(r) for r in rows)
        rows = [r + [""] * (width - len(r)) for r in rows]
        parts.append(f"## {ws.title}\n\n" + "| " + " | ".join(rows[0]) + " |\n|" + "---|" * width + "\n" +
                     "\n".join("| " + " | ".join(c.replace("|", "\\|") for c in r) + " |" for r in rows[1:]))
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
        rows = [[c.text.strip().replace("|", "\\|") for c in r.cells] for r in table.rows]
        if rows:
            w = max(len(r) for r in rows)
            out.append("| " + " | ".join(rows[0]) + " |\n|" + "---|" * w + "\n" + "\n".join("| " + " | ".join(r) + " |" for r in rows[1:]))
    return "\n\n".join(out)


def _pptx_markdown(path: str) -> str:
    from pptx import Presentation
    prs = Presentation(path)
    out = []
    for i, slide in enumerate(prs.slides, 1):
        texts = [s.text_frame.text.strip() for s in slide.shapes if getattr(s, "has_text_frame", False) and s.text_frame.text.strip()]
        out.append(f"## Slide {i}\n\n" + "\n\n".join(texts))
    return "\n\n".join(out)


def _pdf_view(path: str, settings: Any, paths: Any) -> str:
    """Pages rendered as images (pdftoppm) so the PDF looks like itself; text if that's unavailable."""
    title = os.path.basename(path)
    if shutil.which("pdftoppm"):
        st = os.stat(path)
        key = hashlib.sha1(f"{path}:{st.st_mtime}:{st.st_size}".encode()).hexdigest()[:16]
        out_dir = Path(paths.web) / "pdf" / key
        if not any(out_dir.glob("p-*.png")):
            out_dir.mkdir(parents=True, exist_ok=True)
            subprocess.run(["pdftoppm", "-png", "-r", "110", "-l", "30", path, str(out_dir / "p")],
                           capture_output=True, timeout=120)
        imgs = sorted(out_dir.glob("p-*.png"), key=lambda p: int(p.stem.split("-")[-1]))
        if imgs:
            body = (f'<article><h1 class="title">{htmllib.escape(title)}</h1><div class="pages">' +
                    "".join(f'<img src="{i.as_uri()}" alt="page {n}">' for n, i in enumerate(imgs, 1)) + "</div></article>")
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
        return "html", _shell(name, f'<div class="image"><img src="{uri}" alt="{htmllib.escape(name)}"></div>', settings)
    if ext in VIDEO:
        return "html", _shell(name, f'<div class="media"><video src="{uri}" autoplay playsinline></video></div>', settings)
    if ext in AUDIO:
        return "html", _shell(name, f'<div class="media"><div class="audio">♪ {htmllib.escape(name)}'
                                    f'<audio src="{uri}" autoplay></audio></div></div>', settings)
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
                                       "ClaudeOS can't show this kind of file directly.", settings)
