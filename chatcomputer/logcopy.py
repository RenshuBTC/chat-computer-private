"""A copy of Chat Computer's logs on Windows' drive, in the Chat Computer folder Chat Computer Setup made there
(C:\\Users\\<you>\\AppData\\Local\\ChatComputer\\from-chatcomputer). Chat Computer's own drive can't be opened from Windows,
so when something doesn't work on a real PC, this is how what happened can be read from Windows.

Only the logs: never the conversation, the memory or a sign-in, and anything in the logs that looks like a
sign-in or a password is blanked out first."""
from __future__ import annotations

import json
import logging
import os
import re
import time
from pathlib import Path
from typing import Any

log = logging.getLogger("chatcomputer.logcopy")

# The end of each log is enough to see what happened lately.
TAIL_BYTES = 1_500_000
_SECRETS = [
    (re.compile(r"sk-ant-[A-Za-z0-9_\-]+"), "sk-ant-(hidden)"),
    (re.compile(r"(?i)\b(bearer)\s+[A-Za-z0-9._\-]{16,}"), r"\1 (hidden)"),
    (re.compile(r"(?i)(\"?(?:access_?token|refresh_?token|password|secret|api_?key)\"?\s*[:=]\s*\"?)[^\s\",}]+"),
     r"\1(hidden)"),
]


def scrub(text: str) -> str:
    for pattern, replacement in _SECRETS:
        text = pattern.sub(replacement, text)
    return text


def _tail(path: Path, limit: int | None = None) -> str:
    limit = limit or TAIL_BYTES
    try:
        with open(path, "rb") as f:
            size = f.seek(0, os.SEEK_END)
            f.seek(max(0, size - limit))
            data = f.read()
    except OSError:
        return ""
    text = data.decode("utf-8", errors="replace")
    if size > limit:
        text = text.split("\n", 1)[-1]      # the first line read may start partway through
    return text


def target(windows_home: str | None) -> Path | None:
    """Where the copy goes, if Windows' drive is there and Chat Computer Setup's folder is on it."""
    if not windows_home:
        return None
    base = Path(windows_home) / "AppData" / "Local" / "ChatComputer"
    return base / "from-chatcomputer" if base.is_dir() else None


def copy_logs(logs: Path, windows_home: str | None, info: dict[str, Any] | None = None) -> Path | None:
    """Copies the logs' ends (and `info`, a few facts about this PC and Chat Computer) to Windows' drive. Returns where,
    or None if Windows' drive isn't there or can't be written."""
    dest = target(windows_home)
    if dest is None:
        return None
    try:
        dest.mkdir(exist_ok=True)
        for name in ("chatcomputer.log", "session.log"):
            text = _tail(logs / name)
            if text:
                # .txt, so Windows opens it with Notepad; Windows line endings, so Notepad shows the lines.
                (dest / (name + ".txt")).write_text(scrub(text).replace("\r\n", "\n").replace("\n", "\r\n"),
                                                    encoding="utf-8")
        facts = {"copied": time.strftime("%Y-%m-%d %H:%M:%S %z"), **(info or {})}
        (dest / "info.json").write_text(scrub(json.dumps(facts, indent=2, ensure_ascii=False, default=str)),
                                        encoding="utf-8")
    except OSError as exc:
        log.info("couldn't copy the logs to Windows' drive: %s", exc)
        return None
    return dest
