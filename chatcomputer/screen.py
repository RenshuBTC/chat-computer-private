"""What the tools need from the display. The GTK interface implements this for real; the
`RecordingScreen` stand-in is used by tests and when no display is available."""
from __future__ import annotations

from typing import Any


class Screen:
    """Async interface to the one Chat Computer window: the conversation and the content layer."""

    # --- content layer (websites, documents, files, pages the model made)
    async def show_url(self, url: str, title: str | None = None) -> dict[str, Any]:
        """Opens an address in the middle; `title` names it at the top of its frame (else its site's name)."""
        raise NotImplementedError
    async def show_html(self, html: str, title: str, base_dir: str | None = None) -> dict[str, Any]: raise NotImplementedError
    async def hide_content(self) -> None: raise NotImplementedError
    async def content_info(self) -> dict[str, Any]: raise NotImplementedError
    async def settle(self, quiet_s: float = 0.15, max_s: float = 1.5, load_s: float = 10.0) -> dict[str, Any]:
        """After a click or keys: wait for the page to react (a page loading, then a moment without changes), and
        return what's showing (content_info)."""
        return await self.content_info()
    async def eval_js(self, script: str) -> Any: raise NotImplementedError
    async def navigate(self, action: str) -> dict[str, Any]: raise NotImplementedError
    async def focus_content(self) -> None: raise NotImplementedError
    async def content_origin(self) -> tuple[int, int, float]: raise NotImplementedError
    async def content_rect(self) -> tuple[int, int, int, int] | None:
        """Where what's open is on the screen, in the screen's pixels (x, y, width, height); None when nothing is."""
        return None
    async def to_pdf(self, path: str, html: str | None = None, base_dir: str | None = None) -> dict[str, Any]:
        """Print to a PDF file, with no printer and nothing shown: the page given (html), or what's open in the
        middle. {"ok": True, "path": path} or {"ok": False, "error": why}."""
        return {"ok": False, "error": "PDFs can't be made here."}
    # --- conversation layer
    def user_text(self, msg_id: str, text: str, final: bool) -> None: ...
    def partial(self, text: str) -> None: ...
    def ai_delta(self, msg_id: str, text: str) -> None: ...
    def ai_done(self, msg_id: str) -> None: ...
    def activity(self, msg_id: str, text: str) -> None: ...
    def notice(self, text: str, kind: str = "info") -> None: ...
    def state(self, name: str) -> None: ...
    def history(self, items: list[dict[str, Any]]) -> None: ...
    def clear(self) -> None:
        """The conversation taken off the screen (a new chat): its greeting again."""
    def chats(self, items: list[dict[str, Any]], current: str) -> None:
        """The chats (id, title, ts; newest first) in the right sidebar's lower half, and which is on the screen."""
    def apply_settings(self, text_size: int, user_name: str, model: str = "") -> None: ...
    def panels(self, shown: dict[str, bool]) -> None:
        """Which panels show: {"left": Messages, "right": Actions, "bottom": the conversation under what's open}."""
    def mic(self, on: bool) -> None:
        """Whether the microphone is on."""
    def here(self, info: dict[str, Any]) -> None:
        """Where they are, and the weather there now when it's known, for the greeting (here.py): {place, code,
        temp, day}."""
    # --- "/" in the message box: searching this computer
    def search_results(self, n: int, query: str, items: list[dict[str, Any]], note: str = "") -> None:
        """What on this computer matches the words after "/" (search number n): [{name, path, where, kind, when}]."""
    # --- Actions (chatcomputer/done.py), in a panel at the right of the conversation
    def done_list(self, entries: list[dict[str, Any]]) -> None:
        """The whole list (at the start), oldest first. Each: id, ts, text, and the message that asked for it
        (prompt_id, prompt_ts)."""
    def done(self, entry: dict[str, Any]) -> None:
        """Something just done, at the end of the list."""
    def done_states(self, entries: list[dict[str, Any]]) -> None:
        """Which of those shown can be reverted ("revert") and which have been ("reverted"): after a revert, or "undo
        that". Only their marks change, not the list."""
    def earlier(self, items: list[dict[str, Any]], target_id: str | None, target_ts: float) -> None:
        """Earlier messages (from the transcript) put back above the conversation, then scrolled to the one asked
        for: the answer to on_goto."""
    # Set by the controller: an item in Actions was tapped whose message isn't on the screen any more
    # (its id, its time, and the time of the first message the screen still has, or None).
    on_goto: Any = None
    # --- Messages (chatcomputer/messages), at the left of the conversation
    def messages_list(self, items: list[dict[str, Any]], accounts: list[dict[str, Any]] | None = None) -> None:
        """All the messages shown (newest first), and the accounts they come from (without their sign-ins)."""
    def message(self, item: dict[str, Any]) -> None:
        """A new or changed message."""
    def set_pinned(self, kind: str, item_id: str, pinned: float | None) -> None:
        """A pin made or taken off on the page ("done" or "messages")."""
    def account_form(self, form: dict[str, Any] | None) -> None:
        """A sign-in form for an account, on the conversation page (None closes it); see chat.js showForm."""
    def source_open(self, provider: str, url: str, reload: bool = False) -> None:
        """Loads a site Messages reads (WhatsApp, Instagram...) in its own browser, off the screen."""
    def source_show(self, provider: str, url: str | None = None, chat: str | None = None) -> None:
        """Shows that site between the sidebars: to sign in, or to read a conversation."""
    def source_unload(self, provider: str) -> None:
        """Lets a site's page go until it's looked at again."""
    def source_close(self, provider: str) -> None:
        """Takes the site away from between the sidebars, if it's there."""
    def source_forget(self, provider: str) -> None:
        """The site's account was removed here: its page goes."""
    def watch_redirect(self, prefix: str, then: Any) -> None:
        """When the content view goes to an address starting with `prefix`, it isn't loaded: then(address)."""
    # Set by the controller: what the conversation page tells it (pins, a message opened, a form filled in), what a
    # site Messages reads says it shows, and that what was open was closed.
    on_page_event: Any = None
    on_source: Any = None
    on_closed: Any = None
    # The onscreen keyboard: it came up, moved or went (where its keys are, or None); a key tapped on it.
    on_keyboard: Any = None
    on_keyboard_action: Any = None
    async def show_keyboard(self, show: bool) -> tuple | None:
        """Put the onscreen keyboard up: returns where its keys are (x, y, width, height in the display's pixels,
        and how many keys across and rows down); or take it down (None)."""
        return None
    def lock_screen(self, info: dict[str, Any] | None) -> None:
        """The lock screen over everything ({"error": what to say about the last PIN, "note": the line under
        "Locked"}), or None to take it away. What's open stays open, off the screen, until then. The PIN typed on
        it comes back as the page event {"type": "unlock", "pin": ...}."""
    def layout_info(self) -> dict[str, Any]:
        """The screen's size, the estimated viewing distance and the text size worked out from them."""
        return {}
    def quit(self) -> None:
        """Close the window, which ends Chat Computer (the session starts it again)."""


