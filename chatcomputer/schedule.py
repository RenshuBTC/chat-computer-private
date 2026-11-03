"""Timers, reminders and alarms: what's to ring when, kept in ~/ChatComputer/schedule.json so they last through a
restart. The controller rings them (Controller._keep_time); the model sets them with the reminders tool.

Each item: {"id", "kind": "timer" | "reminder" | "alarm", "at": when it rings (seconds since 1970), "created",
"label", "repeat": "" | "daily" | "weekdays" | "weekends", and for a timer "duration" (seconds)}."""
from __future__ import annotations

import datetime as dt
import json
import re
import threading
import time
import uuid
from pathlib import Path
from typing import Any

from .config import atomic_write

KINDS = ("timer", "reminder", "alarm")
REPEATS = ("", "daily", "weekdays", "weekends")
LATE_S = 10 * 60          # one that comes due more than this late (the computer was off or asleep) is "missed"


class Schedule:
    def __init__(self, path: Path) -> None:
        self.path = Path(path)
        self.lock = threading.Lock()
        self._items: list[dict[str, Any]] = []
        try:
            data = json.loads(self.path.read_text(encoding="utf-8"))
            self._items = [i for i in data if isinstance(i, dict) and i.get("kind") in KINDS
                           and isinstance(i.get("at"), (int, float))]
        except (OSError, ValueError, TypeError):
            self._items = []

    def _save(self) -> None:
        atomic_write(self.path, json.dumps(self._items, indent=1, ensure_ascii=False))

    def items(self) -> list[dict[str, Any]]:
        with self.lock:
            return sorted((dict(i) for i in self._items), key=lambda i: i["at"])

    def add(self, kind: str, at: float, label: str = "", repeat: str = "", duration: float | None = None,
            now: float | None = None) -> dict[str, Any]:
        if kind not in KINDS:
            raise ValueError(f"kind must be one of {KINDS}")
        if repeat not in REPEATS:
            raise ValueError("repeat is daily, weekdays, weekends, or nothing")
        item: dict[str, Any] = {"id": kind[0] + uuid.uuid4().hex[:5], "kind": kind, "at": float(at),
                                "created": float(now if now is not None else time.time()),
                                "label": (label or "").strip()[:200], "repeat": repeat}
        if duration is not None:
            item["duration"] = float(duration)
        if repeat:
            item["at"] = next_repeat(item["at"] - 1, repeat, item["at"])
        with self.lock:
            self._items.append(item)
            self._save()
        return dict(item)

    def cancel(self, which: str) -> list[dict[str, Any]]:
        """Cancel one by its id, all of a kind ("timer"s...), or "all". Returns what was cancelled."""
        w = (which or "").strip().lower()
        with self.lock:
            gone = [i for i in self._items if w in ("all", i["id"], i["kind"], i["kind"] + "s")]
            self._items = [i for i in self._items if i not in gone]
            self._save()
        return gone

    def due(self, now: float | None = None) -> list[dict[str, Any]]:
        now = time.time() if now is None else now
        with self.lock:
            return [dict(i) for i in self._items if i["at"] <= now]

    def rang(self, item_id: str, now: float | None = None) -> None:
        """It rang (or was missed): gone, or for one that repeats, set for its next time."""
        now = time.time() if now is None else now
        with self.lock:
            for i in list(self._items):
                if i["id"] != item_id:
                    continue
                if i.get("repeat"):
                    i["at"] = next_repeat(max(now, i["at"]), i["repeat"], i["at"])
                else:
                    self._items.remove(i)
            self._save()

    def next_at(self) -> float | None:
        with self.lock:
            return min((i["at"] for i in self._items), default=None)


def next_repeat(after: float, repeat: str, clock_from: float) -> float:
    """The next time after `after` at the clock time of `clock_from`, on a day `repeat` allows."""
    t = dt.datetime.fromtimestamp(clock_from)
    day = dt.datetime.fromtimestamp(after).date()
    for _ in range(9):
        cand = dt.datetime.combine(day, t.time())
        ok = repeat == "daily" or (repeat == "weekdays" and cand.weekday() < 5) or \
            (repeat == "weekends" and cand.weekday() >= 5) or not repeat
        if ok and cand.timestamp() > after:
            return cand.timestamp()
        day += dt.timedelta(days=1)
    return after + 86400


