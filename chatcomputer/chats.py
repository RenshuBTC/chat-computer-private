"""Chats: each task's conversation, listed in the right sidebar's lower half (newest first), as the Claude app's
sidebar lists its chats. Kept in chats.json: which is on the screen, and each one's id, title (its first words), when
it was last used, and the engine's session it continues (so opening one carries it on). What was said is in the
transcript, each line marked with its chat."""
from __future__ import annotations

import json
import os
import time
from typing import Any

from .config import Paths

MOST = 200               # chats kept in the list
TITLE_CHARS = 48


def new_id() -> str:
    return "c" + format(int(time.time() * 1000), "x")


class Chats:
    def __init__(self, paths: Paths) -> None:
        self.path = paths.root / "chats.json"
        self.list: list[dict[str, Any]] = []
        self.current = ""
        try:
            data = json.loads(self.path.read_text(encoding="utf-8"))
            if isinstance(data, dict) and isinstance(data.get("list"), list):
                self.list = [c for c in data["list"] if isinstance(c, dict) and isinstance(c.get("id"), str)]
                self.current = str(data.get("current") or "")
        except (OSError, ValueError):
            pass
        if not any(c["id"] == self.current for c in self.list):
            self.new()

    def save(self) -> None:
        self.list.sort(key=lambda c: -float(c.get("ts") or 0))
        self.list = self.list[:MOST]
        if not any(c["id"] == self.current for c in self.list):
            self.list.insert(0, {"id": self.current, "title": "", "ts": time.time()})
        tmp = self.path.with_suffix(".tmp")
        try:
            tmp.write_text(json.dumps({"current": self.current, "list": self.list}, ensure_ascii=False), encoding="utf-8")
            os.replace(tmp, self.path)
        except OSError:
            pass

    def on_screen(self) -> dict[str, Any]:
        for c in self.list:
            if c["id"] == self.current:
                return c
        c = {"id": self.current or new_id(), "title": "", "ts": time.time()}
        self.current = c["id"]
        self.list.insert(0, c)
        return c

    def new(self) -> dict[str, Any]:
        """A new chat on the screen (the one there already, if nothing's been said in it)."""
        now = self.on_screen() if self.current else None
        if now and not now.get("title"):
            return now
        c = {"id": new_id(), "title": "", "ts": time.time()}
        self.list.insert(0, c)
        self.current = c["id"]
        self.save()
        return c

    def open(self, chat_id: str) -> dict[str, Any] | None:
        for c in self.list:
            if c["id"] == chat_id:
                self.current = chat_id
                self.save()
                return c
        return None

    def said(self, text: str) -> bool:
        """Something said in the chat on the screen: its title (the first time), and it's the newest. True if its
        title is new (the list shown changes)."""
        c = self.on_screen()
        c["ts"] = time.time()
        titled = False
        if not c.get("title"):
            words = " ".join(str(text).split())
            c["title"] = words if len(words) <= TITLE_CHARS else words[:TITLE_CHARS - 1].rstrip() + "…"
            titled = True
        self.save()
        return titled

    def session(self, session_id: str | None) -> None:
        """The engine's session the chat on the screen continues."""
        c = self.on_screen()
        if session_id and c.get("session") != session_id:
            c["session"] = session_id
            self.save()

    def shown(self) -> list[dict[str, Any]]:
        """The list as the page shows it: id, title, when."""
        return [{"id": c["id"], "title": c.get("title") or "", "ts": c.get("ts") or 0} for c in self.list]
