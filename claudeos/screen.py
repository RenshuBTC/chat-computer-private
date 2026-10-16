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
    def apply_settings(self, theme: str, text_size: int, user_name: str) -> None: ...
    def layout_info(self) -> dict[str, Any]:
        """The screen's size, the estimated viewing distance and the text size worked out from them."""
        return {}


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

    def apply_settings(self, theme: str, text_size: int, user_name: str) -> None:
        self.events.append(("settings", (theme, text_size, user_name)))

    # test helpers
    def ai_text(self) -> str:
        return "".join(t for k, v in self.events if k == "ai" for _, t in [v])
