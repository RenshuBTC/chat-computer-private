"""Logging, the action journal (what Chat Computer did) and undo.

Every change Chat Computer makes that can be taken back is recorded with enough information to reverse it:
overwritten files are copied to ~/ChatComputer/undo first, moves remember both ends, trashed files remember where
they came from, and a setting changed remembers how to put it back (the tool call that does). Each record names the
tool call that made it, so that one action's changes can be taken back together (Revert, in Actions: done.py), and
"undo that" walks the file changes backwards (undo_last). Taking a change back is recorded the same way, so it can be
taken back too, and it never destroys anything: a file it would overwrite is copied first, one it would remove goes to
the trash.
"""
from __future__ import annotations

import contextvars
import glob
import json
import logging
import logging.handlers
import os
import shutil
import threading
import time
import uuid
from collections import OrderedDict
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


# The tool call making changes now when it isn't one of the model's (a revert, set in the task doing it). It comes before
# Journal.call, which the permission gate sets just before each of the model's tool calls runs (they run one at a time).
CALL: contextvars.ContextVar[str | None] = contextvars.ContextVar("chatcomputer_journal_call", default=None)
# The change being taken back now (by a revert): what's recorded meanwhile takes it back, and taking that back in turn
# puts it back.
TAKING_BACK: contextvars.ContextVar[str | None] = contextvars.ContextVar("chatcomputer_taking_back", default=None)
# The kinds of record that can be taken back: a file changed, and anything else (a setting, the memory...).
UNDOABLE = ("file_change", "change")
CALLS_KEPT = 300                # tool calls whose changes are kept in memory, for Actions to link to


class CantUndo(RuntimeError):
    """Why a change can't be taken back, in words for the person."""


def undone_ids(entries: list[dict[str, Any]]) -> set[str]:
    """The changes taken back (and not put back since: a revert reverted), from the journal's records in order."""
    undone: set[str] = set()
    for e in entries:
        if e.get("kind") == "undo":
            if e.get("undoes"):
                undone.add(e["undoes"])
            if e.get("redoes"):
                undone.discard(e["redoes"])
    return undone


