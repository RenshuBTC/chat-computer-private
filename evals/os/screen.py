"""The screen of the simulated computer: Chat Computer's stand-in screen (RecordingScreen), drawn on the virtual
display, so a screenshot or look_at_screen sees what a person would (the conversation, or the site or document
that's open); and the sites the cases open answer read_screen, click and media the way those pages would (Gmail's
inbox, YouTube's results and a playing video, BBC News, Spotify's web player). Any other site shows its address and
a line saying it's open.

Without this, the display stayed black and every page was unreadable: a model that checks its work (as Opus does)
then reports that nothing shows, which is the simulation's failing, not Chat Computer's."""
from __future__ import annotations

import contextlib
import html as htmlmod
import json
import re
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, quote_plus, urlparse

from chatcomputer.screen import RecordingScreen

FONT = Path(__file__).resolve().parents[2] / "chatcomputer" / "ui" / "assets" / "fonts" / "Lora.ttf"
W, H = 1920, 1080


def _site(url: str) -> dict[str, Any]:
    """What a page at url shows: its title, its text, and what can be clicked ([(kind, name, where it goes)])."""
    u = urlparse(url or "")
    host = (u.hostname or "").removeprefix("www.")
    q = parse_qs(u.query)
    if host == "mail.google.com":
        mails = ["DBS Bank: Your October statement is ready", "Mum: Dinner on Sunday?",
                 "Singapore Airlines: Online check-in for SQ322 opens on 22 October"]
        return {"title": "Inbox (3) - sam@example.com - Gmail", "heading": "Gmail",
                "text": "Compose\nInbox 3\nStarred\nSent\n" + "\n".join(mails),
                "links": [("button", "Compose", None), ("link", "Inbox", url)] + [("link", m, url) for m in mails]}
    if host == "youtube.com" and u.path.startswith("/results"):
        query = (q.get("search_query") or ["music"])[0]
        vids = [f"{query.title()} | 3 Hours of Smooth Music", f"Relaxing {query} for work and study",
                f"Best of {query} - live session"]
        return {"title": f"{query} - YouTube", "heading": "YouTube",
                "text": f"Search: {query}\n" + "\n".join(f"{v}\n1.2M views" for v in vids),
                "links": [("link", v, f"https://www.youtube.com/watch?v=sim{i}") for i, v in enumerate(vids)]}
    if host == "youtube.com" and u.path.startswith("/watch"):
        return {"title": "Smooth Jazz | 3 Hours of Relaxing Music - YouTube", "heading": "YouTube",
                "text": "Smooth Jazz | 3 Hours of Relaxing Music\nCafe Music BGM channel\n1.2M views",
                "links": [("button", "Pause (k)", None), ("button", "Full screen (f)", None)], "video": True}
    if host == "youtube.com":
        return {"title": "YouTube", "heading": "YouTube", "text": "Home\nShorts\nSubscriptions\nSearch",
                "links": [("input", "Search", None)]}
    if host in ("bbc.com", "bbc.co.uk") and "news" in u.path:
        heads = ["Typhoon heads for Hong Kong as schools close", "Markets steady ahead of rate decision",
                 "Scientists map the deepest coral reef yet"]
        return {"title": "BBC News - Home", "heading": "BBC NEWS", "text": "\n".join(heads),
                "links": [("link", h, url) for h in heads]}
    if host == "open.spotify.com":
        login = "https://accounts.spotify.com/en/login?continue=https%3A%2F%2Fopen.spotify.com%2F"
        return {"title": "Spotify – Web Player: Music for everyone", "heading": "Spotify",
                "text": "Good afternoon\nLog in\nSign up free\nPopular artists\nPopular albums",
                "links": [("button", "Log in", login), ("button", "Sign up free", "https://www.spotify.com/signup")]}
    if host == "accounts.spotify.com":
        return {"title": "Log in - Spotify", "heading": "Log in to Spotify",
                "text": "Continue with Google\nContinue with Facebook\nContinue with Apple\nEmail or username\nPassword\n"
                        "Log In\nForgot your password?\nDon't have an account? Sign up for Spotify",
                "links": [("button", "Continue with Google", None), ("button", "Continue with Apple", None),
                          ("input", "Email or username", None), ("input", "Password", None), ("button", "Log In", None)]}
    if url.startswith("file://") and "/pages/" in url:
        return {"title": "A page Chat Computer made", "heading": "", "text": "(a page made by Chat Computer)",
                "links": []}
    return {"title": host or url, "heading": host or url, "text": f"{host or url}\n(the page is open)", "links": []}


