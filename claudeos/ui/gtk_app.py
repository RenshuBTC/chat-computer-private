"""The ClaudeOS window: one full-screen GTK window with two layers.

* The conversation (a local web page with only your words and Claude's words).
* The content layer: websites, videos, documents and pages, shown by a WebKit browser engine
  with a caption strip over it. Sign-ins and cookies are kept in ~/ClaudeOS/web.

GTK runs on the main thread; ClaudeOS's logic runs on an asyncio thread. Everything that touches
the window is handed to the main thread with GLib.idle_add.
"""
from __future__ import annotations

import asyncio
import ctypes
import ctypes.util
import json
import logging
import os
import threading
import time
import urllib.parse
from collections import deque
from pathlib import Path
from typing import Any, Callable

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import Gdk, GLib, Gtk, WebKit2  # noqa: E402

from ..brain.permissions import is_private  # noqa: E402
from ..config import Paths, Settings  # noqa: E402
from ..screen import Screen  # noqa: E402
from .airtouch_layer import AirLayer  # noqa: E402
from .layout import Layout, compute, css_vars, screen_from_monitor  # noqa: E402
from .render import ASSETS, FONTS, NATIVE, asset  # noqa: E402

log = logging.getLogger("claudeos.ui")
WORLD = "claudeos"


def private_file_uri(paths: Paths, uri: str) -> bool:
    """Whether `uri` is one of ClaudeOS's private files (sign-ins, the engine's settings, the browser's logins), which
    must never be shown on screen: from there, read_screen would hand it to Claude."""
    if not (uri or "").lower().startswith("file:"):
        return False
    return is_private(paths, urllib.parse.unquote(urllib.parse.urlparse(uri).path) or "/")


MEDIA_KEYS = {"XF86AudioPlay", "XF86AudioPause", "XF86AudioPlayPause", "XF86AudioMedia", "XF86AudioNext",
              "XF86AudioPrev", "XF86AudioStop"}
# The captions over websites look like the conversation (theme.css): black on white in Atkinson Hyperlegible
# Next, your words in italics. register_fonts() makes ClaudeOS's own copy of the font known to GTK.
COLORS = {"bg": "#FFFFFF", "ink": "#000000"}

CSS_TEMPLATE = """
window, stack {{ background-color: {bg}; }}
.captions {{ background-color: {bg}; border: 1px solid {ink}; padding: {pad_v}px {pad_h}px; }}
.cap-you, .cap-ai {{ color: {ink}; font-family: "Atkinson Hyperlegible Next", sans-serif; font-size: {size}px; }}
.cap-you {{ font-style: italic; }}
"""
SET_VARS_JS = ("(function(){{var r=document.documentElement.style;r.setProperty('--co-size','{size}px');"
               "r.setProperty('--co-column','{column}px');}})()")
_fonts_registered: bool | None = None


def register_fonts(folder: Path = FONTS) -> bool:
    """Make ClaudeOS's fonts (assets/fonts) known to GTK in this process, for the captions. Pages load them
    themselves (theme.css). Without them the captions use the system's standard font. Call it before the
    window is built."""
    global _fonts_registered
    if _fonts_registered is None:
        try:
            lib = ctypes.util.find_library("fontconfig") or "libfontconfig.so.1"
            fc = ctypes.CDLL(lib)
            fc.FcConfigAppFontAddDir.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
            fc.FcConfigAppFontAddDir.restype = ctypes.c_int
            _fonts_registered = bool(fc.FcConfigAppFontAddDir(None, os.fsencode(folder)))
        except Exception:
            log.warning("ClaudeOS's fonts couldn't be given to GTK; captions use the standard font", exc_info=True)
            _fonts_registered = False
    return _fonts_registered


# WebKit draws pages with the graphics chip. Where the system offers none for drawing (a virtual
# machine, graphics the kernel can't drive yet) that crashes and leaves a blank screen; these make it
# draw with the processor instead. Renderer processes read them when they start.
CPU_RENDERING = {"WEBKIT_SKIA_ENABLE_CPU_RENDERING": "1", "WEBKIT_DISABLE_DMABUF_RENDERER": "1", "CLAUDEOS_GPU": "0"}


def use_cpu_rendering(environ: Any = os.environ) -> bool:
    """Draw pages with the processor from now on. True if that is a change."""
    changed = any(environ.get(k) != v for k, v in CPU_RENDERING.items())
    environ.update(CPU_RENDERING)
    return changed


