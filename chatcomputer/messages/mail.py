"""Email over IMAP: Gmail, iCloud Mail, Yahoo and any other provider with IMAP, signed in with an app password the
person types into Chat Computer's own form; Outlook.com with Microsoft's sign-in (msauth.py, XOAUTH2).

The inbox is only read, never changed: messages are fetched with BODY.PEEK, so they stay unread in the account
until the person reads them somewhere. New mail is looked for every couple of minutes; for each message Chat Computer
keeps who it's from, the subject and the first lines (a full message is fetched when it's opened). Gmail shows
only the Primary tab's mail, not Promotions or Social.
"""
from __future__ import annotations

import email
import email.policy
import email.utils
import html.parser
import imaplib
import re
import ssl
import time
from typing import Any, Callable

from . import msauth

FIRST_DAYS = 14                  # the first time: mail from the last two weeks...
FIRST_MAX = 40                   # ...at most this many
BATCH = 20
PREVIEW_BYTES = 24_000           # of each new message: enough for its headers and first lines
FLAGS_BACK = 60                  # read/unread is checked again for this many of the latest
TIMEOUT_S = 30


class AuthError(Exception):
    """The account didn't take the sign-in (a changed or revoked app password, an expired Microsoft sign-in)."""


class _Text(html.parser.HTMLParser):
    """The words of an HTML email, without its styles and scripts."""

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.out: list[str] = []
        self._skip = 0

    def handle_starttag(self, tag: str, attrs: Any) -> None:
        if tag in ("style", "script", "head", "title"):
            self._skip += 1
        elif tag in ("br", "p", "div", "tr", "li", "h1", "h2", "h3", "h4", "table"):
            self.out.append("\n")

    def handle_endtag(self, tag: str) -> None:
        if tag in ("style", "script", "head", "title") and self._skip:
            self._skip -= 1
        elif tag in ("p", "div", "tr", "li", "h1", "h2", "h3", "h4", "table"):
            self.out.append("\n")

    def handle_data(self, data: str) -> None:
        if not self._skip:
            self.out.append(data)


def html_to_text(s: str) -> str:
    p = _Text()
    try:
        p.feed(s)
        p.close()
    except Exception:
        return re.sub(r"<[^>]+>", " ", s)
    text = "".join(p.out)
    text = re.sub(r"[ \t\r\f\v ‌​]+", " ", text)
    return re.sub(r"\n\s*\n\s*(\n\s*)+", "\n\n", text).strip()


def _part_text(part: Any) -> str:
    try:
        return part.get_content()
    except Exception:
        raw = part.get_payload(decode=True) or b""
        return raw.decode(part.get_content_charset() or "utf-8", errors="replace")


def body_text(msg: Any) -> str:
    """The readable words of a message: its plain text part, else its HTML made plain."""
    plain = htm = None
    for part in msg.walk():
        if part.is_multipart() or part.get_content_disposition() == "attachment":
            continue
        ctype = part.get_content_type()
        if ctype == "text/plain" and plain is None:
            plain = _part_text(part)
        elif ctype == "text/html" and htm is None:
            htm = _part_text(part)
    if plain and plain.strip():
        return plain.strip()
    return html_to_text(htm) if htm else ""


def attachments(msg: Any) -> list[str]:
    return [p.get_filename() for p in msg.walk() if p.get_content_disposition() == "attachment" and p.get_filename()]


def _who(msg: Any) -> tuple[str, str]:
    name, addr = email.utils.parseaddr(str(msg.get("From", "")))
    return (name or addr or "Unknown sender"), addr


def _when(msg: Any, internal: str | None) -> float:
    try:
        return email.utils.parsedate_to_datetime(str(msg.get("Date"))).timestamp()
    except Exception:
        pass
    if internal:
        try:
            return time.mktime(imaplib.Internaldate2tuple(f'INTERNALDATE "{internal}"'.encode()))
        except Exception:
            pass
    return time.time()


def snippet(text: str, n: int = 240) -> str:
    t = " ".join(text.split())
    return t[:n].rstrip() + ("…" if len(t) > n else "")


