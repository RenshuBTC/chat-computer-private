"""Messages (chatcomputer/messages): every account's messages in one list at the left of the screen. These run against
stand-ins for the services (an IMAP server, Microsoft's sign-in, Telegram, a site's inbox page): what a real
account does can only be seen on a real PC."""
import asyncio
import email.message
import imaplib
import json
import os
import stat
import time
from datetime import datetime
from pathlib import Path
from types import SimpleNamespace

import pytest

from chatcomputer.config import Paths, Settings
from chatcomputer.messages import Inbox, msauth, web
from chatcomputer.messages.accounts import Accounts, public
from chatcomputer.messages.mail import AuthError, Mailbox, body_text, html_to_text, parse_fetch
from chatcomputer.messages.store import Store
from chatcomputer.messages.telegram import AuthError as TgAuthError, TelegramSignIn, TelegramSource
from chatcomputer.pins import Pins
from chatcomputer.screen import RecordingScreen


# ------------------------------------------------------------------ the store and pins
def _msg(i, **kw):
    base = {"id": f"m{i}", "source": "email", "account": "a1", "label": "Gmail", "who": f"Person {i}",
            "address": f"p{i}@example.com", "title": f"Subject {i}", "text": f"Text {i}", "ts": 1000.0 + i,
            "unread": True, "chat": f"p{i}@example.com", "ref": f"7:{i}"}
    base.update(kw)
    return base


def test_the_store_keeps_messages_and_what_was_read_here(tmp_path):
    s = Store(tmp_path / "m.db")
    assert [m["id"] for m in s.upsert([_msg(1), _msg(2)])] == ["m1", "m2"]
    assert s.upsert([_msg(1)]) == []                                  # nothing changed
    s.mark_read("m1")
    # Read here, it stays read even though the account still says unread; a changed text is a change.
    assert s.upsert([_msg(1, unread=True)]) == []
    assert [m["text"] for m in s.upsert([_msg(1, text="New")])] == ["New"] and not s.get("m1")["unread"]
    assert [m["id"] for m in s.recent(1)] == ["m2"] and [m["id"] for m in s.recent(1, ids=["m1"])] == ["m2", "m1"]
    assert [m["id"] for m in s.search("person 2", days=1e6)] == ["m2"]
    assert s.search("person", source="telegram", days=1e6) == []
    s.upsert([_msg(3, chat="p1@example.com"), _msg(4, account="a2", chat="p1@example.com")])
    assert [m["id"] for m in s.thread("a1", "p1@example.com")] == ["m1", "m3"]          # oldest first
    s.drop_account("a1")
    assert [m["id"] for m in s.recent()] == ["m4"]


def test_pins_last_and_newest_pin_comes_first(tmp_path):
    p = Pins(tmp_path / "pins.json")
    assert p.set("messages", "m1", True) and p.set("done", "d1", True)
    assert p.set("nowhere", "x", True) is None and p.set("messages", "", True) is None
    again = Pins(tmp_path / "pins.json")
    assert set(again.of("messages")) == {"m1"} and set(again.of("done")) == {"d1"}
    assert again.set("messages", "m1", False) is None and Pins(tmp_path / "pins.json").of("messages") == {}


# ------------------------------------------------------------------ sites' inbox pages
def test_times_as_inbox_pages_write_them():
    now = datetime(2026, 10, 19, 14, 30).timestamp()            # a Monday
    at = lambda s: datetime.fromtimestamp(web.parse_when(s, now))  # noqa: E731
    assert at("13:05").strftime("%d %H:%M") == "19 13:05" and at("1:05 PM").strftime("%H:%M") == "13:05"
    assert at("12:10 am").strftime("%H:%M") == "00:10" and at("23:50").day == 18    # later than now: yesterday
    assert at("2h").hour == 12 and at("5m").minute == 25 and web.parse_when("now", now) == now
    assert at("Yesterday").day == 18 and at("Friday").day == 16 and at("Mon").day == 12
    assert at("18/10/2026").day == 18 and at("10/18/26").month == 10 and at("Oct 3").day == 3
    assert at("3 Oct 2024").year == 2024 and at("Dec 30").year == 2025          # not in the future
    for s in ("", "soon", "25:61", "31/02/2026"):
        assert web.parse_when(s, now) is None


