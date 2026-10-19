"""What the tools need from the display. The GTK interface implements this for real; the
`RecordingScreen` stand-in is used by tests and when no display is available."""
from __future__ import annotations

from typing import Any


class Screen:
    """Async interface to the one ClaudeOS window: the conversation and the content layer."""

    # --- content layer (websites, documents, files, pages Claude made)
    async def show_url(self, url: str) -> dict[str, Any]: raise NotImplementedError
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
    # --- conversation layer
    def user_text(self, msg_id: str, text: str, final: bool) -> None: ...
    def partial(self, text: str) -> None: ...
    def ai_delta(self, msg_id: str, text: str) -> None: ...
    def ai_done(self, msg_id: str) -> None: ...
    def activity(self, msg_id: str, text: str) -> None: ...
    def notice(self, text: str, kind: str = "info") -> None: ...
    def state(self, name: str) -> None: ...
    def history(self, items: list[dict[str, Any]]) -> None: ...
    def apply_settings(self, text_size: int, user_name: str, model: str = "") -> None: ...
    def mic(self, on: bool) -> None:
        """Whether the microphone is on: the line under the message box says "Listening" only then."""
    # --- What's done (claudeos/done.py), in a panel at the right of the conversation
    def done_list(self, entries: list[dict[str, Any]]) -> None:
        """The whole list (at the start), oldest first. Each: id, ts, text, and the message that asked for it
        (prompt_id, prompt_ts)."""
    def done(self, entry: dict[str, Any]) -> None:
        """Something just done, at the end of the list."""
    def earlier(self, items: list[dict[str, Any]], target_id: str | None, target_ts: float) -> None:
        """Earlier messages (from the transcript) put back above the conversation, then scrolled to the one asked
        for: the answer to on_goto."""
    # Set by the controller: an item in What's done was tapped whose message isn't on the screen any more
    # (its id, its time, and the time of the first message the screen still has, or None).
    on_goto: Any = None
    def dots(self, dots: list[tuple[float, float, str]]) -> None:
        """AirTouch's dots, one per finger it follows (x, y in the display's pixels, and what the finger is
        doing: "point", "pinch", "scroll"; with the keyboard up "key", "press"). None left: none drawn."""
    async def show_keyboard(self, show: bool) -> tuple | None:
        """Put AirTouch's keyboard up: returns where its keys are (x, y, width, height in the display's pixels,
        and how many keys across and rows down); or take it down (None)."""
        return None
    async def keyboard_press(self, x: float, y: float) -> tuple | None:
        """A finger pressed the keyboard at x, y (the display's pixels): what to type (("text", "a") or
        ("key", "Return")), ("hide",), or None (shift, another page of keys, or no key there)."""
        return None
    def screen_bounds(self) -> tuple[int, int, int, int] | None:
        """Where ClaudeOS's screen is (x, y, width, height in the display's pixels), for AirTouch's pointer."""
        return None
    def layout_info(self) -> dict[str, Any]:
        """The screen's size, the estimated viewing distance and the text size worked out from them."""
        return {}
    def quit(self) -> None:
        """Close the window, which ends ClaudeOS (the session starts it again)."""


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

    async def show_url(self, url: str) -> dict[str, Any]:
        self.mode, self.url, self.title, self.html = "content", url, url, None
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

    def apply_settings(self, text_size: int, user_name: str, model: str = "") -> None:
        self.events.append(("settings", (text_size, user_name, model)))

    def mic(self, on: bool) -> None:
        self.events.append(("mic", on))

    def done_list(self, entries: list[dict[str, Any]]) -> None:
        self.events.append(("done_list", [e["text"] for e in entries]))

    def done(self, entry: dict[str, Any]) -> None:
        self.events.append(("done", dict(entry)))

    def earlier(self, items: list[dict[str, Any]], target_id: str | None, target_ts: float) -> None:
        self.events.append(("earlier", ([(i["role"], i["text"]) for i in items], target_id)))

    def dots(self, dots: list[tuple[float, float, str]]) -> None:
        self.events.append(("dots", [(round(x), round(y), kind) for x, y, kind in dots]))

    def quit(self) -> None:
        self.events.append(("quit", None))

    # A 1920x1080 screen with the keyboard along its bottom 432 pixels.
    KEYBOARD_TOP, KEYBOARD_SIZE = 648, (1920, 432)

    async def show_keyboard(self, show: bool) -> tuple | None:
        from .airtouch.keyboard import ROWS, UNITS, Keyboard
        self.events.append(("keyboard", show))
        if not show:
            self.keyboard = None
            return None
        self.keyboard = Keyboard()
        self.keyboard.layout(*self.KEYBOARD_SIZE)
        x, y, w, h = self.keyboard.area()
        return (x, self.KEYBOARD_TOP + y, w, h, UNITS, ROWS)

    async def keyboard_press(self, x: float, y: float) -> tuple | None:
        import time
        kb = getattr(self, "keyboard", None)
        key = kb.hit(x, y - self.KEYBOARD_TOP) if kb else None
        if key is None:
            return None
        state = (kb.page, kb.shift)
        action = kb.press(key, time.monotonic())
        if (kb.page, kb.shift) != state:
            kb.layout(*self.KEYBOARD_SIZE)
        self.events.append(("key", key.label))
        return action

    # test helpers
    def ai_text(self) -> str:
        return "".join(t for k, v in self.events if k == "ai" for _, t in [v])