def _font(size: int) -> Any:
    from PIL import ImageFont
    try:
        return ImageFont.truetype(str(FONT), size)
    except OSError:
        return ImageFont.load_default()


def _draw_lines(d: Any, x: int, y: int, lines: list[str], size: int, width: int, fill=(0, 0, 0)) -> int:
    f = _font(size)
    for raw in lines:
        line = ""
        for word in raw.split(" "):
            test = (line + " " + word).strip()
            if d.textlength(test, font=f) > width and line:
                d.text((x, y), line, font=f, fill=fill)
                y += int(size * 1.4)
                line = word
            else:
                line = test
        d.text((x, y), line, font=f, fill=fill)
        y += int(size * 1.4)
        if y > H - 60:
            break
    return y


class SimScreen(RecordingScreen):
    def __init__(self, platform: Any = None, settings: Any = None) -> None:
        super().__init__()
        self.platform = platform
        self.settings = settings
        self.said: list[tuple[str, str]] = []           # the conversation: (who, words)
        self._ai: dict[str, str] = {}
        self._go: str | None = None                       # where a click on what was located goes
        self._paused = False
        self.locked = False
        self.pin = "4826"         # the PIN the person chooses when Chat Computer asks for one (they type it)

    # --- what's shown, drawn on the display
    def paint(self) -> None:
        try:
            from PIL import Image, ImageDraw
            img = Image.new("RGB", (W, H), (255, 255, 255))
            d = ImageDraw.Draw(img)
            if self.locked:
                import time
                d.text((W // 2 - 150, 120), time.strftime("%H:%M"), font=_font(110), fill=(0, 0, 0))
                d.text((W // 2 - 70, 330), "Locked", font=_font(44), fill=(0, 0, 0))
                d.text((W // 2 - 170, 400), "Type your PIN to unlock.", font=_font(32), fill=(0, 0, 0))
                for i, k in enumerate("123456789 0 "):
                    if k.strip():
                        x, y = W // 2 - 190 + (i % 3) * 140, 500 + (i // 3) * 120
                        d.ellipse((x, y, x + 100, y + 100), outline=(200, 200, 200), width=3)
                        d.text((x + 38, y + 28), k, font=_font(40), fill=(0, 0, 0))
            elif self.mode == "content":
                if self.url:
                    s = _site(self.url)
                    d.rectangle((0, 0, W, 64), fill=(236, 236, 236))
                    d.text((32, 18), self.url[:150], font=_font(24), fill=(60, 60, 60))
                    d.text((120, 110), s["heading"], font=_font(56), fill=(200, 0, 0) if "YouTube" in s["heading"] else (0, 0, 0))
                    if s.get("video"):
                        d.rectangle((120, 200, 1240, 830), fill=(20, 20, 20))
                        d.text((150, 760), "Playing  0:12 / 3:00:00", font=_font(28), fill=(255, 255, 255))
                        _draw_lines(d, 120, 860, s["text"].splitlines(), 30, 1600)
                    else:
                        _draw_lines(d, 120, 210, s["text"].splitlines(), 34, 1600)
                else:
                    text = re.sub(r"<[^>]+>", " ", self.html or "")
                    text = htmlmod.unescape(re.sub(r"[ \t]+", " ", text))
                    d.text((120, 80), self.title or "", font=_font(48), fill=(0, 0, 0))
                    _draw_lines(d, 120, 170, [ln.strip() for ln in text.splitlines() if ln.strip()][:40], 30, 1600)
            else:
                # The conversation's text at the size the real screen works out (the screen, the distance, and
                # the text_size and text_scale settings), its column as many letters wide.
                fp = max(12, int(round(float(self.layout_info().get("font_px") or 32))))
                col = min(W - 300, int(680 * fp / 32))
                d.text((W // 2 - 190, 60), "Chat Computer", font=_font(52), fill=(0, 0, 0))
                y = 180
                for who, words in self.said[-4:]:
                    x = W - 240 - col if who == "user" else 240
                    y = _draw_lines(d, x, y, [words[:600]], fp, col, (40, 40, 40) if who == "user" else (0, 0, 0)) + 30
                d.rectangle((240, H - 120, W - 240, H - 50), outline=(180, 180, 180), width=2)
                d.text((270, H - 105), "Type anything or “/” or press “Esc”", font=_font(int(fp * 0.875)),
                       fill=(160, 160, 160))
            _to_display(img)
        except Exception:
            pass        # no display: nothing to draw on

    def open(self, url: str, title: str | None = None) -> None:
        self.mode, self.url, self.title, self.html = "content", url, title or _site(url)["title"], None
        self.paint()

    async def show_url(self, url: str, title: str | None = None) -> dict[str, Any]:
        await super().show_url(url, title)
        self.title = _site(url)["title"]
        self._paused = False
        self.paint()
        return {"ok": True, "url": url, "title": self.title}

    async def show_html(self, html: str, title: str, base_dir: str | None = None) -> dict[str, Any]:
        r = await super().show_html(html, title, base_dir)
        self.paint()
        return r

    async def hide_content(self) -> None:
        await super().hide_content()
        self.paint()

    async def navigate(self, action: str) -> dict[str, Any]:
        r = await super().navigate(action)
        self.paint()
        return r

    async def settle(self, quiet_s: float = 0.15, max_s: float = 1.5, load_s: float = 10.0) -> dict[str, Any]:
        if self._go:
            await self.show_url(self._go)
            self._go = None
        elif self.url and "youtube.com" in self.url and self.platform is not None:
            typed = [a[0] for n, a in self.platform.calls if n == "type_text"]
            if typed and any(n == "key" and a[0] == "Return" for n, a in self.platform.calls[-3:]):
                await self.show_url("https://www.youtube.com/results?search_query=" + quote_plus(typed[-1]))
        return await self.content_info()

    def user_text(self, msg_id: str, text: str, final: bool) -> None:
        super().user_text(msg_id, text, final)
        if final:
            self.said.append(("user", text))
            if self.mode != "content":
                self.paint()

    def ai_delta(self, msg_id: str, text: str) -> None:
        super().ai_delta(msg_id, text)
        self._ai[msg_id] = self._ai.get(msg_id, "") + text

    def ai_done(self, msg_id: str) -> None:
        super().ai_done(msg_id)
        if self._ai.get(msg_id):
            self.said.append(("ai", self._ai.pop(msg_id)))
            if self.mode != "content":
                self.paint()

    # --- the screen's size and the text size worked out from it, as the real screen says them: a laptop's 15.6-inch
    # screen, or a desktop's 24-inch monitor, both 1920x1080
    def layout_info(self) -> dict[str, Any]:
        from chatcomputer.ui.layout import compute, screen_from_monitor

        from . import world
        s = self.settings
        num = lambda k, d=0.0: float(getattr(s, k, d) or d) if s is not None else d    # noqa: E731
        mm = (531, 299) if world.load().get("device") == "desktop" else (344, 194)
        lay = compute(screen_from_monitor(W, H, W, H, *mm, diagonal_in=num("screen_inches")),
                      distance_cm=num("viewing_distance_cm"), text_px=num("text_size"),
                      text_scale=num("text_scale", 1.0), web_zoom=num("web_zoom"))
        d = lay.to_dict()
        d["summary"] = lay.describe()
        return d

    # --- what's open: the whole display; printed to a PDF, its words on a page
    async def content_rect(self) -> tuple[int, int, int, int] | None:
        return (0, 0, W, H) if self.mode == "content" else None

    async def to_pdf(self, path: str, html: str | None = None, base_dir: str | None = None) -> dict[str, Any]:
        from .home import _pdf
        if html is None:
            if self.mode != "content":
                return {"ok": False, "error": "Nothing is open on the screen."}
            text = _site(self.url)["text"] if self.url else (self.html or "")
            title = self.title or ""
        else:
            text = html
            title = (re.search(r"<title>([^<]*)</title>", html) or [None, ""])[1]
            md = re.search(r"renderMarkdown\((\".*?\")\);", html, re.S)     # a document: its words are in the script
            if md:
                with contextlib.suppress(ValueError):
                    text = json.loads(md.group(1))
        if "<" in text:
            text = re.sub(r"<script.*?</script>|<style.*?</style>", " ", text, flags=re.S)
            text = htmlmod.unescape(re.sub(r"<[^>]+>", "\n", text))
        lines = [title] + [ln.strip() for ln in text.splitlines() if ln.strip()][:50]
        safe = [ln.encode("latin-1", "replace").decode("latin-1") for ln in lines if ln]
        Path(path).write_bytes(_pdf(safe or ["(blank)"]))
        self.events.append(("pdf", (path, title)))
        return {"ok": True, "path": path}

    def apply_settings(self, text_size: int, user_name: str, model: str = "") -> None:
        super().apply_settings(text_size, user_name, model)
        if self.mode != "content":
            self.paint()          # a new text size shows at once, as on the real screen

    # --- the lock screen, and the person typing a PIN into the form Chat Computer shows for one
    def lock_screen(self, info: dict[str, Any] | None) -> None:
        super().lock_screen(info)
        self.paint()

    def account_form(self, form: dict[str, Any] | None) -> None:
        super().account_form(form)
        if not form or form.get("id") != "lock_pin" or self.on_page_event is None:
            return
        import asyncio
        if form.get("error"):          # what they typed didn't do: they leave it
            msg: dict[str, Any] = {"type": "account_cancel", "form": "lock_pin"}
        else:
            values = {f["name"]: self.pin for f in form.get("fields", [])}
            msg = {"type": "account", "form": "lock_pin", "values": values}
        asyncio.get_running_loop().call_later(1.0, self.on_page_event, msg)

    # --- the page's own answers (what chatcomputer/ui's page script returns for the tools)
    async def eval_js(self, script: str) -> Any:
        self.js_calls.append(script)
        m = re.search(r"__CO\.(\w+)\((.*)\)\s*:", script, re.S)
        if not m or self.mode != "content":
            return None
        fn, args = m.group(1), m.group(2)
        s = _site(self.url) if self.url else {"title": self.title, "text": re.sub(r"<[^>]+>", " ", self.html or ""),
                                              "links": []}
        links = s.get("links", [])
        if fn == "text":
            return {"title": s["title"], "url": self.url or "", "text": s["text"], "scrollable": False}
        if fn == "elements":
            return {"title": s["title"], "url": self.url or "",
                    "lines": [f"[{i + 1}] {kind}: {name}" for i, (kind, name, _) in enumerate(links)]}
        if fn == "locate":
            try:
                ref, text = json.loads("[" + args + "]")
            except ValueError:
                ref, text = None, ""
            for i, (_kind, name, go) in enumerate(links):
                if (ref is not None and ref == i + 1) or (text and text.lower() in name.lower()):
                    self._go = go if go and go != self.url else None
                    return {"x": 200, "y": 300 + 60 * i, "w": 600, "h": 40, "dpr": 1, "ref": i + 1, "name": name}
            return {"error": "not found"}
        if fn == "media":
            if not s.get("video"):
                return {"found": False}
            act = (json.loads("[" + args + "]") or ["status"])[0]
            if act in ("pause", "toggle"):
                self._paused = act == "pause" or not self._paused
            elif act == "play":
                self._paused = False
            return {"found": True, "playing": not self._paused, "position": 12, "duration": 10800, "volume": 100,
                    "muted": False, "speed": 1, "title": s["title"].removesuffix(" - YouTube")}
        if fn in ("focus", "insertText", "scroll", "jsClick"):
            if fn == "jsClick" and self._go:
                await self.show_url(self._go)
                self._go = None
            return {"ok": True, "scrollPercent": 0}
        return None


class _Display:
    """One connection to the display for the whole case: Xvfb clears its screen when its last client goes, so this
    stays connected, and the picture stays as the display's background (what shows, with no windows on it)."""

    def __init__(self) -> None:
        from Xlib import display
        self.d = display.Display()
        self.scr = self.d.screen()
        self.pixmap = None

    def show(self, img: Any) -> None:
        root, w, h = self.scr.root, self.scr.width_in_pixels, self.scr.height_in_pixels
        pm = root.create_pixmap(w, h, self.scr.root_depth)
        gc = pm.create_gc()
        pm.put_pil_image(gc, 0, 0, img.resize((w, h)) if img.size != (w, h) else img)
        gc.free()
        root.change_attributes(background_pixmap=pm)
        root.clear_area(0, 0, w, h)
        if self.pixmap is not None:
            self.pixmap.free()
        self.pixmap = pm
        self.d.sync()


_shown: list[_Display] = []


def _to_display(img: Any) -> None:
    if not _shown:
        _shown.append(_Display())
    _shown[0].show(img)
