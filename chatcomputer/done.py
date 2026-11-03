"""Actions: everything Chat Computer did for the person (opened a site, searched the web, changed a file, joined a
Wi-Fi network, installed a program...), each linked to the message that asked for it. Kept in
~/ChatComputer/done.jsonl, newest last.

The conversation screen lists it in a panel at the right, "Actions". Tapping one scrolls back to the message
that asked for it; when that message isn't on the screen any more (an earlier day), the conversation from there on
is put back from the transcript first. The model reads the list with recent_actions ("what did I change yesterday?").

Each one keeps the ids of the changes it made that can be taken back (the journal's records: files changed, and
settings with how to put them back, journal.py). One with changes not yet taken back has Revert, which takes them back
together (Controller.revert) and is listed itself ("Reverted …"): it can be reverted in turn, which puts the action
back. One whose changes have all been taken back (by Revert, or by "undo that") shows as reverted.

Only what was really done is kept: a tool that failed, or that only looked at something, isn't listed. (Steps that
only read or looked, like reading a file, are kept for recent_actions but not listed.)
"""
from __future__ import annotations

import json
import os
import re
import threading
import time
import uuid
from typing import Any
from urllib.parse import parse_qs, urlparse

from .config import Paths

DONE_TAIL_BYTES = 8 * 1024 * 1024          # how much of the end of done.jsonl is read (tens of thousands of entries)
DONE_MAX_BYTES = 16 * 1024 * 1024          # past this, only the newest half is kept
TRANSCRIPT_TAIL_BYTES = 64 * 1024 * 1024
EARLIER_MAX = 300                          # messages put back on the screen at most, going back to an earlier one
# How Chat Computer's own tools say something didn't work (chatcomputer/platform: "Failed: ...", "I couldn't ...").
FAILED = re.compile(r"^\s*(failed|i couldn't|couldn't|i can't|can't|install failed|remove failed|update failed|"
                    r"there's no|there is no|which |no volume|those don't|unknown |nothing to)", re.I)
SEARCH_PARAMS = ("q", "search_query", "query", "k", "p", "text", "search", "wd")


def _base(p: Any) -> str:
    p = str(p or "").rstrip("/")
    return os.path.basename(p) or p


def _names(paths: list[str], n: int = 3) -> str:
    names = [_base(p) for p in paths if p]
    return ", ".join(names[:n]) + (f" and {len(names) - n} more" if len(names) > n else "")


def _quote(s: str, n: int = 60) -> str:
    s = " ".join(str(s).split())
    return "“" + (s[:n].rstrip() + "…" if len(s) > n else s) + "”"


def _host(url: str) -> str:
    host = (urlparse(url if "://" in url else "https://" + url).netloc or url)[:60]
    return host[4:] if host.startswith("www.") else host


def _site(url: str) -> str:
    """'Opened youtube.com', or 'Searched youtube.com for “lofi”' for a search page."""
    u = urlparse(url if "://" in url else "https://" + url)
    if u.scheme == "file":
        return "Opened " + _base(u.path)
    qs = parse_qs(u.query)
    for k in SEARCH_PARAMS:
        if qs.get(k) and qs[k][0].strip():
            return f"Searched {_host(url)} for {_quote(qs[k][0])}"
    return "Opened " + _host(url)


def result_text(content: Any) -> str:
    """The words of a tool's result (a string, or a list of text parts)."""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(str(p.get("text", "")) for p in content if isinstance(p, dict) and p.get("type") == "text")
    return ""


