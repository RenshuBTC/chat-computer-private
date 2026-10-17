"""Updating ClaudeOS from the folder on Windows' drive that ClaudeOS Setup ran from.

ClaudeOS Setup notes where that folder is when it installs ClaudeOS (/etc/claudeos/update-source, a path on
Windows' drive; the file is root's, so nothing ClaudeOS runs can point updates elsewhere). When a newer
ClaudeOS is put in that folder (downloaded again over the old one), ClaudeOS sees it when it starts and asks
on the screen whether to update. The root helper then copies in only the parts that run as the person
(claudeos-root update-app; never anything that runs as root, and never a Python package it would have to
install), and ClaudeOS starts again on the new version. If that version keeps failing to start, the session
goes back to the one before by itself (update-undo); once it has run for a little while, it's kept
(update-done)."""
from __future__ import annotations

import filecmp
import os
import re
from dataclasses import dataclass
from pathlib import Path

UPDATE_SOURCE = Path("/etc/claudeos/update-source")
UPDATE_PENDING = Path("/var/lib/claudeos/update-pending")   # an update not yet known to start well
APP = Path("/opt/claudeos/app")
WINDOWS_MOUNT = Path("/mnt/windows")
# What an update replaces: everything else changes only when ClaudeOS is installed again (claudeos-root).
PARTS = ("claudeos", "linux/session")
HEALTHY_AFTER_S = 45.0    # a new version that has run this long is kept


@dataclass
class Offer:
    source: Path              # the folder with the other version
    changed: int              # how many of its files differ from the ones installed
    version: str | None       # its version number, if it says
    needs_setup: bool         # it needs other Python packages: only ClaudeOS Setup can install it


def source_dir(update_source: Path | None = None, mount: Path | None = None) -> Path | None:
    """The folder ClaudeOS Setup ran from, on Windows' drive (it may not be mounted yet). None if Setup noted
    none, or what it noted isn't a plain path below the drive's top."""
    try:
        rel = (update_source or UPDATE_SOURCE).read_text(encoding="utf-8").split("\n", 1)[0].rstrip("\r")
    except (OSError, UnicodeDecodeError):
        return None
    parts = [p for p in rel.split("/") if p]
    if (not parts or rel.startswith("/") or any(p in (".", "..") for p in parts)
            or any(ord(c) < 32 or ord(c) == 127 for c in rel)):
        return None
    return (mount or WINDOWS_MOUNT).joinpath(*parts)


def _files(root: Path) -> dict[str, Path]:
    out: dict[str, Path] = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d != "__pycache__"]
        for name in filenames:
            if not name.endswith((".pyc", ".pyo")):
                p = Path(dirpath, name)
                out[p.relative_to(root).as_posix()] = p
    return out


def changed_files(source: Path, app: Path | None = None, parts: tuple[str, ...] = PARTS) -> int:
    """How many files of `parts` differ between the two versions (new, gone or changed)."""
    app = app or APP
    n = 0
    for part in parts:
        new, old = _files(source / part), _files(app / part)
        for rel in new.keys() | old.keys():
            a, b = new.get(rel), old.get(rel)
            if a is None or b is None or not filecmp.cmp(a, b, shallow=False):
                n += 1
    return n


def version_of(root: Path) -> str | None:
    try:
        text = (root / "claudeos" / "__init__.py").read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return None
    m = re.search(r"""^__version__\s*=\s*["']([^"'\n]{1,40})["']""", text, re.M)
    return m.group(1) if m else None


def check(update_source: Path | None = None, mount: Path | None = None, app: Path | None = None) -> Offer | None:
    """A version of ClaudeOS different from this one in the folder ClaudeOS Setup ran from, if there is one.
    Reads the files of both: run it in a worker thread."""
    app = app or APP
    src = source_dir(update_source, mount)
    if src is None or not app.is_dir():
        return None
    try:
        if not (src / "claudeos" / "__init__.py").is_file():
            return None
        n = changed_files(src, app)
    except OSError:
        return None
    if n == 0:
        return None
    try:
        same_packages = filecmp.cmp(src / "requirements-linux.txt", app / "requirements-linux.txt", shallow=False)
    except OSError:
        same_packages = False
    return Offer(src, n, version_of(src), not same_packages)


def source_there(update_source: Path | None = None, mount: Path | None = None) -> bool:
    """Whether the folder ClaudeOS Setup ran from can be read now (Windows' drive is mounted while ClaudeOS
    starts, so at first it may not be)."""
    src = source_dir(update_source, mount)
    try:
        return src is not None and (src / "claudeos" / "__init__.py").is_file()
    except OSError:
        return False


def pending(marker: Path | None = None) -> bool:
    """Whether this version was put in place by an update that hasn't yet been seen to start well."""
    return (marker or UPDATE_PENDING).exists()