def test_an_inbox_page_s_conversations_become_messages():
    now = datetime(2026, 10, 19, 14, 30).timestamp()
    chats = [{"name": "Mum", "text": "Dinner at 7?", "time": "14:02", "unread": True},
             {"name": "  Sam  K ", "text": "Liked\nyour photo", "time": "Yesterday", "link": "https://x.com/m/1"},
             {"name": "", "text": "no name"}, {"name": "Empty", "text": ""}]
    items = web.snapshot_items("whatsapp", "a9", chats, now)
    assert [(i["who"], i["text"], i["unread"], i["label"]) for i in items] == [
        ("Mum", "Dinner at 7?", True, "WhatsApp"), ("Sam K", "Liked your photo", False, "WhatsApp")]
    assert items[1]["ref"] == "https://x.com/m/1" and items[0]["chat"] == "Mum"
    # The same latest message is the same item; a new one is a new item.
    assert web.snapshot_items("whatsapp", "a9", chats[:1], now)[0]["id"] == items[0]["id"]
    assert web.snapshot_items("whatsapp", "a9", [{**chats[0], "text": "Or 8?"}], now)[0]["id"] != items[0]["id"]


def test_wechat_is_read_from_wechat_for_web():
    """WeChat connects as WhatsApp does: WeChat for Web, its code scanned with the phone, kept open; its list read
    by sources.js (tried on a stand-in for its page, where node and Playwright are)."""
    site = web.SITES["wechat"]
    assert site["label"] == "WeChat" and site["inbox"] == "https://web.wechat.com/?lang=en_US" and site["live"] is True
    assert "Scan the code with WeChat" in site["how"] and "can't use WeChat for Web" in site["how"]
    from chatcomputer.messages import CHOICES
    assert ("WeChat", "wechat") in CHOICES
    import shutil
    import subprocess
    node = shutil.which("node")
    root = subprocess.run(["npm", "root", "-g"], capture_output=True, text=True).stdout.strip() if shutil.which("npm") else ""
    if not node or not root or not (Path(root) / "playwright").exists():
        pytest.skip("node with Playwright not available")
    r = subprocess.run([node, str(Path(__file__).parent / "sources_check.js")], capture_output=True, text=True, timeout=120,
                       env={**os.environ, "NODE_PATH": root})
    res = json.loads([x for x in r.stdout.splitlines() if x.startswith("{")][-1])
    assert "failed" not in res, res["failed"]
    assert [(c["name"], c["time"], c["text"], c["unread"]) for c in res["chats"]] == [
        ("Mum", "13:05", "Dinner at 7? Bring the photos", True), ("Alex Tan", "Yesterday", "[Photo]", False),
        ("Family group", "10/18/26", "Lin: see you Sunday", True)]
    assert res["login"] == {"chats": 0, "code": True}
    items = web.snapshot_items("wechat", "a3", res["chats"])
    assert [i["label"] for i in items] == ["WeChat"] * 3 and items[0]["who"] == "Mum"


# ------------------------------------------------------------------ email over IMAP
def _raw(subject, frm, text, html=None, attach=None, date="Mon, 19 Oct 2026 10:00:00 +0800"):
    m = email.message.EmailMessage()
    m["Subject"], m["From"], m["To"], m["Date"] = subject, frm, "sam@example.com", date
    m.set_content(text)
    if html:
        m.add_alternative(html, subtype="html")
    if attach:
        m.add_attachment(b"data", maintype="application", subtype="pdf", filename=attach)
    return m.as_bytes()