def describe(name: str, inp: dict[str, Any], result: str = "") -> tuple[str, bool] | None:
    """What a tool call that worked did, in the past tense, and whether it's a minor step (reading or looking:
    kept for recent_actions, not listed). None when there's nothing to keep: it only looked at something, or one of
    Chat Computer's own tools said it didn't work (they say so in words: "Failed: ...")."""
    short = name.split("__")[-1]
    ours = name.startswith("mcp__chatcomputer__")
    if ours and FAILED.match(result or ""):
        return None
    g = lambda k: str(inp.get(k, "") or "")           # noqa: E731
    act = g("action")
    items = [i for i in inp.get("items") or [] if isinstance(i, dict)]
    srcs = [str(i.get("from", "")) for i in items]
    if short == "show_website":
        return _site(g("url")), False
    if short == "show_file":
        return "Opened " + _base(g("path")), False
    if short == "show_document":
        return "Showed " + _quote(g("title")), False
    if short == "show_page":
        return "Made " + _quote(g("title")), False
    if short == "saved_pages":
        return {"open": ("Opened the saved page " + _quote(g("name")), False),
                "delete": ("Deleted the saved page " + _quote(g("name")), False)}.get(act)
    if short == "click":
        return ("Clicked " + _quote(g("text"), 40)) if g("text") else "Clicked on the page", True
    if short == "type_text":
        return ("Typed " + _quote(g("text"), 40) + (" and pressed Enter" if inp.get("submit") else ""),
                not inp.get("submit"))
    if short == "press_keys":
        return "Pressed " + g("keys")[:40], True
    if short == "media":
        return {"play": "Played the video", "pause": "Paused the video", "seek": "Skipped in the video",
                "volume": "Changed the video's volume", "fullscreen": "Made the video full screen"}.get(
            act, "Controlled the video"), True
    if short == "search_files":
        return ("Searched your files for " + _quote(g("query")), False) if g("query") else None
    if short == "move_files":
        return "Moved " + (_names(srcs) or "files"), False
    if short == "copy_files":
        return "Copied " + (_names(srcs) or "files"), False
    if short == "trash_files":
        return "Moved " + (_names([str(p) for p in inp.get("paths") or []]) or "files") + " to the trash", False
    if short == "make_folder":
        return "Made the folder " + _base(g("path")), False
    if short == "undo_last_change":
        return "Undid the last change", False
    if short == "wifi":
        return {"connect": ("Joined the Wi-Fi network " + g("network")[:50], False),
                "disconnect": ("Disconnected the Wi-Fi", False),
                "forget": ("Forgot the Wi-Fi network " + g("network")[:50], False),
                "on": ("Turned the Wi-Fi on", False), "off": ("Turned the Wi-Fi off", False),
                "speed": ("Tested the internet's speed", False)}.get(act)
    if short == "bluetooth":
        if act == "airpods":
            return None if "already" in (result or "") else ("Connected your AirPods", False)
        device = g("device")[:50] or "a Bluetooth device"
        return {"pair": ("Paired " + device, False), "connect": ("Connected " + device, False),
                "disconnect": ("Disconnected " + device, False), "forget": ("Forgot " + device, False),
                "import_from_windows": ("Copied the Bluetooth pairings from Windows", False)}.get(act)
    if short == "sound":
        if act == "set_volume":
            return f"Set the volume to {g('value').rstrip('%')}%", False
        if act == "output":
            return "Played the sound on " + g("value")[:40], False
        if act == "input":
            return "Listened with the microphone " + g("value")[:40], False
        return {"volume_up": ("Turned the volume up", False), "volume_down": ("Turned the volume down", False),
                "mute": ("Muted the sound", False), "unmute": ("Unmuted the sound", False)}.get(act)
    if short == "power":
        return {"shutdown": "Shut down the computer", "restart": "Restarted the computer",
                "sleep": "Put the computer to sleep", "switch_to_windows": "Restarted into Windows",
                "switch_to_macos": "Restarted into macOS"}.get(act, act.replace("_", " ").capitalize()), False
    if short == "start_by_default":
        return ("Set Chat Computer to start first" if g("system") == "chatcomputer" else
                "Set the other system to start first"), False
    if short == "software":
        if act == "check":
            return "Checked for updates", True
        pk = ", ".join(str(x) for x in inp.get("packages") or [])
        return {"install": "Installed " + pk, "remove": "Removed " + pk}.get(act, "Updated the software"), False
    if short == "set_timezone":
        return "Set the time zone to " + g("zone"), False
    if short == "drives":
        if act == "eject":
            return "Ejected " + (g("drive")[:50] or "the drive"), False
        return ("Opened the drive " + g("drive")[:50], False) if act == "open" else None
    if short == "printer":
        if act == "print":
            n = int(inp.get("copies") or 1) if str(inp.get("copies") or "1").isdigit() else 1
            return "Printed " + _base(g("path")) + (f" ({n} copies)" if n > 1 else ""), False
        if act == "save_pdf":
            saved = re.match(r"Saved (.+)$", (result or "").strip())
            return "Saved " + (_base(saved.group(1)) if saved else "a PDF"), False
        return ("Cancelled printing", False) if act == "cancel" else None
    if short == "screenshot":
        return "Took a screenshot" + (" of the page" if g("part") == "open" else ""), False
    if short == "backup":
        if act == "now" and (result or "").startswith("Backed up"):
            return result.split(":")[0].strip()[:120], False
        return None
    if short == "vpn":
        name = g("name")[:40] or "the VPN"
        return {"on": ("Turned on " + name, False), "off": ("Turned off " + name, False),
                "add": ("Added the VPN from " + _base(g("file")), False)}.get(act)
    if short == "display":
        if act == "brightness":
            return f"Set the brightness to {g('value').rstrip('%')}%", False
        if act in ("brighter", "dimmer"):
            return ("Made the screen brighter" if act == "brighter" else "Dimmed the screen"), False
        if act == "night_light":
            return {"on": "Turned the night light on", "off": "Turned the night light off",
                    "evenings": "Set the night light to come on in the evenings"}.get(g("mode") or "on"), False
        return None
    if short == "lock_screen":
        return ("Locked the screen", False) if act == "lock" and (result or "").startswith("Locked") else None
    if short == "reminders":
        if act in ("timer", "reminder", "alarm", "cancel") and (result or "").startswith(("Set ", "Cancelled ")):
            return re.sub(r"\s*\[\w+\]\s*$", "", result.strip()).rstrip(".")[:120], False
        return None
    if short == "memory":
        return {"remember": ("Remembered " + _quote(g("text"), 80), False),
                "forget": ("Forgot " + _quote(g("text"), 80), False)}.get(act)
    if short == "chatcomputer_settings":
        return (f"Changed {g('key')} to {g('value')[:30]}", False) if act == "set" else None
    if short == "Bash":
        from .brain.permissions import classify_bash
        desc = g("description").strip().rstrip(".")
        looked = classify_bash(g("command"))[0] == "allow"     # it only looked at something
        return (desc[:80] or "Ran a command"), looked
    if short in ("Write", "Edit", "MultiEdit", "NotebookEdit"):
        return ("Saved " if short == "Write" else "Edited ") + _base(g("file_path") or g("notebook_path")), False
    if short in ("WebSearch", "web_search"):
        return "Searched the web for " + _quote(g("query")), False
    if short in ("WebFetch", "web_fetch"):
        return "Read a page on " + _host(g("url")), True
    if short in ("Read", "NotebookRead"):
        return "Read " + _base(g("file_path") or g("notebook_path")), True
    if short in ("Glob", "Grep", "LS"):
        return "Looked through files", True
    if short == "messages":
        return ("Disconnected an account from Messages", False) if act == "disconnect" else None
    if ours or short in ("TodoWrite", "ToolSearch", "Task", "Agent", "advisor"):
        return None             # looking (read_screen, system_status...), asking, going to sleep
    return short.replace("_", " ").capitalize(), True


