"""The everyday tools for the model: run a command, read, write and edit files, find files and what's in them,
read a web page and search the web.

They have the names and fields Claude Code's own tools had (Bash, Read, Write, Edit, Glob, Grep, WebFetch,
WebSearch), so the permission gate (permissions.py) sees each call exactly as before: a command by its words, a
file by its path. What each gives back is text, cut to a size a conversation can hold.
"""
from __future__ import annotations

import asyncio
import contextlib
import fnmatch
import html
import os
import re
import shutil
import signal
import time
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Awaitable, Callable

MOST_OUTPUT = 30_000          # characters of a command's output, a page or a file kept in the conversation
COMMAND_TIMEOUT_S = 180.0     # a command may run this long unless it asks for more (at most ten minutes)
MOST_LINES = 2000             # lines of a file read at once
MOST_FOUND = 200              # files listed by Glob
WEB_TIMEOUT_S = 20.0
MOST_PAGE_BYTES = 3 << 20
USER_AGENT = "Mozilla/5.0 (X11; Linux x86_64) ChatComputer/1.0"

S = {"type": "string"}
I = {"type": "integer"}
B = {"type": "boolean"}


def _obj(props: dict[str, Any], required: list[str]) -> dict[str, Any]:
    return {"type": "object", "properties": props, "required": required}


SPECS: list[dict[str, Any]] = [
    {"name": "Bash", "description": "Run a shell command (bash) on this computer and get back what it printed. Use "
     "Chat Computer's own tools first; a command is the last resort.",
     "input_schema": _obj({"command": S,
                           "description": {"type": "string", "description": "What it does for them, in a few plain "
                                           "words (\"check whether a printer is connected\")"},
                           "timeout": {"type": "integer", "description": "Milliseconds it may take (at most 600000)"}},
                          ["command", "description"])},
    {"name": "Read", "description": "Read a file (text, or a PDF's words), with its lines numbered.",
     "input_schema": _obj({"file_path": S, "offset": {"type": "integer", "description": "The first line to read"},
                           "limit": {"type": "integer", "description": "How many lines"}}, ["file_path"])},
    {"name": "Write", "description": "Write a file (making its folder if needed), replacing what was in it.",
     "input_schema": _obj({"file_path": S, "content": S}, ["file_path", "content"])},
    {"name": "Edit", "description": "Change a file: old_string, exactly as it is in the file (and only once, unless "
     "replace_all), becomes new_string.",
     "input_schema": _obj({"file_path": S, "old_string": S, "new_string": S, "replace_all": B},
                          ["file_path", "old_string", "new_string"])},
    {"name": "Glob", "description": "Find files by a name pattern (\"**/*.pdf\"), newest first.",
     "input_schema": _obj({"pattern": S, "path": {"type": "string", "description": "The folder to look in"}},
                          ["pattern"])},
    {"name": "Grep", "description": "Search inside files for a regular expression.",
     "input_schema": _obj({"pattern": S, "path": S, "glob": {"type": "string", "description": "Only files like this"},
                           "-i": {"type": "boolean", "description": "Ignore case"},
                           "output_mode": {"type": "string", "enum": ["files_with_matches", "content", "count"]}},
                          ["pattern"])},
    {"name": "WebFetch", "description": "Read a web page's words (without showing it on the screen).",
     "input_schema": _obj({"url": S, "prompt": {"type": "string", "description": "What you're looking for in it"}},
                          ["url"])},
    {"name": "WebSearch", "description": "Search the web; gives back the first results' titles, addresses and "
     "snippets.",
     "input_schema": _obj({"query": S}, ["query"])},
]
NAMES = {s["name"] for s in SPECS}


def _text(s: str) -> dict[str, Any]:
    return {"content": [{"type": "text", "text": s}]}


def _err(s: str) -> dict[str, Any]:
    return {"content": [{"type": "text", "text": s}], "is_error": True}


def _cut(s: str, most: int = MOST_OUTPUT) -> str:
    if len(s) <= most:
        return s
    half = most // 2
    return s[:half] + f"\n\n[... {len(s) - most} characters left out ...]\n\n" + s[-half:]


