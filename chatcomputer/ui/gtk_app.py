"""The Chat Computer window: one full-screen GTK window with two layers.

* The conversation (a local web page with only your words and Chat Computer's).
* The content layer: websites, videos, documents and pages, shown by a WebKit browser engine
  with a caption strip over it. Sign-ins and cookies are kept in ~/ChatComputer/web.

GTK runs on the main thread; Chat Computer's logic runs on an asyncio thread. Everything that touches
the window is handed to the main thread with GLib.idle_add.
"""
from __future__ import annotations

import asyncio
import contextlib
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

import cairo
import gi

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import Gdk, GLib, Gtk, WebKit2  # noqa: E402

from ..brain.permissions import is_private  # noqa: E402
from ..config import Paths, Settings  # noqa: E402
from ..screen import Screen  # noqa: E402
from .keyboard_layer import KeyboardLayer  # noqa: E402
from .layout import Layout, compute, css_vars, screen_from_monitor  # noqa: E402
from .touch import HOLD, KEY, PASS, RELEASE, TouchGestures  # noqa: E402
from .render import ASSETS, FONTS, NATIVE, asset  # noqa: E402

log = logging.getLogger("chatcomputer.ui")
WORLD = "chatcomputer"
# Keys in the message box that scroll what's open (chat.js "scroll_open"): which way, and how many screens.
SCROLL_OPEN = {"page_up": ("up", 1), "page_down": ("down", 1), "up": ("up", 0.2), "down": ("down", 0.2),
               "top": ("top", 1), "bottom": ("bottom", 1)}
SOURCE_WORLD = "chatcomputer-sources"  # where sources.js reads WhatsApp's and the social sites' pages
SAFARI_UA = ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) "
             "Version/18.5 Safari/605.1.15")


def private_file_uri(paths: Paths, uri: str) -> bool:
    """Whether `uri` is one of Chat Computer's private files (sign-ins, the engine's settings, the browser's logins), which
    must never be shown on screen: from there, read_screen would hand it to the model."""
    if not (uri or "").lower().startswith("file:"):
        return False
    return is_private(paths, urllib.parse.unquote(urllib.parse.urlparse(uri).path) or "/")


MEDIA_KEYS = {"XF86AudioPlay", "XF86AudioPause", "XF86AudioPlayPause", "XF86AudioMedia", "XF86AudioNext",
              "XF86AudioPrev", "XF86AudioStop"}
# The window behind the pages is the conversation's dark (theme.css). register_fonts() makes Chat Computer's own copy
# of its font known to GTK, for the onscreen keyboard.
COLORS = {"bg": "#191919", "ink": "#F0F0F0"}

CSS_TEMPLATE = """
window {{ background-color: {bg}; }}
"""
SET_VARS_JS = ("(function(){{var r=document.documentElement.style;r.setProperty('--co-size','{size}px');"
               "r.setProperty('--co-column','{column}px');}})()")
_fonts_registered: bool | None = None


