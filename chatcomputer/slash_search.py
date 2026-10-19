""" "/" in the message box: it searches this computer's files (Chat Computer's index of them, files_index.py) by name, as
the words after it are typed, and the person opens one between the sidebars. The page draws the list
(ui/assets/chat.js); this is what Chat Computer answers with. Chat Computer's private folders are never in it.
"""
from __future__ import annotations

import os
import time
from typing import Any

from .brain.permissions import is_private
from .config import Paths

SEARCH_MAX = 8                  # matches shown


def when_text(ts: float, now: float | None = None) -> str:
    """A file's time as a person would say it: 2:03 PM today, Oct 3 this year, Oct 3, 2024 before."""
    now = time.time() if now is None else now
    t, n = time.localtime(ts), time.localtime(now)
    if t[:3] == n[:3]:
        return time.strftime("%I:%M %p", t).lstrip("0")
    if t.tm_year == n.tm_year:
        return time.strftime("%b ", t) + str(t.tm_mday)
    return time.strftime("%b ", t) + f"{t.tm_mday}, {t.tm_year}"


def search(index: Any, query: str, paths: Paths, limit: int = SEARCH_MAX) -> tuple[list[dict[str, Any]], str]:
    """ "/" in the message box: the files on this computer that best match (the most recently changed ones for no
    words), and a line to show when there are none."""
    if index is None:
        return [], "Searching this computer's files is off."
    try:
        rows = index.search(query, limit=limit * 2)
    except Exception:
        rows = []
    out = []
    for r in rows:
        path = r.get("path") or ""
        if not path or is_private(paths, path):
            continue
        try:
            mtime = os.stat(path).st_mtime
        except OSError:
            continue            # gone since the index last looked
        out.append({"name": os.path.basename(path), "path": path, "kind": r.get("kind") or "other",
                    "where": index.friendly(os.path.dirname(path)), "when": when_text(mtime)})
        if len(out) == limit:
            break
    if out:
        return out, ""
    try:
        empty = index.count() == 0
    except Exception:
        empty = False
    if empty:
        return [], "I'm still going through this computer's files. Try again in a minute."
    return [], f"Nothing on this computer matches “{query}”." if query else "No files yet."