class FakeIMAP:
    """An IMAP server's answers, as imaplib gives them."""

    def __init__(self, mails, password="right", gmail=False, validity="7"):
        self.mails, self.password, self.validity = mails, password, validity
        self.capabilities = ("IMAP4REV1", "X-GM-EXT-1") if gmail else ("IMAP4REV1",)
        self.calls, self.auth, self.seen = [], None, set()

    def login(self, user, password):
        self.calls.append(("LOGIN", user))
        if password != self.password:
            raise imaplib.IMAP4.error("[AUTHENTICATIONFAILED] Invalid credentials (Failure)")

    def authenticate(self, mech, cb):
        self.auth = (mech, cb(None))

    def select(self, box, readonly=False):
        self.calls.append(("SELECT", box, readonly))
        return "OK", [str(len(self.mails)).encode()]

    def response(self, code):
        return code, [self.validity.encode()]

    def uid(self, cmd, *args):
        self.calls.append((cmd,) + args)
        if cmd == "SEARCH":
            if args[0] == "UID":
                lo = int(args[1].split(":")[0])
                return "OK", [" ".join(str(u) for u in sorted(self.mails) if u >= lo).encode()]
            return "OK", [" ".join(str(u) for u in sorted(self.mails)).encode()]
        if cmd == "FETCH":
            if args[1] == "(UID FLAGS)":
                lo, hi = (int(x) for x in args[0].split(":"))
                return "OK", [f"{i} (UID {u} FLAGS ({self._flags(u)}))".encode()
                              for i, u in enumerate(sorted(self.mails), 1) if lo <= u <= hi]
            out = []
            for u in (int(x) for x in args[0].split(",")):
                if u in self.mails:
                    head = (f'1 (UID {u} FLAGS ({self._flags(u)}) INTERNALDATE "19-Oct-2026 10:00:00 +0800" '
                            'BODY[] {9}')
                    out += [(head.encode(), self.mails[u]), b")"]
            return "OK", out
        raise AssertionError(cmd)

    def _flags(self, uid):
        return "\\Seen" if uid in self.seen else ""

    def logout(self):
        self.calls.append(("LOGOUT",))


def _account(**kw):
    a = {"id": "a1", "kind": "imap", "provider": "gmail", "label": "Gmail", "address": "sam@example.com",
         "user": "sam@example.com", "host": "imap.gmail.com", "port": 993, "password": "right"}
    a.update(kw)
    return a


def test_new_mail_is_read_without_marking_it_read():
    server = FakeIMAP({5: _raw("Lunch?", "Jane <jane@example.com>", "Lunch on Friday?"),
                       6: _raw("Hi", "bob@example.com", "", html="<style>p{}</style><p>Hello <b>there</b></p>",
                               attach="plan.pdf")}, gmail=True)
    box = Mailbox(_account(), imap_factory=lambda host, port: server)
    items = box.poll()
    assert [(i["who"], i["address"], i["title"], i["text"], i["unread"]) for i in items] == [
        ("Jane", "jane@example.com", "Lunch?", "Lunch on Friday?", True),
        ("bob@example.com", "bob@example.com", "Hi", "Hello there", True)]
    assert items[0]["id"] == "mail:a1:7:5" and items[0]["ref"] == "7:5" and items[0]["chat"] == "jane@example.com"
    # Only peeked at (the account still says unread), only the Primary tab on Gmail, read-only.
    search = next(c for c in server.calls if c[0] == "SEARCH")
    assert search[1] == "SINCE" and search[-2:] == ("X-GM-RAW", '"category:primary"')
    assert any(c[0] == "FETCH" and "BODY.PEEK[]<0.24000>" in c[2] for c in server.calls)
    assert ("SELECT", "INBOX", True) in server.calls and server.calls[-1] == ("LOGOUT",)
    # Next time: only what's new, and what was read somewhere else since.
    server.mails[7] = _raw("Later", "jane@example.com", "Again")
    server.seen.add(5)
    server.calls.clear()
    again = box.poll()
    assert [i.get("title") for i in again if not i.get("_seen")] == ["Later"]
    assert {"id": "mail:a1:7:5", "_seen": True} in again
    assert ("SEARCH", "UID", "7:*", "X-GM-RAW", '"category:primary"') in server.calls
    whole = box.read_many(["7:6", "7:99"])
    assert list(whole) == ["7:6"] and whole["7:6"]["attachments"] == ["plan.pdf"]


def test_a_wrong_password_is_said_plainly():
    box = Mailbox(_account(password="wrong"), imap_factory=lambda host, port: FakeIMAP({}))
    with pytest.raises(AuthError, match="Invalid credentials"):
        box.poll()


def test_outlook_signs_in_with_microsoft_s_tokens(monkeypatch):
    server = FakeIMAP({})
    saved = []
    monkeypatch.setattr(msauth, "refresh", lambda cid, rt: {"access_token": "new", "expires_at": time.time() + 3600,
                                                            "refresh_token": "rt2"})
    a = _account(kind="outlook", provider="outlook", oauth=True, refresh_token="rt1", access_token="old",
                 expires_at=time.time() - 10, client_id="cid", password=None)
    Mailbox(a, save_tokens=saved.append, imap_factory=lambda host, port: server).poll()
    assert server.auth == ("XOAUTH2", b"user=sam@example.com\x01auth=Bearer new\x01\x01")
    assert saved and saved[0]["refresh_token"] == "rt2" and saved[0]["access_token"] == "new"