def choose_rendering(environ: Any = os.environ, dri: str = "/dev/dri", nvidia: str = "/sys/module/nvidia_drm") -> str:
    """'gpu' when the system offers the graphics chip for drawing (a /dev/dri/renderD* device), else 'cpu'."""
    if environ.get("CLAUDEOS_GPU") == "0" or not list(Path(dri).glob("renderD*")):
        use_cpu_rendering(environ)
        return "cpu"
    if Path(nvidia).exists():
        # NVIDIA's own driver can show WebKit's newer (DMA-BUF) way of passing pictures as a blank window.
        environ.setdefault("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
    return "gpu"


class GtkScreen(Screen):
    def __init__(self, paths: Paths, settings: Settings, windowed: bool = False) -> None:
        self.paths = paths
        self.settings = settings
        self.loop: asyncio.AbstractEventLoop | None = None
        self.on_media_key: Callable[[str], None] = lambda k: None
        self.on_media_state: Callable[[bool], None] = lambda p: None
        self.on_typed: Callable[[str], None] = lambda t: None
        self.on_notice: Callable[[str], None] = lambda t: None
        self.mode = "chat"
        self.chat_ready = False
        self.chat_backlog: list[str] = []
        self.pending_ai: dict[str, str] = {}
        self.flush_scheduled = False
        self.load_waiters: list[asyncio.Future] = []
        self.load_error: str | None = None
        self.view_seq = 0
        self.typed = ""
        self.cap_ai_text = ""
        self.cap_tid: str | None = None
        self.cap_hide_id = 0
        self.windowed = windowed
        self.layout: Layout | None = None
        self._view_w = 0
        self.native_uris: set[str] = set()
        self._relayout_id = 0
        self._last_alloc = (0, 0)
        # What the conversation page shows, to rebuild it if its renderer stops: the conversation in
        # order, and the latest settings, layout and state.
        self._replay: deque[str] = deque(maxlen=2000)
        self._replay_last: dict[str, str] = {}
        self._reloading = False
        self._renderer_stops: list[float] = []
        self._bounds: tuple[int, int, int, int] | None = None
        self.on_bounds: Callable[[tuple[int, int, int, int]], None] | None = None
        self.on_keyboard: Callable[[tuple | None], None] | None = None      # the keyboard moved (AirTouch)
        self._build(windowed)

    def bind(self, loop: asyncio.AbstractEventLoop, *, on_media_key: Callable[[str], None],
             on_media_state: Callable[[bool], None], on_typed: Callable[[str], None]) -> None:
        self.loop = loop
        self.on_media_key = on_media_key
        self.on_media_state = on_media_state
        self.on_typed = on_typed

    # ================================================================ building the window
    def _build(self, windowed: bool) -> None:
        register_fonts()   # before any text is laid out
        self.css = Gtk.CssProvider()
        Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), self.css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

        self.win = Gtk.Window(title="ClaudeOS")
        self.win.set_decorated(False)
        if windowed:
            self.win.set_default_size(int(os.environ.get("CLAUDEOS_W", 1280)), int(os.environ.get("CLAUDEOS_H", 800)))
        else:
            # Fill the monitor even if no window manager honours "fullscreen".
            try:
                disp = Gdk.Display.get_default()
                mon = disp.get_primary_monitor() or disp.get_monitor(0)
                geo = mon.get_geometry()
                self.win.set_default_size(geo.width, geo.height)
                self.win.move(geo.x, geo.y)
            except Exception:
                pass
            self.win.fullscreen()
        self.win.connect("destroy", lambda *_: Gtk.main_quit())
        self.win.connect("key-press-event", self._on_key)

        self.stack = Gtk.Stack()
        self.stack.set_transition_type(Gtk.StackTransitionType.CROSSFADE)
        self.stack.set_transition_duration(220)
        # AirTouch's dots and keyboard, drawn over everything (it lets clicks through to what's under it).
        self.root = Gtk.Overlay()
        self.root.add(self.stack)
        self.pointer_area = Gtk.DrawingArea()
        self.air = AirLayer(self.pointer_area, lambda: self.layout.font_px if self.layout else 24.0)
        self.pointer_area.connect("draw", self.air.draw)
        self.root.add_overlay(self.pointer_area)
        self.root.set_overlay_pass_through(self.pointer_area, True)
        self.win.add(self.root)

        # --- conversation layer
        chat_ucm = WebKit2.UserContentManager()
        chat_ucm.connect("script-message-received::claudeos", self._on_chat_message)
        chat_ucm.register_script_message_handler("claudeos")
        chat_ctx = WebKit2.WebContext.new_ephemeral()
        self.chat = WebKit2.WebView(web_context=chat_ctx, user_content_manager=chat_ucm)
        self._quiet_view(self.chat)
        if os.environ.get("CLAUDEOS_GPU", "1") != "1":
            self.chat.get_settings().set_hardware_acceleration_policy(WebKit2.HardwareAccelerationPolicy.NEVER)
        self.chat.connect("web-process-terminated", self._on_renderer_stopped)
        # Not set_allow_file_access_from_file_urls: pages opened from files can still show other files (fonts,
        # pictures), but scripts can't read them, so a page can't read ClaudeOS's sign-in.
        self.chat.load_uri((ASSETS / "chat.html").as_uri())
        self.stack.add_named(self.chat, "chat")

        # --- content layer: a real browser with a persistent profile
        web = self.paths.web
        mgr = WebKit2.WebsiteDataManager(base_data_directory=str(web / "data"), base_cache_directory=str(web / "cache"))
        self.ctx = WebKit2.WebContext.new_with_website_data_manager(mgr)
        cm = self.ctx.get_cookie_manager()
        cm.set_persistent_storage(str(web / "cookies.sqlite"), WebKit2.CookiePersistentStorage.SQLITE)
        cm.set_accept_policy(WebKit2.CookieAcceptPolicy.NO_THIRD_PARTY)
        self.ctx.connect("download-started", self._on_download)
        ucm = WebKit2.UserContentManager()
        ucm.add_script(WebKit2.UserScript.new_for_world(
            asset("content.js"), WebKit2.UserContentInjectedFrames.TOP_FRAME,
            WebKit2.UserScriptInjectionTime.END, WORLD, None, None))
        ucm.connect("script-message-received::claudeosPage", self._on_page_message)
        ucm.register_script_message_handler_in_world("claudeosPage", WORLD)
        policies = WebKit2.WebsitePolicies(autoplay=WebKit2.AutoplayPolicy.ALLOW)
        self.content = WebKit2.WebView(web_context=self.ctx, user_content_manager=ucm, website_policies=policies)
        s = self.content.get_settings()
        s.set_enable_javascript(True)
        s.set_media_playback_requires_user_gesture(False)
        s.set_enable_mediasource(True)
        s.set_enable_webgl(True)
        s.set_enable_smooth_scrolling(True)
        s.set_enable_fullscreen(True)
        s.set_javascript_can_open_windows_automatically(False)
        s.set_enable_developer_extras(False)
        s.set_hardware_acceleration_policy(WebKit2.HardwareAccelerationPolicy.ALWAYS
                                           if os.environ.get("CLAUDEOS_GPU", "1") == "1" else WebKit2.HardwareAccelerationPolicy.NEVER)
        # Not set_allow_file_access_from_file_urls (see the conversation layer): a downloaded .html file shown with
        # show_file runs in this view.
        self._quiet_view(self.content)
        self.content.connect("load-changed", self._on_load_changed)
        self.content.connect("load-failed", self._on_load_failed)
        self.content.connect("create", self._on_create)
        self.content.connect("decide-policy", self._on_decide_policy)
        self.content.connect("permission-request", lambda v, req: (req.deny(), True)[1])
        self.content.connect("close", lambda v: self._show_chat())
        self.content.connect("web-process-terminated", self._on_renderer_stopped)

        overlay = Gtk.Overlay()
        overlay.add(self.content)
        self.cap_box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        self.cap_box.get_style_context().add_class("captions")
        self.cap_box.set_halign(Gtk.Align.FILL)      # width set by side margins (see _apply_layout)
        self.cap_box.set_valign(Gtk.Align.END)
        self.cap_you = Gtk.Label()
        self.cap_ai = Gtk.Label()
        for lbl, cls in ((self.cap_you, "cap-you"), (self.cap_ai, "cap-ai")):
            lbl.set_line_wrap(True)
            lbl.set_max_width_chars(1)
            lbl.set_xalign(0.0)
            lbl.get_style_context().add_class(cls)
            self.cap_box.pack_start(lbl, False, False, 0)
        overlay.add_overlay(self.cap_box)
        overlay.set_overlay_pass_through(self.cap_box, True)
        self.stack.add_named(overlay, "content")
        self.overlay = overlay

        self._apply_layout()
        self.win.show_all()
        self.cap_box.hide()
        self.stack.set_visible_child_name("chat")
        # Work the layout out again whenever the window or the monitor changes.
        self.win.connect("size-allocate", self._on_size_allocate)
        Gdk.Screen.get_default().connect("monitors-changed", lambda *_: self._schedule_relayout())

    def _load_css(self) -> None:
        rem = self.layout.font_px if self.layout else 16
        self.css.load_from_data(CSS_TEMPLATE.format(
            size=round(rem, 1), pad_v=round(rem * 0.5), pad_h=round(rem * 0.75), **COLORS).encode())

    # ================================================================ text size and line width
    def _monitor(self) -> Any:
        disp = Gdk.Display.get_default()
        gw = self.win.get_window() if hasattr(self, "win") else None
        mon = disp.get_monitor_at_window(gw) if gw is not None else None
        return mon or disp.get_primary_monitor() or disp.get_monitor(0)

    def _compute_layout(self) -> Layout:
        s = self.settings
        mon = self._monitor()
        geo = mon.get_geometry()
        vw, vh = self.win.get_allocated_width(), self.win.get_allocated_height()
        if vw <= 1 or vh <= 1:
            vw, vh = self.win.get_default_size()
            if vw <= 1 or vh <= 1 or not self.windowed:
                vw, vh = geo.width, geo.height
        self._view_w = vw
        scr = screen_from_monitor(vw, vh, geo.width, geo.height, mon.get_width_mm(), mon.get_height_mm(),
                                  float(mon.get_scale_factor() or 1), diagonal_in=s.screen_inches)
        return compute(scr, distance_cm=s.viewing_distance_cm, text_px=s.text_size,
                       text_scale=s.text_scale, web_zoom=s.web_zoom)

    def _apply_layout(self) -> None:
        self._note_bounds()
        self._replace_keyboard()
        lay = self._compute_layout()
        first = self.layout is None
        changed = first or lay.to_dict() != self.layout.to_dict()
        self.layout = lay
        self._load_css()
        if not changed:
            return
        self._chat_run(f"CO.layout({lay.font_px},{lay.column_px})", "layout")
        # Captions over websites and videos: the same column, the same size.
        side = max(0, round((self._view_w - lay.column_px) / 2))
        self.cap_box.set_margin_start(side)
        self.cap_box.set_margin_end(side)
        self.cap_box.set_margin_bottom(round(lay.font_px * 1.5))
        self._apply_content_layout()
        log.info("Layout: %s", lay.describe())

    def _apply_content_layout(self) -> None:
        """ClaudeOS's own pages get the computed size; websites get the computed zoom."""
        lay = self.layout
        if lay is None:
            return
        uri = self.content.get_uri() or ""
        if uri in self.native_uris:
            self.content.set_zoom_level(1.0)
            js = SET_VARS_JS.format(size=lay.font_px, column=lay.column_px)
            self.content.evaluate_javascript(js, -1, WORLD, None, None, None, None)
        else:
            self.content.set_zoom_level(lay.web_zoom)

    def _on_size_allocate(self, w: Gtk.Widget, alloc: Any) -> None:
        size = (alloc.width, alloc.height)
        if size != self._last_alloc:
            self._last_alloc = size
            self._schedule_relayout()

    def _schedule_relayout(self) -> None:
        if self._relayout_id:
            GLib.source_remove(self._relayout_id)

        def run() -> bool:
            self._relayout_id = 0
            try:
                self._apply_layout()
            except Exception:
                log.exception("Layout failed")
            return False
        self._relayout_id = GLib.timeout_add(150, run)

    # ================================================================ AirTouch: the dots and the keyboard
    def _note_bounds(self) -> None:
        """Where this screen is on the display, in its pixels (for AirTouch, which moves the display's pointer)."""
        try:
            mon = self._monitor()
            g, f = mon.get_geometry(), mon.get_scale_factor() or 1
            bounds = (g.x * f, g.y * f, g.width * f, g.height * f)
        except Exception:
            return
        if bounds != self._bounds:
            self._bounds = bounds
            if self.on_bounds:
                try:
                    self.on_bounds(bounds)
                except Exception:
                    log.exception("on_bounds")

    def screen_bounds(self) -> tuple[int, int, int, int] | None:
        return self._bounds

    def _to_window(self, x: float, y: float) -> tuple[float, float]:
        """Display pixels (AirTouch's) to this window's."""
        gw = self.win.get_window()
        if gw is None:
            return x, y
        origin = gw.get_origin()
        f = gw.get_scale_factor() or 1
        return x / f - origin[-2], y / f - origin[-1]

    def _to_display(self, x: float, y: float) -> tuple[float, float]:
        gw = self.win.get_window()
        if gw is None:
            return x, y
        origin = gw.get_origin()
        f = gw.get_scale_factor() or 1
        return (x + origin[-2]) * f, (y + origin[-1]) * f

    def dots(self, dots: list[tuple[float, float, str]]) -> None:
        self._ui(self._dots_now, list(dots))

    def _dots_now(self, dots: list[tuple[float, float, str]]) -> None:
        self.air.set_dots([self._to_window(x, y) + (kind,) for x, y, kind in dots])

    async def show_keyboard(self, show: bool) -> tuple | None:
        return await self._ui_call(self._show_keyboard_now, show)

    def _show_keyboard_now(self, show: bool) -> tuple | None:
        if not show:
            if self.air.up:
                self.air.hide_keyboard()
                self.stack.set_margin_bottom(0)
            return None
        w, h = self.win.get_allocated_width(), self.win.get_allocated_height()
        if w <= 1 or h <= 1:
            w, h = self.win.get_default_size()
        x, y, aw, ah = self.air.show_keyboard(w, h)
        # Pages get the room above it, the way a tablet's do, so what's being typed into stays in view (the
        # conversation keeps its newest words at the bottom).
        self.stack.set_margin_bottom(int(self.air.panel[3]))
        f = (self.win.get_window().get_scale_factor() if self.win.get_window() else 1) or 1
        dx, dy = self._to_display(x, y)
        return self.air.spec((dx, dy, aw * f, ah * f))

    async def keyboard_press(self, x: float, y: float) -> tuple | None:
        return await self._ui_call(self._keyboard_press_now, x, y)

    def _keyboard_press_now(self, x: float, y: float) -> tuple | None:
        return self.air.press(*self._to_window(x, y))

    def _replace_keyboard(self) -> None:
        """The window changed size with the keyboard up: put it along the new bottom, and tell AirTouch."""
        if self.air.up:
            spec = self._show_keyboard_now(True)
            if self.on_keyboard:
                try:
                    self.on_keyboard(spec)
                except Exception:
                    log.exception("on_keyboard")

    def layout_info(self) -> dict[str, Any]:
        lay = self.layout
        if lay is None:
            return {}
        d = lay.to_dict()
        d["summary"] = lay.describe()
        return d

    @staticmethod
    def _quiet_view(view: WebKit2.WebView) -> None:
        view.connect("context-menu", lambda *a: True)

    # ================================================================ thread helpers
    def _ui(self, fn: Callable, *args: Any) -> None:
        def run() -> bool:
            try:
                fn(*args)
            except Exception:
                log.exception("UI call failed")
            return False
        GLib.idle_add(run)

    async def _ui_call(self, fn: Callable, *args: Any) -> Any:
        assert self.loop is not None
        fut = self.loop.create_future()

        def run() -> bool:
            try:
                res = fn(*args)
                self.loop.call_soon_threadsafe(lambda: fut.done() or fut.set_result(res))
            except Exception as exc:
                # Bind it now: Python unbinds `exc` when the except block ends, before the loop runs this.
                self.loop.call_soon_threadsafe(lambda e=exc: fut.done() or fut.set_exception(e))
            return False
        GLib.idle_add(run)
        return await fut

    def _chat_js(self, script: str, kind: str = "log") -> None:
        self._ui(self._chat_run, script, kind)

    def _chat_run(self, script: str, kind: str = "log") -> None:
        """Run a script on the conversation page (UI thread). kind says how to rebuild the page later:
        "log" is part of the conversation, "transient" isn't kept, anything else keeps only the latest."""
        if kind == "log":
            self._replay.append(script)
        elif kind != "transient":
            self._replay_last[kind] = script
        if self.chat_ready:
            self.chat.evaluate_javascript(script, -1, None, None, None, None, None)
        else:
            self.chat_backlog.append(script)

    def _on_chat_message(self, ucm: Any, result: Any) -> None:
        self.chat_ready = True
        if self._reloading:
            # The page was loaded again after its renderer stopped: put back what it showed.
            self._reloading = False
            self.chat_backlog.clear()
            last = self._replay_last
            scripts = [last[k] for k in ("settings", "layout") if k in last] + list(self._replay)
            scripts += [last["state"]] if "state" in last else []
        else:
            scripts, self.chat_backlog = self.chat_backlog, []
        for s in scripts:
            self.chat.evaluate_javascript(s, -1, None, None, None, None, None)

    def _on_renderer_stopped(self, view: WebKit2.WebView, reason: Any) -> None:
        """A page's renderer process stopped (it crashed or ran out of memory): load the page again,
        drawing with the processor from now on in case the graphics chip was the problem."""
        if reason == WebKit2.WebProcessTerminationReason.TERMINATED_BY_API:
            return
        now = time.monotonic()
        self._renderer_stops = [t for t in self._renderer_stops if now - t < 120] + [now]
        switched = use_cpu_rendering()
        for v in (self.chat, self.content):
            v.get_settings().set_hardware_acceleration_policy(WebKit2.HardwareAccelerationPolicy.NEVER)
        log.error("The %s page's renderer stopped (%s)%s", "conversation" if view is self.chat else "content",
                  getattr(reason, "value_nick", reason), "; drawing with the processor from now on" if switched else "")
        if len(self._renderer_stops) > 5:
            log.error("Pages keep failing to draw; restarting ClaudeOS")
            Gtk.main_quit()   # the session starts ClaudeOS again
            return
        if view is self.chat:
            self.chat_ready = False
            self._reloading = True
            self.chat_backlog.clear()
            GLib.timeout_add(300, lambda: self.chat.load_uri((ASSETS / "chat.html").as_uri()) and False)
        else:
            GLib.timeout_add(300, lambda: self.content.reload() and False)

    # ================================================================ content layer
    def _show_content(self) -> None:
        self.mode = "content"
        self.stack.set_visible_child_name("content")
        self.content.grab_focus()

    def _show_chat(self) -> None:
        self.mode = "chat"
        self.stack.set_visible_child_name("chat")
        self.cap_box.hide()

    def _info(self) -> dict[str, Any]:
        return {"mode": self.mode, "url": self.content.get_uri(), "title": self.content.get_title()}

    def _resolve_loads(self, ok: bool, error: str | None = None) -> None:
        info = self._info()
        info["ok"] = ok
        if error:
            info["error"] = error
        waiters, self.load_waiters = self.load_waiters, []
        for fut in waiters:
            self.loop.call_soon_threadsafe(lambda f=fut: f.done() or f.set_result(dict(info)))

    def _on_load_changed(self, view: WebKit2.WebView, event: WebKit2.LoadEvent) -> None:
        if event == WebKit2.LoadEvent.COMMITTED:
            # Zoom websites for this screen; ClaudeOS's own pages are already sized.
            if self.layout is not None:
                native = (view.get_uri() or "") in self.native_uris
                view.set_zoom_level(1.0 if native else self.layout.web_zoom)
        if event == WebKit2.LoadEvent.FINISHED:
            if self.layout is not None and (view.get_uri() or "") in self.native_uris:
                self._apply_content_layout()   # in case the layout changed since the page was written
            err = self.load_error
            self.load_error = None
            # The page title can arrive a moment after the load finishes.
            tries = {"n": 0}

            def settle() -> bool:
                tries["n"] += 1
                if not self.content.get_title() and tries["n"] < 6 and err is None:
                    return True
                self._resolve_loads(err is None, err)
                return False
            if settle():
                GLib.timeout_add(150, settle)
        elif event == WebKit2.LoadEvent.STARTED:
            self.load_error = None

    def _on_load_failed(self, view: WebKit2.WebView, event: Any, uri: str, error: Any) -> bool:
        msg = getattr(error, "message", str(error))
        if "interrupted" in msg.lower() or "cancel" in msg.lower():
            return False
        self.load_error = msg
        return False

    def _on_create(self, view: WebKit2.WebView, action: Any) -> None:
        req = action.get_request()
        if req and req.get_uri():
            view.load_uri(req.get_uri())
        return None

    def _on_decide_policy(self, view: WebKit2.WebView, decision: Any, kind: Any) -> bool:
        if kind in (WebKit2.PolicyDecisionType.NAVIGATION_ACTION, WebKit2.PolicyDecisionType.NEW_WINDOW_ACTION):
            uri = decision.get_navigation_action().get_request().get_uri() or ""
            if private_file_uri(self.paths, uri):
                log.warning("not showing a private file on screen")
                decision.ignore()
                return True
        if kind == WebKit2.PolicyDecisionType.RESPONSE and not decision.is_mime_type_supported():
            decision.download()
            return True
        return False

    def _on_download(self, ctx: Any, download: Any) -> None:
        def decide(dl: Any, suggested: str) -> bool:
            dest = Path.home() / "Downloads"
            dest.mkdir(parents=True, exist_ok=True)
            name = os.path.basename(suggested or "download")
            target = dest / name
            i = 1
            while target.exists():
                target = dest / f"{Path(name).stem} ({i}){Path(name).suffix}"
                i += 1
            dl.set_destination(target.as_uri())
            self.notice(f"Downloading {target.name} to Downloads.")
            return True
        download.connect("decide-destination", decide)
        download.connect("finished", lambda dl: self.notice("Download finished."))

    def _on_page_message(self, ucm: Any, result: Any) -> None:
        try:
            val = result.get_js_value() if hasattr(result, "get_js_value") else result
            msg = json.loads(val.to_string())
        except Exception:
            return
        if msg.get("type") == "media":
            playing = bool(msg.get("playing"))
            self.media_playing = playing
            if self.loop:
                self.loop.call_soon_threadsafe(self.on_media_state, playing)

    async def show_url(self, url: str) -> dict[str, Any]:
        assert self.loop is not None
        fut = self.loop.create_future()

        def go() -> None:
            self.load_waiters.append(fut)
            self.load_error = None
            self.content.load_uri(url)
            self._show_content()
        await self._ui_call(go)
        try:
            return await asyncio.wait_for(asyncio.shield(fut), 20)
        except asyncio.TimeoutError:
            info = await self._ui_call(self._info)
            info.update(ok=True, note="still loading")
            return info

    async def show_html(self, html: str, title: str, base_dir: str | None = None) -> dict[str, Any]:
        self.view_seq = (self.view_seq + 1) % 4
        path = self.paths.pages / f"_view{self.view_seq}.html"
        path.parent.mkdir(parents=True, exist_ok=True)
        native = NATIVE in html[:800]
        lay = self.layout
        if native and lay is not None:
            html = html.replace("<head>", f'<head><style id="co-layout">{css_vars(lay)}</style>', 1)
        path.write_text(html, encoding="utf-8")
        uri = path.as_uri()
        if native:
            self.native_uris.add(uri)
        else:
            self.native_uris.discard(uri)
        return await self.show_url(uri)

    async def hide_content(self) -> None:
        await self._ui_call(self._show_chat)

    async def content_info(self) -> dict[str, Any]:
        return await self._ui_call(self._info)

    async def eval_js(self, script: str) -> Any:
        assert self.loop is not None
        fut = self.loop.create_future()

        def done(view: WebKit2.WebView, res: Any, *_: Any) -> None:
            try:
                val = view.evaluate_javascript_finish(res)
                text = val.to_json(0) if val is not None and not val.is_undefined() else None
                out = json.loads(text) if text else None
                self.loop.call_soon_threadsafe(lambda: fut.done() or fut.set_result(out))
            except Exception as exc:
                err = str(getattr(exc, "message", exc))
                self.loop.call_soon_threadsafe(lambda: fut.done() or fut.set_result({"error": err}))

        self._ui(lambda: self.content.evaluate_javascript(script, -1, WORLD, None, None, done, None))
        try:
            return await asyncio.wait_for(fut, 12)
        except asyncio.TimeoutError:
            return {"error": "The page didn't respond."}

    async def navigate(self, action: str) -> dict[str, Any]:
        assert self.loop is not None
        fut = self.loop.create_future()

        def go() -> bool:
            v = self.content
            if action == "back" and v.can_go_back():
                self.load_waiters.append(fut)
                v.go_back()
                return True
            if action == "forward" and v.can_go_forward():
                self.load_waiters.append(fut)
                v.go_forward()
                return True
            if action == "reload":
                self.load_waiters.append(fut)
                v.reload()
                return True
            if action == "stop":
                v.stop_loading()
            return False
        waiting = await self._ui_call(go)
        if waiting:
            try:
                return await asyncio.wait_for(asyncio.shield(fut), 15)
            except asyncio.TimeoutError:
                pass
        return await self._ui_call(self._info)

    async def focus_content(self) -> None:
        await self._ui_call(lambda: self.content.grab_focus() if self.mode == "content" else None)

    async def content_origin(self) -> tuple[int, int, float]:
        def calc() -> tuple[int, int, float]:
            gw = self.content.get_window()
            ox, oy = (0, 0)
            if gw is not None:
                _, ox, oy = gw.get_origin()
            sf = float(self.content.get_scale_factor() or 1)
            return int(ox * sf), int(oy * sf), sf
        return await self._ui_call(calc)

    # ================================================================ conversation layer
    def user_text(self, msg_id: str, text: str, final: bool) -> None:
        self._chat_js(f"CO.user({json.dumps(msg_id)},{json.dumps(text)},{json.dumps(final)})")
        self.typed = ""
        self._caption(you=text, ai="")

    def partial(self, text: str) -> None:
        self._chat_js(f"CO.partial({json.dumps(text)})", "transient")
        if text:
            self._caption(you=text)

    def ai_delta(self, msg_id: str, text: str) -> None:
        def add() -> None:
            self.pending_ai[msg_id] = self.pending_ai.get(msg_id, "") + text
            if self.cap_tid != msg_id:
                self.cap_tid = msg_id
                self.cap_ai_text = ""
            self.cap_ai_text += text
            if not self.flush_scheduled:
                self.flush_scheduled = True
                GLib.timeout_add(40, self._flush_ai)
        self._ui(add)

    def _flush_ai(self) -> bool:
        """The words that streamed in during the last 40 ms, onto the page in one go; and the captions over a
        website, which only show there (once per flush, not once for every few letters)."""
        self.flush_scheduled = False
        pending, self.pending_ai = self.pending_ai, {}
        for mid, txt in pending.items():
            self._chat_run(f"CO.ai({json.dumps(mid)},{json.dumps(txt)})" if txt else f"CO.aiStart({json.dumps(mid)})")
        if self.mode == "content" and any(pending.values()) and self.cap_ai_text:
            self._caption_now(ai=_tail(self.cap_ai_text))
        return False

    def ai_done(self, msg_id: str) -> None:
        def run() -> None:
            self._flush_ai()
            self._chat_run(f"CO.aiDone({json.dumps(msg_id)})")
        self._ui(run)

    def activity(self, msg_id: str, text: str) -> None:
        def run() -> None:
            self._flush_ai()
            self._chat_run(f"CO.activity({json.dumps(msg_id)},{json.dumps(text)})")
        self._ui(run)

    def notice(self, text: str, kind: str = "info") -> None:
        self._chat_js(f"CO.notice({json.dumps(text)},{json.dumps(kind)})")
        self._caption(ai=text)

    def state(self, name: str) -> None:
        self._chat_js(f"CO.state({json.dumps(name)})", "state")
        if name == "sleeping":
            self._caption(you="", ai="Sleeping. Say “Claude” or press an AirPod.")
        elif name in ("listening", "resting"):
            # Claude has finished: let the captions go soon, so they cover the page as
            # little as possible (the words stay in the conversation).
            def soon() -> None:
                if self.cap_hide_id:
                    GLib.source_remove(self.cap_hide_id)
                self.cap_hide_id = GLib.timeout_add(2500, self._hide_captions)
            self._ui(soon)

    def history(self, items: list[dict[str, Any]]) -> None:
        self._chat_js(f"CO.history({json.dumps(items)})")

    def apply_settings(self, text_size: int, user_name: str) -> None:
        # Size settings (text_size, text_scale, viewing_distance_cm, screen_inches, web_zoom) are
        # read from self.settings, the same object the controller changes.
        self._chat_js(f"CO.settings({json.dumps(user_name)})", "settings")
        self._ui(self._apply_layout)

    # captions over content
    def _caption(self, you: str | None = None, ai: str | None = None) -> None:
        self._ui(self._caption_now, you, ai)

    def _caption_now(self, you: str | None = None, ai: str | None = None) -> None:
        if you is not None:
            self.cap_you.set_text(you)
            self.cap_you.set_visible(bool(you))
        if ai is not None:
            self.cap_ai.set_text(ai)
            self.cap_ai.set_visible(bool(ai))
        show = self.mode == "content" and (self.cap_you.get_text() or self.cap_ai.get_text())
        self.cap_box.set_visible(bool(show))
        if self.cap_hide_id:
            GLib.source_remove(self.cap_hide_id)
        self.cap_hide_id = GLib.timeout_add_seconds(8, self._hide_captions)

    def _hide_captions(self) -> bool:
        self.cap_hide_id = 0
        self.cap_box.hide()
        return False

    # ================================================================ keys
    def _on_key(self, w: Gtk.Widget, ev: Gdk.EventKey) -> bool:
        name = Gdk.keyval_name(ev.keyval) or ""
        if name in MEDIA_KEYS:
            if self.loop:
                self.on_media_key(name)
            return True
        ctrl = bool(ev.state & Gdk.ModifierType.CONTROL_MASK)
        if ctrl and (ev.state & Gdk.ModifierType.SHIFT_MASK) and name in ("Q", "q"):
            Gtk.main_quit()
            return True
        if self.mode != "chat":
            return False  # keys go to the website (forms, sign-ins)
        # A keyboard is optional: typing on the conversation screen sends a message.
        if name in ("Return", "KP_Enter"):
            text, self.typed = self.typed.strip(), ""
            self._chat_js("CO.partial('')", "transient")
            if text and self.loop:
                self.loop.call_soon_threadsafe(self.on_typed, text)
            return True
        if name == "BackSpace":
            self.typed = self.typed[:-1]
        elif name == "Escape":
            self.typed = ""
        else:
            ch = Gdk.keyval_to_unicode(ev.keyval)
            if ch and not ctrl and chr(ch).isprintable():
                self.typed += chr(ch)
            else:
                return False
        self._chat_js(f"CO.partial({json.dumps(self.typed)})", "transient")
        return True


