"""PDFs and screenshots from a real Chat Computer window: a document printed to a PDF without being shown (a Word
file made into its page, as the printer tool does), what's open between the panels printed as it is, and where what's
open sits on the screen (for a screenshot of just that part).

Runs a real Chat Computer window (GTK + WebKit) on a virtual display and prints JSON.
Run:  xvfb-run -a -s "-screen 0 1280x800x24" python tests/screen_pdf_check.py
"""
import asyncio
import json
import os
import sys
import tempfile
import threading
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
os.environ["CHATCOMPUTER_HOME"] = tempfile.mkdtemp(prefix="chatcomputer-pdf-")

import gi  # noqa: E402

gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk  # noqa: E402

from chatcomputer.config import Paths, Settings  # noqa: E402
from chatcomputer.ui import render  # noqa: E402
from chatcomputer.ui.gtk_app import GtkScreen, choose_rendering  # noqa: E402

choose_rendering()
settings = Settings()
screen = GtkScreen(Paths().ensure(), settings, windowed=True)
loop = asyncio.new_event_loop()
threading.Thread(target=loop.run_forever, daemon=True).start()
screen.bind(loop, on_media_key=lambda k: None, on_media_state=lambda p: None, on_typed=lambda t: None)
out: dict = {}
tmp = Path(tempfile.mkdtemp(prefix="pdfs-"))


def pdf_text(path: Path) -> dict:
    if not path.exists():
        return {"exists": False}
    from pypdf import PdfReader
    r = PdfReader(str(path))
    box = r.pages[0].mediabox
    return {"exists": True, "pages": len(r.pages), "text": " ".join((p.extract_text() or "") for p in r.pages),
            "size_pt": [round(float(box.width)), round(float(box.height))]}


async def run() -> None:
    await asyncio.sleep(1.0)
    out["rect_chat"] = await screen.content_rect()
    out["open_nothing"] = await screen.to_pdf(str(tmp / "nothing.pdf"))
    doc = render.document_html("CV.docx", "# Sam Lee\n\nCurriculum Vitae\n\n- Singapore\n- Designer", settings)
    out["doc"] = await screen.to_pdf(str(tmp / "CV.pdf"), doc, str(tmp))
    out["doc_pdf"] = pdf_text(tmp / "CV.pdf")
    out["doc_shown"] = (await screen.content_info()).get("mode")      # printed without being shown
    await screen.show_html(render.document_html("Packing list", "# Packing list\n\n- Passport\n- Charger", settings),
                           "Packing list")
    await asyncio.sleep(1.0)
    out["rect_open"] = await screen.content_rect()
    out["open"] = await screen.to_pdf(str(tmp / "open.pdf"))
    out["open_pdf"] = pdf_text(tmp / "open.pdf")
    out["screen"] = await screen._ui_call(lambda: list(screen.win.get_size()))


def main() -> None:
    def go() -> bool:
        fut = asyncio.run_coroutine_threadsafe(run(), loop)

        def done(_f) -> None:
            if _f.exception():
                out["error"] = repr(_f.exception())
            GLib.idle_add(Gtk.main_quit)
        fut.add_done_callback(done)
        return False
    GLib.timeout_add(3000, go)                  # the conversation page loaded first
    GLib.timeout_add_seconds(90, lambda: (out.setdefault("timeout", True), Gtk.main_quit()))
    Gtk.main()
    print(json.dumps(out))


main()
