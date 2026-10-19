"""Telegram through Telegram's own API (MTProto, with the Telethon library), as an unofficial client that only
reads: the latest message of each private chat and group (not channels), then new ones as they arrive.

Signing in takes an api_id and api_hash (made once at my.telegram.org, "API development tools": Telegram asks
every client to use its own), the phone number, the code Telegram sends to the Telegram app, and the two-step
verification password if there is one. All of it is typed by the person into Chat Computer's form; the sign-in
(Telethon's session string) is kept with the other accounts' in secrets/accounts.json.

Telegram's API terms: core.telegram.org/api/terms (an app must say it uses the Telegram API, guard its users'
privacy, and not act for them without their knowledge).
"""
from __future__ import annotations

import asyncio
import logging
import time
from typing import Any, Callable

from .. import __version__

log = logging.getLogger("chatcomputer.messages.telegram")
BACKFILL_DIALOGS = 60            # chats whose latest message is brought in at the start
CHAT_LINES = 30                  # messages shown when a chat is opened


class AuthError(Exception):
    pass


def _client(session: str, api_id: int, api_hash: str) -> Any:
    from telethon import TelegramClient
    from telethon.sessions import StringSession
    return TelegramClient(StringSession(session or None), int(api_id), api_hash, device_model="Chat Computer",
                          system_version="Linux", app_version=__version__)


def describe_media(msg: Any) -> str:
    """Words for a message that isn't only text."""
    for attr, word in (("photo", "Photo"), ("voice", "Voice message"), ("video_note", "Video message"),
                       ("gif", "GIF"), ("sticker", "Sticker"), ("video", "Video"), ("audio", "Audio"),
                       ("contact", "Contact"), ("geo", "Location"), ("poll", "Poll"), ("document", "File")):
        if getattr(msg, attr, None):
            return word
    return "Message"


def _name(entity: Any) -> str:
    try:
        from telethon import utils
        name = utils.get_display_name(entity) or ""
    except Exception:
        name = ""
    if not name and entity is not None:         # a kind Telethon doesn't name: its title, or first and last name
        name = getattr(entity, "title", None) or " ".join(
            x for x in (getattr(entity, "first_name", None), getattr(entity, "last_name", None)) if x)
    return str(name or "")


class TelegramSource:
    """One signed-in Telegram account: brings its messages into Messages while Chat Computer runs."""

    def __init__(self, account: dict[str, Any], add: Callable[[list[dict[str, Any]]], None],
                 client_factory: Callable[..., Any] | None = None) -> None:
        self.account = account
        self.add = add
        self.client_factory = client_factory or _client
        self.client: Any = None

    def _item(self, msg: Any, chat: Any, sender: Any, unread: bool, group: bool) -> dict[str, Any]:
        a = self.account
        chat_name = _name(chat) or "Telegram"
        sender_name = _name(sender) or chat_name
        text = (getattr(msg, "message", "") or "").strip() or describe_media(msg)
        date = getattr(msg, "date", None)
        chat_id = getattr(msg, "chat_id", None) or getattr(chat, "id", 0)
        return {"id": f"tg:{a['id']}:{chat_id}:{msg.id}", "source": "telegram", "account": a["id"],
                "label": "Telegram", "who": chat_name, "address": "",
                "title": "", "text": (f"{sender_name}: {text}" if group else text)[:500],
                "ts": date.timestamp() if date else time.time(), "unread": unread, "chat": str(chat_id),
                "ref": f"{chat_id}:{msg.id}"}

    async def start(self) -> None:
        a = self.account
        self.client = self.client_factory(a.get("session") or "", a["api_id"], a["api_hash"])
        await self.client.connect()
        if not await self.client.is_user_authorized():
            await self.client.disconnect()
            raise AuthError("Telegram needs signing in again")
        from telethon import events
        self.client.add_event_handler(self._on_new, events.NewMessage(incoming=True))
        await self.backfill()

    async def backfill(self) -> None:
        items = []
        async for d in self.client.iter_dialogs(limit=BACKFILL_DIALOGS):
            if (d.is_channel and not d.is_group) or d.message is None or getattr(d.message, "out", False):
                continue
            try:
                sender = await d.message.get_sender()
            except Exception:
                sender = None
            items.append(self._item(d.message, d.entity, sender, d.unread_count > 0, bool(d.is_group)))
        self.add(items)

    async def _on_new(self, event: Any) -> None:
        try:
            if event.is_channel and not event.is_group:
                return                  # a channel's posts aren't messages to you
            chat = await event.get_chat()
            sender = await event.get_sender()
            self.add([self._item(event.message, chat, sender, True, bool(event.is_group))])
        except Exception:
            log.exception("a new Telegram message couldn't be shown")

    async def chat(self, ref: str) -> tuple[str, list[tuple[str, str, str]]]:
        """A chat's latest messages: (its name, [(who, when, words)]), oldest first. Reading them here doesn't
        mark them read in Telegram."""
        chat_id = int(ref.split(":")[0])
        entity = await self.client.get_entity(chat_id)
        lines = []
        for m in reversed(await self.client.get_messages(entity, limit=CHAT_LINES)):
            who = "You" if m.out else (_name(await m.get_sender()) or _name(entity))
            when = time.strftime("%a %d %b %H:%M", time.localtime(m.date.timestamp())) if m.date else ""
            lines.append((who, when, (m.message or "").strip() or describe_media(m)))
        return _name(entity) or "Telegram", lines

    async def stop(self) -> None:
        if self.client is not None:
            try:
                await self.client.disconnect()
            except Exception:
                pass