class Builtins:
    """The tools, run in `cwd` (the person's home) with `env`; `resolve` turns a path as written (~, C:\\...) into
    this computer's."""

    def __init__(self, cwd: str, env: dict[str, str] | None = None,
                 resolve: Callable[[str], str] | None = None) -> None:
        self.cwd = cwd
        self.env = env
        self.resolve = resolve or (lambda p: os.path.abspath(os.path.expanduser(p)))

    def handler(self, name: str) -> Callable[[dict[str, Any]], Awaitable[dict[str, Any]]] | None:
        return {"Bash": self.bash, "Read": self.read, "Write": self.write, "Edit": self.edit, "Glob": self.glob,
                "Grep": self.grep, "WebFetch": self.web_fetch, "WebSearch": self.web_search}.get(name)

    # ------------------------------------------------------------------ commands
    async def bash(self, a: dict[str, Any]) -> dict[str, Any]:
        command = str(a.get("command") or "")
        if not command.strip():
            return _err("No command given.")
        try:
            timeout = min(max(float(a.get("timeout") or COMMAND_TIMEOUT_S * 1000) / 1000, 1.0), 600.0)
        except (TypeError, ValueError):
            timeout = COMMAND_TIMEOUT_S
        env = dict(os.environ)
        env.update(self.env or {})
        proc = await asyncio.create_subprocess_exec(
            "bash", "-c", command, cwd=self.cwd, env=env, stdin=asyncio.subprocess.DEVNULL,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT, start_new_session=True)
        try:
            out, _ = await asyncio.wait_for(proc.communicate(), timeout)
        except asyncio.TimeoutError:
            _kill(proc)
            return _err(f"The command was still running after {timeout:.0f} seconds and was stopped.")
        except asyncio.CancelledError:
            _kill(proc)       # (the person spoke over it)
            raise
        text = _cut(out.decode("utf-8", "replace"))
        if proc.returncode:
            return {"content": [{"type": "text", "text": (text + f"\n(exit code {proc.returncode})").strip()}],
                    "is_error": True}
        return _text(text or "(no output)")

    # ------------------------------------------------------------------ files
    async def read(self, a: dict[str, Any]) -> dict[str, Any]:
        path = self.resolve(str(a.get("file_path") or ""))
        if os.path.isdir(path):
            return _err(f"{path} is a folder. Use Glob, or Bash with ls.")
        if not os.path.exists(path):
            return _err(f"There's no file {path}.")
        try:
            offset = max(int(a.get("offset") or 1), 1)
            limit = min(max(int(a.get("limit") or MOST_LINES), 1), MOST_LINES)
        except (TypeError, ValueError):
            return _err("offset and limit are line numbers.")
        if path.lower().endswith(".pdf"):
            return await asyncio.to_thread(_pdf_text, path)
        with open(path, "rb") as f:
            head = f.read(8192)
        if b"\x00" in head:
            kind = "a picture" if re.search(r"\.(png|jpe?g|gif|webp|bmp|heic)$", path, re.I) else "not text"
            return _err(f"{os.path.basename(path)} is {kind}: this model reads text only. (show_file shows it on the "
                        "screen.)")
        lines: list[str] = []
        total = 0
        with open(path, encoding="utf-8", errors="replace") as f:
            for n, line in enumerate(f, 1):
                total = n
                if offset <= n < offset + limit:
                    line = line.rstrip("\n")
                    lines.append(f"{n:6}\t{line[:2000]}")
        if not lines:
            return _text(f"(The file has {total} lines; nothing from line {offset}.)" if total else "(The file is empty.)")
        more = total - (offset + len(lines) - 1)
        tail = f"\n(... {more} more lines: read on with offset {offset + len(lines)}.)" if more > 0 else ""
        return _text(_cut("\n".join(lines)) + tail)

    async def write(self, a: dict[str, Any]) -> dict[str, Any]:
        path = self.resolve(str(a.get("file_path") or ""))
        content = a.get("content")
        if not isinstance(content, str):
            return _err("content is the file's text.")
        os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(content)
        return _text(f"Wrote {path} ({len(content.encode('utf-8'))} bytes).")

    async def edit(self, a: dict[str, Any]) -> dict[str, Any]:
        path = self.resolve(str(a.get("file_path") or ""))
        old, new = a.get("old_string"), a.get("new_string")
        if not isinstance(old, str) or not isinstance(new, str) or not old:
            return _err("old_string (as it is in the file) and new_string are needed.")
        try:
            with open(path, encoding="utf-8") as f:
                text = f.read()
        except FileNotFoundError:
            return _err(f"There's no file {path}.")
        except UnicodeDecodeError:
            return _err(f"{path} isn't a text file.")
        count = text.count(old)
        if count == 0:
            return _err("old_string isn't in the file (it must match exactly, spaces and all). Read the file first.")
        if count > 1 and not a.get("replace_all"):
            return _err(f"old_string is in the file {count} times: give more of the text around it, or replace_all.")
        with open(path, "w", encoding="utf-8") as f:
            f.write(text.replace(old, new) if a.get("replace_all") else text.replace(old, new, 1))
        return _text(f"Changed {path} ({count if a.get('replace_all') else 1} place{'s' if count > 1 and a.get('replace_all') else ''}).")

    async def glob(self, a: dict[str, Any]) -> dict[str, Any]:
        pattern = str(a.get("pattern") or "")
        root = self.resolve(str(a.get("path") or self.cwd))
        if not pattern:
            return _err("No pattern given.")
        found = await asyncio.to_thread(_glob, root, pattern)
        if not found:
            return _text("No files found.")
        more = len(found) - MOST_FOUND
        return _text("\n".join(found[:MOST_FOUND]) + (f"\n(and {more} more)" if more > 0 else ""))

    async def grep(self, a: dict[str, Any]) -> dict[str, Any]:
        pattern = str(a.get("pattern") or "")
        root = self.resolve(str(a.get("path") or self.cwd))
        mode = a.get("output_mode") or "files_with_matches"
        if not pattern:
            return _err("No pattern given.")
        try:
            rx = re.compile(pattern, re.I if a.get("-i") else 0)
        except re.error as exc:
            return _err(f"That isn't a regular expression Python reads: {exc}")
        out = await asyncio.to_thread(_grep, root, rx, a.get("glob"), mode)
        return _text(_cut(out) if out else "Nothing found.")

    # ------------------------------------------------------------------ the web
    async def web_fetch(self, a: dict[str, Any]) -> dict[str, Any]:
        url = str(a.get("url") or "").strip()
        if not re.match(r"^https?://", url, re.I):
            url = "https://" + url
        try:
            kind, body = await asyncio.to_thread(_get, url)
        except Exception as exc:
            return _err(f"Couldn't read {url}: {exc}")
        text = _page_text(body) if "html" in kind or body.lstrip()[:1] == "<" else body
        return _text(_cut(text.strip()) or "(The page has no words.)")

    async def web_search(self, a: dict[str, Any]) -> dict[str, Any]:
        query = str(a.get("query") or "").strip()
        if not query:
            return _err("No query given.")
        url = "https://html.duckduckgo.com/html/?q=" + urllib.parse.quote_plus(query)
        try:
            _, body = await asyncio.to_thread(_get, url)
        except Exception as exc:
            return _err(f"The search didn't work ({exc}). show_website can open a search on the screen instead.")
        results = _search_results(body)
        if not results:
            return _err("The search found nothing (or its page changed). show_website can open a search on the "
                        "screen instead.")
        lines = [f"{i}. {t}\n   {u}\n   {s}" for i, (t, u, s) in enumerate(results[:8], 1)]
        return _text("\n".join(lines))


