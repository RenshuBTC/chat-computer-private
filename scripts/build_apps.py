"""Builds what chatcomputer.ai/get offers from what's in apps/ (docs/get/):

- Chat-Computer-Chrome.zip: the Chrome extension (apps/chrome), in a folder of its own, for "Load unpacked" (as
  Annotated's "Add to Chrome" is). It's small: its side panel shows the website itself (chatcomputer.ai, so it's
  always the newest), and opens the sites asked for in real Chrome tabs.

The same bytes each time (fixed dates in the zip), so tests/test_apps.py can check docs/get is what this makes.
Run it after changing apps/:  python scripts/build_apps.py
"""
from __future__ import annotations

import io
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
APPS = ROOT / "apps"
GET = ROOT / "docs" / "get"
CHROME_FILES = ["manifest.json", "background.js", "panel.html", "panel.css", "panel.js"]
CHROME_FOLDER = "Chat Computer"


def chrome_zip() -> bytes:
    """The extension's files, under one folder, with fixed dates: the same zip for the same files."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        for name in CHROME_FILES:
            info = zipfile.ZipInfo(f"{CHROME_FOLDER}/{name}", date_time=(2026, 1, 1, 0, 0, 0))
            info.external_attr = 0o644 << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, (APPS / "chrome" / name).read_bytes())
    return buf.getvalue()


def files() -> dict[Path, bytes]:
    return {GET / "Chat-Computer-Chrome.zip": chrome_zip()}


def main() -> None:
    for path, data in files().items():
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists() or path.read_bytes() != data:
            path.write_bytes(data)
            print("wrote", path.relative_to(ROOT))


if __name__ == "__main__":
    main()