def test_html_mail_becomes_words():
    assert html_to_text("<html><head><title>x</title><style>.a{}</style></head><body><p>One</p><div>Two&amp;"
                        "three</div><script>bad()</script></body></html>") == "One\n\nTwo&three"
    m = email.message_from_bytes(_raw("s", "a@b.c", "plain words", html="<p>html words</p>"))
    assert body_text(m) == "plain words"
    assert parse_fetch([(b'1 (UID 9 FLAGS (\\Seen \\Answered) INTERNALDATE "x" BODY[] {1}', b"r"), b")"]) == \
        [(9, {"\\Seen", "\\Answered"}, "x", b"r")]


# ------------------------------------------------------------------ Microsoft's sign-in
class Reply:
    def __init__(self, body):
        self.body = body

    def read(self):
        return json.dumps(self.body).encode()

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False


def test_microsoft_sign_in_with_pkce():
    import base64
    import hashlib
    import urllib.parse
    flow = msauth.start("cid")
    q = urllib.parse.parse_qs(urllib.parse.urlparse(flow["url"]).query)
    assert q["client_id"] == ["cid"] and q["code_challenge_method"] == ["S256"] and q["redirect_uri"] == [msauth.REDIRECT]
    want = base64.urlsafe_b64encode(hashlib.sha256(flow["verifier"].encode()).digest()).rstrip(b"=").decode()
    assert q["code_challenge"] == [want] and "IMAP.AccessAsUser.All" in q["scope"][0]
    back = f"{msauth.REDIRECT}?code=abc&state={flow['state']}"
    assert msauth.code_from(back, flow["state"]) == "abc"
    with pytest.raises(msauth.SignInError, match="didn't match"):
        msauth.code_from(f"{msauth.REDIRECT}?code=abc&state=other", flow["state"])
    with pytest.raises(msauth.SignInError, match="declined"):
        msauth.code_from(f"{msauth.REDIRECT}?error=access_denied&error_description=declined", flow["state"])
    sent = []

    def opener(req, timeout=30):
        sent.append(urllib.parse.parse_qs(req.data.decode()))
        claims = base64.urlsafe_b64encode(json.dumps({"email": "sam@example.com"}).encode()).decode().rstrip("=")
        return Reply({"access_token": "at", "refresh_token": "rt", "expires_in": 3600, "id_token": f"h.{claims}.s"})
    tokens = msauth.finish("cid", "abc", flow["verifier"], opener=opener)
    assert sent[0]["grant_type"] == ["authorization_code"] and sent[0]["code_verifier"] == [flow["verifier"]]
    assert msauth.address_of(tokens) == "sam@example.com" and tokens["expires_at"] > time.time()
    with pytest.raises(msauth.SignInError, match="expired"):
        msauth.refresh("cid", "rt", opener=lambda req, timeout=30: Reply({"error": "invalid_grant",
                                                                         "error_description": "expired"}))


# ------------------------------------------------------------------ Telegram
class FakeTelegram:
    def __init__(self, session, api_id, api_hash, password_needed=False):
        self.args = (session, api_id, api_hash)
        self.password_needed = password_needed
        self.signed = []
        self.session = SimpleNamespace(save=lambda: "SESSION")
        self.handlers = []
        self.dialogs = []

    async def connect(self):
        pass

    async def disconnect(self):
        pass

    async def is_user_authorized(self):
        return True

    async def send_code_request(self, phone):
        return SimpleNamespace(phone_code_hash="h1")

    async def sign_in(self, **kw):
        self.signed.append(kw)
        if "code" in kw and self.password_needed:
            from telethon.errors import SessionPasswordNeededError
            raise SessionPasswordNeededError(request=None)
        if kw.get("code") == "000":
            raise ValueError("PHONE_CODE_INVALID")

    def add_event_handler(self, fn, ev):
        self.handlers.append(fn)

    async def iter_dialogs(self, limit=60):
        for d in self.dialogs:
            yield d