def _tail(text: str, n: int = 170) -> str:
    t = " ".join(text.split())
    if len(t) <= n:
        return t
    cut = t[-n:]
    sp = cut.find(" ")
    return "…" + (cut[sp + 1:] if 0 <= sp < 30 else cut)


def run_app(paths: Paths, settings: Settings, controller_factory: Callable[[GtkScreen], Any], windowed: bool = False) -> int:
    """Start GTK on this thread and ClaudeOS's logic on a background asyncio loop. The window comes first: the
    controller is made on the loop's thread once GTK is running, because loading its modules (the Agent SDK)
    takes most of a second, and the screen shouldn't stay black meanwhile."""
    if choose_rendering() == "cpu":
        log.info("No graphics chip available for drawing pages; drawing them with the processor")
    screen = GtkScreen(paths, settings, windowed=windowed)
    loop = asyncio.new_event_loop()
    t = threading.Thread(target=loop.run_forever, name="claudeos-loop", daemon=True)
    t.start()
    screen.bind(loop, on_media_key=lambda k: None, on_media_state=lambda p: None, on_typed=lambda text: None)
    holder: dict[str, Any] = {}

    async def boot() -> None:
        controller = holder["controller"] = controller_factory(screen)
        screen.bind(loop, on_media_key=controller.media_key, on_media_state=controller.media_state,
                    on_typed=lambda text: controller._spawn(controller.handle(text)))
        await controller.start()

    def started(f: Any) -> None:
        exc = f.exception()
        if exc:
            log.error("ClaudeOS failed to start: %s", exc, exc_info=exc)
            holder["failed"] = True
            GLib.idle_add(Gtk.main_quit)
    # Once GTK runs, so the window is drawn while the controller loads.
    GLib.idle_add(lambda: asyncio.run_coroutine_threadsafe(boot(), loop).add_done_callback(started) and False)
    try:
        Gtk.main()
    finally:
        controller = holder.get("controller")
        if controller is not None:
            try:
                asyncio.run_coroutine_threadsafe(controller.stop(), loop).result(10)
            except Exception:
                pass
        loop.call_soon_threadsafe(loop.stop)
    if holder.get("failed"):
        return 1      # a crash to the session: after a few, it shows what went wrong instead of retrying forever
    return (controller.exit_code if controller is not None else None) or 0