def detail(inp: dict[str, Any]) -> str:
    """The specifics of a tool call, for recent_actions to look through: addresses, words searched for, files..."""
    out: list[str] = []
    for k in ("url", "query", "path", "file_path", "notebook_path", "command", "title", "text", "network",
              "device", "zone", "key", "value", "name", "keys", "system"):
        v = inp.get(k)
        if isinstance(v, (str, int, float)) and str(v).strip():
            out.append(str(v)[:400])
    for k in ("packages", "paths"):
        if isinstance(inp.get(k), list):
            out += [str(x)[:300] for x in inp[k]]
    for i in inp.get("items") or []:
        if isinstance(i, dict):
            out += [str(i.get("from", ""))[:300], str(i.get("to", ""))[:300]]
    return " | ".join(x for x in out if x)


def _tail_lines(path: Any, limit: int) -> list[str]:
    """The lines at the end of a file, up to `limit` bytes of them (the first one read may be cut: it's dropped)."""
    try:
        with open(path, "rb") as f:
            size = f.seek(0, os.SEEK_END)
            f.seek(max(0, size - limit))
            lines = f.read().decode("utf-8", errors="replace").splitlines()
    except OSError:
        return []
    return lines[1:] if size > limit else lines


def _records(lines: list[str]) -> list[dict[str, Any]]:
    out = []
    for line in lines:
        try:
            item = json.loads(line)
        except ValueError:
            continue           # a line cut short when the power went off: the others still count
        if isinstance(item, dict) and isinstance(item.get("ts"), (int, float)) and isinstance(item.get("text"), str):
            out.append(item)
    return out


def _when(ts: float) -> str:
    return time.strftime("%a %d %b %H:%M", time.localtime(ts))