def parse_fetch(data: list[Any]) -> list[tuple[int, set[str], str | None, bytes]]:
    """imaplib's answer to UID FETCH: (uid, flags, internal date, the bytes) for each message."""
    out = []
    for part in data:
        if not isinstance(part, tuple) or len(part) < 2:
            continue
        head = part[0].decode(errors="replace") if isinstance(part[0], bytes) else str(part[0])
        m = re.search(r"UID (\d+)", head)
        if not m:
            continue
        flags = set(re.findall(r"\\\w+", (re.search(r"FLAGS \(([^)]*)\)", head) or [None, ""])[1]))
        idate = re.search(r'INTERNALDATE "([^"]+)"', head)
        out.append((int(m.group(1)), flags, idate.group(1) if idate else None, part[1] or b""))
    return out


def parse_flags(data: list[Any]) -> dict[int, set[str]]:
    out = {}
    for part in data:
        line = part[0] if isinstance(part, tuple) else part
        line = line.decode(errors="replace") if isinstance(line, bytes) else str(line or "")
        m = re.search(r"UID (\d+)", line)
        f = re.search(r"FLAGS \(([^)]*)\)", line)
        if m and f:
            out[int(m.group(1))] = set(re.findall(r"\\\w+", f.group(1)))
    return out


class Mailbox:
    """One email account's inbox, read over IMAP (blocking: run it in a worker thread)."""

    def __init__(self, account: dict[str, Any], save_tokens: Callable[[dict[str, Any]], None] | None = None,
                 imap_factory: Callable[..., Any] | None = None, client_id: str = "") -> None:
        self.account = account
        self.save_tokens = save_tokens or (lambda t: None)
        self.imap_factory = imap_factory or (lambda host, port: imaplib.IMAP4_SSL(
            host, port, ssl_context=ssl.create_default_context(), timeout=TIMEOUT_S))
        self.client_id = client_id
        self.uidvalidity: str | None = account.get("_uidvalidity")
        self.last_uid = int(account.get("_last_uid") or 0)

    # ------------------------------------------------------------------ signing in
    def _token(self) -> str:
        a = self.account
        if a.get("access_token") and float(a.get("expires_at") or 0) > time.time():
            return a["access_token"]
        if not a.get("refresh_token"):
            raise AuthError("this Outlook account needs signing in again")
        try:
            t = msauth.refresh(a.get("client_id") or self.client_id, a["refresh_token"])
        except msauth.SignInError as e:
            raise AuthError(str(e)) from None
        a.update(access_token=t["access_token"], expires_at=t["expires_at"],
                 refresh_token=t.get("refresh_token") or a["refresh_token"])
        self.save_tokens({k: a[k] for k in ("access_token", "expires_at", "refresh_token")})
        return a["access_token"]

    def connect(self) -> Any:
        a = self.account
        m = self.imap_factory(a["host"], int(a.get("port") or 993))
        try:
            if a.get("oauth"):
                token = self._token()
                m.authenticate("XOAUTH2", lambda _: msauth.xoauth2(a.get("user") or a["address"], token))
            else:
                m.login(a.get("user") or a["address"], a.get("password") or "")
        except imaplib.IMAP4.error as e:
            try:
                m.logout()
            except Exception:
                pass
            raise AuthError(str(e)) from None
        return m

    # ------------------------------------------------------------------ reading
    def _select(self, m: Any) -> str:
        typ, _ = m.select("INBOX", readonly=True)
        if typ != "OK":
            raise RuntimeError("couldn't open the inbox")
        v = m.response("UIDVALIDITY")[1]
        return (v[0].decode() if v and isinstance(v[0], bytes) else str(v[0] if v else "0"))

    def _search(self, m: Any, *criteria: str) -> list[int]:
        typ, data = m.uid("SEARCH", *criteria)
        if typ != "OK" or not data:
            return []
        return sorted(int(x) for x in (data[0] or b"").split() if x.isdigit())

    def poll(self) -> list[dict[str, Any]]:
        """New mail since last time (or the last two weeks' the first time), and read/unread of the latest."""
        a = self.account
        m = self.connect()
        try:
            validity = self._select(m)
            if validity != self.uidvalidity:
                self.uidvalidity, self.last_uid = validity, 0
            gmail = b"X-GM-EXT-1" in getattr(m, "capabilities", ()) or "X-GM-EXT-1" in getattr(m, "capabilities", ())
            primary = ("X-GM-RAW", '"category:primary"') if gmail else ()
            if self.last_uid:
                uids = [u for u in self._search(m, "UID", f"{self.last_uid + 1}:*", *primary) if u > self.last_uid]
            else:
                since = time.strftime("%d-%b-%Y", time.gmtime(time.time() - FIRST_DAYS * 86400))
                uids = self._search(m, "SINCE", since, *primary)[-FIRST_MAX:]
            items = []
            for i in range(0, len(uids), BATCH):
                chunk = ",".join(str(u) for u in uids[i:i + BATCH])
                typ, data = m.uid("FETCH", chunk, f"(UID FLAGS INTERNALDATE BODY.PEEK[]<0.{PREVIEW_BYTES}>)")
                if typ == "OK":
                    items += [self._item(uid, flags, idate, raw) for uid, flags, idate, raw in parse_fetch(data)]
            if uids:
                self.last_uid = max(self.last_uid, max(uids))
            # Read somewhere else since: no longer unread here either.
            if self.last_uid:
                typ, data = m.uid("FETCH", f"{max(1, self.last_uid - FLAGS_BACK)}:{self.last_uid}", "(UID FLAGS)")
                if typ == "OK":
                    fresh = {i["ref"] for i in items}
                    for uid, flags in parse_flags(data).items():
                        ref = f"{self.uidvalidity}:{uid}"
                        if ref not in fresh and "\\Seen" in flags:
                            items.append({"id": self._id(uid), "_seen": True})
            a["_uidvalidity"], a["_last_uid"] = self.uidvalidity, self.last_uid
            return items
        finally:
            try:
                m.logout()
            except Exception:
                pass

    def _id(self, uid: int) -> str:
        return f"mail:{self.account['id']}:{self.uidvalidity}:{uid}"

    def _item(self, uid: int, flags: set[str], idate: str | None, raw: bytes) -> dict[str, Any]:
        msg = email.message_from_bytes(raw, policy=email.policy.default)
        who, addr = _who(msg)
        try:
            subject = " ".join(str(msg.get("Subject", "")).split())
        except Exception:
            subject = ""
        try:
            text = snippet(body_text(msg))
        except Exception:
            text = ""
        a = self.account
        return {"id": self._id(uid), "source": "email", "account": a["id"], "label": a.get("label") or "Email",
                "who": who, "address": addr, "title": subject or "(no subject)", "text": text,
                "ts": _when(msg, idate), "unread": "\\Seen" not in flags, "chat": addr,
                "ref": f"{self.uidvalidity}:{uid}"}

    def read(self, ref: str) -> dict[str, Any]:
        """A whole message: who, to, when, subject, its words and the names of its attachments."""
        got = self.read_many([ref])
        if ref not in got:
            raise RuntimeError("that message isn't in the inbox any more")
        return got[ref]

    def read_many(self, refs: list[str]) -> dict[str, dict[str, Any]]:
        """Several whole messages (by ref), over one connection: those still in the inbox. All of them must be from
        this mailbox as it is now (its UIDVALIDITY)."""
        if not refs:
            return {}
        validity = refs[0].partition(":")[0]
        m = self.connect()
        out: dict[str, dict[str, Any]] = {}
        try:
            if self._select(m) != validity:
                raise RuntimeError("the mailbox changed; this message isn't there any more")
            for ref in refs:
                v, _, uid = ref.partition(":")
                if v != validity or not uid.isdigit():
                    continue
                typ, data = m.uid("FETCH", uid, "(UID BODY.PEEK[])")
                got = parse_fetch(data) if typ == "OK" else []
                if not got:
                    continue
                msg = email.message_from_bytes(got[0][3], policy=email.policy.default)
                who, addr = _who(msg)
                out[ref] = {"who": who, "address": addr, "to": str(msg.get("To", "")), "date": str(msg.get("Date", "")),
                            "subject": str(msg.get("Subject", "")), "text": body_text(msg),
                            "attachments": attachments(msg)}
            return out
        finally:
            try:
                m.logout()
            except Exception:
                pass
