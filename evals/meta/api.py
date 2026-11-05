"""Meta's Model API, for the evals: one chat-completions request, asked again on the errors the kernel asks again on
(429, 5xx, model_not_found), and what it costs, estimated from the tokens it reports at publicly reported prices
(Muse Spark: $1.25 a million tokens read, $4.25 written; Muse Wonder's isn't published: $5 and $15 assumed). The key
is read from the file META_API_KEY_FILE names and never written anywhere."""
from __future__ import annotations

import json
import os
import ssl
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

URL = "https://api.meta.ai/v1/chat/completions"
PRICE = {"muse-spark-1.3": (1.25, 4.25), "muse-spark-1.2": (1.25, 4.25), "muse-wonder-1.0": (5.0, 15.0)}


class Refused(RuntimeError):
    pass


class Meta:
    def __init__(self, budget: float):
        self.key = Path(os.environ["META_API_KEY_FILE"]).read_text().strip()
        self.ctx = ssl.create_default_context(cafile=os.environ.get("SSL_CERT_FILE") or None)
        self.budget = budget
        self.spent = {"usd": 0.0, "in": 0, "out": 0, "calls": 0}
        self.lock = threading.Lock()

    def over(self) -> bool:
        return self.spent["usd"] >= self.budget

    def ask(self, body: dict) -> dict:
        if self.over():
            raise Refused("budget")
        req = urllib.request.Request(URL, data=json.dumps(body).encode(), method="POST", headers={
            "Authorization": "Bearer " + self.key, "Content-Type": "application/json"})
        last = ""
        for attempt in range(6):
            try:
                with urllib.request.urlopen(req, timeout=180, context=self.ctx) as r:
                    got = json.loads(r.read())
                self.charge(body["model"], got.get("usage") or {})
                return got
            except urllib.error.HTTPError as e:
                text = e.read().decode("utf-8", "replace")[:300]
                last = f"{e.code}: {text}"
                if e.code in (402,) or "credit" in text.lower() or "quota" in text.lower():
                    raise Refused(last)
                if e.code == 429 or e.code >= 500 or "model_not_found" in text:
                    time.sleep(2 * (attempt + 1))
                    continue
                raise RuntimeError(last)
            except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
                last = str(e)
                time.sleep(2 * (attempt + 1))
        raise RuntimeError("no answer after 6 tries: " + last)

    def charge(self, model: str, usage: dict):
        i, o = usage.get("prompt_tokens", 0) or 0, usage.get("completion_tokens", 0) or 0
        cached = (usage.get("prompt_tokens_details") or {}).get("cached_tokens", 0) or 0
        pin, pout = PRICE.get(model, (5.0, 15.0))
        with self.lock:
            self.spent["in"] += i
            self.spent["out"] += o
            self.spent["calls"] += 1
            self.spent["usd"] += ((i - cached) * pin + cached * pin * 0.12 + o * pout) / 1e6
