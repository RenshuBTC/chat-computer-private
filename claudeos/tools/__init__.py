"""ClaudeOS's system services: everything Claude can do on this computer, as tools.

Each tool has a short description (Claude reads it to decide what to use) and a policy that
returns None (just do it), a spoken yes/no question, or "DENY: reason". Built-in Claude Code
tools (Bash, Read, Write, Edit, Glob, Grep, WebSearch, WebFetch) are available alongside these.
"""
from __future__ import annotations

import asyncio
import base64
import json
import logging
import os
import re
import shutil
import time
import zoneinfo
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Awaitable, Callable

from claude_agent_sdk import create_sdk_mcp_server, tool

from ..config import Paths, Settings
from ..files_index import FileIndex
from ..journal import Journal, move_to_trash
from ..memory import Memory
from ..platform import Platform
from ..platform.linux import png_to_jpeg_scaled
from ..screen import Screen
from ..ui import render

log = logging.getLogger("claudeos.tools")

SERVER = "claudeos"
ConfirmFn = Callable[[str], Awaitable[tuple[str, str]]]

SETTABLE = {"voice", "speech_speed", "text_size", "text_scale", "viewing_distance_cm", "screen_inches", "web_zoom",
            "theme", "auto_sleep_minutes", "barge_in", "release_mic_during_media", "show_activity", "wake_words",
            "model", "effort", "stt_model", "user_name", "fresh_session_after_idle_hours", "phone_access"}
WINDOWS_SYSTEM = re.compile(r"/(Windows|Program Files( \(x86\))?|ProgramData|\$Recycle\.Bin|Recovery|Boot|EFI)(/|$)", re.I)


@dataclass
class ToolContext:
    paths: Paths
    settings: Settings
    platform: Platform
    journal: Journal
    screen: Screen
    memory: Memory
    index: FileIndex | None = None
    confirm: ConfirmFn | None = None
    on_sleep: Callable[[], None] | None = None
    on_settings_changed: Callable[[list[str]], None] | None = None
    rate_limit: dict[str, Any] = field(default_factory=dict)
    shot_scale: float = 1.0
    power_delay_s: float = 4.0

    def resolve(self, p: str) -> str:
        """Accept ~/x, C:\\Users\\x (the Windows drive) and plain paths."""
        p = (p or "").strip().strip('"')
        m = re.match(r"^([A-Za-z]):[\\/](.*)$", p)
        if m and m.group(1).upper() == "C":
            p = os.path.join(self.settings.windows_mount, m.group(2).replace("\\", "/"))
        return os.path.abspath(os.path.expanduser(p))


def _text(s: str) -> dict[str, Any]:
    return {"content": [{"type": "text", "text": s}]}


def _err(s: str) -> dict[str, Any]:
    return {"content": [{"type": "text", "text": s}], "is_error": True}


def _json(obj: Any) -> str:
    return json.dumps(obj, ensure_ascii=False, indent=1, default=str)


def _names(paths: list[str], n: int = 4) -> str:
    names = [os.path.basename(p.rstrip("/")) or p for p in paths]
    return ", ".join(names[:n]) + (f" and {len(names) - n} more" if len(names) > n else "")


def normalize_url(url: str) -> str:
    u = (url or "").strip()
    if not u:
        return "about:blank"
    if re.match(r"^(https?|file|about|data):", u, re.I):
        return u
    if " " in u or "." not in u:
        from urllib.parse import quote_plus
        return "https://www.google.com/search?q=" + quote_plus(u)
    return "https://" + u


