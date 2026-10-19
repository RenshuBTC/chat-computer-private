"""Logging, the action journal (what Chat Computer did) and undo for file changes.

Every file change Chat Computer makes is recorded with enough information to reverse it:
overwritten files are copied to ~/ChatComputer/undo first, moves remember both ends, and
trashed files remember where they came from. "Undo that" walks this journal backwards.
"""
from __future__ import annotations

import glob
import json
import logging
import logging.handlers
import os
import shutil
import threading
import time
import uuid
from pathlib import Path
from typing import Any

from .config import Paths

log = logging.getLogger("chatcomputer")

SECRET_KEYS = {"password", "passphrase", "token", "secret", "psk"}


def setup_logging(paths: Paths, verbose: bool = False) -> None:
    paths.logs.mkdir(parents=True, exist_ok=True)
    root = logging.getLogger()
    root.setLevel(logging.DEBUG if verbose else logging.INFO)
    fmt = logging.Formatter("%(asctime)s %(levelname)s %(name)s: %(message)s")
    fh = logging.handlers.RotatingFileHandler(paths.logs / "chatcomputer.log", maxBytes=2_000_000,
                                              backupCount=3, encoding="utf-8")
    fh.setFormatter(fmt)
    root.addHandler(fh)
    sh = logging.StreamHandler()
    sh.setFormatter(fmt)
    sh.setLevel(logging.WARNING if not verbose else logging.DEBUG)
    root.addHandler(sh)


def redact(obj: Any) -> Any:
    """Copy of a tool input with passwords and tokens blanked out."""
    if isinstance(obj, dict):
        return {k: ("•••" if any(s in k.lower() for s in SECRET_KEYS) else redact(v)) for k, v in obj.items()}
    if isinstance(obj, list):
        return [redact(v) for v in obj]
    if isinstance(obj, str) and obj.startswith("sk-ant-"):
        return "•••"
    return obj


class Journal:
    def __init__(self, paths: Paths) -> None:
        self.paths = paths
        self.lock = threading.Lock()

    # ------------------------------------------------------------------ records
    def record(self, kind: str, summary: str, **data: Any) -> dict[str, Any]:
        entry = {"id": uuid.uuid4().hex[:12], "ts": time.time(), "kind": kind, "summary": summary,
                 **redact(data)}
        with self.lock:
            self.paths.journal.parent.mkdir(parents=True, exist_ok=True)
            with open(self.paths.journal, "a", encoding="utf-8") as f:
                f.write(json.dumps(entry, ensure_ascii=False) + "\n")
            self._rotate()
        return entry

    def _rotate(self) -> None:
        try:
            if self.paths.journal.stat().st_size > 5_000_000:
                lines = self.paths.journal.read_text(encoding="utf-8").splitlines()[-5000:]
                self.paths.journal.write_text("\n".join(lines) + "\n", encoding="utf-8")
        except OSError:
            pass

    def entries(self, limit: int = 50) -> list[dict[str, Any]]:
        try:
            lines = self.paths.journal.read_text(encoding="utf-8").splitlines()
        except OSError:
            return []
        out = []
        for line in lines[-max(limit, 1) * 4:]:
            try:
                out.append(json.loads(line))
            except ValueError:
                continue
        return out[-limit:]

    # ------------------------------------------------------------------ file changes
    def backup(self, path: str) -> str | None:
        """Copy a file that is about to be overwritten. Returns the backup path."""
        p = Path(path)
        if not p.is_file():
            return None
        dest_dir = self.paths.undo / time.strftime("%Y%m%d") / uuid.uuid4().hex[:8]
        dest_dir.mkdir(parents=True, exist_ok=True)
        dest = dest_dir / p.name
        shutil.copy2(p, dest)
        self._prune_backups()
        return str(dest)

    def _prune_backups(self, keep_days: int = 30) -> None:
        cutoff = time.time() - keep_days * 86400
        try:
            for day in self.paths.undo.iterdir():
                if day.is_dir() and day.stat().st_mtime < cutoff:
                    shutil.rmtree(day, ignore_errors=True)
        except OSError:
            pass

    def record_overwrite(self, path: str, backup: str | None, how: str) -> None:
        self.record("file_change", f"{how} {path}", undo={"type": "restore", "path": path, "backup": backup})

    def record_move(self, src: str, dst: str, backup_of_dst: str | None = None) -> None:
        self.record("file_change", f"moved {src} to {dst}",
                    undo={"type": "move_back", "src": src, "dst": dst, "backup": backup_of_dst})

    def record_copy(self, dst: str, backup_of_dst: str | None = None, merged: bool = False) -> None:
        """`merged`: a folder was copied into a folder of the same name that was already there."""
        self.record("file_change", f"copied to {dst}",
                    undo={"type": "remove_copy", "dst": dst, "backup": backup_of_dst, "merged": merged})

    def record_trash(self, original: str, trashed: str | None) -> None:
        self.record("file_change", f"moved {original} to the trash",
                    undo={"type": "untrash", "original": original, "trashed": trashed})

    def record_mkdir(self, path: str) -> None:
        self.record("file_change", f"made folder {path}", undo={"type": "rmdir", "path": path})

    def undo_last(self, count: int = 1) -> list[str]:
        """Reverse the last `count` file changes that haven't been undone yet."""
        done: list[str] = []
        entries = self.entries(limit=500)
        undone_ids = {e.get("undoes") for e in entries if e.get("kind") == "undo"}
        candidates = [e for e in reversed(entries) if e.get("kind") == "file_change" and e["id"] not in undone_ids]
        for e in candidates[:count]:
            try:
                msg = self._apply_undo(e.get("undo") or {})
            except Exception as exc:  # report and continue with the next one
                msg = f"couldn't undo '{e.get('summary')}': {exc}"
            self.record("undo", msg, undoes=e["id"])
            done.append(msg)
        return done

    def _apply_undo(self, u: dict[str, Any]) -> str:
        t = u.get("type")
        if t == "restore":
            path, backup = u["path"], u.get("backup")
            if backup and os.path.exists(backup):
                shutil.copy2(backup, path)
                return f"restored the earlier version of {path}"
            if os.path.exists(path):
                # It was a new file. To the trash rather than deleted: it may have changed since.
                move_to_trash(path)
                return f"moved {path}, which didn't exist before, to the trash"
            return f"nothing to restore for {path}"
        if t == "move_back":
            src, dst = u["src"], u["dst"]
            if not os.path.exists(dst):
                return f"{dst} is no longer there, so it can't be moved back"
            if os.path.exists(src):
                return f"something is already at {src}, so I left {dst} where it is"
            os.makedirs(os.path.dirname(src) or ".", exist_ok=True)
            shutil.move(dst, src)
            if u.get("backup") and os.path.exists(u["backup"]):
                shutil.copy2(u["backup"], dst)
            return f"moved {dst} back to {src}"
        if t == "remove_copy":
            dst = u["dst"]
            if u.get("backup") and os.path.exists(u["backup"]):
                shutil.copy2(u["backup"], dst)
                return f"restored the earlier {dst}"
            if u.get("merged"):
                # What was in that folder before can't be told apart from the copy: deleting it would lose both.
                return f"the copy went into the folder {dst}, which was already there, so I left it as it is"
            if os.path.lexists(dst):
                move_to_trash(dst)     # not deleted: it may have changed since
            return f"moved the copy at {dst} to the trash"
        if t == "untrash":
            return untrash(u["original"], u.get("trashed"))
        if t == "rmdir":
            p = u["path"]
            try:
                os.rmdir(p)
                return f"removed the empty folder {p}"
            except OSError:
                return f"left {p} because it isn't empty any more"
        return "nothing to undo"


