"""A searchable index of your files, including everything in your Windows user folder.

This is how ClaudeOS "remembers" what's already on the PC: names, folders and the text inside
documents are indexed in the background (SQLite full-text search), so "find the tax PDF I
saved last year" is instant. Nothing leaves the machine.
"""
from __future__ import annotations

import logging
import os
import re
import sqlite3
import threading
import time
from datetime import datetime
from pathlib import Path
from typing import Any, Iterable

log = logging.getLogger("claudeos.index")

KINDS = {
    "document": {".pdf", ".doc", ".docx", ".odt", ".rtf", ".txt", ".md", ".pages", ".tex"},
    "spreadsheet": {".xls", ".xlsx", ".xlsm", ".csv", ".tsv", ".ods", ".numbers"},
    "presentation": {".ppt", ".pptx", ".odp", ".key"},
    "image": {".jpg", ".jpeg", ".png", ".gif", ".webp", ".heic", ".bmp", ".tif", ".tiff", ".svg", ".raw", ".cr2", ".nef", ".dng"},
    "video": {".mp4", ".mov", ".mkv", ".avi", ".webm", ".m4v", ".wmv", ".flv"},
    "audio": {".mp3", ".m4a", ".wav", ".flac", ".aac", ".ogg", ".opus", ".wma"},
    "archive": {".zip", ".rar", ".7z", ".tar", ".gz", ".tgz", ".bz2", ".xz"},
    "code": {".py", ".js", ".ts", ".tsx", ".jsx", ".html", ".css", ".json", ".java", ".c", ".cpp", ".h", ".rs", ".go",
             ".ipynb", ".sh", ".ps1", ".sql", ".rb", ".php", ".yaml", ".yml", ".toml", ".xml"},
    "ebook": {".epub", ".mobi", ".azw3"},
    "program": {".exe", ".msi", ".appimage", ".deb"},
}
EXT_KIND = {ext: kind for kind, exts in KINDS.items() for ext in exts}
TEXT_EXTS = {".txt", ".md", ".csv", ".tsv", ".tex", ".log", ".rtf"} | KINDS["code"]
SKIP_DIRS = {"node_modules", "__pycache__", ".git", ".venv", "venv", "site-packages", "$RECYCLE.BIN",
             "System Volume Information", "AppData", "Application Data", "Local Settings", "snap", ".cache",
             ".local", ".config", ".engine", "undo", "web", "index", "models", "logs", "Temp", "tmp",
             "OneDriveTemp", ".Trash-1000"}
# Folders that are really one thing (an app, the Photos or Music library, e.g. copied over from macOS): not
# walked into.
PACKAGE_SUFFIXES = (".app", ".photoslibrary", ".musiclibrary", ".tvlibrary", ".imovielibrary", ".fcpbundle",
                    ".bundle", ".framework", ".plugin")
# Sign-ins ClaudeOS Setup may leave on the Windows drive (normally removed at start): never indexed.
SECRET_FILES = {"claude_token", "claude_login.json"}
MAX_FILES = 2_000_000
MAX_CONTENT_BYTES = 15_000_000
CONTENT_CHARS = 20_000


def kind_of(path: str) -> str:
    return EXT_KIND.get(os.path.splitext(path)[1].lower(), "other")


def words_of(path: str) -> str:
    """Searchable words from a path: folders and the file name split on _ - . and camelCase."""
    p = re.sub(r"([a-z])([A-Z])", r"\1 \2", path)
    return " ".join(w for w in re.split(r"[\\/_\-.\s()\[\],]+", p) if w)


def extract_text(path: str) -> str:
    """Best-effort text from a document, limited to CONTENT_CHARS."""
    ext = os.path.splitext(path)[1].lower()
    try:
        if ext == ".pdf":
            from pypdf import PdfReader
            r = PdfReader(path)
            out = []
            for page in r.pages[:40]:
                out.append(page.extract_text() or "")
                if sum(map(len, out)) > CONTENT_CHARS:
                    break
            return "\n".join(out)[:CONTENT_CHARS]
        if ext == ".docx":
            import docx
            d = docx.Document(path)
            return "\n".join(p.text for p in d.paragraphs)[:CONTENT_CHARS]
        if ext in (".xlsx", ".xlsm"):
            import openpyxl
            wb = openpyxl.load_workbook(path, read_only=True, data_only=True)
            out = []
            for ws in list(wb.worksheets)[:3]:
                out.append(f"[{ws.title}]")
                for row in ws.iter_rows(max_row=300, values_only=True):
                    out.append(" ".join(str(c) for c in row if c is not None))
                    if sum(map(len, out)) > CONTENT_CHARS:
                        break
            wb.close()
            return "\n".join(out)[:CONTENT_CHARS]
        if ext == ".pptx":
            from pptx import Presentation
            prs = Presentation(path)
            out = []
            for i, slide in enumerate(prs.slides, 1):
                out.append(f"[slide {i}]")
                for shp in slide.shapes:
                    if getattr(shp, "has_text_frame", False):
                        out.append(shp.text_frame.text)
            return "\n".join(out)[:CONTENT_CHARS]
        if ext in TEXT_EXTS:
            with open(path, "r", encoding="utf-8", errors="replace") as f:
                return f.read(CONTENT_CHARS)
    except Exception as exc:  # unreadable, encrypted, OneDrive placeholder...
        log.debug("no text from %s: %s", path, exc)
    return ""