_CLOCK = re.compile(r"^\s*(\d{1,2})(?:[:.h](\d{2}))?\s*(am|pm|a\.m\.|p\.m\.)?\s*$", re.I)


def clock_time(text: str) -> dt.time:
    """'18:00', '6pm', '6:30 pm', '07.15', '7' as a clock time."""
    m = _CLOCK.match(text or "")
    if not m:
        raise ValueError(f"{text!r} isn't a time of day (like 18:00 or 6pm)")
    h, mi = int(m.group(1)), int(m.group(2) or 0)
    ap = (m.group(3) or "").lower().replace(".", "")
    if ap == "pm" and h < 12:
        h += 12
    elif ap == "am" and h == 12:
        h = 0
    if not (0 <= h < 24 and 0 <= mi < 60):
        raise ValueError(f"{text!r} isn't a time of day")
    return dt.time(h, mi)


def when(at: str | None = None, date: str | None = None, in_minutes: float | None = None,
         now: float | None = None) -> float:
    """When something should ring, from what the model gives: in_minutes from now; or at (a clock time, or
    'YYYY-MM-DD HH:MM') with an optional date ('YYYY-MM-DD', 'today', 'tomorrow'). A clock time without a date is
    the next time it comes (today if it's still ahead, else tomorrow)."""
    now = time.time() if now is None else now
    if in_minutes is not None and str(in_minutes).strip() != "":
        mins = float(in_minutes)
        if mins <= 0:
            raise ValueError("in_minutes must be more than 0")
        return now + mins * 60
    if not at:
        raise ValueError("Say when: at (like 18:00) or in_minutes")
    at = at.strip()
    m = re.match(r"^(\d{4}-\d{2}-\d{2})[ T](.+)$", at)
    if m:
        date, at = m.group(1), m.group(2)
    t = clock_time(at)
    today = dt.datetime.fromtimestamp(now).date()
    d = (date or "").strip().lower()
    if d in ("", "next"):
        cand = dt.datetime.combine(today, t)
        if cand.timestamp() <= now:
            cand += dt.timedelta(days=1)
        return cand.timestamp()
    if d == "today":
        day = today
    elif d == "tomorrow":
        day = today + dt.timedelta(days=1)
    else:
        try:
            day = dt.date.fromisoformat(d)
        except ValueError:
            raise ValueError(f"{date!r} isn't a date (like 2026-10-21, today or tomorrow)") from None
    ts = dt.datetime.combine(day, t).timestamp()
    if ts <= now:
        raise ValueError("That time has already passed.")
    return ts


def say_duration(seconds: float) -> str:
    """600 -> '10-minute', 90 -> '90-second', 5400 -> '90-minute', 7200 -> '2-hour' (for "your 10-minute timer")."""
    s = round(seconds)
    if s % 3600 == 0 and s >= 7200:
        return f"{s // 3600}-hour"
    if s % 60 == 0:
        return f"{s // 60}-minute"
    if s < 120:
        return f"{s}-second"
    return f"{s // 60}-minute {s % 60}-second"


def clock(ts: float) -> str:
    """A time as it's said: 18:00 -> '6 pm', 07:30 -> '7:30 am'."""
    t = dt.datetime.fromtimestamp(ts)
    h = t.hour % 12 or 12
    return f"{h}{':%02d' % t.minute if t.minute else ''} {'am' if t.hour < 12 else 'pm'}"


def describe(item: dict[str, Any], now: float | None = None) -> str:
    """'a 10-minute timer (rings at 3:10 pm)', 'a reminder at 6 pm today: call Mum', 'an alarm at 7 am tomorrow'."""
    now = time.time() if now is None else now
    at = item["at"]
    days = (dt.datetime.fromtimestamp(at).date() - dt.datetime.fromtimestamp(now).date()).days
    day = "today" if days == 0 else "tomorrow" if days == 1 else dt.datetime.fromtimestamp(at).strftime("%A %d %B")
    rep = {"daily": " every day", "weekdays": " every weekday", "weekends": " at weekends"}.get(item.get("repeat") or "", "")
    label = item.get("label") or ""
    if item["kind"] == "timer":
        return f"a {say_duration(item.get('duration') or at - item['created'])} timer" + \
            (f" for {label}" if label else "") + f" (rings at {clock(at)})"
    what = "a reminder" if item["kind"] == "reminder" else "an alarm"
    return f"{what} at {clock(at)}{rep or ' ' + day}" + (f": {label}" if label else "")