def build(ctx: ToolContext) -> tuple[dict[str, Any], dict[str, Callable[[dict[str, Any]], str | None]], list[str]]:
    """Returns (mcp server config, {tool name: policy}, [tool names])."""
    on_mac = getattr(ctx.platform, "name", "") == "mac"
    tools: list[Any] = []
    policies: dict[str, Callable[[dict[str, Any]], str | None]] = {}

    def reg(name: str, description: str, schema: dict[str, Any], policy: Callable[[dict[str, Any]], str | None] | None = None):
        def deco(fn):
            async def wrapped(args: dict[str, Any]) -> dict[str, Any]:
                try:
                    return await fn(args or {})
                except Exception as exc:
                    log.exception("tool %s failed", name)
                    return _err(f"{name} failed: {exc}")
            tools.append(tool(name, description, schema)(wrapped))
            if policy:
                policies[name] = policy
            return fn
        return deco

    def obj(props: dict[str, Any], required: list[str] | None = None) -> dict[str, Any]:
        return {"type": "object", "properties": props, "required": required or []}

    S = {"type": "string"}
    I = {"type": "integer"}
    B = {"type": "boolean"}

    async def page_js(call: str) -> Any:
        info = await ctx.screen.content_info()
        if info.get("mode") != "content":
            raise RuntimeError("Nothing is open on screen right now (the conversation is showing). "
                               "Open something first with show_website, show_file or show_page.")
        res = await ctx.screen.eval_js(f"(window.__CO ? __CO.{call} : {{error: 'page still loading'}})")
        if isinstance(res, dict) and res.get("error"):
            raise RuntimeError(res["error"])
        return res

    # ================================================================ screen and web
    @reg("show_website", "Open a website or web app full screen on the user's display (YouTube, Gmail, news, "
         "shops, maps...). Pass a URL, or words to search Google. The user's sign-ins are remembered between visits.",
         obj({"url": S}, ["url"]))
    async def show_website(a):
        url = normalize_url(a.get("url", ""))
        r = await ctx.screen.show_url(url)
        if not r.get("ok"):
            return _err(f"Couldn't open {url}: {r.get('error', 'unknown error')}")
        ctx.journal.record("screen", f"opened {r.get('url') or url}")
        return _text(f"Showing: {r.get('title') or ''} — {r.get('url') or url}\n"
                     "Use read_screen to read it or find things to click.")

    @reg("read_screen", "Read what is currently shown: mode 'text' gives the page's text; mode 'elements' lists "
         "links, buttons and inputs with [ref] numbers to use with click and type_text. Optional 'find' filters "
         "elements by words.", obj({"mode": {"type": "string", "enum": ["text", "elements"]}, "find": S}))
    async def read_screen(a):
        info = await ctx.screen.content_info()
        if info.get("mode") != "content":
            return _text("The screen is showing our conversation; nothing else is open.")
        if a.get("mode") == "elements":
            res = await page_js(f"elements({json.dumps(a.get('find') or '')})")
            lines = res.get("lines", []) if isinstance(res, dict) else []
            head = f"{res.get('title', '')} — {res.get('url', '')}\n" if isinstance(res, dict) else ""
            return _text(head + ("\n".join(lines) if lines else "No clickable elements found."))
        res = await page_js("text(30000)")
        if not isinstance(res, dict):
            return _err("The page isn't readable yet; try again in a moment.")
        where = f"(scrolled {res.get('scrollPercent', 0)}% down)" if res.get("scrollable") else ""
        return _text(f"{res.get('title', '')} — {res.get('url', '')} {where}\n\n{res.get('text', '')}")

    async def _click_at_element(loc: dict[str, Any]) -> bool:
        ox, oy, _ = await ctx.screen.content_origin()
        dpr = float(loc.get("dpr") or 1)
        x = ox + (loc["x"] + loc["w"] / 2) * dpr
        y = oy + (loc["y"] + loc["h"] / 2) * dpr
        try:
            await asyncio.to_thread(ctx.platform.click, int(x), int(y))
            return True
        except Exception as exc:
            log.info("trusted click failed (%s); using a page click", exc)
            return False

    @reg("click", "Click something on screen: by [ref] from read_screen elements, by its visible text, or by "
         "x/y pixel position in the latest look_at_screen image. Ask the user first (confirm_with_user) before "
         "anything that buys, sends, posts, deletes or signs up.",
         obj({"ref": I, "text": S, "x": I, "y": I, "button": {"type": "string", "enum": ["left", "right", "middle"]},
              "double": B}))
    async def click(a):
        if a.get("x") is not None and a.get("y") is not None:
            s = ctx.shot_scale
            await asyncio.to_thread(ctx.platform.click, int(a["x"] * s), int(a["y"] * s), a.get("button", "left"),
                                    2 if a.get("double") else 1)
            await asyncio.sleep(0.5)
            return _text(f"Clicked at ({a['x']}, {a['y']}).")
        if a.get("ref") is None and not a.get("text"):
            return _err("Give a ref, some text, or x and y.")
        loc = await page_js(f"locate({json.dumps(a.get('ref'))}, {json.dumps(a.get('text') or '')})")
        if not isinstance(loc, dict) or "x" not in loc:
            return _err("I couldn't find that on the page. Use read_screen with mode 'elements' to see what's there.")
        if not await _click_at_element(loc):
            await page_js(f"jsClick({loc['ref']})")
        await asyncio.sleep(0.6)
        info = await ctx.screen.content_info()
        return _text(f"Clicked “{loc.get('name', '')}”. Now showing: {info.get('title', '')} — {info.get('url', '')}")

    @reg("type_text", "Type text into the page: into the [ref] input if given (cleared first unless clear is false), "
         "otherwise wherever the cursor is. submit presses Enter afterwards. Never type passwords or card numbers.",
         obj({"text": S, "ref": I, "submit": B, "clear": B}, ["text"]))
    async def type_text(a):
        text = str(a.get("text", ""))
        if a.get("ref") is not None:
            f = await page_js(f"focus({int(a['ref'])}, {json.dumps(a.get('clear', True))})")
            if isinstance(f, dict) and f.get("password"):
                return _err("That's a password box. ClaudeOS doesn't type passwords: ask the user to use a keyboard, "
                            "a passkey, or their phone to sign in.")
        try:
            await asyncio.to_thread(ctx.platform.type_text, text)
            if a.get("submit"):
                await asyncio.to_thread(ctx.platform.key, "Return")
        except Exception:
            await page_js(f"insertText({json.dumps(text)}, {json.dumps(bool(a.get('submit')))})")
        await asyncio.sleep(0.4)
        return _text("Typed it" + (" and pressed Enter." if a.get("submit") else "."))

    @reg("press_keys", "Press a key or shortcut, like 'Return', 'Escape', 'ctrl+f', 'space', 'Down', 'k' "
         "(YouTube play/pause), 'f' (YouTube fullscreen).", obj({"keys": S, "repeat": I}, ["keys"]))
    async def press_keys(a):
        await ctx.screen.focus_content()
        await asyncio.to_thread(ctx.platform.key, str(a["keys"]), int(a.get("repeat") or 1))
        await asyncio.sleep(0.3)
        return _text(f"Pressed {a['keys']}.")

    @reg("scroll", "Scroll the page up or down (amount = screens, default 1), or bring element [ref] into view.",
         obj({"direction": {"type": "string", "enum": ["up", "down", "top", "bottom"]}, "amount": {"type": "number"}, "ref": I}))
    async def scroll(a):
        res = await page_js(f"scroll({json.dumps(a.get('direction', 'down'))}, {float(a.get('amount') or 1)}, "
                            f"{json.dumps(a.get('ref'))})")
        pct = res.get("scrollPercent") if isinstance(res, dict) else None
        return _text("Scrolled." + (f" Now {pct}% down the page." if pct is not None else ""))

    @reg("look_at_screen", "Take a picture of exactly what the user sees right now. Use it when the page text "
         "isn't enough (images, video, layout, apps without text). Coordinates in this image work with click x/y.",
         obj({}))
    async def look_at_screen(a):
        png = await asyncio.to_thread(ctx.platform.screenshot_png)
        jpg, scale = await asyncio.to_thread(png_to_jpeg_scaled, png, 1280, 800)
        ctx.shot_scale = scale
        return {"content": [{"type": "image", "data": base64.b64encode(jpg).decode(), "mimeType": "image/jpeg"},
                            {"type": "text", "text": "This is the whole screen. For click, use x/y in this image's pixels."}]}

    @reg("browser", "Go back, forward, reload or stop the page that's showing.",
         obj({"action": {"type": "string", "enum": ["back", "forward", "reload", "stop"]}}, ["action"]))
    async def browser(a):
        r = await ctx.screen.navigate(a["action"])
        return _text(f"Done. Now showing: {r.get('title', '')} — {r.get('url', '')}")

    @reg("media", "Control the video or music playing on screen (YouTube etc.): play, pause, seek (seconds), "
         "forward/rewind (seconds), volume (0-100), mute, unmute, speed (e.g. 1.5), fullscreen, exit_fullscreen, status.",
         obj({"action": {"type": "string", "enum": ["play", "pause", "toggle", "seek", "forward", "rewind", "volume",
                                                    "mute", "unmute", "speed", "fullscreen", "exit_fullscreen", "status"]},
              "value": {"type": "number"}}, ["action"]))
    async def media(a):
        res = await page_js(f"media({json.dumps(a['action'])}, {json.dumps(a.get('value'))})")
        if not isinstance(res, dict) or not res.get("found"):
            return _err("There's no video or audio on screen.")
        return _text(_json({k: res[k] for k in ("playing", "position", "duration", "volume", "muted", "speed", "title") if k in res}))

    @reg("show_document", "Show a document you wrote (Markdown: headings, lists, tables, bold) full screen, for "
         "long answers, summaries, emails, plans or anything better read than heard.",
         obj({"title": S, "markdown": S}, ["title", "markdown"]))
    async def show_document(a):
        html = render.document_html(a["title"], a["markdown"], ctx.settings)
        r = await ctx.screen.show_html(html, a["title"])
        return _text("Showing the document." if r.get("ok") else f"Couldn't show it: {r.get('error')}")

    def _page_file(name: str) -> Path:
        slug = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")[:60] or "page"
        return ctx.paths.pages / f"{slug}.html"

    def _registry() -> dict[str, Any]:
        try:
            return json.loads((ctx.paths.pages / "saved.json").read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return {}

    def _save_registry(reg_: dict[str, Any]) -> None:
        (ctx.paths.pages / "saved.json").write_text(json.dumps(reg_, indent=1), encoding="utf-8")

    @reg("show_page", "Build and show your own interactive page or mini-app (a full HTML document with CSS and "
         "JavaScript; it may fetch public data from the internet). Use for charts, dashboards, tools, games, "
         "calculators. Give save_as a short name to keep it so the user can ask for it again later.",
         obj({"title": S, "html": S, "save_as": S}, ["title", "html"]))
    async def show_page(a):
        name = a.get("save_as") or ""
        path = _page_file(name) if name else ctx.paths.pages / "_current.html"
        ctx.paths.pages.mkdir(parents=True, exist_ok=True)
        path.write_text(render.wrap_page(a["html"], a["title"], ctx.settings), encoding="utf-8")
        if name:
            reg_ = _registry()
            reg_[name] = {"file": path.name, "title": a["title"], "saved": time.strftime("%Y-%m-%d %H:%M")}
            _save_registry(reg_)
            ctx.journal.record("page", f"saved page '{name}'")
        r = await ctx.screen.show_url(path.as_uri())
        return _text(("Showing it" + (f" (kept as '{name}')." if name else ".")) if r.get("ok") else f"Couldn't show it: {r.get('error')}")

    def _saved_policy(a: dict[str, Any]) -> str | None:
        if a.get("action") == "delete":
            return f"Delete your saved page “{a.get('name', '')}”?"
        return None

    @reg("saved_pages", "List, open or delete the pages and mini-apps you saved for the user earlier.",
         obj({"action": {"type": "string", "enum": ["list", "open", "delete"]}, "name": S}, ["action"]), _saved_policy)
    async def saved_pages(a):
        reg_ = _registry()
        if a["action"] == "list":
            return _text(_json(reg_) if reg_ else "No saved pages yet.")
        name = a.get("name", "")
        key = next((k for k in reg_ if k.lower() == name.lower()), None) or \
            next((k for k in reg_ if name.lower() in k.lower()), None)
        if not key:
            return _err(f"No saved page called {name!r}. Saved: {', '.join(reg_) or 'none'}")
        f = ctx.paths.pages / reg_[key]["file"]
        if a["action"] == "open":
            r = await ctx.screen.show_url(f.as_uri())
            return _text(f"Showing {key}." if r.get("ok") else f"Couldn't open it: {r.get('error')}")
        if f.exists():
            move_to_trash(str(f))
        reg_.pop(key)
        _save_registry(reg_)
        return _text(f"Deleted {key}.")

    @reg("show_file", "Show a file from this computer full screen: PDFs, photos, videos, music, Word, Excel, "
         "PowerPoint, text. Paths can be like ~/Downloads/x.pdf" +
         (" or C:\\Users\\...\\x.pdf (the Windows drive)." if not on_mac else "."),
         obj({"path": S}, ["path"]))
    async def show_file(a):
        path = ctx.resolve(a["path"])
        if ctx.paths.is_secret(path):
            return _err("That file is private.")
        if not os.path.exists(path):
            return _err(f"There's no file at {path}. Try search_files.")
        kind, payload = await asyncio.to_thread(render.file_view, path, ctx.settings, ctx.paths)
        r = await (ctx.screen.show_url(payload) if kind == "url" else ctx.screen.show_html(payload, os.path.basename(path)))
        ctx.journal.record("screen", f"showed {path}")
        return _text(f"Showing {os.path.basename(path)}." if r.get("ok") else f"Couldn't show it: {r.get('error')}")

    @reg("show_conversation", "Hide whatever is open and go back to showing our conversation.", obj({}))
    async def show_conversation(a):
        await ctx.screen.hide_content()
        return _text("Back to the conversation.")

    @reg("confirm_with_user", "Ask the user a yes/no question out loud and wait for the answer. Use before "
         "purchases, sending messages or emails, posting, deleting online data, or anything you're unsure they want.",
         obj({"question": S}, ["question"]))
    async def confirm_with_user(a):
        if not ctx.confirm:
            return _err("No one is there to answer.")
        verdict, said = await ctx.confirm(a["question"])
        if verdict == "phone":
            return _text("They're messaging you from the Claude app on their phone, so nobody is at the PC to hear "
                         "this. Ask the question in your reply and stop; their next message is the answer.")
        return _text(f"Answer: {verdict}. They said: “{said}”" if said else f"Answer: {verdict} (no reply).")

    # ================================================================ files
    @reg("search_files", ("Search the user's files on this Mac (home folder and iCloud Drive), " if on_mac else
                           "Search the user's files on this PC, including their Windows Desktop, Documents, Downloads, "
                           "Pictures and OneDrive, ") +
         "by name and by the text inside documents. Leave query empty for the most recent "
         "files. kind: document, spreadsheet, presentation, image, video, audio, archive, code, ebook. Dates: "
         "YYYY-MM-DD or like '30d'.",
         obj({"query": S, "kind": S, "modified_after": S, "modified_before": S, "folder": S, "limit": I}))
    async def search_files(a):
        if not ctx.index:
            return _err("The file index isn't running; use Glob or Grep instead.")
        folder = ctx.resolve(a["folder"]) if a.get("folder") else None
        res = await asyncio.to_thread(ctx.index.search, a.get("query", ""), a.get("kind") or None,
                                      a.get("modified_after"), a.get("modified_before"), folder, a.get("limit") or 20)
        note = ""
        if ctx.index.status.get("state") != "idle":
            note = f"\n(Still indexing: {ctx.index.status.get('state')}; results may be incomplete.)"
        return _text((_json(res) if res else "Nothing found.") + note)

    def _move_policy(a: dict[str, Any]) -> str | None:
        items = a.get("items") or []
        srcs = [ctx.resolve(i.get("from", "")) for i in items]
        if any(WINDOWS_SYSTEM.search(s[len(ctx.settings.windows_mount):]) for s in srcs if s.startswith(ctx.settings.windows_mount)):
            return "DENY: Those are Windows system files; moving them could stop Windows from starting."
        replaced = []
        for i in items:
            dst = ctx.resolve(i.get("to", ""))
            if os.path.isdir(dst):
                dst = os.path.join(dst, os.path.basename(ctx.resolve(i.get("from", "")).rstrip("/")))
            if os.path.exists(dst):
                replaced.append(dst)
        if replaced:
            return f"That would replace {_names(replaced)}. Go ahead?"
        if len(items) > 20:
            return f"Move {len(items)} items?"
        return None

    def _do_transfer(items: list[dict[str, Any]], copy: bool) -> list[str]:
        out = []
        for i in items:
            src, dst = ctx.resolve(i.get("from", "")), ctx.resolve(i.get("to", ""))
            if not os.path.exists(src):
                out.append(f"missing: {src}")
                continue
            if os.path.isdir(dst):
                dst = os.path.join(dst, os.path.basename(src.rstrip("/")))
            os.makedirs(os.path.dirname(dst) or ".", exist_ok=True)
            backup = ctx.journal.backup(dst) if os.path.isfile(dst) else None
            if copy:
                if os.path.isdir(src):
                    shutil.copytree(src, dst, dirs_exist_ok=True)
                else:
                    shutil.copy2(src, dst)
                ctx.journal.record_copy(dst, backup)
            else:
                shutil.move(src, dst)
                ctx.journal.record_move(src, dst, backup)
            out.append(f"{'copied' if copy else 'moved'} {src} → {dst}")
        return out

    item_schema = {"type": "array", "items": obj({"from": S, "to": S}, ["from", "to"])}

    @reg("move_files", "Move or rename files and folders. Each item: from, to (a new name or a folder). Undoable.",
         obj({"items": item_schema}, ["items"]), _move_policy)
    async def move_files(a):
        return _text("\n".join(await asyncio.to_thread(_do_transfer, a.get("items") or [], False)))

    @reg("copy_files", "Copy files or folders. Each item: from, to (a new name or a folder). Undoable.",
         obj({"items": item_schema}, ["items"]), _move_policy)
    async def copy_files(a):
        return _text("\n".join(await asyncio.to_thread(_do_transfer, a.get("items") or [], True)))

    def _trash_policy(a: dict[str, Any]) -> str | None:
        paths = [ctx.resolve(p) for p in a.get("paths") or []]
        if any(ctx.paths.is_secret(p) for p in paths):
            return "DENY: That's ClaudeOS's private sign-in folder."
        if any(WINDOWS_SYSTEM.search(p[len(ctx.settings.windows_mount):]) for p in paths if p.startswith(ctx.settings.windows_mount)):
            return "DENY: Those are Windows system files."
        if not paths:
            return None
        return f"Move {len(paths)} item{'s' if len(paths) != 1 else ''} to the trash: {_names(paths)}?"

    @reg("trash_files", "Move files or folders to the trash (they can be restored with undo_last_change).",
         obj({"paths": {"type": "array", "items": S}}, ["paths"]), _trash_policy)
    async def trash_files(a):
        out = []
        for raw in a.get("paths") or []:
            p = ctx.resolve(raw)
            if not os.path.exists(p):
                out.append(f"missing: {p}")
                continue
            try:
                trashed = await asyncio.to_thread(move_to_trash, p)
            except OSError as exc:
                out.append(f"couldn't trash {p}: {exc}")
                continue
            ctx.journal.record_trash(p, trashed)
            out.append(f"trashed {p}")
        return _text("\n".join(out))

    @reg("make_folder", "Create a folder (and any missing parent folders).", obj({"path": S}, ["path"]))
    async def make_folder(a):
        p = ctx.resolve(a["path"])
        existed = os.path.isdir(p)
        os.makedirs(p, exist_ok=True)
        if not existed:
            ctx.journal.record_mkdir(p)
        return _text(f"Folder ready: {p}")

    @reg("undo_last_change", "Undo the most recent file changes ClaudeOS made (edits, moves, copies, trash).",
         obj({"count": I}))
    async def undo_last_change(a):
        msgs = await asyncio.to_thread(ctx.journal.undo_last, max(1, min(int(a.get("count") or 1), 20)))
        return _text("\n".join(msgs) if msgs else "There's nothing to undo.")

    @reg("recent_actions", "What ClaudeOS did recently (opened, changed, ran), newest last.", obj({"limit": I}))
    async def recent_actions(a):
        es = ctx.journal.entries(int(a.get("limit") or 15))
        return _text("\n".join(time.strftime("%H:%M", time.localtime(e["ts"])) + f" {e['kind']}: {e['summary']}" for e in es)
                     or "Nothing yet.")

    # ================================================================ the machine
    @reg("system_status", "Check the computer: internet, Wi-Fi, Bluetooth/AirPods, sound, disks, time, " +
         ("" if on_mac else "the Windows drive, ") + "and the user's Claude usage limits.", obj({}))
    async def system_status(a):
        def gather() -> dict[str, Any]:
            st: dict[str, Any] = {"internet": ctx.platform.internet_ok()}
            parts = [("machine", ctx.platform.system_info), ("wifi", ctx.platform.wifi_status),
                     ("bluetooth", ctx.platform.bt_status), ("sound", ctx.platform.audio_status)]
            if not on_mac:
                parts.append(("windows_drive", ctx.platform.windows_status))
            for key, fn in parts:
                try:
                    st[key] = fn()
                except Exception as exc:
                    st[key] = f"unavailable ({exc})"
            return st
        st = await asyncio.to_thread(gather)
        if ctx.index:
            st["file_index"] = {**ctx.index.status, "files": ctx.index.status.get("files")}
        if ctx.rate_limit:
            st["claude_usage"] = ctx.rate_limit
        return _text(_json(st))

    def _wifi_policy(a: dict[str, Any]) -> str | None:
        act = a.get("action")
        if act in ("disconnect", "off"):
            return "Turn off the Wi-Fi? I won't be able to reach Claude until it's back."
        if act == "forget":
            return f"Forget the Wi-Fi network {a.get('network', '')}?"
        return None

    @reg("wifi", "Wi-Fi: status, list networks, connect (network + password), disconnect, forget, on, off.",
         obj({"action": {"type": "string", "enum": ["status", "list", "connect", "disconnect", "forget", "on", "off"]},
              "network": S, "password": S}, ["action"]), _wifi_policy)
    async def wifi(a):
        act, p = a["action"], ctx.platform
        if act == "status":
            return _text(_json(await asyncio.to_thread(p.wifi_status)))
        if act == "list":
            return _text(_json(await asyncio.to_thread(p.wifi_scan)))
        if act == "connect":
            if not a.get("network"):
                return _err("Which network?")
            return _text(await asyncio.to_thread(p.wifi_connect, a["network"], a.get("password")))
        if act == "disconnect":
            return _text(await asyncio.to_thread(p.wifi_disconnect))
        if act == "forget":
            return _text(await asyncio.to_thread(p.wifi_forget, a.get("network", "")))
        return _text(await asyncio.to_thread(p.wifi_radio, act == "on"))

    def _bt_policy(a: dict[str, Any]) -> str | None:
        if a.get("action") == "forget":
            return f"Forget the Bluetooth device {a.get('device', '')}? You'd have to pair it again."
        if a.get("action") == "import_from_windows":
            return "Copy your Bluetooth pairings, like your AirPods, from Windows? Bluetooth restarts for a moment."
        return None

    async def _bt_address(device: str) -> str | None:
        if re.match(r"^[0-9A-Fa-f]{2}(:[0-9A-Fa-f]{2}){5}$", device or ""):
            return device
        st = await asyncio.to_thread(ctx.platform.bt_status)
        for d in st.get("paired", []) + st.get("connected", []):
            if device and device.lower() in d["name"].lower():
                return d["address"]
        found = await asyncio.to_thread(ctx.platform.bt_scan, 8)
        for d in found:
            if device and device.lower() in d["name"].lower():
                return d["address"]
        return None

    bt_actions = ["status", "scan", "pair", "connect", "disconnect", "forget"] + ([] if on_mac else ["import_from_windows"])

    @reg("bluetooth", "Bluetooth (AirPods, speakers, keyboards): status, scan, pair (put the device in pairing "
         "mode first; for AirPods hold the button on the case), connect, disconnect, forget" +
         (". On a Mac, pairing and forgetting happen in System Settings, which these open." if on_mac
          else ", import_from_windows."),
         obj({"action": {"type": "string", "enum": bt_actions}, "device": S}, ["action"]), _bt_policy)
    async def bluetooth(a):
        act, p = a["action"], ctx.platform
        if act == "status":
            return _text(_json(await asyncio.to_thread(p.bt_status)))
        if act == "scan":
            return _text(_json(await asyncio.to_thread(p.bt_scan, 10)))
        if act == "import_from_windows":
            return _text(await asyncio.to_thread(p.bt_import_windows_keys))
        addr = await _bt_address(a.get("device", ""))
        if not addr:
            return _err(f"I can't find a Bluetooth device called {a.get('device')!r}. Is it in pairing mode?")
        fn = {"pair": p.bt_pair, "connect": p.bt_connect, "disconnect": p.bt_disconnect, "forget": p.bt_forget}[act]
        return _text(await asyncio.to_thread(fn, addr))

    @reg("sound", "Sound: status, set_volume (0-100), volume_up / volume_down (by value, default 10), mute, unmute, "
         "output (switch where sound plays, by name).",
         obj({"action": {"type": "string", "enum": ["status", "set_volume", "volume_up", "volume_down", "mute", "unmute", "output"]},
              "value": {"type": ["number", "string"]}}, ["action"]))
    async def sound(a):
        act, p, v = a["action"], ctx.platform, a.get("value")
        if act == "status":
            return _text(_json(await asyncio.to_thread(p.audio_status)))
        if act == "set_volume":
            return _text(await asyncio.to_thread(p.set_volume, int(float(v or 50)), None))
        if act in ("volume_up", "volume_down"):
            step = int(float(v or 10))
            return _text(await asyncio.to_thread(p.set_volume, None, step if act == "volume_up" else -step))
        if act in ("mute", "unmute"):
            return _text(await asyncio.to_thread(p.set_mute, act == "mute"))
        return _text(await asyncio.to_thread(p.set_output, str(v or "")))

    POWER_Q: dict[str, str | None] = {
        "shutdown": "Shut down the computer?",
        "restart": "Restart the computer?",
        "sleep": "Put the computer to sleep?",
    }
    if on_mac:
        POWER_Q["switch_to_macos"] = None     # stepping aside changes nothing: no need to ask
    else:
        POWER_Q["switch_to_windows"] = "Restart into Windows? To come back to ClaudeOS, just restart: it starts by default."

    other = ctx.platform.other_os

    def _default_q(a: dict[str, Any]) -> str:
        if a.get("system") == "claudeos":
            return "Start ClaudeOS when the computer turns on?"
        return f"Start {other} when the computer turns on instead of ClaudeOS? ClaudeOS stays installed."

    @reg("start_by_default", f"Choose what starts when the computer turns on: ClaudeOS or {other} (system "
         f"'other'). Removes nothing: the one not chosen stays installed. For people who'd rather use {other} "
         "day to day. Removing ClaudeOS completely is done from ClaudeOS Setup on " + other + ".",
         obj({"system": {"type": "string", "enum": ["claudeos", "other"]}}, ["system"]), _default_q)
    async def start_by_default(a):
        ctx.journal.record("start_by_default", a["system"])
        return _text(await asyncio.to_thread(ctx.platform.set_default_os, a["system"]))

    @reg("power", ("Shut down, restart, sleep, or step aside to show macOS (switch_to_macos; saying “Claude” brings "
                    "you back)." if on_mac else "Shut down, restart, sleep, or restart into Windows (switch_to_windows)."),
         obj({"action": {"type": "string", "enum": list(POWER_Q)}}, ["action"]),
         lambda a: POWER_Q.get(a.get("action", ""), "Do that?"))
    async def power(a):
        act = a["action"]
        ctx.journal.record("power", act)
        loop = asyncio.get_running_loop()
        loop.call_later(ctx.power_delay_s, lambda: loop.run_in_executor(None, ctx.platform.power, act))
        return _text(f"OK — {act.replace('_', ' ')} in {ctx.power_delay_s:.0f} seconds. Say a short goodbye.")

    def _sw_policy(a: dict[str, Any]) -> str | None:
        pk = ", ".join(a.get("packages") or [])
        if a.get("action") == "install":
            return f"Install {pk}? It comes from " + ("Homebrew." if on_mac else "Ubuntu's software library.")
        if a.get("action") == "remove":
            return f"Remove {pk}?"
        if on_mac:
            return "Open Software Update, to update macOS and Apple's apps?"
        return "Update all the software on this computer? It can take a few minutes."

    @reg("software", ("Install or remove command-line programs with Homebrew, when it's installed (apps come from "
                       "the App Store or websites); update opens Software Update. Most things don't need a program: "
                       "use websites and pages instead." if on_mac else
                       "Install, remove or update programs on the hidden Linux layer (Ubuntu package names, e.g. "
                       "'vlc', 'libreoffice', 'ffmpeg'). Most things don't need a program: use websites and pages instead."),
         obj({"action": {"type": "string", "enum": ["install", "remove", "update"]},
              "packages": {"type": "array", "items": S}}, ["action"]), _sw_policy)
    async def software(a):
        p = ctx.platform
        if a["action"] == "update":
            return _text(await asyncio.to_thread(p.update_system))
        fn = p.install if a["action"] == "install" else p.remove
        res = await asyncio.to_thread(fn, list(a.get("packages") or []))
        ctx.journal.record("software", res)
        return _text(res)

    @reg("set_timezone", "Set the computer's time zone (IANA name like Asia/Singapore).", obj({"zone": S}, ["zone"]))
    async def set_timezone(a):
        z = str(a["zone"])
        try:
            zoneinfo.ZoneInfo(z)
        except Exception:
            return _err(f"{z} isn't a time zone name I know.")
        return _text(await asyncio.to_thread(ctx.platform.set_timezone, z))

    # ================================================================ ClaudeOS itself
    @reg("memory", "Long-term memory about the user: remember a fact or preference, forget something, or list "
         "what you remember. Remember things they'd expect you to know next time.",
         obj({"action": {"type": "string", "enum": ["remember", "forget", "list"]}, "text": S}, ["action"]))
    async def memory(a):
        if a["action"] == "remember":
            return _text(ctx.memory.remember(a.get("text", "")))
        if a["action"] == "forget":
            return _text(ctx.memory.forget(a.get("text", "")))
        return _text(ctx.memory.for_prompt() or "Nothing remembered yet.")

    @reg("claudeos_settings", "Read or change ClaudeOS settings: voice (af_heart, af_bella, af_nicole, am_michael, "
         "am_fenrir, bf_emma, bm_george...), speech_speed (0.7-1.6), theme (light = white background with black "
         "text, dark = the reverse), show_activity (true/false), auto_sleep_minutes (0 = never), barge_in, wake_words, "
         "model, effort, user_name. Text size is worked out from the screen's real size and viewing distance; "
         "'get' shows the result. To make all text bigger or smaller set text_scale (1.0 = automatic; e.g. 1.15 "
         "bigger, 0.87 smaller). When the user says how far they sit from the screen, set viewing_distance_cm. "
         "If the screen's size is wrong (some TVs don't report it), set screen_inches (the diagonal). web_zoom "
         "zooms websites (0 = automatic). text_size forces a fixed pixel size (0 = automatic). phone_access "
         "(true/false) lets them message you from the Claude app on their phone (needs the full sign-in from "
         "ClaudeOS Setup).",
         obj({"action": {"type": "string", "enum": ["get", "set"]}, "key": S, "value": {}}, ["action"]))
    async def claudeos_settings(a):
        if a["action"] == "get":
            pub = ctx.settings.public()
            out = {k: pub[k] for k in sorted(SETTABLE) if k in pub}
            info = ctx.screen.layout_info() if hasattr(ctx.screen, "layout_info") else {}
            if info:
                out["screen_now"] = info.get("summary", info)
            return _text(_json(out))
        key = a.get("key", "")
        if key not in SETTABLE:
            return _err(f"{key!r} can't be changed. Settable: {', '.join(sorted(SETTABLE))}")
        ctx.settings.set(key, a.get("value"))
        ctx.settings.save(ctx.paths)
        if ctx.on_settings_changed:
            ctx.on_settings_changed([key])
        later = " (takes effect next time ClaudeOS starts)" if key in ("model", "effort", "stt_model") else ""
        return _text(f"{key} is now {getattr(ctx.settings, key)!r}{later}.")

    @reg("stop_listening", "Stop listening until the user says the wake word (\"Claude\") or presses an AirPods "
         "stem. Use when they say goodbye, goodnight, or that they're done.", obj({}))
    async def stop_listening(a):
        if ctx.on_sleep:
            ctx.on_sleep()
        return _text("Sleeping after this reply. Say a very short goodbye.")

    server = create_sdk_mcp_server(SERVER, "1.0.0", tools)
    names = [t.name for t in tools]
    return server, policies, names


def full_names(names: list[str]) -> list[str]:
    return [f"mcp__{SERVER}__{n}" for n in names]
