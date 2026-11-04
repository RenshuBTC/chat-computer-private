"""WhatsApp, WeChat and social media, read from their own web pages inside Chat Computer.

None of them offers a way for an app to read a personal account's messages (WhatsApp and WeChat have no such API;
Instagram's and Facebook's are for business accounts; X's costs money per message read). So, as the person chose, Chat Computer
opens each site's own inbox page in a browser of its own (its sign-ins kept apart from the model's browser, in
~/ChatComputer/sources, where the model's tools can't look), and reads the list of conversations shown there: who, the
latest message, when, unread. It only reads: it never sends, posts or clicks anything by itself. The person
signs in once, on the site's own page, shown between the sidebars (WhatsApp and WeChat: by scanning its code with
the phone). WeChat's is WeChat for Web, which not every WeChat account may use (WeChat says so on the page), and
which signs out now and then (scanning again signs back in).

These sites' terms forbid reading them by automated means, so an account could be restricted; the person was told
and chose this. The pages change often; when one can't be read, Messages says so (see the log for what was found).
WhatsApp and WeChat stay open all the time (they're live connections); the others are looked at every few minutes,
one at a time.
"""
from __future__ import annotations

import hashlib
import re
import time
from datetime import datetime, timedelta
from typing import Any

SITES: dict[str, dict[str, Any]] = {
    "whatsapp": {"label": "WhatsApp", "inbox": "https://web.whatsapp.com/", "live": True,
                 "how": "Scan the code with WhatsApp on your phone: Settings, Linked devices, Link a device."},
    "wechat": {"label": "WeChat", "inbox": "https://web.wechat.com/?lang=en_US", "live": True,
               "how": "Scan the code with WeChat on your phone (+ at the top right, then Scan), and confirm on the "
                      "phone. Some WeChat accounts can't use WeChat for Web: then WeChat says so here."},
    "x": {"label": "X", "inbox": "https://x.com/messages", "how": "Sign in to X on the page."},
    "instagram": {"label": "Instagram", "inbox": "https://www.instagram.com/direct/inbox/",
                  "how": "Sign in to Instagram on the page."},
    "facebook": {"label": "Facebook", "inbox": "https://www.facebook.com/messages/t/",
                 "how": "Sign in to Facebook on the page."},
    "tiktok": {"label": "TikTok", "inbox": "https://www.tiktok.com/messages", "how": "Sign in to TikTok on the page."},
    "linkedin": {"label": "LinkedIn", "inbox": "https://www.linkedin.com/messaging/",
                 "how": "Sign in to LinkedIn on the page."},
}
READ_EVERY_S = 10 * 60           # the sites other than WhatsApp: each looked at this often
WEEKDAYS = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"]
MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]


def parse_when(s: str, now: float | None = None) -> float | None:
    """When a conversation list says a message came: "13:05", "1:05 PM", "Yesterday", "Monday", "18/10/2026",
    "Oct 18", "2h", "5m", "now"... None when it can't tell."""
    now = time.time() if now is None else now
    t = (s or "").strip().lower().replace(".", ":") if re.match(r"^\d{1,2}\.\d{2}", (s or "").strip()) else \
        (s or "").strip().lower()
    t = t.replace(" ", " ").replace("·", "").strip()
    if not t:
        return None
    today = datetime.fromtimestamp(now)
    if t in ("now", "just now", "active now"):
        return now
    m = re.fullmatch(r"(\d+)\s*(s|sec|secs|m|min|mins|h|hr|hrs|d|w|wk|y)", t)
    if m:
        n, unit = int(m.group(1)), m.group(2)
        secs = {"s": 1, "sec": 1, "secs": 1, "m": 60, "min": 60, "mins": 60, "h": 3600, "hr": 3600, "hrs": 3600,
                "d": 86400, "w": 604800, "wk": 604800, "y": 31536000}[unit]
        return now - n * secs
    m = re.fullmatch(r"(\d{1,2}):(\d{2})\s*([ap])?\.?m?\.?", t)
    if m:
        h, mi = int(m.group(1)), int(m.group(2))
        if m.group(3) == "p" and h < 12:
            h += 12
        if m.group(3) == "a" and h == 12:
            h = 0
        if h > 23 or mi > 59:
            return None
        when = today.replace(hour=h, minute=mi, second=0, microsecond=0)
        if when.timestamp() > now + 120:
            when -= timedelta(days=1)            # "23:50" seen just after midnight
        return when.timestamp()
    if t == "today":
        return now
    if t == "yesterday":
        return (today - timedelta(days=1)).replace(hour=12, minute=0, second=0, microsecond=0).timestamp()
    for i, day in enumerate(WEEKDAYS):
        if t in (day, day[:3]):
            back = (today.weekday() - i) % 7 or 7
            return (today - timedelta(days=back)).replace(hour=12, minute=0, second=0, microsecond=0).timestamp()
    m = re.fullmatch(r"(\d{1,2})[/\-](\d{1,2})(?:[/\-](\d{2,4}))?", t)
    if m:
        a, b = int(m.group(1)), int(m.group(2))
        y = int(m.group(3)) if m.group(3) else today.year
        y = y + 2000 if y < 100 else y
        d, mo = (a, b) if a > 12 else (b, a) if b > 12 else (a, b)    # day first, as most of the world writes it
        try:
            return datetime(y, mo, d, 12).timestamp()
        except ValueError:
            return None
    m = re.fullmatch(r"([a-z]{3})[a-z]*\s+(\d{1,2})(?:,?\s+(\d{4}))?|(\d{1,2})\s+([a-z]{3})[a-z]*(?:\s+(\d{4}))?", t)
    if m:
        mon, day, year = (m.group(1), m.group(2), m.group(3)) if m.group(1) else (m.group(5), m.group(4), m.group(6))
        if mon in MONTHS:
            try:
                when = datetime(int(year) if year else today.year, MONTHS.index(mon) + 1, int(day), 12)
            except ValueError:
                return None
            if when.timestamp() > now + 86400:
                when = when.replace(year=when.year - 1)
            return when.timestamp()
    return None


def _key(*parts: str) -> str:
    return hashlib.sha1("\x00".join(parts).encode()).hexdigest()[:16]


def snapshot_items(provider: str, account_id: str, chats: list[dict[str, Any]], now: float | None = None,
                   ) -> list[dict[str, Any]]:
    """The conversations a site's inbox page shows, as messages: each conversation's latest one. A conversation
    whose latest message changed gives a new message (the earlier one stays in Messages)."""
    now = time.time() if now is None else now
    label = SITES.get(provider, {}).get("label", provider)
    items = []
    for c in chats[:40]:
        name = " ".join(str(c.get("name") or "").split())[:120]
        text = " ".join(str(c.get("text") or "").split())[:500]
        if not name or not text:
            continue
        when = parse_when(str(c.get("time") or ""), now) or now
        items.append({"id": f"web:{provider}:{_key(name, text, str(c.get('time') or ''))}", "source": provider,
                      "account": account_id, "label": label, "who": name, "address": "", "title": "",
                      "text": text, "ts": when, "unread": bool(c.get("unread")), "chat": name,
                      "ref": str(c.get("link") or "")})
    return items
