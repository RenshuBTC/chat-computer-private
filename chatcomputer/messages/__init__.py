"""Messages: everything sent to the person, from every account, in one list at the left of the conversation,
newest first. Email over IMAP (mail.py), Telegram through its API (telegram.py), WhatsApp and social media read
from their own pages (web.py). Kept in ~/ChatComputer/messages.db; the accounts' sign-ins in secrets/accounts.json.

Signing in happens on the screen, typed by the person: a form on the conversation page for email and Telegram,
the site's own page for WhatsApp and the social sites, Microsoft's page for Outlook. The model never sees a password:
it asks for an account to be connected (the messages tool), and the form or the page does the rest.
"""
from __future__ import annotations

import asyncio
import logging
import time
from typing import Any, Callable

from ..config import Paths, Settings
from ..ui import render
from . import msauth, web
from .accounts import MAIL, Accounts, public
from .mail import AuthError, Mailbox
from .store import Store
from .telegram import AuthError as TelegramAuthError, TelegramSignIn, TelegramSource

log = logging.getLogger("chatcomputer.messages")
SHOWN = 300                       # messages on the screen at the start (with the pinned ones)
THREAD_MAX = 30                   # an email opened: the latest this many from the same person, as a conversation
THREAD_FULL = 5                   # ...of which this many are read in full (the one opened, and the latest)
RETRY_MAX_S = 30 * 60
SIGNED_OUT_WAIT_S = 30 * 60       # an account that stopped taking its sign-in is tried again this often

CHOICES = [("Gmail", "gmail"), ("Outlook or Hotmail", "outlook"), ("iCloud Mail", "icloud"), ("Yahoo Mail", "yahoo"),
           ("Other email", "imap"), ("Telegram", "telegram"), ("WhatsApp", "whatsapp"), ("WeChat", "wechat"), ("X", "x"),
           ("Instagram", "instagram"), ("Facebook", "facebook"), ("TikTok", "tiktok"), ("LinkedIn", "linkedin")]
# Messages' three buttons: what each offers.
GROUPS = {"email": ("Connect email", {"gmail", "outlook", "icloud", "yahoo", "imap"}),
          "chat": ("Connect chat apps", {"telegram", "whatsapp", "wechat"}),
          "social": ("Connect social media", {"x", "instagram", "facebook", "tiktok", "linkedin"})}
MAIL_HELP = {
    "gmail": ["Gmail needs an app password, not your usual one. Turn on 2-Step Verification for your Google "
              "account, then make an app password at myaccount.google.com/apppasswords and type it here.",
              "Chat Computer only reads your inbox's Primary tab. It doesn't send, delete or mark anything."],
    "icloud": ["iCloud Mail needs an app-specific password: make one at account.apple.com, under Sign-In and "
               "Security, App-Specific Passwords, and type it here.",
               "Chat Computer only reads your inbox. It doesn't send, delete or mark anything."],
    "yahoo": ["Yahoo Mail needs an app password: make one in your Yahoo account's security settings (Generate app "
              "password) and type it here.", "Chat Computer only reads your inbox. It doesn't send, delete or mark anything."],
    "imap": ["Your provider's IMAP server (its website says, often imap. and its domain), your address and your "
             "password, or an app password if your provider has them.",
             "Chat Computer only reads your inbox. It doesn't send, delete or mark anything."],
}
OUTLOOK_SETUP = [
    "Outlook.com and Hotmail don't take passwords from other apps any more: Chat Computer signs in with Microsoft's own "
    "page, which needs a Microsoft sign-in app registered for Chat Computer once (free).",
    "At entra.microsoft.com: App registrations, New registration, \"Personal Microsoft accounts only\". Add the "
    "platform \"Mobile and desktop applications\" with the redirect URI " + msauth.REDIRECT + ", allow public "
    "client flows, and add the permission IMAP.AccessAsUser.All (Office 365 Exchange Online). Then type its "
    "Application (client) ID here.",
]
TELEGRAM_HELP = [
    "Chat Computer reads Telegram through Telegram's own API, as an unofficial Telegram client that only reads your "
    "chats and groups (not channels). It never sends anything.",
    "Telegram asks each such app for an api_id and api_hash of its own: sign in at my.telegram.org, open API "
    "development tools, create an app (any name), and type the two here with your phone number. Telegram then "
    "sends a code to your Telegram app.",
]