class History:
    """done.jsonl, and the transcript for going back to earlier messages. Read when needed (only the end of each
    file), not kept in memory."""

    def __init__(self, paths: Paths) -> None:
        self.paths = paths
        self.file = paths.done
        self._lock = threading.Lock()

    def add(self, text: str, tool: str = "", inp: dict[str, Any] | None = None, minor: bool = False,
            prompt: tuple[str, str, float] | None = None, changes: list[str] | None = None,
            reverts: str | None = None) -> dict[str, Any]:
        """Something done. `prompt`: the message that asked for it (its id on the screen, its words, its time);
        `changes`: what it changed that can be taken back (journal record ids); `reverts`: the action it reverted."""
        entry: dict[str, Any] = {"id": "d" + uuid.uuid4().hex[:10], "ts": time.time(), "text": text}
        if tool:
            entry["tool"] = tool
        d = detail(inp or {})
        if d:
            entry["detail"] = d
        if minor:
            entry["minor"] = True
        if prompt:
            entry["prompt_id"], entry["prompt"], entry["prompt_ts"] = prompt[0], prompt[1][:300], prompt[2]
        if changes:
            entry["changes"] = list(changes)
        if reverts:
            entry["reverts"] = reverts
        with self._lock:
            try:
                with open(self.file, "a", encoding="utf-8") as f:
                    f.write(json.dumps(entry, ensure_ascii=False) + "\n")
                if self.file.stat().st_size > DONE_MAX_BYTES:
                    keep = _tail_lines(self.file, DONE_MAX_BYTES // 2)
                    tmp = self.file.with_suffix(".tmp")
                    tmp.write_text("\n".join(keep) + "\n", encoding="utf-8")
                    os.replace(tmp, self.file)
            except OSError:
                pass
        return entry

    def _all(self) -> list[dict[str, Any]]:
        return _records(_tail_lines(self.file, DONE_TAIL_BYTES))

    def get(self, entry_id: str) -> dict[str, Any] | None:
        """One thing done, by its id."""
        return next((d for d in reversed(self._all()) if d.get("id") == entry_id), None)

    def recent(self, n: int = 150, pinned: dict[str, float] | None = None,
               changes: tuple[set[str], set[str]] | None = None) -> list[dict[str, Any]]:
        """The newest things done, for the panel (not the minor steps), oldest first; and the pinned ones (with when
        they were pinned), however old. `changes`: the journal's records kept, and those taken back (Revert)."""
        pinned = pinned or {}
        listed = [d for d in self._all() if not d.get("minor") or d.get("id") in pinned]
        out = listed[-n:]
        have = {d["id"] for d in out}
        out = [d for d in listed if d.get("id") in pinned and d["id"] not in have] + out
        shown = []
        for d in out:
            e = for_screen(d, changes)
            if d.get("id") in pinned:
                e["pinned"] = pinned[d["id"]]
            shown.append(e)
        return shown

    def find(self, query: str = "", days: float = 7, limit: int = 40) -> list[str]:
        """For recent_actions: what was done in the last `days` days, newest last, one line each with the message
        that asked for it. With `query`, only what has all its words (in what was done, its details, or that
        message), minor steps included."""
        words = re.findall(r"\w+", (query or "").lower())
        since = time.time() - max(0.0, float(days or 7)) * 86400
        out = []
        for d in self._all():
            if d["ts"] < since:
                continue
            if words:
                hay = " ".join((d["text"], d.get("detail", ""), d.get("prompt", ""))).lower()
                if not all(w in hay for w in words):
                    continue
            elif d.get("minor"):
                continue
            asked = f" (asked: {_quote(d['prompt'], 100)})" if d.get("prompt") else ""
            out.append(f"{_when(d['ts'])}  {d['text']}{asked}")
        return out[-max(1, int(limit or 40)):]

    def earlier(self, target_id: str | None, target_ts: float, before_ts: float | None,
                limit: int = EARLIER_MAX) -> list[dict[str, Any]]:
        """The conversation from an earlier message (the one with `target_id`, else the last one you sent by
        `target_ts`) up to `before_ts`, where what's on the screen starts: to put back on the screen. When that's
        more than `limit` messages, the ones in the middle are left out (a line says how many)."""
        msgs = [m for m in _records(_tail_lines(self.paths.transcript, TRANSCRIPT_TAIL_BYTES))
                if m.get("role") in ("user", "assistant") and m["text"].strip()]
        if not msgs:
            return []
        at = next((i for i, m in enumerate(msgs) if target_id and m.get("id") == target_id), None)
        if at is None:
            mine = [i for i, m in enumerate(msgs) if m["role"] == "user" and m["ts"] <= target_ts + 1]
            at = mine[-1] if mine else 0
        end = len(msgs)
        if before_ts is not None:
            end = next((i for i, m in enumerate(msgs) if i >= at and m["ts"] >= before_ts - 0.001), len(msgs))
        seg = [{"id": m.get("id") or "", "ts": m["ts"], "role": m["role"], "text": m["text"]} for m in msgs[at:end]]
        if len(seg) > limit:
            half = limit // 2
            seg = seg[:half] + [{"id": "", "ts": seg[half]["ts"], "role": "gap",
                                 "text": f"{len(seg) - 2 * half} more messages"}] + seg[-half:]
        return seg


def for_screen(d: dict[str, Any], changes: tuple[set[str], set[str]] | None = None) -> dict[str, Any]:
    """What the screen gets of a thing done: with "revert" when it has changes not yet taken back (that the journal
    still has), or "reverted" when they've all been taken back. `changes`: (the journal's record ids, those taken
    back); None: not known, so nothing either way."""
    out = {k: d[k] for k in ("id", "ts", "text", "prompt_id", "prompt_ts") if k in d}
    ids = d.get("changes") or []
    if ids and changes is not None:
        kept, undone = changes
        left = [c for c in ids if c not in undone]
        if not left:
            out["reverted"] = True
        elif any(c in kept for c in left):
            out["revert"] = True
    return out