def _kill(proc: asyncio.subprocess.Process) -> None:
    with contextlib.suppress(Exception):
        os.killpg(proc.pid, signal.SIGKILL)


def _pdf_text(path: str) -> dict[str, Any]:
    try:
        from pypdf import PdfReader
        pages = PdfReader(path).pages
        text = "\n\n".join(f"[page {i}]\n" + (p.extract_text() or "") for i, p in enumerate(pages, 1))
    except Exception as exc:
        return _err(f"Couldn't read the PDF: {exc}")
    return _text(_cut(text.strip()) or "(The PDF has no words in it: it may be scanned pictures.)")


def _glob(root: str, pattern: str) -> list[str]:
    base = Path(root)
    pattern = pattern.lstrip("/") if not os.path.isabs(pattern) else os.path.relpath(pattern, root)
    found = []
    try:
        for p in base.glob(pattern):
            with contextlib.suppress(OSError):
                if p.is_file():
                    found.append((p.stat().st_mtime, str(p)))
            if len(found) > 20_000:
                break
    except (ValueError, OSError):
        return []
    return [p for _, p in sorted(found, reverse=True)]


def _grep(root: str, rx: re.Pattern, glob: str | None, mode: str) -> str:
    rg = shutil.which("rg")
    if rg:
        import subprocess
        args = [rg, "--no-heading", "--color=never", "-m", "50"]
        if rx.flags & re.I:
            args.append("-i")
        if glob:
            args += ["--glob", glob]
        args += {"files_with_matches": ["-l"], "count": ["-c"]}.get(mode, ["-n"])
        args += ["-e", rx.pattern, root]
        try:
            r = subprocess.run(args, capture_output=True, text=True, timeout=60)
            return r.stdout
        except (subprocess.TimeoutExpired, OSError):
            pass
    out: list[str] = []
    files = [root] if os.path.isfile(root) else (
        os.path.join(d, f) for d, _, fs in os.walk(root) for f in fs)
    deadline = time.monotonic() + 60
    for path in files:
        if time.monotonic() > deadline or len(out) > 2000:
            out.append("(stopped: too many files to look through)")
            break
        if glob and not fnmatch.fnmatch(os.path.basename(path), glob):
            continue
        try:
            with open(path, encoding="utf-8") as f:
                lines = f.read(4 << 20).splitlines()
        except (OSError, UnicodeDecodeError):
            continue
        hits = [(n, line) for n, line in enumerate(lines, 1) if rx.search(line)]
        if not hits:
            continue
        if mode == "files_with_matches":
            out.append(path)
        elif mode == "count":
            out.append(f"{path}:{len(hits)}")
        else:
            out += [f"{path}:{n}:{line[:500]}" for n, line in hits[:50]]
    return "\n".join(out)