def parse_date(s: str | None) -> float | None:
    if not s:
        return None
    s = s.strip()
    m = re.fullmatch(r"(\d+)\s*(d|day|days|w|week|weeks|m|month|months|y|year|years)", s)
    if m:
        n, unit = int(m.group(1)), m.group(2)[0]
        return time.time() - n * {"d": 1, "w": 7, "m": 30.4, "y": 365.25}[unit] * 86400
    for fmt in ("%Y-%m-%d", "%Y-%m", "%Y"):
        try:
            return datetime.strptime(s, fmt).timestamp()
        except ValueError:
            continue
    return None


class FileIndex:
    def __init__(self, db_path: Path, roots: list[tuple[str, str]], windows_root: str | None = None) -> None:
        self.db_path = Path(db_path)
        self.roots = [(label, os.path.realpath(p)) for label, p in roots if p]
        self.windows_root = os.path.realpath(windows_root) if windows_root else None
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self.status: dict[str, Any] = {"state": "idle", "files": 0, "last_crawl": None}
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        with self._connect() as db:
            db.executescript("""
                PRAGMA journal_mode=WAL;
                CREATE TABLE IF NOT EXISTS files(
                    id INTEGER PRIMARY KEY, path TEXT UNIQUE, name TEXT, dir TEXT, ext TEXT, kind TEXT,
                    size INTEGER, mtime REAL, seen REAL, root TEXT, content_done INTEGER DEFAULT 0);
                CREATE INDEX IF NOT EXISTS files_mtime ON files(mtime);
                CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(name, words, content,
                    tokenize='unicode61 remove_diacritics 2');
            """)

    def _connect(self) -> sqlite3.Connection:
        db = sqlite3.connect(str(self.db_path), timeout=30)
        db.row_factory = sqlite3.Row
        return db

    # ------------------------------------------------------------------ crawling
    def start(self, interval_s: float = 1800) -> None:
        if self._thread and self._thread.is_alive():
            return
        def loop() -> None:
            while not self._stop.is_set():
                try:
                    self.crawl_once()
                    self.extract_contents()
                except Exception:
                    log.exception("file index crawl failed")
                self._stop.wait(interval_s)
        self._thread = threading.Thread(target=loop, name="file-index", daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()

    def _walk(self, root: str, skip_top: set[str] | frozenset[str] = frozenset()) -> Iterable[os.DirEntry]:
        stack = [(root, 0)]
        while stack and not self._stop.is_set():
            d, depth = stack.pop()
            try:
                with os.scandir(d) as it:
                    for e in it:
                        name = e.name
                        if name.startswith(".") or name in SKIP_DIRS or name in SECRET_FILES:
                            continue
                        if depth == 0 and name in skip_top:
                            continue
                        try:
                            if e.is_dir(follow_symlinks=False):
                                if depth < 25 and not name.endswith(PACKAGE_SUFFIXES):
                                    stack.append((e.path, depth + 1))
                            elif e.is_file(follow_symlinks=False):
                                yield e
                        except OSError:
                            continue
            except OSError:
                continue

    def crawl_once(self) -> int:
        self.status["state"] = "scanning"
        start = time.time()
        count = 0
        db = self._connect()
        try:
            for label, root in self.roots:
                if not os.path.isdir(root):
                    continue
                batch = 0
                for e in self._walk(root):
                    if count >= MAX_FILES:
                        break
                    try:
                        st = e.stat(follow_symlinks=False)
                    except OSError:
                        continue
                    count += 1
                    row = db.execute("SELECT id, mtime, size FROM files WHERE path=?", (e.path,)).fetchone()
                    if row and abs(row["mtime"] - st.st_mtime) < 1 and row["size"] == st.st_size:
                        db.execute("UPDATE files SET seen=? WHERE id=?", (start, row["id"]))
                    else:
                        ext = os.path.splitext(e.name)[1].lower()
                        vals = (e.path, e.name, os.path.dirname(e.path), ext, kind_of(e.name), st.st_size,
                                st.st_mtime, start, label)
                        if row:
                            db.execute("UPDATE files SET path=?, name=?, dir=?, ext=?, kind=?, size=?, mtime=?, seen=?, root=?,"
                                       " content_done=0 WHERE id=?", vals + (row["id"],))
                            fid = row["id"]
                            db.execute("DELETE FROM fts WHERE rowid=?", (fid,))
                        else:
                            cur = db.execute("INSERT INTO files(path, name, dir, ext, kind, size, mtime, seen, root)"
                                             " VALUES(?,?,?,?,?,?,?,?,?)", vals)
                            fid = cur.lastrowid
                        rel = os.path.relpath(e.path, root)
                        db.execute("INSERT INTO fts(rowid, name, words, content) VALUES(?,?,?,?)",
                                   (fid, e.name, words_of(f"{label}/{rel}"), ""))
                    batch += 1
                    if batch >= 2000:
                        db.commit()
                        batch = 0
                        time.sleep(0.01)
                db.commit()
                # Files that disappeared since the last scan of this root.
                gone = [r["id"] for r in db.execute("SELECT id FROM files WHERE root=? AND seen<?", (label, start))]
                for fid in gone:
                    db.execute("DELETE FROM fts WHERE rowid=?", (fid,))
                    db.execute("DELETE FROM files WHERE id=?", (fid,))
                db.commit()
        finally:
            db.close()
        self.status.update(state="idle", files=count, last_crawl=time.time())
        return count

    def extract_contents(self, limit: int = 5000) -> int:
        self.status["state"] = "reading documents"
        db = self._connect()
        done = 0
        try:
            rows = db.execute(
                "SELECT id, path, name, root FROM files WHERE content_done=0 AND size<? AND kind IN"
                " ('document','spreadsheet','presentation','code') ORDER BY mtime DESC LIMIT ?",
                (MAX_CONTENT_BYTES, limit)).fetchall()
            for r in rows:
                if self._stop.is_set():
                    break
                text = extract_text(r["path"])
                fts = db.execute("SELECT words FROM fts WHERE rowid=?", (r["id"],)).fetchone()
                words = fts["words"] if fts else words_of(r["path"])
                db.execute("DELETE FROM fts WHERE rowid=?", (r["id"],))
                db.execute("INSERT INTO fts(rowid, name, words, content) VALUES(?,?,?,?)", (r["id"], r["name"], words, text))
                db.execute("UPDATE files SET content_done=1 WHERE id=?", (r["id"],))
                done += 1
                if done % 50 == 0:
                    db.commit()
                time.sleep(0.005)
            db.commit()
        finally:
            db.close()
        self.status["state"] = "idle"
        return done

    # ------------------------------------------------------------------ searching
    def friendly(self, path: str) -> str:
        if self.windows_root and path.startswith(self.windows_root + os.sep):
            rel = path[len(self.windows_root) + 1:]
            return "C:\\" + rel.replace("/", "\\") + " (Windows)"
        home = os.path.expanduser("~")
        if path.startswith(home + os.sep):
            return "~/" + path[len(home) + 1:]
        return path

    def search(self, query: str = "", kind: str | None = None, after: str | None = None,
               before: str | None = None, folder: str | None = None, limit: int = 20) -> list[dict[str, Any]]:
        limit = max(1, min(int(limit or 20), 100))
        where, args = [], []
        if kind:
            where.append("f.kind=?")
            args.append(kind)
        a, b = parse_date(after), parse_date(before)
        if a:
            where.append("f.mtime>=?")
            args.append(a)
        if b:
            where.append("f.mtime<?")
            args.append(b)
        if folder:
            where.append("f.path LIKE ?")
            args.append(os.path.realpath(os.path.expanduser(folder)).rstrip("/") + "/%")
        db = self._connect()
        try:
            tokens = [t for t in re.findall(r"\w+", query.lower()) if t]
            if tokens:
                match = " ".join(f'"{t}"*' for t in tokens)
                sql = ("SELECT f.*, snippet(fts, 2, '«', '»', '…', 10) AS snip, bm25(fts, 10.0, 4.0, 1.0) AS score"
                       " FROM fts JOIN files f ON f.id = fts.rowid WHERE fts MATCH ?"
                       + (" AND " + " AND ".join(where) if where else "") + " ORDER BY score LIMIT ?")
                rows = db.execute(sql, [match] + args + [limit * 3]).fetchall()
                now = time.time()
                # Blend relevance with recency: newer files win ties.
                scored = sorted(rows, key=lambda r: r["score"] - 0.5 * max(0.0, 1 - (now - r["mtime"]) / (365 * 86400)))
                rows = scored[:limit]
            else:
                sql = ("SELECT f.*, '' AS snip FROM files f" + (" WHERE " + " AND ".join(where) if where else "")
                       + " ORDER BY f.mtime DESC LIMIT ?")
                rows = db.execute(sql, args + [limit]).fetchall()
        finally:
            db.close()
        out = []
        for r in rows:
            out.append({"path": r["path"], "where": self.friendly(r["path"]), "kind": r["kind"],
                        "size_kb": round(r["size"] / 1024), "modified": time.strftime("%Y-%m-%d %H:%M", time.localtime(r["mtime"])),
                        **({"match": r["snip"]} if r["snip"] else {})})
        return out

    def count(self) -> int:
        db = self._connect()
        try:
            return db.execute("SELECT COUNT(*) FROM files").fetchone()[0]
        finally:
            db.close()