def register_fonts(folder: Path = FONTS) -> bool:
    """Make Chat Computer's fonts (assets/fonts) known to GTK in this process, for the onscreen keyboard. Pages load them
    themselves (theme.css). Without them the keyboard uses the system's standard font. Call it before the
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
            log.warning("Chat Computer's fonts couldn't be given to GTK; the onscreen keyboard uses the standard font",
                        exc_info=True)
            _fonts_registered = False
    return _fonts_registered


# WebKit draws pages with the graphics chip. Where the system offers none for drawing (a virtual
# machine, graphics the kernel can't drive yet) that crashes and leaves a blank screen; these make it
# draw with the processor instead. Renderer processes read them when they start.
CPU_RENDERING = {"WEBKIT_SKIA_ENABLE_CPU_RENDERING": "1", "WEBKIT_DISABLE_DMABUF_RENDERER": "1", "CHATCOMPUTER_GPU": "0"}
SPEED_TICK_S = 0.25          # how often the window's thread checks it isn't kept busy (see _watch_speed)
# The touchscreen (ui/touch.py): the iPad's editing gestures, as WebKit's editing commands; and how long after a
# key typed on a real keyboard a touch on a box to type in doesn't bring up the onscreen one (as on an iPad with a
# keyboard attached).
EDIT_COMMANDS = {"copy": WebKit2.EDITING_COMMAND_COPY, "cut": WebKit2.EDITING_COMMAND_CUT,
                 "paste": WebKit2.EDITING_COMMAND_PASTE, "undo": WebKit2.EDITING_COMMAND_UNDO,
                 "redo": WebKit2.EDITING_COMMAND_REDO}
REAL_KEYBOARD_S = 5 * 60
TOUCH_EVENTS = {Gdk.EventType.TOUCH_BEGIN, Gdk.EventType.TOUCH_UPDATE, Gdk.EventType.TOUCH_END,
                Gdk.EventType.TOUCH_CANCEL}
DONE_SHOWN = 300             # Actions: how many the page holds (the rest are in done.jsonl)
MESSAGES_SHOWN = 400         # Messages: how many the page holds (the rest are in messages.db)
# What the conversation page tells the controller (Controller.page_event).
PAGE_EVENTS = {"pin", "revert", "open_message", "add_account", "account", "account_cancel", "panels",
               "new_chat", "open_chat",         # Chats: + pressed, or a chat clicked
               "search", "open_file",           # "/" in the message box: searching this computer, opening a file
               "text_scale",                    # two fingers pinched on the conversation: its text bigger or smaller
               "unlock",                        # the PIN typed on the lock screen
               "speak", "feedback",             # under a message: read it aloud; good or bad (feedback.jsonl)
               "mic_toggle"}                    # the microphone button under the message box: listen, or stop


def with_files(text: str, paths: list[str]) -> str:
    """A typed message with the files added to it (+): each named on a line of its own after the words, so the model
    knows where they are and reads them."""
    if not paths:
        return text
    lines = "\n".join(f"[Added: {p}]" for p in paths)
    if not text:
        text = "Here's the file I added." if len(paths) == 1 else "Here are the files I added."
    return text + "\n\n" + lines


def use_cpu_rendering(environ: Any = os.environ) -> bool:
    """Draw pages with the processor from now on. True if that is a change."""
    changed = any(environ.get(k) != v for k, v in CPU_RENDERING.items())
    environ.update(CPU_RENDERING)
    return changed


def choose_rendering(environ: Any = os.environ, dri: str = "/dev/dri", nvidia: str = "/sys/module/nvidia_drm") -> str:
    """'gpu' when the system offers the graphics chip for drawing (a /dev/dri/renderD* device), else 'cpu'."""
    if environ.get("CHATCOMPUTER_GPU") == "0" or not list(Path(dri).glob("renderD*")):
        use_cpu_rendering(environ)
        return "cpu"
    if Path(nvidia).exists():
        # NVIDIA's own driver can show WebKit's newer (DMA-BUF) way of passing pictures as a blank window.
        environ.setdefault("WEBKIT_DISABLE_DMABUF_RENDERER", "1")
    return "gpu"


def graphics_report(drm: str = "/sys/class/drm") -> str:
    """The graphics chips and their drivers, for the log: "card1: amdgpu (1002:164e)"."""
    out = []
    for card in sorted(Path(drm).glob("card[0-9]*")):
        if "-" in card.name:
            continue
        dev = card / "device"
        try:
            driver = os.path.basename(os.readlink(dev / "driver"))
        except OSError:
            driver = "no driver"
        try:
            ids = (dev / "vendor").read_text().strip()[2:] + ":" + (dev / "device").read_text().strip()[2:]
        except OSError:
            ids = "?"
        out.append(f"{card.name}: {driver} ({ids})")
    return ", ".join(out) or "no graphics chips found"


def displays_report(drm: str = "/sys/class/drm") -> str:
    """The connected screens and their largest mode, for the log: "card1-DP-1 5120x2880"."""
    out = []
    for conn in sorted(Path(drm).glob("card[0-9]*-*")):
        try:
            if (conn / "status").read_text().strip() != "connected":
                continue
            modes = (conn / "modes").read_text().split()
        except OSError:
            continue
        out.append(f"{conn.name} {modes[0] if modes else '?'}")
    return ", ".join(out) or "none found"


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
        self._done_entries: list[dict[str, Any]] = []     # Actions, as the page shows it (to rebuild the page)
        self._msg_items: dict[str, dict[str, Any]] = {}   # Messages, likewise
        self._msg_accounts: list[dict[str, Any]] = []
        # Where the page leaves room for what's open (x, y, width, height in the window), as the conversation page
        # last said; whether a video is full screen.
        self._hole: tuple[int, int, int, int] | None = None
        self._fullscreen = False
        self._locked = False            # the lock screen is up: nothing else comes on the screen
        # WhatsApp and the social sites Messages reads, each in a browser view of its own (chatcomputer/messages/web.py),
        # kept in a window that's never on the screen until one is shown in the middle.
        self.sources: dict[str, Any] = {}
        self._src_windows: dict[str, Any] = {}
        self._src_ctx: Any = None
        self.shown: Any = None          # the view in the middle (the content view, or a source's)
        self.on_source: Callable[[str, dict[str, Any]], None] = lambda provider, msg: None
        self._redirects: dict[str, Callable[[str], None]] = {}   # address prefix -> what to do instead of loading
        self.on_closed: Callable[[], None] = lambda: None     # what was open was closed (its x, or Home)
        self.on_page_event: Callable[[dict[str, Any]], None] = lambda msg: None   # see PAGE_EVENTS
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
        self.on_keyboard: Callable[[tuple | None], None] | None = None      # the keyboard moved, came or went
        # The touchscreen: the system's gestures (ui/touch.py) and the onscreen keyboard for a finger.
        self.touch = TouchGestures(self._touch_gesture)
        self._held: dict[Any, list[Any]] = {}      # touches held back (maybe the system's): their events so far
        self._given: dict[Any, Any] = {}           # each touch's last event the page had (to cancel it there)
        self._real_key_at = -1e9                   # when a key on a real keyboard was last typed
        self._deliver: Callable[[Any], None] = Gtk.main_do_event   # gives GTK (and so the pages) an event
        self.on_keyboard_action: Callable[[tuple], None] | None = None   # a key tapped on it: type it (the controller)
        now = time.monotonic()
        self._speed: dict[str, Any] = {"keys": [], "late": 0, "worst": 0.0, "since": now, "tick": now}
        self._build(windowed)
        GLib.timeout_add(int(SPEED_TICK_S * 1000), self._watch_speed)

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

        self.win = Gtk.Window(title="Chat Computer")
        self.win.set_decorated(False)
        if windowed:
            self.win.set_default_size(int(os.environ.get("CHATCOMPUTER_W", 1280)), int(os.environ.get("CHATCOMPUTER_H", 800)))
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
        # Touches where no page is (the onscreen keyboard's place) come as touches too, not as a mouse's clicks.
        self.win.add_events(Gdk.EventMask.TOUCH_MASK)

        # The conversation page fills the window: the sidebars, the conversation and the message box. What's open (a
        # website, a video, a document) is a second view laid over it, in the conversation's place between the
        # sidebars, above the message box, in the frame the page draws for it (see _place). The onscreen keyboard is
        # drawn over everything.
        self.root = Gtk.Overlay()
        self.win.add(self.root)

        # --- conversation layer
        chat_ucm = WebKit2.UserContentManager()
        chat_ucm.connect("script-message-received::chatcomputer", self._on_chat_message)
        chat_ucm.register_script_message_handler("chatcomputer")
        chat_ctx = WebKit2.WebContext.new_ephemeral()
        self.chat = WebKit2.WebView(web_context=chat_ctx, user_content_manager=chat_ucm)
        self._quiet_view(self.chat)
        if os.environ.get("CHATCOMPUTER_GPU", "1") != "1":
            self.chat.get_settings().set_hardware_acceleration_policy(WebKit2.HardwareAccelerationPolicy.NEVER)
        self.chat.connect("web-process-terminated", self._on_renderer_stopped)
        # Not set_allow_file_access_from_file_urls: pages opened from files can still show other files (fonts,
        # pictures), but scripts can't read them, so a page can't read Chat Computer's sign-in.
        self.chat.load_uri((ASSETS / "chat.html").as_uri())
        self.root.add(self.chat)

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
        ucm.connect("script-message-received::chatcomputerPage", self._on_page_message)
        ucm.register_script_message_handler_in_world("chatcomputerPage", WORLD)
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
                                           if os.environ.get("CHATCOMPUTER_GPU", "1") == "1" else WebKit2.HardwareAccelerationPolicy.NEVER)
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
        # A video made full screen covers everything, as in any browser; Escape brings it back into its frame.
        self.content.connect("enter-fullscreen", lambda v: self._fullscreen_changed(True))
        self.content.connect("leave-fullscreen", lambda v: self._fullscreen_changed(False))
        self.content.connect("notify::title", lambda v, p: self._retitle(v))
        self._add_in_hole(self.content)

        self.keyboard_area = Gtk.DrawingArea()
        self.keyboard_layer = KeyboardLayer(self.keyboard_area, lambda: self.layout.font_px if self.layout else 16.0)
        self.keyboard_area.connect("draw", self.keyboard_layer.draw)
        self.root.add_overlay(self.keyboard_area)
        self.root.set_overlay_pass_through(self.keyboard_area, True)
        # "Pass through" alone isn't enough: the layer has a window of its own, which took every click (a mouse's)
        # before the conversation or the website under it saw it (GTK 3.24). With no area
        # that takes input, they all go through.
        self.keyboard_area.connect("realize", lambda w: w.input_shape_combine_region(cairo.Region()))

        self._apply_layout()
        self.win.show_all()
        self.chat.grab_focus()          # typing goes into the conversation's message box
        # Every event passes here first (and then on to GTK): the touchscreen's system gestures, and which keys come
        # from a real keyboard.
        Gdk.event_handler_set(self._gdk_event)
        # Work the layout out again whenever the window or the monitor changes.
        self.win.connect("size-allocate", self._on_size_allocate)
        Gdk.Screen.get_default().connect("monitors-changed", lambda *_: self._schedule_relayout())

    def _load_css(self) -> None:
        self.css.load_from_data(CSS_TEMPLATE.format(**COLORS).encode())

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
        self._replace_keyboard()
        lay = self._compute_layout()
        changed = self.layout is None or lay.to_dict() != self.layout.to_dict()
        self.layout = lay
        if not changed:
            return          # the same sizes: loading the style again would only make GTK restyle the window
        self._load_css()
        self._chat_run(f"CO.layout({lay.font_px},{lay.column_px})", "layout")
        self._apply_content_layout()
        log.info("Layout: %s", lay.describe())

    def _apply_content_layout(self) -> None:
        """Chat Computer's own pages get the computed size; websites get the computed zoom."""
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

    # ================================================================ the onscreen keyboard
    def _to_display(self, x: float, y: float) -> tuple[float, float]:
        gw = self.win.get_window()
        if gw is None:
            return x, y
        origin = gw.get_origin()
        f = gw.get_scale_factor() or 1
        return (x + origin[-2]) * f, (y + origin[-1]) * f

    async def show_keyboard(self, show: bool) -> tuple | None:
        return await self._ui_call(self._show_keyboard_now, show)

    def _show_keyboard_now(self, show: bool) -> tuple | None:
        if not show:
            if self.keyboard_layer.up:
                self.keyboard_layer.hide_keyboard()
                self.chat.set_margin_bottom(0)
            self.touch.keyboard = None
            return None
        w, h = self.win.get_allocated_width(), self.win.get_allocated_height()
        if w <= 1 or h <= 1:
            w, h = self.win.get_default_size()
        x, y, aw, ah = self.keyboard_layer.show_keyboard(w, h)
        self.touch.keyboard = self.keyboard_layer.panel
        # Pages get the room above it, the way a tablet's do, so what's being typed into stays in view (the
        # conversation keeps its newest words at the bottom).
        self.chat.set_margin_bottom(int(self.keyboard_layer.panel[3]))
        f = (self.win.get_window().get_scale_factor() if self.win.get_window() else 1) or 1
        dx, dy = self._to_display(x, y)
        return self.keyboard_layer.spec((dx, dy, aw * f, ah * f))

    def _replace_keyboard(self) -> None:
        """The window changed size with the keyboard up: put it along the new bottom, and say where it is now."""
        if self.keyboard_layer.up:
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

    def quit(self) -> None:
        GLib.idle_add(lambda: Gtk.main_quit() and False)

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

    def _pick_files(self) -> None:
        """+ under the message box: this computer's file picker (several at once), from the home folder; the files
        chosen are added to the message (CO.attach)."""
        if self._locked:
            return
        chooser = Gtk.FileChooserNative.new("Add files", self.win, Gtk.FileChooserAction.OPEN, "Add", "Cancel")
        chooser.set_select_multiple(True)
        chooser.set_current_folder(os.path.expanduser("~"))

        def done(dialog: Any, response: int) -> None:
            paths = list(dialog.get_filenames() or []) if response == Gtk.ResponseType.ACCEPT else []
            dialog.destroy()
            files = [{"name": os.path.basename(p), "path": p} for p in paths if os.path.isfile(p)]
            if files:
                self._chat_run(f"CO.attach({json.dumps(files)})", "transient")
        chooser.connect("response", done)
        self._chooser = chooser          # (kept while it's open)
        chooser.show()

    def _on_chat_message(self, ucm: Any, result: Any) -> None:
        try:
            val = result.get_js_value() if hasattr(result, "get_js_value") else result
            msg = json.loads(val.to_string())
        except Exception:
            msg = {}
        kind = msg.get("type") if isinstance(msg, dict) else None
        if self._locked and kind not in ("unlock", "ready", "hole", "keys", "keyboard"):
            return              # locked: only the PIN (the page shows nothing else to use)
        if kind == "send":
            # Enter in the message box; with the files added to it (+), named after the words, for the model to read.
            text = str(msg.get("text") or "").strip()
            files = [str(f.get("path")) for f in msg.get("files") or [] if isinstance(f, dict) and f.get("path")]
            text = with_files(text, files[:20])
            if text and self.loop:
                self.loop.call_soon_threadsafe(self.on_typed, text)
            return
        if kind == "add_files":
            # + under the message box: this computer's own file picker; what's chosen is added to the message.
            self._pick_files()
            return
        if kind == "goto":
            # An item of Actions was tapped whose message isn't on the page any more.
            if self.on_goto is not None and self.loop:
                try:
                    ts = float(msg.get("ts") or 0)
                    before = float(msg["before"]) if msg.get("before") is not None else None
                except (TypeError, ValueError):
                    return
                self.loop.call_soon_threadsafe(self.on_goto, msg.get("id") or None, ts, before)
            return
        if kind == "copy":
            # Copy, under a message: onto the clipboard (the page tries too, but a WebKitGTK page may not be allowed).
            text = str(msg.get("text") or "")
            if text:
                clip = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
                clip.set_text(text, -1)
                clip.store()
            return
        if kind == "keys":
            self._speed["keys"] += [ms for ms in msg.get("ms") or [] if isinstance(ms, (int, float)) and 0 <= ms < 60_000]
            return
        if kind == "hole":
            # Where the conversation page leaves room for what's open: its frame, in the middle.
            try:
                self._hole = tuple(max(0, int(msg[k])) for k in ("x", "y", "w", "h"))      # type: ignore[assignment]
            except (KeyError, TypeError, ValueError):
                return
            if self.mode == "content" and self.shown is not None:
                self._place(self.shown, full=self._fullscreen and self.shown is self.content)
            return
        if kind == "close_content":
            # The x at the top of what's open (or a tap on Actions) on the conversation page.
            self.close_content(pause=bool(msg.get("pause", True)))
            return
        if kind == "scroll_open":
            # Page Up or Down in the message box (or the arrow keys, Home and End while it's empty) with something
            # open: that scrolls, as it would with the keys itself.
            how = SCROLL_OPEN.get(str(msg.get("how")))
            if how and self.mode == "content" and self.shown is self.content and not self._locked:
                self.content.evaluate_javascript(f"window.__CO && __CO.scroll({json.dumps(how[0])}, {how[1]}) && 1",
                                                 -1, WORLD, None, None, None, None)
            return
        if kind == "keyboard":
            # A finger on the message box (the onscreen keyboard comes up), or somewhere else (it goes).
            self.touch_keyboard(bool(msg.get("show")))
            return
        if kind in PAGE_EVENTS:
            # Pins, a message opened, a sign-in form filled in, "/" searching: the controller's (on its thread).
            # A form's passwords go straight there, never to the model, and are never logged.
            if self.loop:
                self.loop.call_soon_threadsafe(self.on_page_event, msg)
            return
        if kind != "ready":
            return
        self.chat_ready = True
        if self._reloading:
            # The page was loaded again after its renderer stopped: put back what it showed.
            self._reloading = False
            self.chat_backlog.clear()
            last = self._replay_last
            scripts = [last[k] for k in ("settings", "layout", "panels", "mic", "done", "messages", "mode", "lock")
                       if k in last]
            scripts += list(self._replay)
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
            log.error("Pages keep failing to draw; restarting Chat Computer")
            Gtk.main_quit()   # the session starts Chat Computer again
            return
        if view is self.chat:
            self.chat_ready = False
            self._reloading = True
            self.chat_backlog.clear()
            GLib.timeout_add(300, lambda: self.chat.load_uri((ASSETS / "chat.html").as_uri()) and False)
        else:
            GLib.timeout_add(300, lambda: self.content.reload() and False)

    # ================================================================ content layer
    # What's open (a website, a video, a message, a document, a page the model made) fills the middle, between the two
    # sidebars, from the top down to the message box, in place of the conversation: the conversation page draws its
    # frame and says where (a "hole" message), and the view is laid over the page there.
    def _add_in_hole(self, view: Gtk.Widget) -> None:
        view.set_halign(Gtk.Align.FILL)
        view.set_valign(Gtk.Align.FILL)
        view.set_no_show_all(True)        # shown only while it's what's open
        self.root.add_overlay(view)

    def _default_hole(self) -> tuple[int, int, int, int]:
        """Before the page has said where: the whole width, above the message box."""
        w, h = self.root.get_allocated_width(), self.root.get_allocated_height()
        if w <= 1 or h <= 1:
            w, h = self.win.get_default_size()
        rem = self.layout.font_px if self.layout else 16.0
        return 0, 0, w, max(0, int(h - 5 * rem))

    def _place(self, view: Gtk.Widget, full: bool = False) -> None:
        w, h = self.root.get_allocated_width(), self.root.get_allocated_height()
        if w <= 1 or h <= 1:
            w, h = self.win.get_default_size()
        x, y, hw, hh = (0, 0, w, h) if full else (self._hole or self._default_hole())
        view.set_margin_start(x)
        view.set_margin_top(y)
        view.set_margin_end(max(0, w - x - hw))
        view.set_margin_bottom(max(0, h - y - hh))

    def _fullscreen_changed(self, full: bool) -> bool:
        self._fullscreen = full
        self._place(self.content, full=full)
        return False                      # WebKit goes on to make the video fill the view

    def _title_of(self, view: Any) -> str:
        """What's open, as named at the top of its frame: its page's title, else its site's name."""
        title = " ".join((view.get_title() or "").split())
        if title:
            return title[:200]
        host = urllib.parse.urlsplit(view.get_uri() or "").hostname or ""
        return host[4:] if host.startswith("www.") else host

    def _retitle(self, view: Any) -> None:
        """A page's title came (or changed): the top of its frame says so."""
        if self.mode == "content" and self.shown is view and not self._locked:
            title = self._title_of(view)
            if title:
                self._chat_run(f"CO.title({json.dumps(title)})", "transient")

    def lock_screen(self, info: dict[str, Any] | None) -> None:
        def run() -> None:
            was, self._locked = self._locked, info is not None
            if self._locked and not was:
                if self.keyboard_layer.up:
                    self.touch_keyboard(False)
                if self.shown is not None:
                    self._put_away(self.shown)          # stays open, off the screen, until unlocked
            elif was and not self._locked and self.mode == "content" and self.shown is not None:
                self._show_view(self.shown)
            # Kept for the page's rebuild ("lock"): a page started again after its renderer stopped is locked too.
            self._chat_run(f"CO.lock({json.dumps(info)})", "lock")
            self.chat.grab_focus()
        self._ui(run)

    def _show_view(self, view: Any, title: str | None = None) -> None:
        """Puts a view in the middle, in place of the conversation or what was there, named at the top of its frame
        (`title`, else its page's). A website has the keys: typing and the keys that scroll are the page's (its
        search box, its shortcuts) until the message box is tapped. A page of Chat Computer's own (a document, a
        message, a page the model made) leaves them in the message box until it's tapped itself: it scrolls under the
        pointer, and with Page Up and Down there. While locked, it's only noted: it comes on the screen once
        unlocked."""
        if self.shown is not None and self.shown is not view:
            self._put_away(self.shown)
        self.mode = "content"
        self.shown = view
        if self._locked:
            self._put_away(view)
            return
        name = self._title_of(view) if title is None else title
        self._chat_run(f'CO.mode("content", {json.dumps(name)})', "mode")
        self._place(view, full=self._fullscreen and view is self.content)
        view.show()
        if view is self.content and (view.get_uri() or "") in self.native_uris:
            self.chat.grab_focus()
        else:
            view.grab_focus()

    def _put_away(self, view: Any) -> None:
        if view is self.content:
            view.hide()
        else:
            self._source_offscreen(view)

    def _show_chat(self) -> None:
        if self.shown is not None:
            self._put_away(self.shown)
        self.shown = None
        self.mode = "chat"
        self._chat_run('CO.mode("chat")', "mode")
        self.chat.grab_focus()

    def close_content(self, pause: bool = True) -> None:
        """The x at the top of what's open: it goes away (a video stops), and the conversation is back.
        (show_conversation, the model's, leaves music playing.)"""
        if self.mode != "content":
            return
        if pause and self.shown is self.content:
            self.content.evaluate_javascript(
                "document.querySelectorAll('video,audio').forEach(function(m){try{m.pause()}catch(e){}}); 1",
                -1, None, None, None, None, None)
        self._show_chat()
        if self.loop:
            self.loop.call_soon_threadsafe(self.on_closed)

    # ---------------------------------------------------------------- WhatsApp and social sites (Messages)
    def _source_context(self) -> Any:
        if self._src_ctx is None:
            base = self.paths.sources
            mgr = WebKit2.WebsiteDataManager(base_data_directory=str(base / "data"),
                                             base_cache_directory=str(base / "cache"))
            self._src_ctx = WebKit2.WebContext.new_with_website_data_manager(mgr)
            cm = self._src_ctx.get_cookie_manager()
            cm.set_persistent_storage(str(base / "cookies.sqlite"), WebKit2.CookiePersistentStorage.SQLITE)
            cm.set_accept_policy(WebKit2.CookieAcceptPolicy.NO_THIRD_PARTY)
        return self._src_ctx

    def _source_view(self, provider: str) -> Any:
        view = self.sources.get(provider)
        if view is not None:
            return view
        ucm = WebKit2.UserContentManager()
        ucm.add_script(WebKit2.UserScript.new_for_world(
            asset("sources.js"), WebKit2.UserContentInjectedFrames.TOP_FRAME,
            WebKit2.UserScriptInjectionTime.END, SOURCE_WORLD, None, None))
        ucm.connect("script-message-received::chatcomputerSource", lambda u, r, p=provider: self._on_source_message(p, r))
        ucm.register_script_message_handler_in_world("chatcomputerSource", SOURCE_WORLD)
        view = WebKit2.WebView(web_context=self._source_context(), user_content_manager=ucm)
        st = view.get_settings()
        st.set_enable_javascript(True)
        st.set_javascript_can_open_windows_automatically(False)
        st.set_enable_developer_extras(False)
        st.set_media_playback_requires_user_gesture(True)
        # As Safari on a Mac: the engine is the same (WebKit), and some of these sites turn away a browser they
        # don't know.
        st.set_user_agent(SAFARI_UA)
        if os.environ.get("CHATCOMPUTER_GPU", "1") != "1":
            st.set_hardware_acceleration_policy(WebKit2.HardwareAccelerationPolicy.NEVER)
        self._quiet_view(view)
        view.connect("decide-policy", self._on_decide_policy)
        view.connect("create", lambda v, action: None)          # no pop-up windows
        view.connect("permission-request", lambda v, req: (req.deny(), True)[1])
        view.connect("notify::title", lambda v, p: self._retitle(v))
        view.set_halign(Gtk.Align.FILL)
        view.set_valign(Gtk.Align.FILL)
        self.sources[provider] = view
        self._source_offscreen(view)
        return view

    def _source_offscreen(self, view: Any) -> None:
        """Into a window that's never on the screen, the size of a laptop's, where its page goes on (WhatsApp keeps
        its connection) and can be read."""
        parent = view.get_parent()
        if parent is self.root:
            self.root.remove(view)
        provider = next((p for p, v in self.sources.items() if v is view), "")
        win = self._src_windows.get(provider)
        if win is None:
            win = self._src_windows[provider] = Gtk.OffscreenWindow()
            win.set_default_size(1280, 900)
            view.set_size_request(1280, 900)
        if view.get_parent() is None:
            win.add(view)
        win.show_all()

    def _source_to_hole(self, view: Any) -> None:
        parent = view.get_parent()
        if parent is not None and parent is not self.root:
            parent.remove(view)
        view.set_size_request(-1, -1)
        if view.get_parent() is None:
            self.root.add_overlay(view)
            self.root.reorder_overlay(view, 1)       # under the onscreen keyboard, which stays on top
        view.set_no_show_all(False)

    def source_open(self, provider: str, url: str, reload: bool = False) -> None:
        """Loads a site's page in its own view (off the screen), unless it's already there."""
        def run() -> None:
            view = self._source_view(provider)
            if reload or not (view.get_uri() or "").startswith(url.split("#")[0][:30]):
                view.load_uri(url)
        self._ui(run)

    def source_show(self, provider: str, url: str | None = None, chat: str | None = None) -> None:
        """A site's page in the middle: to sign in, or to read a conversation (by its address, or for
        WhatsApp by its name)."""
        def run() -> None:
            view = self._source_view(provider)
            if url and (view.get_uri() or "") != url:
                view.load_uri(url)
            self._source_to_hole(view)
            self._show_view(view)
            if chat:
                view.evaluate_javascript(f"window.__CS && __CS.open({json.dumps(chat)})", -1, SOURCE_WORLD,
                                         None, None, None, None)
        self._ui(run)

    def source_unload(self, provider: str) -> None:
        """A social site read for now: its page goes (it's loaded again for the next look), unless it's showing."""
        def run() -> None:
            view = self.sources.get(provider)
            if view is not None and self.shown is not view:
                view.load_uri("about:blank")
        self._ui(run)

    def source_close(self, provider: str) -> None:
        def run() -> None:
            view = self.sources.get(provider)
            if view is not None and self.shown is view:
                self._show_chat()
        self._ui(run)

    def source_forget(self, provider: str) -> None:
        """Signed out (the account was removed here): the site's page goes, and its sign-in with the others'
        site data (cookies are kept per site by WebKit; the whole folder goes when no site is left)."""
        def run() -> None:
            view = self.sources.pop(provider, None)
            if view is None:
                return
            if self.shown is view:
                self._show_chat()
            parent = view.get_parent()
            if parent is not None:
                parent.remove(view)
            win = self._src_windows.pop(provider, None)
            if win is not None:
                win.destroy()
            view.try_close()
        self._ui(run)

    def _on_source_message(self, provider: str, result: Any) -> None:
        try:
            val = result.get_js_value() if hasattr(result, "get_js_value") else result
            msg = json.loads(val.to_string())
        except Exception:
            return
        if isinstance(msg, dict) and self.loop:
            self.loop.call_soon_threadsafe(self.on_source, provider, msg)

    def watch_redirect(self, prefix: str, then: Callable[[str], None] | None) -> None:
        """When a page in the content view goes to an address starting with `prefix`, it isn't loaded: `then` gets
        the address (a sign-in's answer, like Microsoft's for Outlook). None stops watching."""
        def run() -> None:
            if then is None:
                self._redirects.pop(prefix, None)
            else:
                self._redirects[prefix] = then
        self._ui(run)

    def _info(self) -> dict[str, Any]:
        if self.mode == "content" and self.shown is not None and self.shown is not self.content:
            # A site Messages reads (WhatsApp, Instagram...): shown to the person, not for the model to use.
            provider = next((p for p, v in self.sources.items() if v is self.shown), "")
            return {"mode": "source", "source": provider, "url": self.shown.get_uri(), "title": self.shown.get_title()}
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
            # Zoom websites for this screen; Chat Computer's own pages are already sized.
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
            for prefix, then in list(self._redirects.items()):
                if uri.startswith(prefix):
                    decision.ignore()
                    del self._redirects[prefix]
                    if self.loop:
                        self.loop.call_soon_threadsafe(then, uri)
                    return True
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
        elif msg.get("type") == "keyboard":
            # A finger on a website's box to type in (the onscreen keyboard comes up), or it left it.
            self.touch_keyboard(bool(msg.get("show")))

    async def show_url(self, url: str, title: str | None = None) -> dict[str, Any]:
        """Opens an address in the middle, named `title` at the top of its frame (else its site's name) until its
        page's own title comes."""
        assert self.loop is not None
        fut = self.loop.create_future()
        if title is None:
            host = urllib.parse.urlsplit(url).hostname or ""
            title = host[4:] if host.startswith("www.") else host

        def go() -> None:
            self.load_waiters.append(fut)
            self.load_error = None
            self.content.load_uri(url)
            self._show_view(self.content, title)
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
        return await self.show_url(uri, title)

    async def hide_content(self) -> None:
        await self._ui_call(self._show_chat)

    async def settle(self, quiet_s: float = 0.15, max_s: float = 1.5, load_s: float = 10.0) -> dict[str, Any]:
        """After a click or keys: waits for the page to react, rather than a fixed pause. A page that starts
        loading is waited for (up to `load_s`), then until nothing on it has changed for `quiet_s` (up to `max_s`).
        Returns what's showing. A fixed pause was too short for a new page (the model then read the old one, or a
        page still loading, and had to look again: another trip to the model) and too long for a menu that opens
        at once."""
        assert self.loop is not None
        await asyncio.sleep(0.08)               # the click reaches the page; a link starts loading
        fut = self.loop.create_future()

        def arm() -> bool:
            if self.mode == "content" and self.content.is_loading():
                self.load_waiters.append(fut)
                return True
            return False
        if await self._ui_call(arm):
            with contextlib.suppress(asyncio.TimeoutError):
                await asyncio.wait_for(asyncio.shield(fut), load_s)
        end = time.monotonic() + max_s
        while time.monotonic() < end and self.mode == "content":
            quiet = await self.eval_js("(window.__CO && __CO.quietFor ? __CO.quietFor() : 1e9)")
            if not isinstance(quiet, (int, float)) or quiet >= quiet_s * 1000:
                break
            await asyncio.sleep(min(0.05, max(0.0, quiet_s - quiet / 1000)))
        return await self._ui_call(self._info)

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

    async def content_rect(self) -> tuple[int, int, int, int] | None:
        def calc() -> tuple[int, int, int, int] | None:
            gw = self.content.get_window()
            if self.mode != "content" or gw is None or not self.content.get_mapped():
                return None
            _, ox, oy = gw.get_origin()
            sf = float(self.content.get_scale_factor() or 1)
            a = self.content.get_allocation()
            return int(ox * sf), int(oy * sf), int(a.width * sf), int(a.height * sf)
        return await self._ui_call(calc)

    async def to_pdf(self, path: str, html: str | None = None, base_dir: str | None = None) -> dict[str, Any]:
        """Print to a PDF with WebKit's own printing ("Print to File", no dialog), on the paper this country uses:
        a page given as HTML, in a view of its own that's never shown, or what's open between the panels."""
        assert self.loop is not None
        fut = self.loop.create_future()

        def finish(error: str | None) -> None:
            self.loop.call_soon_threadsafe(lambda: fut.done() or fut.set_result(
                {"ok": False, "error": error} if error else {"ok": True, "path": path}))

        def start() -> None:
            if html is None:
                if self.mode != "content":
                    finish("Nothing is open on the screen.")
                    return
                self._print_view(self.content, path, finish)
                return
            # A window off the screen, for WebKit to lay the page out in; gone once it's printed.
            win = Gtk.OffscreenWindow()
            win.set_default_size(900, 1200)
            view = WebKit2.WebView(web_context=self.ctx)
            view.get_settings().set_enable_javascript(True)      # md.js turns a document's Markdown into its page
            win.add(view)
            win.show_all()

            def done(error: str | None) -> None:
                finish(error)
                GLib.idle_add(lambda: win.destroy() and False)

            def loaded(v: WebKit2.WebView, event: WebKit2.LoadEvent) -> None:
                if event == WebKit2.LoadEvent.FINISHED:
                    v.disconnect_by_func(loaded)
                    # The page's own scripts (a document's Markdown) run as it loads: a moment more, then print.
                    GLib.timeout_add(300, lambda: self._print_view(v, path, done) and False)
            view.connect("load-changed", loaded)
            view.load_html(html, Path(base_dir).as_uri() + "/" if base_dir else None)

        self._ui(start)
        try:
            return await asyncio.wait_for(fut, 60)
        except asyncio.TimeoutError:
            return {"ok": False, "error": "Making the PDF took too long."}

    @staticmethod
    def _print_view(view: WebKit2.WebView, path: str, done: Callable[[str | None], None]) -> None:
        settings = Gtk.PrintSettings()
        settings.set_printer("Print to File")
        settings.set(Gtk.PRINT_SETTINGS_OUTPUT_FILE_FORMAT, "pdf")
        settings.set(Gtk.PRINT_SETTINGS_OUTPUT_URI, Path(path).as_uri())
        setup = Gtk.PageSetup()
        setup.set_paper_size(Gtk.PaperSize.new(Gtk.PaperSize.get_default()))   # A4, or Letter where that's used
        op = WebKit2.PrintOperation.new(view)
        op.set_print_settings(settings)
        op.set_page_setup(setup)
        said: list[str] = []
        # "failed" comes before "finished" when it goes wrong; "finished" always comes.
        op.connect("failed", lambda o, err: said.append(str(getattr(err, "message", err)) or "printing failed"))
        op.connect("finished", lambda o: done(said[0] if said else None))
        op.print_()

    # ================================================================ conversation layer
    def user_text(self, msg_id: str, text: str, final: bool) -> None:
        self._chat_js(f"CO.user({json.dumps(msg_id)},{json.dumps(text)},{json.dumps(final)})")

    def partial(self, text: str) -> None:
        self._chat_js(f"CO.partial({json.dumps(text)})", "transient")

    def ai_delta(self, msg_id: str, text: str) -> None:
        def add() -> None:
            self.pending_ai[msg_id] = self.pending_ai.get(msg_id, "") + text
            if not self.flush_scheduled:
                self.flush_scheduled = True
                GLib.timeout_add(40, self._flush_ai)
        self._ui(add)

    def _flush_ai(self) -> bool:
        """The words that streamed in during the last 40 ms, onto the page in one go."""
        self.flush_scheduled = False
        pending, self.pending_ai = self.pending_ai, {}
        for mid, txt in pending.items():
            self._chat_run(f"CO.ai({json.dumps(mid)},{json.dumps(txt)})" if txt else f"CO.aiStart({json.dumps(mid)})")
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

    def state(self, name: str) -> None:
        self._chat_js(f"CO.state({json.dumps(name)})", "state")

    def history(self, items: list[dict[str, Any]]) -> None:
        self._chat_js(f"CO.history({json.dumps(items)})")

    def clear(self) -> None:
        self._chat_js("CO.clear()")

    def chats(self, items: list[dict[str, Any]], current: str) -> None:
        self._chat_js(f"CO.chats({json.dumps(items)},{json.dumps(current)})", "chats")

    def mic(self, on: bool) -> None:
        self._chat_js(f"CO.mic({json.dumps(bool(on))})", "mic")

    # "/" in the message box: what on this computer matches.
    def search_results(self, n: int, query: str, items: list[dict[str, Any]], note: str = "") -> None:
        self._chat_js(f"CO.searchResults({int(n)},{json.dumps(query)},{json.dumps(items)},{json.dumps(note)})",
                      "transient")

    def apply_settings(self, text_size: int, user_name: str, model: str = "") -> None:
        # Size settings (text_size, text_scale, viewing_distance_cm, screen_inches, web_zoom) are
        # read from self.settings, the same object the controller changes.
        self._chat_js(f"CO.settings({json.dumps(user_name)},{json.dumps(model)})", "settings")
        self._ui(self._apply_layout)

    def panels(self, shown: dict[str, bool]) -> None:
        self._chat_js(f"CO.panels({json.dumps(shown)})", "panels")

    def here(self, info: dict[str, Any]) -> None:
        self._chat_js(f"CO.here({json.dumps(info)})", "here")

    # Actions (at the right) and Messages (at the left). The page is sent each new one; the whole lists are
    # kept here too, so the page can be rebuilt after its renderer stops.
    def done_list(self, entries: list[dict[str, Any]]) -> None:
        def run() -> None:
            self._done_entries = list(entries)[-DONE_SHOWN:]
            script = f"CO.doneList({json.dumps(self._done_entries)})"
            self._replay_last["done"] = script
            self._chat_run(script, "transient")
        self._ui(run)

    def done(self, entry: dict[str, Any]) -> None:
        def run() -> None:
            self._done_entries = (self._done_entries + [entry])[-DONE_SHOWN:]
            self._replay_last["done"] = f"CO.doneList({json.dumps(self._done_entries)})"
            self._chat_run(f"CO.done({json.dumps(entry)})", "transient")
        self._ui(run)

    def done_states(self, entries: list[dict[str, Any]]) -> None:
        def run() -> None:
            marks = {e["id"]: e for e in entries}
            for e in self._done_entries:
                m = marks.get(e.get("id"))
                if m is not None:
                    for k in ("revert", "reverted"):
                        if m.get(k):
                            e[k] = m[k]
                        else:
                            e.pop(k, None)
            self._replay_last["done"] = f"CO.doneList({json.dumps(self._done_entries)})"
            states = [{k: e[k] for k in ("id", "revert", "reverted") if k in e} for e in entries]
            self._chat_run(f"CO.doneStates({json.dumps(states)})", "transient")
        self._ui(run)

    def messages_list(self, items: list[dict[str, Any]], accounts: list[dict[str, Any]] | None = None) -> None:
        def run() -> None:
            self._msg_items = {m["id"]: m for m in items}
            if accounts is not None:
                self._msg_accounts = list(accounts)
            script = f"CO.messagesList({json.dumps(list(self._msg_items.values()))},{json.dumps(self._msg_accounts)})"
            self._replay_last["messages"] = script
            self._chat_run(script, "transient")
        self._ui(run)

    def message(self, item: dict[str, Any]) -> None:
        def run() -> None:
            self._msg_items[item["id"]] = item
            if len(self._msg_items) > MESSAGES_SHOWN:
                keep = sorted(self._msg_items.values(), key=lambda m: (bool(m.get("pinned")), m.get("ts", 0)))
                self._msg_items = {m["id"]: m for m in keep[-MESSAGES_SHOWN:]}
            self._replay_last["messages"] = (f"CO.messagesList({json.dumps(list(self._msg_items.values()))},"
                                             f"{json.dumps(self._msg_accounts)})")
            self._chat_run(f"CO.message({json.dumps(item)})", "transient")
        self._ui(run)

    def set_pinned(self, kind: str, item_id: str, pinned: float | None) -> None:
        """Keeps the copy here in step with a pin made or taken off on the page (the page already shows it)."""
        def run() -> None:
            src = self._msg_items.get(item_id) if kind == "messages" else next(
                (e for e in self._done_entries if e.get("id") == item_id), None)
            if src is not None:
                if pinned:
                    src["pinned"] = pinned
                else:
                    src.pop("pinned", None)
        self._ui(run)

    def earlier(self, items: list[dict[str, Any]], target_id: str | None, target_ts: float) -> None:
        self._chat_js(f"CO.earlier({json.dumps(items)},{json.dumps(target_id)},{json.dumps(target_ts)})",
                      "transient")

    def account_form(self, form: dict[str, Any] | None) -> None:
        """A sign-in form for a mail or Telegram account on the conversation page (None closes it). What's typed
        in it comes back to on_account, never through the model."""
        self._chat_js(f"CO.accountForm({json.dumps(form)})", "transient")

    # ================================================================ the touchscreen
    def _gdk_event(self, ev: Any, *_data: Any) -> None:
        """Every event, before GTK has it: touches go through the iPad's system gestures (ui/touch.py), and a key
        typed on a real keyboard is noted (it puts the onscreen keyboard away, as on an iPad)."""
        kind = ev.get_event_type()
        try:
            if kind in TOUCH_EVENTS:
                if self._touch_event(ev, kind):
                    return                   # the screen's (a system gesture, the onscreen keyboard): not the page's
            elif kind == Gdk.EventType.KEY_PRESS:
                self._note_key(ev)
        except Exception:
            log.exception("touch")
        self._deliver(ev)

    def _touch_event(self, ev: Any, kind: Any) -> bool:
        """A touch's event: True when the page under it mustn't have it."""
        seq = ev.get_event_sequence()
        _ok, rx, ry = ev.get_root_coords()
        gw = self.win.get_window()
        ox, oy = gw.get_origin()[-2:] if gw is not None else (0, 0)
        x, y, now, tg = rx - ox, ry - oy, time.monotonic(), self.touch
        tg.size = (float(self.win.get_allocated_width()), float(self.win.get_allocated_height()))
        if kind == Gdk.EventType.TOUCH_BEGIN:
            verdict, cancel = tg.begin(seq, x, y, now)
            for s in cancel:
                self._cancel_on_page(s)
            if verdict == RELEASE:
                for s in list(self._held):
                    self._release(s)
                verdict = PASS
            return self._route(seq, verdict, ev, x, y, begin=True)
        if kind == Gdk.EventType.TOUCH_UPDATE:
            verdict = tg.move(seq, x, y, now)
            if verdict == RELEASE:
                self._release(seq)
                verdict = PASS
            return self._route(seq, verdict, ev, x, y)
        if kind == Gdk.EventType.TOUCH_END:
            verdict, _gesture = tg.end(seq, x, y, now)
            if verdict == RELEASE:
                self._release(seq)
                verdict = PASS
            taken = self._route(seq, verdict, ev, x, y, end=True)
            self._given.pop(seq, None)
            self._held.pop(seq, None)
            return taken
        verdict = tg.cancel(seq)                     # cancelled (the window lost it)
        self._held.pop(seq, None)
        self._given.pop(seq, None)
        if verdict == KEY:
            self.keyboard_layer.touch_key(None)
        return verdict != PASS

    def _route(self, seq: Any, verdict: str, ev: Any, x: float, y: float, begin: bool = False,
               end: bool = False) -> bool:
        if verdict == PASS:
            self._given[seq] = ev.copy()
            return False
        if verdict == HOLD:
            self._held.setdefault(seq, []).append(ev.copy())
            return True
        if verdict == KEY:
            self._touch_key(x, y, end)
        return True

    def _release(self, seq: Any) -> None:
        """A touch held back wasn't the system's: the page gets what it missed."""
        for e in self._held.pop(seq, []):
            self._deliver(e)
            self._given[seq] = e

    def _cancel_on_page(self, seq: Any) -> None:
        """A touch the page had became part of the screen's gesture (a third finger came): cancelled there."""
        last = self._given.pop(seq, None)
        if last is not None:
            c = last.copy()
            c.touch.type = Gdk.EventType.TOUCH_CANCEL
            self._deliver(c)

    def _touch_key(self, x: float, y: float, end: bool) -> None:
        """A finger on the onscreen keyboard: the key under it shows pressed; lifted, it's typed (as on an iPad,
        sliding to another key before lifting types that one)."""
        if not end:
            self.keyboard_layer.touch_key((x, y))
            return
        self.keyboard_layer.touch_key(None)
        action = self.keyboard_layer.press(x, y)
        if not action:
            return
        if action[0] == "hide":
            self.touch_keyboard(False)
        elif self.loop and self.on_keyboard_action:
            self.loop.call_soon_threadsafe(self.on_keyboard_action, action)

    def _touch_gesture(self, name: str) -> None:
        """One of the iPad's system gestures (ui/touch.py) was made."""
        log.info("touch: %s", name)
        if self._locked:
            return              # locked: only the keypad works
        if name == "home":
            if self.mode == "content":
                self.close_content()
            elif self.keyboard_layer.up:
                self.touch_keyboard(False)
        elif name in ("back", "forward"):
            v = self.shown if self.mode == "content" else None
            if v is not None and (v.can_go_back() if name == "back" else v.can_go_forward()):
                v.go_back() if name == "back" else v.go_forward()
        elif name == "messages":
            self._chat_js("CO.showMessages()", "transient")
        elif name in EDIT_COMMANDS:
            view = self.shown if self.mode == "content" and self.shown is not None and self.shown.has_focus() \
                else self.chat
            view.execute_editing_command(EDIT_COMMANDS[name])

    def touch_keyboard(self, show: bool) -> None:
        """The onscreen keyboard for a finger on a box to type in, or away (its hide key, a key typed on a real
        keyboard, Home, or a touch somewhere else). Not while a real keyboard is in use, as on an iPad."""
        if show == self.keyboard_layer.up or (show and time.monotonic() - self._real_key_at < REAL_KEYBOARD_S):
            return
        spec = self._show_keyboard_now(show)
        if self.on_keyboard:
            try:
                self.on_keyboard(spec)
            except Exception:
                log.exception("on_keyboard")

    def _note_key(self, ev: Any) -> None:
        """A key: from a real keyboard (not one Chat Computer typed itself, for the onscreen keyboard), it
        puts the onscreen keyboard away."""
        dev = ev.get_source_device()
        name = (dev.get_name() if dev is not None else "") or ""
        if "XTEST" in name.upper():
            return
        self._real_key_at = time.monotonic()
        if self.keyboard_layer.up:
            self.touch_keyboard(False)

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
        if self._locked:
            if not self.chat.has_focus():
                self.chat.grab_focus()        # every key is for the lock screen's PIN box
            return False
        if name == "Escape" and self.mode == "content" and not self._fullscreen and not self.chat.has_focus():
            # Escape while what's open has the keys: it's closed, the conversation back, as on the conversation page
            # (chat.js, which has Escape otherwise). The sidebars stay (their switches are for them). A full-screen
            # video leaves full screen first, as anywhere.
            self._chat_run("CO.escape()", "transient")
            return True
        if self.mode == "chat" and not self.chat.has_focus():
            self.chat.grab_focus()      # typing goes into the conversation's message box (chat.js), whatever has focus
        # Everything else goes to the page that's showing: the conversation's message box (a real text box, so the
        # cursor, selecting, pasting and input methods work as anywhere else), or a website's forms and sign-ins.
        return False

    # ================================================================ how quick it feels
    # Typing on a real PC once felt slow, with nothing in the log to say why. So the log now says, once a minute
    # while it happens: how long typed letters took to reach the screen (measured by the page, chat.js), and how
    # often the window's own thread was busy for more than a tenth of a second (when it is, nothing on the screen
    # moves).
    def _watch_speed(self) -> bool:
        now = time.monotonic()
        late = (now - self._speed["tick"] - SPEED_TICK_S) * 1000
        self._speed["tick"] = now
        if late > 100:
            self._speed["late"] += 1
            self._speed["worst"] = max(self._speed["worst"], late)
        if now - self._speed["since"] >= 60:
            keys = sorted(self._speed["keys"])
            if keys or self._speed["late"]:
                median = keys[len(keys) // 2] if keys else 0
                log.info("speed: %d letters typed took %.0f ms (median) and %.0f ms (slowest) to reach the screen; "
                         "the window was busy %d times, up to %.0f ms", len(keys), median, keys[-1] if keys else 0,
                         self._speed["late"], self._speed["worst"])
            self._speed.update(keys=[], late=0, worst=0.0, since=now)
        return True


def run_app(paths: Paths, settings: Settings, controller_factory: Callable[[GtkScreen], Any], windowed: bool = False) -> int:
    """Start GTK on this thread and Chat Computer's logic on a background asyncio loop. The window comes first: the
    controller is made on the loop's thread once GTK is running, because loading its modules (the Agent SDK)
    takes most of a second, and the screen shouldn't stay black meanwhile."""
    how = choose_rendering()
    if how == "cpu":
        log.info("No graphics chip available for drawing pages; drawing them with the processor")
    log.info("graphics: pages drawn with the %s; %s; screens: %s", "graphics chip" if how == "gpu" else "processor",
             graphics_report(), displays_report())
    screen = GtkScreen(paths, settings, windowed=windowed)
    loop = asyncio.new_event_loop()
    t = threading.Thread(target=loop.run_forever, name="chatcomputer-loop", daemon=True)
    t.start()
    # What's typed while the controller is still loading (most of a second) is kept and sent once it's there.
    early: list[str] = []
    screen.bind(loop, on_media_key=lambda k: None, on_media_state=lambda p: None, on_typed=early.append)
    holder: dict[str, Any] = {}

    async def boot() -> None:
        controller = holder["controller"] = controller_factory(screen)
        controller.loop = asyncio.get_running_loop()

        def typed(text: str) -> None:
            controller._spawn(controller.handle(text, typed=True))
        screen.bind(loop, on_media_key=controller.media_key, on_media_state=controller.media_state, on_typed=typed)
        for text in early:
            typed(text)      # handled once start() below has set the controller up (its first pause)
        await controller.start()

    def started(f: Any) -> None:
        exc = f.exception()
        if exc:
            log.error("Chat Computer failed to start: %s", exc, exc_info=exc)
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