class TelegramSignIn:
    """The steps of signing in: the phone number (Telegram sends a code), the code, and maybe the password."""

    def __init__(self, client_factory: Callable[..., Any] | None = None) -> None:
        self.client_factory = client_factory or _client
        self.client: Any = None
        self.api_id = 0
        self.api_hash = ""
        self.phone = ""
        self.code_hash = ""

    async def send_code(self, api_id: str, api_hash: str, phone: str) -> None:
        try:
            self.api_id = int(str(api_id).strip())
        except ValueError:
            raise AuthError("The api_id is a number (from my.telegram.org).") from None
        self.api_hash = str(api_hash).strip()
        self.phone = "".join(ch for ch in str(phone) if ch.isdigit() or ch == "+")
        if not self.api_hash or len(self.phone) < 6:
            raise AuthError("Fill in the api_hash and your phone number, with its country code (+65...).")
        await self.close()
        self.client = self.client_factory("", self.api_id, self.api_hash)
        await self.client.connect()
        try:
            sent = await self.client.send_code_request(self.phone)
        except Exception as e:
            raise AuthError(f"Telegram didn't send a code: {e}") from None
        self.code_hash = getattr(sent, "phone_code_hash", "") or ""

    async def code(self, code: str) -> bool:
        """True when signed in; False when Telegram wants the two-step verification password next."""
        from telethon.errors import SessionPasswordNeededError
        try:
            await self.client.sign_in(phone=self.phone, code=str(code).strip(), phone_code_hash=self.code_hash or None)
        except SessionPasswordNeededError:
            return False
        except Exception as e:
            raise AuthError(f"That code didn't work: {e}") from None
        return True

    async def password(self, password: str) -> None:
        try:
            await self.client.sign_in(password=password)
        except Exception as e:
            raise AuthError(f"That password didn't work: {e}") from None

    def account(self) -> dict[str, Any]:
        return {"kind": "telegram", "provider": "telegram", "label": "Telegram", "address": self.phone,
                "api_id": self.api_id, "api_hash": self.api_hash, "session": self.client.session.save()}

    async def close(self) -> None:
        if self.client is not None:
            try:
                await asyncio.wait_for(self.client.disconnect(), 10)
            except Exception:
                pass
            self.client = None
