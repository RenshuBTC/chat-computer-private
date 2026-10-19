"""A searchable index of your files, including everything in your Windows user folder.

This is how Chat Computer "remembers" what's already on the PC: names, folders and the text inside
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

log = logging.getLogger("chatcomputer.index")

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
# Folders that only hold programs' own files, wherever they are. (Chat Computer's own folders are skipped by where they
# are, `skip`: a folder of the person's called web or models is theirs.)
SKIP_DIRS = {"node_modules", "__pycache__", ".git", ".venv", "venv", "site-packages", "$RECYCLE.BIN",
             "System Volume Information", "AppData", "Application Data", "Local Settings", "snap", ".cache",
             ".local", ".config", "logs", "Temp", "tmp", "OneDriveTemp", ".Trash-1000"}
# Folders that are really one thing (an app, the Photos or Music library, e.g. copied over from macOS): not
# walked into.
PACKAGE_SUFFIXES = (".app", ".photoslibrary", ".musiclibrary", ".tvlibrary", ".imovielibrary", ".fcpbundle",
                    ".bundle", ".framework", ".plugin")
# Sign-ins Chat Computer Setup may leave on the Windows drive (normally removed at start): never indexed.
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


class TextReader:
    """Reads documents' text in a process of its own, at low priority. The readers (pypdf, python-docx,
    openpyxl, python-pptx) are pure Python: in Chat Computer's own process they held the interpreter for long
    stretches, and Chat Computer's speech skipped while the index read documents (6 to 11 gaps in 6 seconds of speech,
    measured; none while it only walks folders). A document that hangs a reader stops only that process."""

    def __init__(self, timeout_s: float = 60.0, cancel: threading.Event | None = None) -> None:
        self.timeout_s = timeout_s
        self.cancel = cancel or threading.Event()       # Chat Computer is stopping: don't wait for the document
        self.proc: Any = None
        self.conn: Any = None

    def _start(self) -> None:
        import multiprocessing
        ctx = multiprocessing.get_context("spawn")      # never fork a process with threads (GTK, the voice)
        mine, theirs = ctx.Pipe()
        self.proc = ctx.Process(target=_reader_main, args=(theirs,), name="chatcomputer-reader", daemon=True)
        self.proc.start()
        theirs.close()
        self.conn = mine

    def read(self, path: str) -> str:
        try:
            if self.proc is None or not self.proc.is_alive():
                self.stop()
                self._start()
            self.conn.send(path)
            deadline = time.monotonic() + self.timeout_s
            while not self.cancel.is_set() and time.monotonic() < deadline:
                if self.conn.poll(0.5):
                    return self.conn.recv()
            if not self.cancel.is_set():
                log.info("gave up reading %s after %.0f s", path, self.timeout_s)
        except (OSError, EOFError, ValueError) as exc:
            log.info("the document reader stopped (%s) on %s", exc, path)
        self.stop()             # the next document gets a new reader
        return ""

    def stop(self) -> None:
        if self.conn is not None:
            self.conn.close()           # an idle reader ends at this
            self.conn = None
        if self.proc is not None:
            self.proc.join(2)
            if self.proc.is_alive():    # still reading (a document it hangs on)
                self.proc.kill()
                self.proc.join(5)
            self.proc = None


def _reader_main(conn: Any) -> None:
    try:
        os.nice(10)             # Chat Computer's voice and screen first
    except OSError:
        pass
    # Some PDFs make pypdf warn about every page; that isn't Chat Computer's log's business.
    logging.getLogger("pypdf").setLevel(logging.ERROR)
    while True:
        try:
            path = conn.recv()
            conn.send(extract_text(path))
        except (EOFError, OSError):         # Chat Computer stopped, or gave up on this document
            return


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
    def __init__(self, db_path: Path, roots: list[tuple[str, str]], windows_root: str | None = None,
                 skip: Iterable[str | os.PathLike[str]] = ()) -> None:
        self.db_path = Path(db_path)
        self.roots = [(label, os.path.realpath(p)) for label, p in roots if p]
        self.windows_root = os.path.realpath(windows_root) if windows_root else None
        # Folders never walked into (Chat Computer's own: the browser's logins, the voice models, backups...).
        self.skip = {os.path.realpath(p) for p in skip}
        self._stop = threading.Event()
        self._wake = threading.Event()        # crawl now (a folder was added), not at the next interval
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
    def start(self, interval_s: float = 1800, first_delay_s: float = 0) -> None:
        """Crawls in the background now and every `interval_s`. `first_delay_s` lets a start-up finish first:
        a crawl keeps one processor core busy for seconds (searches use the last crawl's results meanwhile)."""
        if self._thread and self._thread.is_alive():
            return
        def loop() -> None:
            self._wake.wait(first_delay_s)
            while not self._stop.is_set():
                self._wake.clear()
                try:
                    self.crawl_once()
                    self.extract_contents()
                except Exception:
                    log.exception("file index crawl failed")
                self._wake.wait(interval_s)
        self._thread = threading.Thread(target=loop, name="file-index", daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        self._wake.set()

    def add_root(self, label: str, path: str) -> None:
        """A folder to search that arrived after the start (Windows' drive, mounted moments later), or one
        whose files did: it's crawled now."""
        root = (label, os.path.realpath(path))
        if root not in self.roots:
            self.roots.append(root)
        self._wake.set()

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
                        try:
                            name.encode("utf-8")
                        except UnicodeEncodeError:
                            # A name that isn't UTF-8 (from an old system): SQLite can't store it, and the error
                            # used to stop the whole scan there, every time.
                            continue
                        if depth == 0 and name in skip_top:
                            continue
                        try:
                            if e.is_dir(follow_symlinks=False):
                                if depth < 25 and not name.endswith(PACKAGE_SUFFIXES) and e.path not in self.skip:
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
        # The files found this time. Unchanged ones are only noted here, not written to the database: a crawl
        # used to rewrite every file's row (tens of MB of disk writes each time, for nothing).
        seen: set[int] = set()
        try:
            for label, root in list(self.roots):
                if not os.path.isdir(root):
                    continue
                batch = 0
                complete = True
                for e in self._walk(root):
                    if count >= MAX_FILES:
                        complete = False
                        break
                    try:
                        st = e.stat(follow_symlinks=False)
                    except OSError:
                        continue
                    count += 1
                    row = db.execute("SELECT id, mtime, size FROM files WHERE path=?", (e.path,)).fetchone()
                    if row and abs(row["mtime"] - st.st_mtime) < 1 and row["size"] == st.st_size:
                        seen.add(row["id"])
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
                        seen.add(fid)
                        rel = os.path.relpath(e.path, root)
                        db.execute("INSERT INTO fts(rowid, name, words, content) VALUES(?,?,?,?)",
                                   (fid, e.name, words_of(f"{label}/{rel}"), ""))
                    batch += 1
                    if batch >= 2000:
                        db.commit()
                        batch = 0
                        time.sleep(0.01)
                db.commit()
                if not complete or self._stop.is_set():
                    continue      # cut short: files not reached yet aren't gone
                # Files that disappeared since the last scan of this root.
                gone = [r["id"] for r in db.execute("SELECT id FROM files WHERE root=?", (label,)) if r["id"] not in seen]
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
        reader = TextReader(cancel=self._stop)
        done = 0
        try:
            rows = db.execute(
                "SELECT id, path, name, root FROM files WHERE content_done=0 AND size<? AND kind IN"
                " ('document','spreadsheet','presentation','code') ORDER BY mtime DESC LIMIT ?",
                (MAX_CONTENT_BYTES, limit)).fetchall()
            for r in rows:
                if self._stop.is_set():
                    break
                text = reader.read(r["path"])
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
            reader.stop()           # until the next crawl finds new documents: its memory back meanwhile
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
            # Exactly what's under that folder: LIKE would read _ and % in its name as wildcards, and ignore case.
            prefix = os.path.realpath(os.path.expanduser(folder)).rstrip("/") + "/"
            where.append("substr(f.path, 1, ?) = ?")
            args += [len(prefix), prefix]
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
