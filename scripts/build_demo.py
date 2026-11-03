"""Builds the browser demo in docs/ (for GitHub Pages): Chat Computer's own conversation screen, the same page and code
as on a Chat Computer (chatcomputer/ui/assets), with docs/demo/host.js standing in for the computer behind it (and
"/" in its message box listing what can be used there: this computer's model, where the Chat Computer app runs on it,
and Connect AirPods). No icon: the tab shows the browser's own.

Run it after changing the screen:  python scripts/build_demo.py
tests/test_demo.py checks that docs/ is what this makes, so the demo is always Chat Computer's real screen.
"""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

from chatcomputer.ui.render import DOC_CSS  # noqa: E402

ASSETS = ROOT / "chatcomputer" / "ui" / "assets"
DOCS = ROOT / "docs"
DEMO = DOCS / "demo"
COPIED = ["theme.css", "chat.css", "md.js", "chat.js"]
FONTS = ["Lora.ttf", "Lora-Italic.ttf", "AtkinsonHyperlegibleMono.ttf",
         "AtkinsonHyperlegibleMono-Italic.ttf", "OFL-Lora.txt", "OFL-AtkinsonHyperlegibleMono.txt",
         "README.md"]
TITLE = "Chat Computer"
DOMAIN = "chatcomputer.ai"                 # where GitHub Pages serves docs/
DESCRIPTION = ("Chat Computer runs a whole computer for you, free, with its own model on your computer: you talk, type "
               "or touch, and it does things for you. Try its screen here, in your browser.")



def index_html() -> str:
    """chat.html, with its files from demo/ and the demo's host script before Chat Computer's own."""
    page = (ASSETS / "chat.html").read_text(encoding="utf-8")
    swaps = [
        ("<title>Chat Computer</title>",
         f'<title>{TITLE}</title>\n<meta name="description" content="{DESCRIPTION}">'),
        ('<link rel="stylesheet" href="theme.css">', '<link rel="stylesheet" href="demo/theme.css">'),
        ('<link rel="stylesheet" href="chat.css">',
         '<link rel="stylesheet" href="demo/chat.css">'),
        ('<script src="md.js"></script>',
         '<script src="demo/engine/engine.js"></script>\n<script src="demo/host.js"></script>\n<script src="demo/md.js"></script>'),
        ('<script src="chat.js"></script>', '<script src="demo/chat.js"></script>'),
    ]
    for old, new in swaps:
        if page.count(old) != 1:
            raise SystemExit(f"chat.html changed: {old!r} isn't there once; update scripts/build_demo.py")
        page = page.replace(old, new)
    return page


def files() -> dict[Path, bytes]:
    """Everything this makes in docs/ (host.js is written by hand, not made here)."""
    out: dict[Path, bytes] = {DOCS / "index.html": index_html().encode("utf-8"),
                              DEMO / "doc.css": DOC_CSS.lstrip().encode("utf-8")}
    for name in COPIED:
        out[DEMO / name] = (ASSETS / name).read_bytes()
    for name in FONTS:
        out[DEMO / "fonts" / name] = (ASSETS / "fonts" / name).read_bytes()
    out[DOCS / ".nojekyll"] = b""            # GitHub Pages serves the files as they are
    out[DOCS / "CNAME"] = DOMAIN.encode()    # ...at this address (as GitHub writes it: the name alone)
    return out


def main() -> None:
    for path, data in files().items():
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists() or path.read_bytes() != data:
            path.write_bytes(data)
            print("wrote", path.relative_to(ROOT))
    for name in ("host.js",):
        if not (DEMO / name).exists():
            raise SystemExit(f"docs/demo/{name} is missing")


if __name__ == "__main__":
    main()