def test_telegram_signs_in_step_by_step():
    pytest.importorskip("telethon")
    clients = []

    def factory(session, api_id, api_hash):
        clients.append(FakeTelegram(session, api_id, api_hash, password_needed=True))
        return clients[-1]
    s = TelegramSignIn(factory)
    with pytest.raises(TgAuthError, match="number"):
        asyncio.run(s.send_code("abc", "hash", "+6591234567"))

    async def run():
        await s.send_code("12345", " hash ", "+65 9123 4567")
        assert clients[-1].args == ("", 12345, "hash") and s.code_hash == "h1" and s.phone == "+6591234567"
        assert await s.code("11111") is False                    # two-step verification: the password next
        await s.password("pw")
        return s.account()
    acct = asyncio.run(run())
    assert acct["session"] == "SESSION" and acct["address"] == "+6591234567" and acct["api_id"] == 12345
    assert clients[-1].signed[-1] == {"password": "pw"}


def test_telegram_brings_in_chats_not_channels_or_your_own():
    pytest.importorskip("telethon")
    now = datetime(2026, 10, 19, 14, 0)

    def msg(i, text, out=False):
        return SimpleNamespace(id=i, message=text, date=now, chat_id=100 + i, out=out, photo=None,
                               get_sender=lambda: asyncio.sleep(0, SimpleNamespace(first_name="Alex", last_name=None)))
    dialogs = [SimpleNamespace(is_channel=False, is_group=False, message=msg(1, "Sent the files"), unread_count=2,
                               entity=SimpleNamespace(first_name="Alex", last_name=None, id=101)),
               SimpleNamespace(is_channel=True, is_group=False, message=msg(2, "News"), unread_count=9, entity=None),
               SimpleNamespace(is_channel=False, is_group=False, message=msg(3, "mine", out=True), unread_count=0,
                               entity=None)]
    got = []
    client = FakeTelegram("S", 1, "h")
    client.dialogs = dialogs
    src = TelegramSource({"id": "t1", "session": "S", "api_id": 1, "api_hash": "h"}, got.extend, lambda *a: client)
    asyncio.run(src.start())
    assert [(i["who"], i["text"], i["unread"], i["id"]) for i in got] == [("Alex", "Sent the files", True, "tg:t1:101:1")]
    assert client.handlers


# ------------------------------------------------------------------ accounts and the Inbox
def test_sign_ins_stay_in_the_secrets_folder(tmp_path):
    paths = Paths(tmp_path / "ChatComputer").ensure()
    acc = Accounts(paths.secrets / "accounts.json")
    a = acc.add({"kind": "imap", "provider": "gmail", "label": "Gmail", "address": "Sam@example.com", "password": "pw"})
    b = acc.add({"kind": "imap", "provider": "gmail", "label": "Gmail", "address": "sam@example.com", "password": "pw2"})
    assert a["id"] == b["id"] and len(acc.all()) == 1 and acc.get(a["id"])["password"] == "pw2"
    assert stat.S_IMODE(os.stat(paths.secrets / "accounts.json").st_mode) == 0o600
    shown = public({**b, "refresh_token": "x", "session": "s", "api_hash": "h", "_last_uid": 4})
    assert "password" not in shown and not any(k in shown for k in ("refresh_token", "session", "api_hash", "_last_uid"))


def _inbox(tmp_path, server=None):
    paths = Paths(tmp_path / "ChatComputer").ensure()
    screen = RecordingScreen()
    notices = []
    inbox = Inbox(paths, Settings(), screen, Pins(paths.root / "pins.json"),
                  lambda text, kind="info": notices.append(text),
                  imap_factory=(lambda host, port: server) if server else None)
    return inbox, screen, notices


