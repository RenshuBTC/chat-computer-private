"""The lock screen's PIN. It's typed on the screen (the lock screen's keypad, or the form that sets it), checked here,
and kept only as a salted PBKDF2 hash in Chat Computer's private folder (secrets/lock_pin): it never reaches the model,
the conversation or the logs.

Wrong PINs make the next try wait, longer each time (as phones do), so a PIN can't be guessed by trying them all."""
from __future__ import annotations

import base64
import hashlib
import hmac
import json
import os
import re
import time
from pathlib import Path

from .config import atomic_write

PIN_RE = re.compile(r"^\d{4,12}$")
ITERATIONS = 300_000
FREE_TRIES = 5              # wrong PINs before having to wait
FIRST_WAIT_S = 30.0         # then 30 s, doubling each time, up to...
MAX_WAIT_S = 15 * 60.0


class PinLock:
    def __init__(self, path: Path) -> None:
        self.path = Path(path)
        self.failures = 0
        self.wait_until = 0.0

    def has_pin(self) -> bool:
        return self.path.is_file()

    def set_pin(self, pin: str) -> None:
        if not PIN_RE.match(pin or ""):
            raise ValueError("A PIN is 4 to 12 digits.")
        salt = os.urandom(16)
        digest = hashlib.pbkdf2_hmac("sha256", pin.encode(), salt, ITERATIONS)
        atomic_write(self.path, json.dumps({"salt": base64.b64encode(salt).decode(), "iterations": ITERATIONS,
                                            "hash": base64.b64encode(digest).decode()}), mode=0o600)
        self.failures, self.wait_until = 0, 0.0

    def remove_pin(self) -> None:
        try:
            self.path.unlink()
        except FileNotFoundError:
            pass
        self.failures, self.wait_until = 0, 0.0

    def _matches(self, pin: str) -> bool:
        try:
            d = json.loads(self.path.read_text(encoding="utf-8"))
            salt, want = base64.b64decode(d["salt"]), base64.b64decode(d["hash"])
            got = hashlib.pbkdf2_hmac("sha256", (pin or "").encode(), salt, int(d.get("iterations", ITERATIONS)))
        except (OSError, ValueError, KeyError, TypeError):
            return False
        return hmac.compare_digest(got, want)

    def check(self, pin: str, now: float | None = None) -> tuple[bool, str]:
        """Whether pin is the PIN; when it isn't, what to say (and whether to wait before the next try)."""
        now = time.time() if now is None else now
        if not self.has_pin():
            return True, ""
        if now < self.wait_until:
            return False, f"Too many wrong PINs. Try again in {int(self.wait_until - now) + 1} seconds."
        if self._matches(pin):
            self.failures, self.wait_until = 0, 0.0
            return True, ""
        self.failures += 1
        if self.failures >= FREE_TRIES:
            wait = min(MAX_WAIT_S, FIRST_WAIT_S * 2 ** (self.failures - FREE_TRIES))
            self.wait_until = now + wait
            return False, f"That's not the PIN. Try again in {int(wait)} seconds."
        return False, "That's not the PIN."
