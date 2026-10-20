"""Builds the browser demo in docs/ (for GitHub Pages): Chat Computer's own conversation screen, the same page and code
as on a Chat Computer (chatcomputer/ui/assets), with docs/demo/host.js standing in for the computer behind it (and
docs/demo/demo.css for the demo's own two buttons, Connect Claude and Connect AirPods). The tab's icon is Chat
Computer's (assets/icon.svg, as docs/favicon.svg), with docs/favicon.ico and docs/apple-touch-icon.png drawn from it
by scripts/make_icons.js.

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
FONTS = ["AtkinsonHyperlegibleNext.ttf", "AtkinsonHyperlegibleNext-Italic.ttf", "AtkinsonHyperlegibleMono.ttf",
         "AtkinsonHyperlegibleMono-Italic.ttf", "OFL-AtkinsonHyperlegibleNext.txt", "OFL-AtkinsonHyperlegibleMono.txt",
         "README.md"]
TITLE = "Chat Computer"
DOMAIN = "chatcomputer.ai"                 # where GitHub Pages serves docs/
DESCRIPTION = ("Chat Computer is Claude running a whole computer: you talk, type or touch, and it does things for you. "
               "Try its screen here, in your browser.")


# The tab's icon: the SVG where the browser takes one (sizes on the .ico keeps Chrome from preferring it), the .ico
# elsewhere, and the home screen's for phones and tablets.
ICONS = ('<link rel="icon" href="favicon.ico" sizes="32x32">\n<link rel="icon" href="favicon.svg" type="image/svg+xml">\n'
         '<link rel="apple-touch-icon" href="apple-touch-icon.png">')
DRAWN = ["favicon.ico", "apple-touch-icon.png"]       # made by scripts/make_icons.js


def index_html() -> str:
    """chat.html, with its files from demo/ and the demo's host script before Chat Computer's own."""
    page = (ASSETS / "chat.html").read_text(encoding="utf-8")
    swaps = [
        ("<title>Chat Computer</title>",
         f'<title>{TITLE}</title>\n<meta name="description" content="{DESCRIPTION}">\n' + ICONS),
        ('<link rel="stylesheet" href="theme.css">', '<link rel="stylesheet" href="demo/theme.css">'),
        ('<link rel="stylesheet" href="chat.css">',
         '<link rel="stylesheet" href="demo/chat.css">\n<link rel="stylesheet" href="demo/demo.css">'),
        ('<script src="md.js"></script>', '<script src="demo/host.js"></script>\n<script src="demo/md.js"></script>'),
        ('<script src="chat.js"></script>', '<script src="demo/chat.js"></script>'),
    ]
    for old, new in swaps:
        if page.count(old) != 1:
            raise SystemExit(f"chat.html changed: {old!r} isn't there once; update scripts/build_demo.py")
        page = page.replace(old, new)
    return page


def files() -> dict[Path, bytes]:
    """Everything this makes in docs/ (host.js and demo.css are written by hand, not made here)."""
    out: dict[Path, bytes] = {DOCS / "index.html": index_html().encode("utf-8"),
                              DEMO / "doc.css": DOC_CSS.lstrip().encode("utf-8"),
                              DOCS / "favicon.svg": (ASSETS / "icon.svg").read_bytes()}
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
    for name in ("host.js", "demo.css"):
        if not (DEMO / name).exists():
            raise SystemExit(f"docs/demo/{name} is missing")
    for name in DRAWN:
        if not (DOCS / name).exists():
            raise SystemExit(f"docs/{name} is missing: run node scripts/make_icons.js")


if __name__ == "__main__":
    main()