def _get(url: str) -> tuple[str, str]:
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, "Accept-Language": "en"})
    with urllib.request.urlopen(req, timeout=WEB_TIMEOUT_S) as r:
        kind = r.headers.get("Content-Type", "")
        body = r.read(MOST_PAGE_BYTES)
        charset = r.headers.get_content_charset() or "utf-8"
    if not any(k in kind for k in ("text", "html", "json", "xml")) and kind:
        raise ValueError(f"it isn't a page of text ({kind.split(';')[0]})")
    return kind, body.decode(charset, "replace")


def _page_text(page: str) -> str:
    """A page's words: without its scripts, styles and tags; links keep their words, paragraphs their breaks."""
    page = re.sub(r"(?is)<(script|style|noscript|svg|template)\b.*?</\1>", " ", page)
    page = re.sub(r"(?is)<!--.*?-->", " ", page)
    title = re.search(r"(?is)<title[^>]*>(.*?)</title>", page)
    page = re.sub(r"(?i)<br\s*/?>|</(p|div|li|h[1-6]|tr|section|article|header|footer)>", "\n", page)
    page = re.sub(r"(?s)<[^>]+>", " ", page)
    text = html.unescape(page)
    text = re.sub(r"[ \t\r\f\v]+", " ", text)
    text = re.sub(r"\n\s*\n+", "\n\n", text)
    head = html.unescape(title.group(1)).strip() + "\n\n" if title else ""
    return head + text.strip()


def _search_results(page: str) -> list[tuple[str, str, str]]:
    """DuckDuckGo's results page (its plain HTML one): each result's title, address and snippet."""
    out = []
    for m in re.finditer(r'(?is)<a[^>]+class="result__a"[^>]+href="([^"]+)"[^>]*>(.*?)</a>(.*?)(?=<a[^>]+class="result__a"|$)',
                         page):
        href, title, rest = m.group(1), m.group(2), m.group(3)
        q = urllib.parse.urlparse(html.unescape(href))
        target = urllib.parse.parse_qs(q.query).get("uddg", [html.unescape(href)])[0]
        if "duckduckgo.com/y.js" in target:          # an advertisement
            continue
        snippet = re.search(r'(?is)class="result__snippet"[^>]*>(.*?)</a>', rest)
        clean = lambda s: re.sub(r"\s+", " ", html.unescape(re.sub(r"(?s)<[^>]+>", "", s))).strip()   # noqa: E731
        out.append((clean(title), target, clean(snippet.group(1)) if snippet else ""))
    return out
