"""Pins: items pinned to the top of the two sidebars (Actions and Messages), kept in ~/ChatComputer/pins.json as
{"done": {id: when pinned}, "messages": {id: when pinned}}. Newest pin first."""
from __future__ import annotations

import json
import os
import threading
import time
from pathlib import Path

KINDS = ("done", "messages")
MAX_PINS = 200                  # per list: the oldest pins go past this


class Pins:
    def __init__(self, path: Path) -> None:
        self.path = path
        self._lock = threading.Lock()
        self._pins: dict[str, dict[str, float]] = {k: {} for k in KINDS}
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
            for k in KINDS:
                if isinstance(data.get(k), dict):
                    self._pins[k] = {str(i): float(t) for i, t in data[k].items() if isinstance(t, (int, float))}
        except (OSError, ValueError, AttributeError):
            pass

    def of(self, kind: str) -> dict[str, float]:
        with self._lock:
            return dict(self._pins.get(kind, {}))

    def set(self, kind: str, item_id: str, on: bool) -> float | None:
        """Pin (on) or unpin; returns when it was pinned (None: not pinned)."""
        if kind not in KINDS or not item_id:
            return None
        with self._lock:
            pins = self._pins[kind]
            if on:
                pins[item_id] = pins.get(item_id) or time.time()
                while len(pins) > MAX_PINS:
                    pins.pop(min(pins, key=pins.__getitem__))
            else:
                pins.pop(item_id, None)
            self._save()
            return pins.get(item_id)

    def _save(self) -> None:
        tmp = self.path.with_suffix(".tmp")
        try:
            tmp.write_text(json.dumps(self._pins), encoding="utf-8")
            os.replace(tmp, self.path)
        except OSError:
            pass