class RecordingScreen(Screen):
    """Remembers what would have been shown. Pages can be given canned JS results."""

    def __init__(self) -> None:
        self.events: list[tuple[str, Any]] = []
        self.mode = "chat"
        self.url: str | None = None
        self.title: str | None = None
        self.html: str | None = None
        self.js_results: dict[str, Any] = {}
        self.js_calls: list[str] = []

    async def show_url(self, url: str, title: str | None = None) -> dict[str, Any]:
        self.mode, self.url, self.title, self.html = "content", url, title or url, None
        self.events.append(("show_url", url))
        return {"ok": True, "url": url, "title": url}

    async def show_html(self, html: str, title: str, base_dir: str | None = None) -> dict[str, Any]:
        self.mode, self.url, self.title, self.html = "content", None, title, html
        self.events.append(("show_html", title))
        return {"ok": True, "title": title}

    async def hide_content(self) -> None:
        self.mode = "chat"
        self.events.append(("hide", None))

    async def content_info(self) -> dict[str, Any]:
        return {"mode": self.mode, "url": self.url, "title": self.title}

    async def eval_js(self, script: str) -> Any:
        self.js_calls.append(script)
        for key, val in self.js_results.items():
            if key in script:
                return val
        return None

    async def navigate(self, action: str) -> dict[str, Any]:
        self.events.append(("navigate", action))
        return {"ok": True, "url": self.url, "title": self.title}

    async def focus_content(self) -> None:
        self.events.append(("focus", None))

    async def content_origin(self) -> tuple[int, int, float]:
        return 0, 0, 1.0

    async def content_rect(self) -> tuple[int, int, int, int] | None:
        return (100, 50, 800, 600) if self.mode == "content" else None

    async def to_pdf(self, path: str, html: str | None = None, base_dir: str | None = None) -> dict[str, Any]:
        """A one-page PDF saying what it was made from (the page's title), for tests."""
        if html is None and self.mode != "content":
            return {"ok": False, "error": "Nothing is open on the screen."}
        import re
        title = (re.search(r"<title>([^<]*)</title>", html or "") or [None, self.title or self.url or "page"])[1]
        self.events.append(("pdf", (path, title)))
        with open(path, "wb") as f:
            f.write(b"%PDF-1.4\n% " + str(title).encode() + b"\n%%EOF\n")
        return {"ok": True, "path": path}

    def user_text(self, msg_id: str, text: str, final: bool) -> None:
        self.events.append(("user", (msg_id, text, final)))

    def partial(self, text: str) -> None:
        self.events.append(("partial", text))

    def ai_delta(self, msg_id: str, text: str) -> None:
        self.events.append(("ai", (msg_id, text)))

    def ai_done(self, msg_id: str) -> None:
        self.events.append(("ai_done", msg_id))

    def activity(self, msg_id: str, text: str) -> None:
        self.events.append(("activity", (msg_id, text)))

    def notice(self, text: str, kind: str = "info") -> None:
        self.events.append(("notice", (text, kind)))

    def state(self, name: str) -> None:
        self.events.append(("state", name))

    def history(self, items: list[dict[str, Any]]) -> None:
        self.events.append(("history", len(items)))

    def clear(self) -> None:
        self.events.append(("clear", None))

    def chats(self, items: list[dict[str, Any]], current: str) -> None:
        self.events.append(("chats", ([c["title"] for c in items], current)))

    def apply_settings(self, text_size: int, user_name: str, model: str = "") -> None:
        self.events.append(("settings", (text_size, user_name, model)))

    def here(self, info: dict[str, Any]) -> None:
        self.events.append(("here", dict(info)))

    def panels(self, shown: dict[str, bool]) -> None:
        self.events.append(("panels", dict(shown)))

    def mic(self, on: bool) -> None:
        self.events.append(("mic", on))

    def search_results(self, n: int, query: str, items: list[dict[str, Any]], note: str = "") -> None:
        self.events.append(("search", (n, query, items, note)))

    def done_list(self, entries: list[dict[str, Any]]) -> None:
        self.events.append(("done_list", [e["text"] for e in entries]))

    def done(self, entry: dict[str, Any]) -> None:
        self.events.append(("done", dict(entry)))

    def done_states(self, entries: list[dict[str, Any]]) -> None:
        self.events.append(("done_states", {e["id"]: "reverted" if e.get("reverted") else "revert" if e.get("revert")
                                             else "" for e in entries}))

    def earlier(self, items: list[dict[str, Any]], target_id: str | None, target_ts: float) -> None:
        self.events.append(("earlier", ([(i["role"], i["text"]) for i in items], target_id)))

    def messages_list(self, items: list[dict[str, Any]], accounts: list[dict[str, Any]] | None = None) -> None:
        self.events.append(("messages_list", ([m["id"] for m in items], [a.get("label") for a in accounts or []])))

    def message(self, item: dict[str, Any]) -> None:
        self.events.append(("message", dict(item)))

    def set_pinned(self, kind: str, item_id: str, pinned: float | None) -> None:
        self.events.append(("pinned", (kind, item_id, bool(pinned))))

    def account_form(self, form: dict[str, Any] | None) -> None:
        self.form = form
        self.events.append(("form", dict(form) if form else None))

    def source_open(self, provider: str, url: str, reload: bool = False) -> None:
        self.events.append(("source_open", (provider, url)))

    def source_show(self, provider: str, url: str | None = None, chat: str | None = None) -> None:
        self.events.append(("source_show", (provider, url, chat)))

    def source_unload(self, provider: str) -> None:
        self.events.append(("source_unload", provider))

    def source_close(self, provider: str) -> None:
        self.events.append(("source_close", provider))

    def source_forget(self, provider: str) -> None:
        self.events.append(("source_forget", provider))

    def watch_redirect(self, prefix: str, then: Any) -> None:
        self.redirects = getattr(self, "redirects", {})
        if then is None:
            self.redirects.pop(prefix, None)
        else:
            self.redirects[prefix] = then

    def quit(self) -> None:
        self.events.append(("quit", None))

    def lock_screen(self, info: dict[str, Any] | None) -> None:
        self.locked = info is not None
        self.events.append(("lock", info))

    # A 1920x1080 screen with the keyboard along its bottom 432 pixels.
    KEYBOARD_TOP, KEYBOARD_SIZE = 648, (1920, 432)

    async def show_keyboard(self, show: bool) -> tuple | None:
        from .ui.keyboard import ROWS, UNITS, Keyboard
        self.events.append(("keyboard", show))
        if not show:
            self.keyboard = None
            return None
        self.keyboard = Keyboard()
        self.keyboard.layout(*self.KEYBOARD_SIZE)
        x, y, w, h = self.keyboard.area()
        return (x, self.KEYBOARD_TOP + y, w, h, UNITS, ROWS)

    # test helpers
    def ai_text(self) -> str:
        return "".join(t for k, v in self.events if k == "ai" for _, t in [v])