def move_to_trash(path: str) -> str | None:
    """Moves a file or folder to the trash. Returns where it is in the trash, for undo (None if unknown)."""
    import send2trash
    send2trash.send2trash(path)
    return find_in_trash(path)


def untrash(original: str, trashed: str | None) -> str:
    """Put a trashed file back (the freedesktop.org trash)."""
    if not trashed:
        return f"couldn't find {original} in the trash"
    if os.path.lexists(original):
        return f"something is already at {original}"
    try:
        os.makedirs(os.path.dirname(original) or ".", exist_ok=True)
        shutil.move(trashed, original)
    except FileNotFoundError:
        return f"couldn't find {original} in the trash"
    except PermissionError as exc:
        return f"couldn't put {original} back: {exc.strerror or exc}"
    info = Path(trashed).parent.parent / "info" / (Path(trashed).name + ".trashinfo")
    try:
        info.unlink()
    except OSError:
        pass
    return f"restored {original} from the trash"


def find_in_trash(original: str) -> str | None:
    """Locate the newest trashed copy of `original` (XDG trash in home or on the same drive)."""
    from urllib.parse import unquote
    candidates: list[tuple[str, int, str]] = []
    homes = [Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")) / "Trash"]
    # Trash folders on other drives: <mountpoint>/.Trash-<uid>
    p = Path(original).resolve()
    for parent in [p] + list(p.parents):
        if os.path.ismount(parent):
            homes.append(parent / f".Trash-{os.getuid()}")
            break
    def look(infos: list[Path], trash: Path) -> None:
        for info in infos:
            try:
                text = info.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            path_line = next((ln for ln in text.splitlines() if ln.startswith("Path=")), "")
            date_line = next((ln for ln in text.splitlines() if ln.startswith("DeletionDate=")), "")
            stored = unquote(path_line[5:])
            if not os.path.isabs(stored):
                stored = str(trash.parent / stored)
            if os.path.normpath(stored) == os.path.normpath(str(p)):
                # DeletionDate is to the second: when the same file was trashed twice in one second, the info
                # written last is the newer.
                try:
                    written = info.stat().st_mtime_ns
                except OSError:
                    written = 0
                candidates.append((date_line, written, str(trash / "files" / info.name[: -len(".trashinfo")])))

    # A trashed file keeps its name, or one made from it: GLib (which send2trash uses when it can) calls a second
    # report.pdf report.2.pdf, send2trash's own way report 1.pdf. Both start with what comes before the first dot,
    # so only those are read; the trash only grows, and reading all of it for each file got slower and slower.
    stem = p.name.split(".", 1)[0]
    for trash in homes:
        info_dir = trash / "info"
        if not info_dir.is_dir():
            continue
        if stem:
            look(list(info_dir.glob(glob.escape(stem) + "*.trashinfo")), trash)
        if not candidates:
            look([i for i in info_dir.glob("*.trashinfo") if not (stem and i.name.startswith(stem))], trash)
    if not candidates:
        return None
    candidates.sort()
    return candidates[-1][2]
