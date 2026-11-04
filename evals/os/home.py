"""The person's home folder for a desktop eval: the files someone would have a few months in (a tax letter, a CV,
a boarding pass, holiday photos, screenshots cluttering the desktop), made fresh for each case."""
from __future__ import annotations

import io
import os
import time
from pathlib import Path


def _pdf(lines: list[str]) -> bytes:
    """A small, valid one-page PDF with these lines of text (Helvetica)."""
    text = "BT /F1 12 Tf 72 760 Td 16 TL " + " ".join(
        "(" + ln.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)") + ") Tj T*" for ln in lines) + " ET"
    objs = [b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R "
            b"/Resources << /Font << /F1 5 0 R >> >> >>",
            b"<< /Length %d >>\nstream\n" % len(text) + text.encode("latin-1") + b"\nendstream",
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
    out, offsets = io.BytesIO(), []
    out.write(b"%PDF-1.4\n")
    for i, o in enumerate(objs, 1):
        offsets.append(out.tell())
        out.write(b"%d 0 obj\n" % i + o + b"\nendobj\n")
    xref = out.tell()
    out.write(b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1))
    for off in offsets:
        out.write(b"%010d 00000 n \n" % off)
    out.write(b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, xref))
    return out.getvalue()


def _jpg(colour: tuple[int, int, int], label: str) -> bytes:
    from PIL import Image, ImageDraw
    img = Image.new("RGB", (640, 480), colour)
    ImageDraw.Draw(img).text((20, 20), label, fill=(255, 255, 255))
    buf = io.BytesIO()
    img.save(buf, "JPEG", quality=80)
    return buf.getvalue()


def _png(label: str) -> bytes:
    from PIL import Image, ImageDraw
    img = Image.new("RGB", (800, 500), (236, 236, 236))
    ImageDraw.Draw(img).text((20, 20), label, fill=(0, 0, 0))
    buf = io.BytesIO()
    img.save(buf, "PNG")
    return buf.getvalue()


def _docx(paragraphs: list[str]) -> bytes:
    import docx
    d = docx.Document()
    for p in paragraphs:
        d.add_paragraph(p)
    buf = io.BytesIO()
    d.save(buf)
    return buf.getvalue()


def _xlsx(rows: list[list[object]]) -> bytes:
    import openpyxl
    wb = openpyxl.Workbook()
    ws = wb.active
    ws.title = "2026"
    for r in rows:
        ws.append(r)
    buf = io.BytesIO()
    wb.save(buf)
    return buf.getvalue()


def _when(y: int, mo: int, d: int, h: int = 10, mi: int = 0) -> float:
    return time.mktime((y, mo, d, h, mi, 0, 0, 0, -1))


def files() -> dict[str, tuple[bytes, float]]:
    """Path (from the home folder) -> (contents, modified time)."""
    f: dict[str, tuple[bytes, float]] = {
        "Documents/Tax letter March 2026.pdf": (_pdf([
            "Inland Revenue Authority of Singapore", "Notice of Assessment, Year of Assessment 2026",
            "Sam Lee", "Chargeable income: S$84,200", "Tax payable: S$3,876.00, due within 30 days.",
            "Date: 14 March 2026"]), _when(2026, 3, 14)),
        "Documents/CV.docx": (_docx(["Sam Lee", "Curriculum Vitae",
                                     "Product designer, 9 years. Singapore.",
                                     "Experience: Grab (2021-2026), Shopee (2017-2021)."]), _when(2026, 5, 2)),
        "Documents/Packing list.md": (b"# Packing list for Bali\n\n- Passport\n- Swimsuit\n- Sunscreen\n- Chargers\n",
                                      _when(2026, 6, 28)),
        "Documents/Budget 2026.xlsx": (_xlsx([["Month", "Income", "Spent"], ["January", 7000, 5210],
                                              ["February", 7000, 4890], ["March", 7000, 6120]]), _when(2026, 4, 1)),
        "Downloads/Boarding pass SQ322.pdf": (_pdf([
            "Singapore Airlines", "Boarding pass", "SQ322 Singapore (SIN) to London (LHR)",
            "Passenger: LEE / SAM", "Date: 24 October 2026, 23:15", "Seat 41C, Gate C19"]), time.time() - 3 * 3600),
        "Pictures/Beach.jpg": (_jpg((40, 120, 200), "Beach"), _when(2026, 7, 6)),
        "Desktop/Notes.txt": (b"Call the bank about the card.\nBuy a birthday present for Mum.\n", _when(2026, 9, 30)),
        "Desktop/Screenshot 2026-01-05 at 10.12.33.png": (_png("Screenshot 1"), _when(2026, 1, 5, 10, 12)),
        "Desktop/Screenshot 2026-01-12 at 18.40.02.png": (_png("Screenshot 2"), _when(2026, 1, 12, 18, 40)),
        "Desktop/Screenshot 2026-02-01 at 09.05.47.png": (_png("Screenshot 3"), _when(2026, 2, 1, 9, 5)),
        "Videos/Old holiday 2009.avi": (b"RIFF\x00\x00\x00\x00AVI LIST" + bytes(2048), _when(2009, 8, 20)),
    }
    for i in range(1, 6):
        f[f"Pictures/Bali 2026/IMG_000{i}.jpg"] = (_jpg((30 * i, 140, 90), f"Bali {i}"), _when(2026, 7, i + 1))
    return f


def usb_files() -> dict[str, bytes]:
    """What's on the USB stick of the cases that have one plugged in (when the computer can reach it)."""
    out = {f"Photos/IMG_100{i}.jpg": _jpg((200, 60 * i, 40), f"USB photo {i}") for i in range(1, 4)}
    out["Recipes.docx"] = _docx(["Recipes", "Laksa: coconut milk, rice noodles, prawns."])
    return out


def build_home(home: Path) -> None:
    for d in ("Desktop", "Documents", "Downloads", "Music", "Pictures", "Videos"):
        (home / d).mkdir(parents=True, exist_ok=True)
    for rel, (data, mtime) in files().items():
        p = home / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
        os.utime(p, (mtime, mtime))


def listing(root: Path) -> dict[str, int]:
    """Every file of the person's under root (not Chat Computer's own folder, nor hidden ones), with its size."""
    out: dict[str, int] = {}
    for dirpath, dirnames, filenames in os.walk(root):
        rel = os.path.relpath(dirpath, root)
        top = rel.split(os.sep)[0]
        if top == "ChatComputer" or (top.startswith(".") and top != "."):
            dirnames[:] = []
            continue
        filenames = [n for n in filenames if not (rel == "." and n.startswith("."))]
        for n in filenames:
            p = os.path.join(dirpath, n)
            try:
                out[os.path.relpath(p, root)] = os.path.getsize(p)
            except OSError:
                pass
    return out