def test_connecting_email_with_its_own_form(tmp_path):
    server = FakeIMAP({5: _raw("Hello", "Jane <jane@example.com>", "Hi Sam")})
    inbox, screen, notices = _inbox(tmp_path, server)

    async def run():
        inbox.loop = asyncio.get_running_loop()
        await inbox.connect(None)
        await inbox.connect("gmail")
        fid = screen.events[-1][1]["id"]
        await inbox.form_submit(fid, {"address": "sam@example.com", "password": "nope"})
        await inbox.form_submit(fid, {"address": "sam@example.com", "password": "right"})
        await asyncio.sleep(0.2)
        await inbox.stop()
    asyncio.run(run())
    forms = [e[1] for e in screen.events if e[0] == "form"]
    assert forms[0]["title"] == "Connect an account" and "Gmail" in [c["label"] for c in forms[0]["choices"]]
    assert forms[1]["title"] == "Connect Gmail" and [f["name"] for f in forms[1]["fields"]] == ["address", "password"]
    # A wrong password: the same form again, with what went wrong, the address still there and the password not.
    retry = forms[2]
    assert retry["title"] == "Connect Gmail" and "didn't take that" in retry["error"]
    assert [(f["name"], f.get("value", "")) for f in retry["fields"]] == [("address", "sam@example.com"), ("password", "")]
    assert forms[3] is None and "is connected" in notices[-1]
    # Kept with its password in the secrets folder; the screen gets it without.
    saved = json.loads((inbox.paths.secrets / "accounts.json").read_text())["accounts"]
    assert saved[0]["password"] == "right" and saved[0]["address"] == "sam@example.com"
    shown = [e[1] for e in screen.events if e[0] == "messages_list"][-1]
    assert shown[1] == ["Gmail"] and "right" not in json.dumps(screen.events, default=str)
    assert [m["who"] for m in inbox.store.recent()] == ["Jane"]


def test_a_site_connects_from_its_own_page(tmp_path):
    inbox, screen, notices = _inbox(tmp_path)

    async def run():
        inbox.loop = asyncio.get_running_loop()
        await inbox.connect(None)                              # the list of accounts is up...
        said = await inbox.connect("whatsapp")                # ...and WhatsApp is chosen
        inbox.web_snapshot("whatsapp", {"type": "snapshot", "state": "login"})
        inbox.web_snapshot("whatsapp", {"type": "snapshot", "state": "ready", "chats": [
            {"name": "Mum", "text": "Dinner at 7?", "time": "14:02", "unread": True}]})
        await asyncio.sleep(2.2)
        return said
    said = asyncio.run(run())
    assert "own page" in said and ("form", None) in screen.events          # the list goes, the page shows
    assert ("source_show", ("whatsapp", web.SITES["whatsapp"]["inbox"], None)) in screen.events
    assert any("restrict" in n for n in notices) and any("WhatsApp is connected" in n for n in notices)
    assert ("source_close", "whatsapp") in screen.events
    assert [m["who"] for m in inbox.store.recent()] == ["Mum"]
    acct = inbox.accounts.find("web", "whatsapp")
    assert acct and inbox.status[acct["id"]] == "ok"
    gone = asyncio.run(inbox.disconnect(acct["id"]))
    assert "no longer connected" in gone and inbox.store.recent() == [] and ("source_forget", "whatsapp") in screen.events


def test_pins_from_the_page_are_kept(tmp_path, monkeypatch):
    from chatcomputer.controller import Controller
    from chatcomputer.platform.fake import FakePlatform
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "ChatComputer"))
    screen = RecordingScreen()
    c = Controller(Paths().ensure(), Settings(), FakePlatform(), screen, voice=False, index=False)
    c.page_event({"type": "pin", "list": "messages", "id": "m1", "on": True})
    c.page_event({"type": "pin", "list": "done", "id": "d1", "on": True})
    c.page_event({"type": "pin", "list": "done", "id": "d1", "on": False})
    assert [e[1] for e in screen.events if e[0] == "pinned"] == [("messages", "m1", True), ("done", "d1", True),
                                                                ("done", "d1", False)]
    saved = json.loads((c.paths.root / "pins.json").read_text())
    assert list(saved["messages"]) == ["m1"] and saved["done"] == {}


def test_messages_buttons_offer_their_own_kind(tmp_path):
    # Messages' three buttons: Connect email, chat apps, social media, each with only its own to choose from.
    inbox, screen, _ = _inbox(tmp_path)

    async def run():
        inbox.loop = asyncio.get_running_loop()
        for group in ("email", "chat", "social"):
            await inbox.connect(None, group)
        await inbox.stop()
    asyncio.run(run())
    forms = [e[1] for e in screen.events if e[0] == "form"]
    shown = [(f["title"], [c["label"] for c in f["choices"]]) for f in forms]
    assert shown == [
        ("Connect email", ["Gmail", "Outlook or Hotmail", "iCloud Mail", "Yahoo Mail", "Other email"]),
        ("Connect chat apps", ["Telegram", "WhatsApp", "WeChat"]),
        ("Connect social media", ["X", "Instagram", "Facebook", "TikTok", "LinkedIn"]),
    ]