class Journal:
    def __init__(self, paths: Paths) -> None:
        self.paths = paths
        self.lock = threading.Lock()
        # The model's tool call running now (its id), set by the permission gate.
        self.call: str | None = None
        # Each recent tool call's changes that can be taken back (record ids, oldest first).
        self._calls: OrderedDict[str, list[str]] = OrderedDict()

    # ------------------------------------------------------------------ records
    def record(self, kind: str, summary: str, **data: Any) -> dict[str, Any]:
        entry = {"id": uuid.uuid4().hex[:12], "ts": time.time(), "kind": kind, "summary": summary,
                 **redact(data)}
        call = CALL.get() or self.call
        if call:
            entry["call"] = call
        back = TAKING_BACK.get()
        if back and kind in UNDOABLE and "inverse_of" not in entry:
            entry["by_undo"], entry["inverse_of"] = True, back
        with self.lock:
            self.paths.journal.parent.mkdir(parents=True, exist_ok=True)
            with open(self.paths.journal, "a", encoding="utf-8") as f:
                f.write(json.dumps(entry, ensure_ascii=False) + "\n")
            self._rotate()
            if call and kind in UNDOABLE and entry.get("undo"):
                self._calls.setdefault(call, []).append(entry["id"])
                self._calls.move_to_end(call)
                while len(self._calls) > CALLS_KEPT:
                    self._calls.popitem(last=False)
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

    def everything(self) -> list[dict[str, Any]]:
        """Every record the journal still has (it keeps about the last 5,000), oldest first."""
        try:
            lines = self.paths.journal.read_text(encoding="utf-8").splitlines()
        except OSError:
            return []
        out = []
        for line in lines:
            try:
                e = json.loads(line)
            except ValueError:
                continue           # a line cut short when the power went off
            if isinstance(e, dict) and e.get("id"):
                out.append(e)
        return out

    def changes_of(self, call: str | None) -> list[str]:
        """The changes tool call `call` made that can be taken back: their records' ids, oldest first."""
        with self.lock:
            return list(self._calls.get(call or "", []))

    def note_undone(self, e: dict[str, Any], said: str) -> None:
        """Change `e` was taken back (`said`: how). Taking back a change that took another back puts that one back."""
        extra = {"redoes": e["inverse_of"]} if e.get("inverse_of") else {}
        self.record("undo", said, undoes=e["id"], **extra)

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

    def record_overwrite(self, path: str, backup: str | None, how: str, **extra: Any) -> None:
        self.record("file_change", f"{how} {path}", undo={"type": "restore", "path": path, "backup": backup}, **extra)

    def record_move(self, src: str, dst: str, backup_of_dst: str | None = None, **extra: Any) -> None:
        self.record("file_change", f"moved {src} to {dst}",
                    undo={"type": "move_back", "src": src, "dst": dst, "backup": backup_of_dst}, **extra)

    def record_copy(self, dst: str, backup_of_dst: str | None = None, merged: bool = False, **extra: Any) -> None:
        """`merged`: a folder was copied into a folder of the same name that was already there."""
        self.record("file_change", f"copied to {dst}",
                    undo={"type": "remove_copy", "dst": dst, "backup": backup_of_dst, "merged": merged}, **extra)

    def record_trash(self, original: str, trashed: str | None, **extra: Any) -> None:
        self.record("file_change", f"moved {original} to the trash",
                    undo={"type": "untrash", "original": original, "trashed": str(trashed) if trashed else None}, **extra)

    def record_mkdir(self, path: str, **extra: Any) -> None:
        self.record("file_change", f"made folder {path}", undo={"type": "rmdir", "path": path}, **extra)

    def record_change(self, summary: str, undo: dict[str, Any], **extra: Any) -> None:
        """A change that isn't a file's, and how to take it back: {"type": "tool", "tool": name, "input": {...}} (a
        tool call that puts it back), or one the controller knows ("memory_forget", "memory_restore")."""
        self.record("change", summary, undo=undo, **extra)

    def undo_last(self, count: int = 1) -> list[str]:
        """Reverse the last `count` file changes that haven't been undone yet (not the ones that undid others)."""
        done: list[str] = []
        entries = self.entries(limit=500)
        # (Not again one it couldn't: the next "undo" goes on to the one before.)
        skip = undone_ids(entries) | {e["failed"] for e in entries if e.get("kind") == "undo" and e.get("failed")}
        candidates = [e for e in reversed(entries)
                      if e.get("kind") == "file_change" and not e.get("by_undo") and e["id"] not in skip]
        for e in candidates[:count]:
            try:
                msg = self.undo_file(e)
                self.note_undone(e, msg)
            except Exception as exc:  # report and continue with the next one
                msg = f"couldn't undo '{e.get('summary')}': {exc}"
                self.record("undo", msg, failed=e["id"])
            done.append(msg)
        return done

    def undo_file(self, e: dict[str, Any]) -> str:
        """Takes back one file change (a record from record_*), keeping everything: a file it would overwrite is
        copied first, one it would remove goes to the trash. What it does is recorded too, marked as taking `e` back,
        so it can be taken back in turn. Says what it did; raises CantUndo (or OSError) when it can't."""
        u = e.get("undo") or {}
        mark = {"by_undo": True, "inverse_of": e.get("id")}
        t = u.get("type")
        if t == "restore":
            path, backup = str(u.get("path") or ""), u.get("backup")
            if backup:
                if not os.path.exists(backup):
                    raise CantUndo(f"the earlier version of {os.path.basename(path)} isn't kept any more")
                now = self.backup(path) if os.path.isfile(path) else None
                os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
                shutil.copy2(backup, path)
                self.record_overwrite(path, now, "restored", **mark)
                return f"restored the earlier version of {path}"
            if not os.path.lexists(path):
                raise CantUndo(f"nothing to restore for {path}")
            # It was a new file. To the trash rather than deleted: it may have changed since.
            trashed = move_to_trash(path)
            self.record_trash(path, trashed, **mark)
            return f"moved {path}, which didn't exist before, to the trash"
        if t == "move_back":
            src, dst = str(u.get("src") or ""), str(u.get("dst") or "")
            if not os.path.lexists(dst):
                raise CantUndo(f"{dst} is no longer there, so it can't be moved back")
            if os.path.lexists(src):
                raise CantUndo(f"something is already at {src}, so I left {dst} where it is")
            os.makedirs(os.path.dirname(src) or ".", exist_ok=True)
            shutil.move(dst, src)
            self.record_move(dst, src, None, **mark)
            backup = u.get("backup")
            if backup and os.path.exists(backup):
                # What the move replaced, back where it was (taking this back: to the trash, then moving again).
                shutil.copy2(backup, dst)
                self.record_overwrite(dst, None, "restored", **mark)
            return f"moved {dst} back to {src}"
        if t == "remove_copy":
            dst = str(u.get("dst") or "")
            if u.get("merged"):
                # What was in that folder before can't be told apart from the copy: deleting it would lose both.
                raise CantUndo(f"the copy went into the folder {dst}, which was already there, so I left it as it is")
            backup = u.get("backup")
            if backup:
                if not os.path.exists(backup):
                    raise CantUndo(f"what was at {dst} before isn't kept any more")
                now = self.backup(dst) if os.path.isfile(dst) else None
                shutil.copy2(backup, dst)
                self.record_overwrite(dst, now, "restored", **mark)
                return f"restored the earlier {dst}"
            if not os.path.lexists(dst):
                raise CantUndo(f"the copy at {dst} is no longer there")
            trashed = move_to_trash(dst)     # not deleted: it may have changed since
            self.record_trash(dst, trashed, **mark)
            return f"moved the copy at {dst} to the trash"
        if t == "untrash":
            original = str(u.get("original") or "")
            said = untrash(original, u.get("trashed"))
            if not said.startswith("restored"):
                raise CantUndo(said)
            self.record("file_change", f"put back {original}", undo={"type": "trash", "path": original}, **mark)
            return said
        if t == "trash":
            path = str(u.get("path") or "")
            if not os.path.lexists(path):
                raise CantUndo(f"{path} is no longer there")
            trashed = move_to_trash(path)
            self.record_trash(path, trashed, **mark)
            return f"moved {path} to the trash again"
        if t == "rmdir":
            p = str(u.get("path") or "")
            if not os.path.isdir(p):
                raise CantUndo(f"the folder {p} is no longer there")
            try:
                os.rmdir(p)
            except OSError:
                raise CantUndo(f"left {p} because it isn't empty any more") from None
            self.record("file_change", f"removed folder {p}", undo={"type": "mkdir", "path": p}, **mark)
            return f"removed the empty folder {p}"
        if t == "mkdir":
            p = str(u.get("path") or "")
            if os.path.lexists(p):
                raise CantUndo(f"something is already at {p}")
            os.makedirs(p)
            self.record_mkdir(p, **mark)
            return f"made the folder {p} again"
        raise CantUndo("there's nothing to undo")


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
