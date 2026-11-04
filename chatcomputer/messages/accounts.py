"""The accounts Messages reads, and their sign-ins, in ~/ChatComputer/secrets/accounts.json (readable by this user only,
in a folder the model's tools can't open). What the person types in a sign-in form is kept here and never shown to
the model; `public()` is what the screen and the model get."""
from __future__ import annotations

import json
import os
import threading
import uuid
from pathlib import Path
from typing import Any

SECRET_KEYS = {"password", "refresh_token", "access_token", "session", "api_hash"}

# Email providers: IMAP server, and how to sign in. Outlook.com no longer takes passwords: it uses Microsoft's
# sign-in (msauth.py).
MAIL = {
    "gmail": {"label": "Gmail", "host": "imap.gmail.com", "port": 993},
    "icloud": {"label": "iCloud Mail", "host": "imap.mail.me.com", "port": 993},
    "yahoo": {"label": "Yahoo Mail", "host": "imap.mail.yahoo.com", "port": 993},
    "outlook": {"label": "Outlook", "host": "outlook.office365.com", "port": 993, "oauth": True},
    "imap": {"label": "Email", "host": "", "port": 993},
}


class Accounts:
    def __init__(self, path: Path) -> None:
        self.path = path
        self._lock = threading.Lock()
        self._list: list[dict[str, Any]] = []
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
            self._list = [a for a in data.get("accounts", []) if isinstance(a, dict) and a.get("id") and a.get("kind")]
        except (OSError, ValueError, AttributeError):
            pass

    def all(self) -> list[dict[str, Any]]:
        with self._lock:
            return [dict(a) for a in self._list]

    def get(self, account_id: str) -> dict[str, Any] | None:
        with self._lock:
            return next((dict(a) for a in self._list if a["id"] == account_id), None)

    def find(self, kind: str, provider: str | None = None, address: str | None = None) -> dict[str, Any] | None:
        with self._lock:
            for a in self._list:
                if a["kind"] == kind and (provider is None or a.get("provider") == provider) and \
                        (address is None or (a.get("address") or "").lower() == address.lower()):
                    return dict(a)
        return None

    def add(self, account: dict[str, Any]) -> dict[str, Any]:
        """Adds an account, or replaces the one with the same kind, provider and address (signing in again)."""
        with self._lock:
            same = next((a for a in self._list if a["kind"] == account["kind"] and a.get("provider") == account.get("provider")
                         and (a.get("address") or "").lower() == (account.get("address") or "").lower()), None)
            account = dict(account)
            account["id"] = same["id"] if same else account.get("id") or "a" + uuid.uuid4().hex[:8]
            self._list = [a for a in self._list if a is not same] + [account]
            self._save()
            return dict(account)

    def update(self, account_id: str, **changes: Any) -> None:
        with self._lock:
            for a in self._list:
                if a["id"] == account_id:
                    a.update(changes)
            self._save()

    def remove(self, account_id: str) -> dict[str, Any] | None:
        with self._lock:
            gone = next((a for a in self._list if a["id"] == account_id), None)
            self._list = [a for a in self._list if a["id"] != account_id]
            self._save()
            return gone

    def _save(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        tmp = self.path.with_suffix(".tmp")
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump({"accounts": self._list}, f)
        os.replace(tmp, self.path)
        os.chmod(self.path, 0o600)


def public(account: dict[str, Any]) -> dict[str, Any]:
    """An account without its sign-in: what the screen and the model may see."""
    return {k: v for k, v in account.items() if k not in SECRET_KEYS and not k.startswith("_")}