class Inbox:
    def __init__(self, paths: Paths, settings: Settings, screen: Any, pins: Any,
                 notice: Callable[[str, str], None], imap_factory: Callable[..., Any] | None = None,
                 telegram_factory: Callable[..., Any] | None = None) -> None:
        self.paths = paths
        self.settings = settings
        self.screen = screen
        self.pins = pins
        self.notice = notice
        self.imap_factory = imap_factory
        self.telegram_factory = telegram_factory
        self.store = Store(paths.root / "messages.db")
        self.accounts = Accounts(paths.secrets / "accounts.json")
        self.loop: asyncio.AbstractEventLoop | None = None
        self.tasks: dict[str, asyncio.Task] = {}
        self.wake: dict[str, asyncio.Event] = {}
        self.mailboxes: dict[str, Mailbox] = {}
        self.telegram: dict[str, TelegramSource] = {}
        self.status: dict[str, str] = {}
        self.forms: dict[str, dict[str, Any]] = {}
        self.connecting: set[str] = set()          # sites being signed in to, on their own page
        self._form_n = 0
        self._told: set[str] = set()
        self._web_reads: dict[str, float] = {}
        self._web_task: asyncio.Task | None = None

    # ================================================================ start, stop, the screen
    async def start(self) -> None:
        self.loop = asyncio.get_running_loop()
        await self.loop.run_in_executor(None, self.store.prune)
        self.show_all()
        for a in self.accounts.all():
            self._start(a)
        self._web_task = self.loop.create_task(self._web_rounds())

    async def stop(self) -> None:
        for t in list(self.tasks.values()) + ([self._web_task] if self._web_task else []):
            t.cancel()
        for src in list(self.telegram.values()):
            await src.stop()
        for f in list(self.forms.values()):
            if f.get("signin"):
                await f["signin"].close()

    def _with_pin(self, m: dict[str, Any]) -> dict[str, Any]:
        pinned = self.pins.of("messages").get(m["id"])
        out = dict(m)
        if pinned:
            out["pinned"] = pinned
        return out

    def show_all(self) -> None:
        pins = self.pins.of("messages")
        items = [self._with_pin(m) for m in self.store.recent(SHOWN, pins.keys())]
        self.screen.messages_list(items, [self._public(a) for a in self.accounts.all()])

    def _public(self, a: dict[str, Any]) -> dict[str, Any]:
        p = public(a)
        p["status"] = self.status.get(a["id"], "")
        return p

    def add(self, items: list[dict[str, Any]]) -> None:
        """New or changed messages from an account: kept, and onto the screen."""
        real = []
        for it in items:
            if it.get("_seen"):
                old = self.store.get(it["id"])
                if old and old["unread"]:
                    self.store.mark_read(it["id"])
                    old["unread"] = False
                    self.screen.message(self._with_pin(old))
            else:
                real.append(it)
        for m in self.store.upsert(real):
            self.screen.message(self._with_pin(m))

    def _set_status(self, account: dict[str, Any], status: str) -> None:
        self.status[account["id"]] = status

    def _tell_once(self, key: str, text: str, kind: str = "warn") -> None:
        if key not in self._told:
            self._told.add(key)
            self.notice(text, kind)

    # ================================================================ the accounts, at work
    def _start(self, a: dict[str, Any]) -> None:
        if a["kind"] in ("imap", "outlook"):
            self.tasks[a["id"]] = self.loop.create_task(self._mail_loop(a))
        elif a["kind"] == "telegram":
            self.tasks[a["id"]] = self.loop.create_task(self._telegram_run(a))
        elif a["kind"] == "web" and web.SITES.get(a.get("provider", ""), {}).get("live"):
            self.screen.source_open(a["provider"], web.SITES[a["provider"]]["inbox"])

    async def _mail_loop(self, a: dict[str, Any]) -> None:
        box = Mailbox(a, save_tokens=lambda t, i=a["id"]: self.accounts.update(i, **t),
                      imap_factory=self.imap_factory, client_id=self.settings.outlook_client_id)
        self.mailboxes[a["id"]] = box
        wake = self.wake.setdefault(a["id"], asyncio.Event())
        failures = 0
        while True:
            try:
                items = await asyncio.to_thread(box.poll)
                self.add(items)
                self.accounts.update(a["id"], _uidvalidity=box.uidvalidity, _last_uid=box.last_uid)
                self._set_status(a, "ok")
                failures = 0
                wait = max(30.0, float(self.settings.mail_check_minutes or 2) * 60)
            except AuthError as e:
                self._set_status(a, "signed out")
                self._tell_once(f"auth:{a['id']}", f"{a.get('label')} ({a.get('address')}) didn't take its sign-in "
                                f"({str(e)[:120]}). Connect it again with + at the top of Messages.")
                wait = SIGNED_OUT_WAIT_S
            except asyncio.CancelledError:
                raise
            except Exception as e:
                failures += 1
                self._set_status(a, "can't reach it")
                log.info("%s: couldn't check mail: %s", a.get("label"), e)
                wait = min(RETRY_MAX_S, 60.0 * 2 ** min(failures, 5))
            wake.clear()
            try:
                await asyncio.wait_for(wake.wait(), wait)
            except asyncio.TimeoutError:
                pass

    async def _telegram_run(self, a: dict[str, Any]) -> None:
        src = TelegramSource(a, self.add, client_factory=self.telegram_factory)
        failures = 0
        while True:
            try:
                await src.start()
                self.telegram[a["id"]] = src
                self._set_status(a, "ok")
                return                      # Telegram's own connection keeps the messages coming
            except TelegramAuthError as e:
                self._set_status(a, "signed out")
                self._tell_once(f"auth:{a['id']}", f"Telegram needs signing in again ({e}). Use + at the top of "
                                "Messages.")
                return
            except asyncio.CancelledError:
                raise
            except Exception as e:
                failures += 1
                self._set_status(a, "can't reach it")
                log.info("Telegram: couldn't connect: %s", e)
                await src.stop()
                await asyncio.sleep(min(RETRY_MAX_S, 30.0 * 2 ** min(failures, 6)))

    async def _web_rounds(self) -> None:
        """The social sites (not WhatsApp, which stays open): each looked at every few minutes, one at a time."""
        await asyncio.sleep(20)
        while True:
            sites = [a for a in self.accounts.all() if a["kind"] == "web"
                     and not web.SITES.get(a.get("provider", ""), {}).get("live")]
            due = sorted(sites, key=lambda a: self._web_reads.get(a["provider"], 0.0))
            if due and time.time() - self._web_reads.get(due[0]["provider"], 0.0) >= web.READ_EVERY_S:
                p = due[0]["provider"]
                self._web_reads[p] = time.time()
                self.screen.source_open(p, web.SITES[p]["inbox"], reload=True)
                await asyncio.sleep(90)              # time to load and be read; then let it go
                if p not in self.connecting:
                    self.screen.source_unload(p)
            else:
                await asyncio.sleep(30)

    def web_snapshot(self, provider: str, msg: dict[str, Any]) -> None:
        """What a site's inbox page shows (sources.js): its conversations, or that it wants signing in."""
        if msg.get("type") != "snapshot" or provider not in web.SITES:
            return
        state = msg.get("state")
        site = web.SITES[provider]
        a = self.accounts.find("web", provider)
        if provider in self.connecting and state == "ready":
            self.connecting.discard(provider)
            a = self.accounts.add({"kind": "web", "provider": provider, "label": site["label"]})
            self.notice(f"{site['label']} is connected: its messages show in Messages.", "info")
            self._web_reads[provider] = time.time()
            self.loop.call_later(2.0, self.screen.source_close, provider)
            self.show_all()
        if a is None:
            return
        if state == "ready":
            chats = msg.get("chats") or []
            self._set_status(a, "ok")
            self._told.discard(f"web-login:{provider}")
            log.info("%s: %d conversations read (%s list items seen)", provider, len(chats), msg.get("found"))
            self.add(web.snapshot_items(provider, a["id"], chats))
        elif state == "login" and provider not in self.connecting:
            self._set_status(a, "signed out")
            self._tell_once(f"web-login:{provider}", f"{site['label']} signed Chat Computer out. Sign in again with + at the "
                            "top of Messages.")

    # ================================================================ opening one
    async def open(self, item_id: str) -> None:
        """A message tapped in Messages (or opened by the model): shown between the sidebars."""
        m = self.store.get(item_id)
        if m is None:
            return
        if m["unread"]:
            self.store.mark_read(item_id)
            m["unread"] = False
            self.screen.message(self._with_pin(m))
        try:
            if m["source"] == "email":
                await self.screen.show_html(await self.thread_html(m), m["who"] or m["address"] or "Email")
            elif m["source"] == "telegram":
                title, lines = await self.read_chat(m)
                await self.screen.show_html(render.chat_html(title, lines, self.settings), title)
            elif m["source"] in web.SITES:
                ref = m.get("ref") or ""
                self.screen.source_show(m["source"], url=ref if ref.startswith("https://") else None,
                                        chat=m["chat"] if m["source"] == "whatsapp" else None)
        except Exception as e:
            log.info("couldn't open a message: %s", e)
            self.notice(f"I couldn't open that message: {str(e)[:200]}", "warn")

    async def thread_html(self, m: dict[str, Any]) -> str:
        """An email opened: the conversation with whoever sent it (the emails from them that Messages has, oldest
        first), the one opened and the latest few in full, read from the mailbox now."""
        box = self.mailboxes.get(m["account"])
        if box is None:
            raise RuntimeError("that email account isn't connected any more")
        thread = self.store.thread(m["account"], m["chat"] or m["address"] or "", THREAD_MAX)
        if not any(t["id"] == m["id"] for t in thread):
            thread.append(m)
        whole = [m["ref"]] + [t["ref"] for t in reversed(thread) if t["ref"] != m["ref"]][:THREAD_FULL - 1]
        got = await asyncio.to_thread(box.read_many, whole)
        if m["ref"] not in got:
            raise RuntimeError("that message isn't in the inbox any more")
        mails = []
        for t in thread:
            full = got.get(t["ref"])
            mails.append({"subject": (full or {}).get("subject") or t.get("title") or "",
                          "when": time.strftime("%a %d %b %Y, %H:%M", time.localtime(t["ts"])),
                          "text": full["text"] if full else (t.get("text") or ""),
                          "attachments": (full or {}).get("attachments") or [], "open": t["id"] == m["id"]})
        first = got[m["ref"]]
        meta = [("From", f"{first['who']} <{first['address']}>" if first["address"] else first["who"]),
                ("To", first["to"])]
        title = m["who"] or m["address"] or "Email"
        return render.thread_html(title, meta, mails, self.settings)

    async def read_mail(self, m: dict[str, Any]) -> dict[str, Any]:
        box = self.mailboxes.get(m["account"])
        if box is None:
            raise RuntimeError("that email account isn't connected any more")
        return await asyncio.to_thread(box.read, m["ref"])

    async def read_chat(self, m: dict[str, Any]) -> tuple[str, list[tuple[str, str, str]]]:
        src = self.telegram.get(m["account"])
        if src is None:
            raise RuntimeError("Telegram isn't connected right now")
        return await src.chat(m["ref"])

    async def read(self, item_id: str) -> str:
        """A message in full, for the model: an email's words, a Telegram chat's latest messages, or what a site's
        inbox showed."""
        m = self.store.get(item_id)
        if m is None:
            return "There's no message with that id."
        if m["source"] == "email":
            mail = await self.read_mail(m)
            att = f"\nAttachments: {', '.join(mail['attachments'])}" if mail["attachments"] else ""
            return (f"From: {mail['who']} <{mail['address']}>\nTo: {mail['to']}\nDate: {mail['date']}\n"
                    f"Subject: {mail['subject']}{att}\n\n{mail['text'][:20000]}")
        if m["source"] == "telegram":
            title, lines = await self.read_chat(m)
            return f"Telegram chat: {title}\n" + "\n".join(f"{when} {who}: {text}" for who, when, text in lines)
        return (f"{m['label']}, {m['who']}, {time.strftime('%a %d %b %H:%M', time.localtime(m['ts']))}: {m['text']}\n"
                f"(Only the latest message of a conversation is known: it's read from the inbox page.)")

    def check_now(self) -> None:
        for e in self.wake.values():
            e.set()

    # ================================================================ connecting an account
    def _form(self, provider: str, **form: Any) -> None:
        """Shows a sign-in form (kept as shown, to show again with what went wrong). One for an account (not the list
        of them) has a back arrow at its top, to the list (chat.js: f.back; "accounts" asks for the list again)."""
        self._drop_forms()
        self._form_n += 1
        fid = f"{provider}-{self._form_n}"
        if provider != "choose":
            form.setdefault("back", "accounts")
        self.forms[fid] = {"provider": provider, "_shown": form}
        self.screen.account_form({"id": fid, **form})

    def _drop_forms(self, keep: Any = None) -> None:
        """One form at a time on the screen: those shown before a new one are let go (with a Telegram sign-in
        still under way in them, unless the new form carries it on)."""
        for k in [k for k in self.forms if k != "outlook-signin"]:
            si = self.forms.pop(k).get("signin")
            if si is not None and si is not keep and self.loop is not None:
                self.loop.create_task(si.close())

    async def connect(self, provider: str | None, group: str | None = None) -> str:
        """Starts connecting an account: the right form or page on the screen (with only `group`'s to choose from:
        email, chat or social, Messages' buttons). Returns what to tell the person."""
        provider = (provider or "").lower().strip()
        aliases = {"hotmail": "outlook", "live": "outlook", "outlook.com": "outlook", "google": "gmail",
                   "twitter": "x", "messenger": "facebook", "email": "imap", "mail": "imap", "apple": "icloud"}
        provider = aliases.get(provider, provider)
        if provider == "accounts":                  # (back, from one account's form: the list of them)
            provider = ""
        if not provider and group in GROUPS:
            title, values = GROUPS[group]
            self._form("choose", title=title, text=["Which one?"], ok="",
                       choices=[{"label": l, "value": v} for l, v in CHOICES if v in values])
            return "The list of accounts to connect is on the screen."
        if not provider:
            self._form("choose", title="Connect an account", text=["Which one?"], ok="",
                       choices=[{"label": l, "value": v} for l, v in CHOICES])
            return "The list of accounts to connect is on the screen."
        if provider in MAIL and provider != "outlook":
            info = MAIL[provider]
            fields = [{"name": "address", "label": "Email address", "type": "email"},
                      {"name": "password", "label": "App password" if provider != "imap" else "Password",
                       "type": "password"}]
            if provider == "imap":
                fields[1:1] = [{"name": "host", "label": "IMAP server", "type": "text"},
                               {"name": "port", "label": "Port", "type": "text", "value": "993"}]
            self._form(provider, title=f"Connect {info['label'] if provider != 'imap' else 'your email'}",
                       text=MAIL_HELP[provider], fields=fields, ok="Connect")
            return "The sign-in form is on the screen: they type the address and password there themselves."
        if provider == "outlook":
            if not self.settings.outlook_client_id:
                self._form("outlook-app", title="Connect Outlook", text=OUTLOOK_SETUP,
                           fields=[{"name": "client_id", "label": "Application (client) ID", "type": "text"}],
                           ok="Continue")
                return ("Outlook needs a Microsoft sign-in app registered once; the steps are on the screen. Then "
                        "Microsoft's own sign-in page opens.")
            return await self._outlook_start()
        if provider == "telegram":
            self._form("telegram", title="Connect Telegram", text=TELEGRAM_HELP, ok="Send the code",
                       fields=[{"name": "api_id", "label": "api_id", "type": "text"},
                               {"name": "api_hash", "label": "api_hash", "type": "password"},
                               {"name": "phone", "label": "Phone number, with its country code", "type": "tel",
                                "value": "+"}])
            return "Telegram's sign-in form is on the screen: they fill it in themselves."
        if provider in web.SITES:
            site = web.SITES[provider]
            self._drop_forms()
            self.screen.account_form(None)      # the list of accounts, if it's up: the site's page instead
            self.connecting.add(provider)
            self.screen.source_show(provider, url=site["inbox"])
            self.notice(f"{site['how']} Chat Computer will only read your conversations there, never send anything. "
                        f"{site['label']} doesn't allow automated reading, so it could restrict your account.", "info")
            return (f"{site['label']}'s own page is on the screen for them to sign in; it connects by itself once "
                    "they're in.")
        return f"I can't connect “{provider}”. I can connect: " + ", ".join(l for l, _ in CHOICES) + "."

    async def _outlook_start(self) -> str:
        self._drop_forms()
        self.screen.account_form(None)          # the list of accounts, if it's up: Microsoft's page instead
        flow = msauth.start(self.settings.outlook_client_id)
        self.forms["outlook-signin"] = {"provider": "outlook", "_flow": flow}
        self.screen.watch_redirect(msauth.REDIRECT, lambda url: self.loop.create_task(self._outlook_done(url)))
        await self.screen.show_url(flow["url"])
        self.notice("Sign in to your Outlook or Hotmail account on Microsoft's page.", "info")
        return "Microsoft's sign-in page is on the screen; they sign in there themselves."

    async def _outlook_done(self, url: str) -> None:
        flow = (self.forms.pop("outlook-signin", None) or {}).get("_flow")
        await self.screen.hide_content()
        if not flow:
            return
        cid = self.settings.outlook_client_id
        try:
            code = msauth.code_from(url, flow["state"])
            tokens = await asyncio.to_thread(msauth.finish, cid, code, flow["verifier"])
            address = msauth.address_of(tokens)
            if not address:
                raise msauth.SignInError("Microsoft didn't say which mailbox this is")
            account = {"kind": "outlook", "provider": "outlook", "label": "Outlook", "address": address,
                       "user": address, "host": MAIL["outlook"]["host"], "port": 993, "oauth": True,
                       "client_id": cid, "refresh_token": tokens.get("refresh_token", ""),
                       "access_token": tokens["access_token"], "expires_at": tokens["expires_at"]}
            await asyncio.to_thread(self._try_mail, account)
        except (msauth.SignInError, AuthError) as e:
            self.notice(f"Outlook didn't connect: {str(e)[:200]}", "warn")
            return
        except Exception as e:
            self.notice(f"Outlook didn't connect: {str(e)[:200]}", "warn")
            return
        self._connected(account)

    def _try_mail(self, account: dict[str, Any]) -> None:
        """Signs in and opens the inbox once, to be sure the details work (in a worker thread)."""
        box = Mailbox(account, imap_factory=self.imap_factory, client_id=self.settings.outlook_client_id)
        m = box.connect()
        try:
            box._select(m)
        finally:
            try:
                m.logout()
            except Exception:
                pass

    def _connected(self, account: dict[str, Any]) -> None:
        a = self.accounts.add(account)
        old = self.tasks.pop(a["id"], None)
        if old:
            old.cancel()
        self._told.discard(f"auth:{a['id']}")
        self._start(a)
        self.show_all()
        what = f"{a.get('label')} ({a['address']})" if a.get("address") else a.get("label")
        self.notice(f"{what} is connected: its messages show in Messages.", "info")

    async def form_submit(self, form_id: str, values: dict[str, Any]) -> None:
        """A sign-in form filled in. (`values` hold passwords: they go to accounts.json, never anywhere else.)"""
        f = self.forms.get(form_id)
        if f is None:
            return
        p = f["provider"]
        v = {k: str(val) for k, val in (values or {}).items()}
        try:
            if p in MAIL and p != "outlook":
                info = MAIL[p]
                address = v.get("address", "").strip()
                if "@" not in address or not v.get("password"):
                    raise ValueError("Type the email address and the password.")
                host = v.get("host", "").strip() or info["host"]
                port = int(v.get("port") or info["port"])
                if not host:
                    raise ValueError("Type the IMAP server.")
                label = info["label"] if p != "imap" else address.split("@")[-1]
                account = {"kind": "imap", "provider": p, "label": label, "address": address, "user": address,
                           "host": host, "port": port, "password": v["password"].replace(" ", "") if p == "gmail"
                           else v["password"]}
                try:
                    await asyncio.to_thread(self._try_mail, account)
                except AuthError as e:
                    raise ValueError(f"{label} didn't take that: {str(e)[:160]}") from None
                except OSError as e:
                    raise ValueError(f"Couldn't reach {host}: {e}") from None
                self.forms.pop(form_id, None)
                self.screen.account_form(None)
                self._connected(account)
            elif p == "outlook-app":
                cid = v.get("client_id", "").strip()
                if len(cid) < 30:
                    raise ValueError("That doesn't look like an Application (client) ID (36 letters and numbers).")
                self.settings.outlook_client_id = cid
                self.settings.save(self.paths)
                self.forms.pop(form_id, None)
                self.screen.account_form(None)
                await self._outlook_start()
            elif p == "telegram":
                signin = f.get("signin") or TelegramSignIn(self.telegram_factory)
                step = f.get("step", "phone")
                if step == "phone":
                    await signin.send_code(v.get("api_id", ""), v.get("api_hash", ""), v.get("phone", ""))
                    self.forms.pop(form_id, None)
                    self._telegram_form("code", signin, "Telegram sent a code to your Telegram app. Type it here.",
                                        [{"name": "code", "label": "Code", "type": "text"}], "Sign in")
                elif step == "code":
                    if await signin.code(v.get("code", "")):
                        await self._telegram_done(form_id, signin)
                    else:
                        self.forms.pop(form_id, None)
                        self._telegram_form("password", signin, "Your Telegram account has two-step verification: "
                                            "type its password.", [{"name": "password", "label": "Password",
                                                                    "type": "password"}], "Sign in")
                else:
                    await signin.password(v.get("password", ""))
                    await self._telegram_done(form_id, signin)
        except (ValueError, TelegramAuthError) as e:
            current = self.forms.get(form_id)
            if current is not None:
                # The same form again, with what went wrong, and what was typed in it (passwords aside).
                shown = dict(current.get("_shown") or {})
                shown["fields"] = [fd if fd.get("type") == "password" else {**fd, "value": v.get(fd["name"], fd.get("value", ""))}
                                   for fd in shown.get("fields") or []]
                self.screen.account_form({**shown, "id": form_id, "error": str(e)})
            else:
                self.notice(str(e), "warn")

    def _telegram_form(self, step: str, signin: TelegramSignIn, text: str, fields: list[dict[str, Any]],
                       ok: str) -> None:
        self._drop_forms(keep=signin)
        self._form_n += 1
        fid = f"telegram-{self._form_n}"
        shown = {"title": "Connect Telegram", "text": [text], "fields": fields, "ok": ok}
        self.forms[fid] = {"provider": "telegram", "step": step, "signin": signin, "_shown": shown}
        self.screen.account_form({"id": fid, **shown})

    async def _telegram_done(self, form_id: str, signin: TelegramSignIn) -> None:
        account = signin.account()
        await signin.close()
        self.forms.pop(form_id, None)
        self.screen.account_form(None)
        self._connected(account)

    def form_cancel(self, form_id: str) -> None:
        f = self.forms.pop(form_id, None)
        if f and f.get("signin"):
            self.loop.create_task(f["signin"].close())

    async def disconnect(self, account_id: str) -> str:
        a = self.accounts.remove(account_id)
        if a is None:
            return "There's no account with that id."
        t = self.tasks.pop(account_id, None)
        if t:
            t.cancel()
        src = self.telegram.pop(account_id, None)
        if src:
            await src.stop()
        self.mailboxes.pop(account_id, None)
        if a["kind"] == "web":
            self.screen.source_forget(a["provider"])
        self.store.drop_account(account_id)
        self.show_all()
        return f"{a.get('label')} {a.get('address') or ''} is no longer connected, and its messages are gone from " \
               "Messages.".replace("  ", " ")

    # ================================================================ for the model
    def lines(self, rows: list[dict[str, Any]]) -> str:
        out = []
        for m in rows:
            when = time.strftime("%a %d %b %H:%M", time.localtime(m["ts"]))
            what = m["title"] if m["source"] == "email" and m.get("title") else m.get("text", "")
            extra = f" — {m['text'][:200]}" if m["source"] == "email" and m.get("text") else ""
            unread = " (unread)" if m["unread"] else ""
            out.append(f"[{m['id']}] {when} {m['label']}, {m['who']}{unread}: {what}{extra}")
        return "\n".join(out)

    def accounts_text(self) -> str:
        accts = self.accounts.all()
        if not accts:
            return "No accounts connected. Connectable: " + ", ".join(l for l, _ in CHOICES) + "."
        return "\n".join(f"[{a['id']}] {a.get('label')} {a.get('address') or ''} — {self.status.get(a['id']) or 'starting'}"
                         for a in accts)
