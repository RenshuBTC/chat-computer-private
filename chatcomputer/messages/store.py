"""The messages Messages shows, from every account, in ~/ChatComputer/messages.db (SQLite). Used from the controller's
thread only."""
from __future__ import annotations

import re
import sqlite3
import time
from pathlib import Path
from typing import Any, Iterable

FIELDS = ("id", "source", "account", "label", "who", "address", "title", "text", "ts", "unread", "chat", "ref")
KEEP_DAYS = 365                  # older messages are let go (the accounts still have them)
KEEP_MAX = 50_000


class Store:
    def __init__(self, path: Path) -> None:
        self.db = sqlite3.connect(str(path), check_same_thread=False)
        self.db.execute(
            "CREATE TABLE IF NOT EXISTS messages (id TEXT PRIMARY KEY, source TEXT, account TEXT, label TEXT, "
            "who TEXT, address TEXT, title TEXT, text TEXT, ts REAL, unread INTEGER, chat TEXT, ref TEXT)")
        self.db.execute("CREATE INDEX IF NOT EXISTS messages_ts ON messages (ts)")
        self.db.execute("CREATE INDEX IF NOT EXISTS messages_account ON messages (account)")
        self.db.commit()

    @staticmethod
    def _row(r: Iterable[Any]) -> dict[str, Any]:
        d = dict(zip(FIELDS, r))
        d["unread"] = bool(d["unread"])
        return d

    def upsert(self, items: list[dict[str, Any]]) -> list[dict[str, Any]]:
        """Adds new messages and updates changed ones; returns those that are new or changed. A message read here
        stays read even when its account still says unread."""
        changed = []
        for it in items:
            row = {k: it.get(k) for k in FIELDS}
            row["unread"] = 1 if it.get("unread") else 0
            row["ts"] = float(row["ts"] or time.time())
            old = self.get(row["id"])
            if old is not None:
                if not old["unread"]:
                    row["unread"] = 0
                if all(old[k] == (bool(row[k]) if k == "unread" else row[k]) for k in FIELDS):
                    continue
            self.db.execute(f"INSERT OR REPLACE INTO messages ({','.join(FIELDS)}) VALUES "
                            f"({','.join('?' * len(FIELDS))})", [row[k] for k in FIELDS])
            changed.append(self._row(row[k] for k in FIELDS))
        if changed:
            self.db.commit()
        return changed

    def get(self, item_id: str) -> dict[str, Any] | None:
        r = self.db.execute(f"SELECT {','.join(FIELDS)} FROM messages WHERE id = ?", (item_id,)).fetchone()
        return self._row(r) if r else None

    def recent(self, n: int = 300, ids: Iterable[str] = ()) -> list[dict[str, Any]]:
        """The newest n, newest first, and the ones with `ids` (pinned) wherever they are."""
        rows = [self._row(r) for r in self.db.execute(
            f"SELECT {','.join(FIELDS)} FROM messages ORDER BY ts DESC LIMIT ?", (n,))]
        have = {r["id"] for r in rows}
        for i in ids:
            if i not in have:
                r = self.get(i)
                if r:
                    rows.append(r)
        return rows

    def thread(self, account: str, chat: str, limit: int = 30) -> list[dict[str, Any]]:
        """The conversation a message belongs to (in one account, with one person or chat): its latest `limit`
        messages, oldest first."""
        rows = [self._row(r) for r in self.db.execute(
            f"SELECT {','.join(FIELDS)} FROM messages WHERE account = ? AND chat = ? ORDER BY ts DESC LIMIT ?",
            (account, chat, max(1, int(limit))))]
        return rows[::-1]

    def mark_read(self, item_id: str) -> None:
        self.db.execute("UPDATE messages SET unread = 0 WHERE id = ?", (item_id,))
        self.db.commit()

    def search(self, query: str = "", source: str = "", days: float = 30, limit: int = 40) -> list[dict[str, Any]]:
        """Newest first: messages with all the words of `query` (in who, address, subject or text), from `source`
        (email, telegram, whatsapp, x, instagram...) if given, in the last `days` days."""
        words = re.findall(r"\w+", (query or "").lower())
        sql = f"SELECT {','.join(FIELDS)} FROM messages WHERE ts >= ?"
        args: list[Any] = [time.time() - max(0.0, float(days or 30)) * 86400]
        if source:
            sql += " AND (source = ? OR lower(label) = ?)"
            args += [source.lower(), source.lower()]
        for w in words:
            sql += " AND lower(coalesce(who,'') || ' ' || coalesce(address,'') || ' ' || coalesce(title,'') || ' ' || " \
                   "coalesce(text,'') || ' ' || coalesce(label,'')) LIKE ?"
            args.append(f"%{w}%")
        sql += " ORDER BY ts DESC LIMIT ?"
        args.append(max(1, min(int(limit or 40), 500)))
        return [self._row(r) for r in self.db.execute(sql, args)]

    def drop_account(self, account: str) -> None:
        self.db.execute("DELETE FROM messages WHERE account = ?", (account,))
        self.db.commit()

    def prune(self) -> None:
        self.db.execute("DELETE FROM messages WHERE ts < ?", (time.time() - KEEP_DAYS * 86400,))
        self.db.execute("DELETE FROM messages WHERE id NOT IN (SELECT id FROM messages ORDER BY ts DESC LIMIT ?)",
                        (KEEP_MAX,))
        self.db.commit()

    def close(self) -> None:
        self.db.close()
