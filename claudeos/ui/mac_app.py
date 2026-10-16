"""The ClaudeOS window on a Mac: one borderless window that fills the screen, with two layers, like the Linux
window (gtk_app.py):

* The conversation (the same local web page: only your words and Claude's words).
* The content layer: websites, videos, documents and pages, in a WebKit view (WKWebView), with a caption
  strip over it. Sign-ins and cookies are kept by macOS's WebKit for this app.

ClaudeOS hides the Dock and the menu bar while it is in front. "Switch to macOS" hides ClaudeOS (the Mac's
desktop shows); saying "Claude", an AirPods press or clicking ClaudeOS in the Dock brings it back. Cmd+H
hides it too, Cmd+Q quits it.

AppKit runs on the main thread; ClaudeOS's logic runs on an asyncio thread. Everything that touches the
window is handed to the main thread with AppHelper.callAfter.
"""
from __future__ import annotations

import asyncio
import io
import json
import logging
import os
import subprocess
import sys
import threading
import time
from collections import deque
from pathlib import Path
from typing import Any, Callable

import objc
from AppKit import (NSApplication, NSApplicationActivationPolicyRegular, NSApplicationPresentationHideDock,
                    NSApplicationPresentationHideMenuBar, NSBackingStoreBuffered, NSBitmapImageFileTypePNG,
                    NSBitmapImageRep, NSColor, NSCursor, NSEvent, NSEventMaskKeyDown, NSEventModifierFlagCommand,
                    NSEventModifierFlagControl, NSFont, NSLineBreakByWordWrapping, NSMenu, NSMenuItem, NSScreen,
                    NSTextField, NSView, NSViewHeightSizable, NSViewWidthSizable, NSWindow,
                    NSWindowStyleMaskBorderless, NSWindowStyleMaskClosable, NSWindowStyleMaskMiniaturizable,
                    NSWindowStyleMaskResizable, NSWindowStyleMaskTitled)
from Foundation import NSURL, NSBundle, NSMakeRect, NSObject, NSURLRequest
from PyObjCTools import AppHelper
from WebKit import (WKContentWorld, WKUserContentController, WKUserScript, WKWebsiteDataStore, WKWebView,
                    WKWebViewConfiguration)

from ..config import Paths, Settings
from ..screen import Screen
from .layout import Layout, compute, css_vars, screen_from_monitor
from .mac_blocks import call_block
from .render import ASSETS, NATIVE, asset

log = logging.getLogger("claudeos.ui")
WORLD_NAME = "claudeos"
INJECT_AT_DOCUMENT_END = 1           # WKUserScriptInjectionTimeAtDocumentEnd
RESPONSE_ALLOW, RESPONSE_DOWNLOAD = 1, 2   # WKNavigationResponsePolicy
PERMISSION_DENY = 2                  # WKPermissionDecisionDeny
TERMINATE_NOW, TERMINATE_LATER = 1, 2   # NSTerminateNow, NSTerminateLater
KEY_RETURN, KEY_ENTER, KEY_BACKSPACE, KEY_ESCAPE = 36, 76, 51, 53
# Same colours as theme.css (plain: black on white, or white on black in dark mode).
LIGHT = {"bg": "#FFFFFF", "ink": "#000000", "soft": "#595959", "rule": "#D9D9D9"}
DARK = {"bg": "#000000", "ink": "#FFFFFF", "soft": "#A6A6A6", "rule": "#404040"}
# The Mac's own typeface (San Francisco) is what "the system's standard font" means here; plain sans-serif
# would be Helvetica in WebKit on a Mac.
MAC_FONTS = ("r.setProperty('--sans','system-ui, -apple-system, sans-serif');"
             "r.setProperty('--mono','ui-monospace, Menlo, monospace');")
FONTS_JS = "(function(){var r=document.documentElement.style;" + MAC_FONTS + "})()"
SET_VARS_JS = ("(function(){{var r=document.documentElement.style;r.setProperty('--co-size','{size}px');"
               "r.setProperty('--co-column','{column}px');" + MAC_FONTS.replace("{", "{{").replace("}", "}}") + "}})()")
INJECT_AT_DOCUMENT_START = 0         # WKUserScriptInjectionTimeAtDocumentStart


def _app() -> Any:
    # The shared application (AppKit's NSApp global is only set once it exists, so ask for it each time).
    return NSApplication.sharedApplication()


def _color(hexstr: str) -> Any:
    h = hexstr.lstrip("#")
    r, g, b = (int(h[i:i + 2], 16) / 255 for i in (0, 2, 4))
    return NSColor.colorWithSRGBRed_green_blue_alpha_(r, g, b, 1.0)


# ================================================================ Objective-C helpers
class _KeyWindow(NSWindow):
    """A borderless window that can still take the keyboard."""

    def canBecomeKeyWindow(self) -> bool:
        return True

    def canBecomeMainWindow(self) -> bool:
        return True


class _Messages(NSObject, protocols=[objc.protocolNamed("WKScriptMessageHandler")]):
    def initWithCallback_(self, cb):
        self = objc.super(_Messages, self).init()
        if self is None:
            return None
        self.cb = cb
        return self

    def userContentController_didReceiveScriptMessage_(self, ucc, message):
        try:
            self.cb(str(message.name()), message.body())
        except Exception:
            log.exception("page message")


class _NavDelegate(NSObject, protocols=[objc.protocolNamed("WKNavigationDelegate")]):
    def initWithScreen_(self, screen):
        self = objc.super(_NavDelegate, self).init()
        if self is None:
            return None
        self.screen = screen
        return self

    def webView_didStartProvisionalNavigation_(self, view, nav):
        if view == self.screen.content:
            self.screen.load_error = None

    def webView_didCommitNavigation_(self, view, nav):
        if view == self.screen.content:
            self.screen._on_content_committed()

    def webView_didFinishNavigation_(self, view, nav):
        if view == self.screen.content:
            self.screen._on_content_finished()

    def webView_didFailNavigation_withError_(self, view, nav, error):
        if view == self.screen.content:
            self.screen._on_content_failed(error)

    def webView_didFailProvisionalNavigation_withError_(self, view, nav, error):
        if view == self.screen.content:
            self.screen._on_content_failed(error)

    def webViewWebContentProcessDidTerminate_(self, view):
        self.screen._on_renderer_stopped(view)

    def webView_decidePolicyForNavigationResponse_decisionHandler_(self, view, response, handler):
        # What WebKit can't show (archives, installers...) goes to Downloads instead.
        try:
            download = view == self.screen.content and not response.canShowMIMEType()
        except Exception:
            download = False
        call_block(handler, b"v@?q", RESPONSE_DOWNLOAD if download else RESPONSE_ALLOW)

    def webView_navigationResponse_didBecomeDownload_(self, view, response, download):
        download.setDelegate_(self.screen._downloads)

    def webView_navigationAction_didBecomeDownload_(self, view, action, download):
        download.setDelegate_(self.screen._downloads)


class _UIDelegate(NSObject, protocols=[objc.protocolNamed("WKUIDelegate")]):
    def initWithScreen_(self, screen):
        self = objc.super(_UIDelegate, self).init()
        if self is None:
            return None
        self.screen = screen
        return self

    def webView_createWebViewWithConfiguration_forNavigationAction_windowFeatures_(self, view, config, action, features):
        # Links that want a new window open in the same view.
        req = action.request()
        if req is not None and req.URL() is not None:
            view.loadRequest_(req)
        return None

    def webView_requestMediaCapturePermissionForOrigin_initiatedByFrame_type_decisionHandler_(self, view, origin, frame,
                                                                                              kind, handler):
        call_block(handler, b"v@?q", PERMISSION_DENY)      # websites don't get the camera or microphone


class _Downloads(NSObject, protocols=[objc.protocolNamed("WKDownloadDelegate")]):
    def initWithScreen_(self, screen):
        self = objc.super(_Downloads, self).init()
        if self is None:
            return None
        self.screen = screen
        return self

    def download_decideDestinationUsingResponse_suggestedFilename_completionHandler_(self, download, response, name, handler):
        dest = Path.home() / "Downloads"
        dest.mkdir(parents=True, exist_ok=True)
        name = os.path.basename(str(name or "download"))
        target = dest / name
        i = 1
        while target.exists():
            target = dest / f"{Path(name).stem} ({i}){Path(name).suffix}"
            i += 1
        self.screen.notice(f"Downloading {target.name} to Downloads.")
        call_block(handler, b"v@?@", NSURL.fileURLWithPath_(str(target)))

    def downloadDidFinish_(self, download):
        self.screen.notice("Download finished.")

    def download_didFailWithError_resumeData_(self, download, error, data):
        self.screen.notice("The download didn't finish.")


class _WindowDelegate(NSObject):
    def initWithScreen_(self, screen):
        self = objc.super(_WindowDelegate, self).init()
        if self is None:
            return None
        self.screen = screen
        return self

    def windowDidResize_(self, note):
        self.screen._schedule_relayout()

    def windowDidChangeScreen_(self, note):
        self.screen._schedule_relayout()

    def windowDidChangeBackingProperties_(self, note):
        self.screen._schedule_relayout()


class _AppDelegate(NSObject):
    def initWithScreen_(self, screen):
        self = objc.super(_AppDelegate, self).init()
        if self is None:
            return None
        self.screen = screen
        return self

    def applicationDidBecomeActive_(self, note):
        self.screen._present()
        self.screen.on_shown()          # brought back by hand (the Dock, Command-Tab): listening again

    def applicationDidHide_(self, note):
        self.screen.on_hidden()         # Command-H, or "switch to macOS"

    def applicationShouldHandleReopen_hasVisibleWindows_(self, app, flag):
        self.screen._do_come_back()
        return True

    def applicationShouldTerminate_(self, app):
        # Command-Q, logging out, restarting or shutting down: ClaudeOS stops cleanly first, then tells macOS
        # it may go on (replyToApplicationShouldTerminate:). macOS then ends the process.
        return self.screen._quit_requested()


def _install_menu(app: Any) -> None:
    """A main menu, hidden with the menu bar, so Cmd+H, Cmd+Q and copy/paste in websites work."""
    bar = NSMenu.alloc().init()
    app_item = NSMenuItem.alloc().init()
    bar.addItem_(app_item)
    app_menu = NSMenu.alloc().initWithTitle_("ClaudeOS")
    app_menu.addItemWithTitle_action_keyEquivalent_("Hide ClaudeOS", "hide:", "h")
    app_menu.addItem_(NSMenuItem.separatorItem())
    app_menu.addItemWithTitle_action_keyEquivalent_("Quit ClaudeOS", "terminate:", "q")
    app_item.setSubmenu_(app_menu)
    edit_item = NSMenuItem.alloc().init()
    bar.addItem_(edit_item)
    edit = NSMenu.alloc().initWithTitle_("Edit")
    for title, sel, key in (("Undo", "undo:", "z"), ("Redo", "redo:", "Z"), ("Cut", "cut:", "x"),
                            ("Copy", "copy:", "c"), ("Paste", "paste:", "v"), ("Select All", "selectAll:", "a")):
        edit.addItemWithTitle_action_keyEquivalent_(title, sel, key)
    edit_item.setSubmenu_(edit)
    app.setMainMenu_(bar)


# ================================================================ the window
class MacScreen(Screen):
    def __init__(self, paths: Paths, settings: Settings, windowed: bool = False) -> None:
        self.paths = paths
        self.settings = settings
        self.loop: asyncio.AbstractEventLoop | None = None
        self.on_media_key: Callable[[str], None] = lambda k: None
        self.on_media_state: Callable[[bool], None] = lambda p: None
        self.on_typed: Callable[[str], None] = lambda t: None
        self.on_hidden: Callable[[], None] = lambda: None
        self.on_shown: Callable[[], None] = lambda: None
        # Set by run_app: starts ClaudeOS's clean stop and calls the function it's given when that's done.
        self.on_quit: Callable[[Callable[[], None]], None] | None = None
        self._quitting = False
        self.relaunch_after_quit = False
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
        self.cap_hide_at = 0.0
        self.windowed = windowed
        self.layout: Layout | None = None
        self.native_uris: set[str] = set()
        self._relayout_pending = False
        self._replay: deque[str] = deque(maxlen=2000)
        self._replay_last: dict[str, str] = {}
        self._reloading = False
        self._renderer_stops: list[float] = []
        self._build(windowed)

    def bind(self, loop: asyncio.AbstractEventLoop, *, on_media_key: Callable[[str], None],
             on_media_state: Callable[[bool], None], on_typed: Callable[[str], None]) -> None:
        self.loop = loop
        self.on_media_key = on_media_key
        self.on_media_state = on_media_state
        self.on_typed = on_typed

    # ---------------------------------------------------------------- building the window
    def _build(self, windowed: bool) -> None:
        screen = NSScreen.mainScreen()
        frame = screen.frame()
        if windowed:
            w, h = int(os.environ.get("CLAUDEOS_W", 1280)), int(os.environ.get("CLAUDEOS_H", 800))
            rect = NSMakeRect(80, 80, w, h)
            style = (NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskResizable |
                     NSWindowStyleMaskMiniaturizable)
        else:
            rect = frame
            style = NSWindowStyleMaskBorderless
        self.win = _KeyWindow.alloc().initWithContentRect_styleMask_backing_defer_(rect, style, NSBackingStoreBuffered, False)
        self.win.setTitle_("ClaudeOS")
        self.win.setReleasedWhenClosed_(False)
        self._win_delegate = _WindowDelegate.alloc().initWithScreen_(self)
        self.win.setDelegate_(self._win_delegate)
        root = NSView.alloc().initWithFrame_(NSMakeRect(0, 0, rect.size.width, rect.size.height))
        root.setAutoresizingMask_(NSViewWidthSizable | NSViewHeightSizable)
        self.win.setContentView_(root)

        # --- conversation layer
        chat_cfg = WKWebViewConfiguration.alloc().init()
        chat_cfg.setWebsiteDataStore_(WKWebsiteDataStore.nonPersistentDataStore())
        chat_ucc = WKUserContentController.alloc().init()
        self._chat_messages = _Messages.alloc().initWithCallback_(self._on_chat_message)
        chat_ucc.addScriptMessageHandler_name_(self._chat_messages, "claudeos")
        chat_ucc.addUserScript_(WKUserScript.alloc().initWithSource_injectionTime_forMainFrameOnly_(
            FONTS_JS, INJECT_AT_DOCUMENT_START, True))
        chat_cfg.setUserContentController_(chat_ucc)
        self.chat = WKWebView.alloc().initWithFrame_configuration_(root.bounds(), chat_cfg)
        self.chat.setAutoresizingMask_(NSViewWidthSizable | NSViewHeightSizable)
        self._nav = _NavDelegate.alloc().initWithScreen_(self)
        self.chat.setNavigationDelegate_(self._nav)
        root.addSubview_(self.chat)
        self._load_chat_page()

        # --- content layer: a real browser with a persistent profile
        cfg = WKWebViewConfiguration.alloc().init()
        cfg.setWebsiteDataStore_(WKWebsiteDataStore.defaultDataStore())
        cfg.setMediaTypesRequiringUserActionForPlayback_(0)       # videos may start by themselves
        try:
            cfg.preferences().setJavaScriptCanOpenWindowsAutomatically_(False)
            cfg.preferences().setElementFullscreenEnabled_(True)
        except Exception:
            pass
        ucc = WKUserContentController.alloc().init()
        self.world = WKContentWorld.worldWithName_(WORLD_NAME)
        ucc.addUserScript_(WKUserScript.alloc().initWithSource_injectionTime_forMainFrameOnly_inContentWorld_(
            asset("content.js"), INJECT_AT_DOCUMENT_END, True, self.world))
        self._page_messages = _Messages.alloc().initWithCallback_(self._on_page_message)
        ucc.addScriptMessageHandler_contentWorld_name_(self._page_messages, self.world, "claudeosPage")
        cfg.setUserContentController_(ucc)
        self.content = WKWebView.alloc().initWithFrame_configuration_(root.bounds(), cfg)
        self.content.setAutoresizingMask_(NSViewWidthSizable | NSViewHeightSizable)
        self.content.setNavigationDelegate_(self._nav)
        self._ui_delegate = _UIDelegate.alloc().initWithScreen_(self)
        self.content.setUIDelegate_(self._ui_delegate)
        self._downloads = _Downloads.alloc().initWithScreen_(self)
        self.content.setHidden_(True)
        root.addSubview_(self.content)

        # --- captions over websites and videos
        self.cap_box = NSView.alloc().initWithFrame_(NSMakeRect(0, 0, 100, 40))
        self.cap_box.setWantsLayer_(True)
        self.cap_you = NSTextField.labelWithString_("")
        self.cap_ai = NSTextField.labelWithString_("")
        for lbl in (self.cap_you, self.cap_ai):
            lbl.setLineBreakMode_(NSLineBreakByWordWrapping)
            lbl.setMaximumNumberOfLines_(0)
            lbl.setSelectable_(False)
            self.cap_box.addSubview_(lbl)
        self.cap_box.setHidden_(True)
        root.addSubview_(self.cap_box)

        self._key_monitor = NSEvent.addLocalMonitorForEventsMatchingMask_handler_(NSEventMaskKeyDown, self._on_key_event)
        self._setup_media_keys()
        self._apply_layout()
        self.win.makeKeyAndOrderFront_(None)
        self._present()
        NSCursor.setHiddenUntilMouseMoves_(True)

    def _load_chat_page(self) -> None:
        self.chat.loadFileURL_allowingReadAccessToURL_(NSURL.fileURLWithPath_(str(ASSETS / "chat.html")),
                                                       NSURL.fileURLWithPath_(str(ASSETS)))

    def _present(self) -> None:
        """While ClaudeOS is in front, the Dock and the menu bar stay out of sight."""
        if self.windowed:
            return
        try:
            _app().setPresentationOptions_(NSApplicationPresentationHideDock | NSApplicationPresentationHideMenuBar)
        except Exception:
            log.exception("hiding the Dock and menu bar")

    def _setup_media_keys(self) -> None:
        """AirPods presses (and the keyboard's play key) reach ClaudeOS as media commands."""
        try:
            from MediaPlayer import MPNowPlayingInfoCenter, MPRemoteCommandCenter  # type: ignore
            cc = MPRemoteCommandCenter.sharedCommandCenter()

            def handler(key: str) -> Callable[[Any], int]:
                def h(event: Any) -> int:
                    if self.loop:
                        self.on_media_key(key)
                    return 0          # MPRemoteCommandHandlerStatusSuccess
                return h
            for cmd, key in ((cc.togglePlayPauseCommand(), "XF86AudioPlay"), (cc.playCommand(), "XF86AudioPlay"),
                             (cc.pauseCommand(), "XF86AudioPause"), (cc.nextTrackCommand(), "XF86AudioNext"),
                             (cc.previousTrackCommand(), "XF86AudioPrev")):
                cmd.setEnabled_(True)
                cmd.addTargetWithHandler_(handler(key))
            center = MPNowPlayingInfoCenter.defaultCenter()
            center.setNowPlayingInfo_({"title": "ClaudeOS"})
            center.setPlaybackState_(1)      # playing: makes ClaudeOS the app the AirPods talk to
        except Exception:
            log.info("AirPods presses aren't available", exc_info=True)

    # ---------------------------------------------------------------- stepping aside (switch to macOS)
    def step_aside(self) -> None:
        self._ui(self._do_step_aside)

    def _do_step_aside(self) -> None:
        self._show_chat()
        _app().hide_(None)

    def come_back(self) -> None:
        self._ui(self._do_come_back)

    def _do_come_back(self) -> None:
        app = _app()
        if app.isHidden():
            app.unhide_(None)
        _bring_to_front(app, self.win)
        self._present()

    def _quit_requested(self) -> int:
        """macOS asks whether ClaudeOS may quit (main thread)."""
        if self.on_quit is None:
            return TERMINATE_NOW
        if not self._quitting:
            self._quitting = True
            replied = []

            def done() -> None:            # any thread; only the first call counts
                if replied:
                    return
                replied.append(True)
                AppHelper.callAfter(lambda: _app().replyToApplicationShouldTerminate_(True))
            self.on_quit(done)
            # If stopping hangs, quit anyway. A thread's timer, because while macOS waits for the reply the
            # run loop only runs in the modal-panel mode, where callLater's timer never fires.
            timer = threading.Timer(10.0, done)
            timer.daemon = True
            timer.start()
        return TERMINATE_LATER

    # ---------------------------------------------------------------- text size and line width
    def _compute_layout(self) -> Layout:
        s = self.settings
        scr = self.win.screen() or NSScreen.mainScreen()
        f = scr.frame()
        view = self.win.contentView().bounds()
        mm_w = mm_h = 0.0
        try:
            import Quartz  # type: ignore
            num = scr.deviceDescription()["NSScreenNumber"]
            size = Quartz.CGDisplayScreenSize(int(num))
            mm_w, mm_h = float(size.width), float(size.height)
        except Exception:
            pass
        info = screen_from_monitor(view.size.width, view.size.height, f.size.width, f.size.height, mm_w, mm_h,
                                   float(scr.backingScaleFactor() or 1.0), diagonal_in=s.screen_inches)
        return compute(info, distance_cm=s.viewing_distance_cm, text_px=s.text_size, text_scale=s.text_scale,
                       web_zoom=s.web_zoom)

    def _apply_layout(self) -> None:
        lay = self._compute_layout()
        changed = self.layout is None or lay.to_dict() != self.layout.to_dict()
        self.layout = lay
        self._apply_colors()
        if not changed:
            self._layout_captions()
            return
        self._chat_run(f"CO.layout({lay.font_px},{lay.column_px})", "layout")
        self._apply_content_layout()
        self._layout_captions()
        log.info("Layout: %s", lay.describe())

    def _apply_colors(self) -> None:
        c = DARK if self.settings.theme == "dark" else LIGHT
        self.win.setBackgroundColor_(_color(c["bg"]))
        layer = self.cap_box.layer()
        if layer is not None:
            layer.setBackgroundColor_(_color(c["bg"]).CGColor())
            layer.setBorderColor_(_color(c["rule"]).CGColor())
            layer.setBorderWidth_(1.0)
        size = self.layout.font_px if self.layout else 16
        self.cap_you.setTextColor_(_color(c["soft"]))
        self.cap_ai.setTextColor_(_color(c["ink"]))
        for lbl in (self.cap_you, self.cap_ai):
            lbl.setFont_(NSFont.systemFontOfSize_(size))

    def _apply_content_layout(self) -> None:
        """ClaudeOS's own pages get the computed size; websites get the computed zoom."""
        lay = self.layout
        if lay is None:
            return
        uri = self._content_uri() or ""
        if uri in self.native_uris:
            self.content.setPageZoom_(1.0)
            self.content.evaluateJavaScript_inFrame_inContentWorld_completionHandler_(
                SET_VARS_JS.format(size=lay.font_px, column=lay.column_px), None, self.world, None)
        else:
            self.content.setPageZoom_(lay.web_zoom)

    def _layout_captions(self) -> None:
        lay = self.layout
        if lay is None:
            return
        bounds = self.win.contentView().bounds()
        col = min(lay.column_px, bounds.size.width)
        pad_v, pad_h = round(lay.font_px * 0.5), round(lay.font_px * 0.75)
        inner = max(50.0, col - 2 * pad_h)
        heights = []
        for lbl in (self.cap_you, self.cap_ai):
            lbl.setPreferredMaxLayoutWidth_(inner)
            heights.append(lbl.sizeThatFits_((inner, 10000)).height if lbl.stringValue() and not lbl.isHidden() else 0)
        total = sum(heights) + 2 * pad_v + (4 if all(heights) else 0)
        x = round((bounds.size.width - col) / 2)
        self.cap_box.setFrame_(NSMakeRect(x, round(lay.font_px * 1.5), col, total))
        y = pad_v
        for lbl, h in ((self.cap_ai, heights[1]), (self.cap_you, heights[0])):   # bottom-up: Claude below you
            lbl.setFrame_(NSMakeRect(pad_h, y, inner, h))
            y += h + (4 if h else 0)

    def _schedule_relayout(self) -> None:
        if self._relayout_pending:
            return
        self._relayout_pending = True

        def run() -> None:
            self._relayout_pending = False
            try:
                self._apply_layout()
            except Exception:
                log.exception("Layout failed")
        AppHelper.callLater(0.15, run)

    def layout_info(self) -> dict[str, Any]:
        lay = self.layout
        if lay is None:
            return {}
        d = lay.to_dict()
        d["summary"] = lay.describe()
        return d

    # ---------------------------------------------------------------- thread helpers
    def _ui(self, fn: Callable, *args: Any) -> None:
        def run() -> None:
            try:
                fn(*args)
            except Exception:
                log.exception("UI call failed")
        AppHelper.callAfter(run)

    async def _ui_call(self, fn: Callable, *args: Any) -> Any:
        assert self.loop is not None
        loop = self.loop
        fut = loop.create_future()

        def run() -> None:
            try:
                res = fn(*args)
                loop.call_soon_threadsafe(lambda: fut.done() or fut.set_result(res))
            except Exception as exc:
                # Bind it now: Python unbinds `exc` when the except block ends, before the loop runs this.
                loop.call_soon_threadsafe(lambda e=exc: fut.done() or fut.set_exception(e))
        AppHelper.callAfter(run)
        return await fut

    # ---------------------------------------------------------------- the conversation page
    def _chat_js(self, script: str, kind: str = "log") -> None:
        self._ui(self._chat_run, script, kind)

    def _chat_run(self, script: str, kind: str = "log") -> None:
        """Run a script on the conversation page (main thread). kind says how to rebuild the page later:
        "log" is part of the conversation, "transient" isn't kept, anything else keeps only the latest."""
        if kind == "log":
            self._replay.append(script)
        elif kind != "transient":
            self._replay_last[kind] = script
        if self.chat_ready:
            self.chat.evaluateJavaScript_completionHandler_(script, None)
        else:
            self.chat_backlog.append(script)

    def _on_chat_message(self, name: str, body: Any) -> None:
        self.chat_ready = True
        if self._reloading:
            self._reloading = False
            self.chat_backlog.clear()
            last = self._replay_last
            scripts = [last[k] for k in ("settings", "layout") if k in last] + list(self._replay)
            scripts += [last["state"]] if "state" in last else []
        else:
            scripts, self.chat_backlog = self.chat_backlog, []
        for s in scripts:
            self.chat.evaluateJavaScript_completionHandler_(s, None)

    def _on_renderer_stopped(self, view: Any) -> None:
        """A page's web process stopped (it crashed or used too much memory): load the page again."""
        now = time.monotonic()
        self._renderer_stops = [t for t in self._renderer_stops if now - t < 120] + [now]
        which = "conversation" if view == self.chat else "content"
        log.error("The %s page's renderer stopped; loading it again", which)
        if len(self._renderer_stops) > 5:
            log.error("Pages keep failing to draw; restarting ClaudeOS")
            self.relaunch_after_quit = True
            _app().terminate_(None)
            return
        if which == "conversation":
            self.chat_ready = False
            self._reloading = True
            self.chat_backlog.clear()
            AppHelper.callLater(0.3, self._load_chat_page)
        else:
            AppHelper.callLater(0.3, self.content.reload)

    # ---------------------------------------------------------------- content layer
    def _content_uri(self) -> str | None:
        u = self.content.URL()
        return str(u.absoluteString()) if u is not None else None

    def _show_content(self) -> None:
        self.mode = "content"
        self.content.setHidden_(False)
        self.chat.setHidden_(True)
        self.win.makeFirstResponder_(self.content)

    def _show_chat(self) -> None:
        self.mode = "chat"
        self.chat.setHidden_(False)
        self.content.setHidden_(True)
        self.cap_box.setHidden_(True)
        self.win.makeFirstResponder_(self.chat)

    def _info(self) -> dict[str, Any]:
        title = self.content.title()
        return {"mode": self.mode, "url": self._content_uri(), "title": str(title) if title is not None else None}

    def _resolve_loads(self, ok: bool, error: str | None = None) -> None:
        info = self._info()
        info["ok"] = ok
        if error:
            info["error"] = error
        waiters, self.load_waiters = self.load_waiters, []
        for fut in waiters:
            self.loop.call_soon_threadsafe(lambda f=fut: f.done() or f.set_result(dict(info)))

    def _on_content_committed(self) -> None:
        if self.layout is not None:
            native = (self._content_uri() or "") in self.native_uris
            self.content.setPageZoom_(1.0 if native else self.layout.web_zoom)

    def _on_content_finished(self) -> None:
        if self.layout is not None and (self._content_uri() or "") in self.native_uris:
            self._apply_content_layout()
        err, self.load_error = self.load_error, None
        tries = {"n": 0}

        def settle() -> None:
            # The page title can arrive a moment after the load finishes.
            tries["n"] += 1
            if not self.content.title() and tries["n"] < 6 and err is None:
                AppHelper.callLater(0.15, settle)
                return
            self._resolve_loads(err is None, err)
        settle()

    def _on_content_failed(self, error: Any) -> None:
        code = int(error.code()) if error is not None else 0
        if code in (-999, 102):          # cancelled, or turned into a download
            return
        msg = str(error.localizedDescription()) if error is not None else "the page didn't load"
        self.load_error = None
        self._resolve_loads(False, msg)

    def _on_page_message(self, name: str, body: Any) -> None:
        try:
            msg = json.loads(str(body))
        except Exception:
            return
        if msg.get("type") == "media":
            playing = bool(msg.get("playing"))
            if self.loop:
                self.loop.call_soon_threadsafe(self.on_media_state, playing)

    def _load(self, url: str) -> bool:
        if url.startswith("file://"):
            u = NSURL.URLWithString_(url)
            if u is None:
                return False
            path = os.path.realpath(str(u.path()))       # no way around the check with ".."
            # Pages ClaudeOS made (in its own folder) show your photos, videos and music from wherever they
            # are, as on a PC, so they may read any file; anything else may read only its own folder.
            own = os.path.realpath(str(self.paths.root)) + os.sep
            root = "/" if path.startswith(own) else os.path.dirname(path)
            self.content.loadFileURL_allowingReadAccessToURL_(u, NSURL.fileURLWithPath_(root))
            return True
        u = NSURL.URLWithString_(url)
        if u is None:
            return False
        self.content.loadRequest_(NSURLRequest.requestWithURL_(u))
        return True

    async def show_url(self, url: str) -> dict[str, Any]:
        assert self.loop is not None
        fut = self.loop.create_future()

        def go() -> bool:
            self.load_waiters.append(fut)
            self.load_error = None
            if not self._load(url):
                self.load_waiters.remove(fut)
                return False
            self._show_content()
            return True
        if not await self._ui_call(go):
            return {"ok": False, "error": f"That isn't a web address I can open: {url}", "mode": self.mode}
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
        loop = self.loop
        fut = loop.create_future()

        def done(result: Any, error: Any) -> None:
            if error is not None:
                out: Any = {"error": str(error.localizedDescription())}
            else:
                try:
                    out = json.loads(str(result)) if result is not None else None
                except Exception:
                    out = None
            loop.call_soon_threadsafe(lambda: fut.done() or fut.set_result(out))

        wrapped = f"JSON.stringify(({script}))"
        self._ui(lambda: self.content.evaluateJavaScript_inFrame_inContentWorld_completionHandler_(
            wrapped, None, self.world, done))
        try:
            return await asyncio.wait_for(fut, 12)
        except asyncio.TimeoutError:
            return {"error": "The page didn't respond."}

    async def navigate(self, action: str) -> dict[str, Any]:
        assert self.loop is not None
        fut = self.loop.create_future()

        def go() -> bool:
            v = self.content
            if action == "back" and v.canGoBack():
                self.load_waiters.append(fut)
                v.goBack()
                return True
            if action == "forward" and v.canGoForward():
                self.load_waiters.append(fut)
                v.goForward()
                return True
            if action == "reload":
                self.load_waiters.append(fut)
                v.reload()
                return True
            if action == "stop":
                v.stopLoading()
            return False
        if await self._ui_call(go):
            try:
                return await asyncio.wait_for(asyncio.shield(fut), 15)
            except asyncio.TimeoutError:
                pass
        return await self._ui_call(self._info)

    async def focus_content(self) -> None:
        await self._ui_call(lambda: self.win.makeFirstResponder_(self.content))

    async def content_origin(self) -> tuple[int, int, float]:
        """Where the page's top-left corner is on the screen, in screen pixels (Quartz is top-left based too)."""
        def calc() -> tuple[int, int, float]:
            scale = float(self.win.backingScaleFactor() or 1.0)
            r = self.win.contentRectForFrameRect_(self.win.frame())
            main_h = NSScreen.screens()[0].frame().size.height
            top = main_h - (r.origin.y + r.size.height)
            return int(r.origin.x * scale), int(top * scale), scale
        return await self._ui_call(calc)

    def snapshot_png(self) -> bytes | None:
        """A picture of what the window shows, in screen pixels (called from a worker thread). WebKit's own
        snapshot needs no Screen Recording permission."""
        box: dict[str, Any] = {}
        ev = threading.Event()

        def take() -> None:
            view = self.content if self.mode == "content" else self.chat

            def done(image: Any, error: Any) -> None:
                try:
                    if image is not None:
                        rep = NSBitmapImageRep.imageRepWithData_(image.TIFFRepresentation())
                        box["png"] = bytes(rep.representationUsingType_properties_(NSBitmapImageFileTypePNG, {}))
                        box["width_px"] = int(view.bounds().size.width * (self.win.backingScaleFactor() or 1.0))
                finally:
                    ev.set()
            try:
                view.takeSnapshotWithConfiguration_completionHandler_(None, done)
            except Exception:
                ev.set()
        AppHelper.callAfter(take)
        if not ev.wait(10) or "png" not in box:
            return None
        png = box["png"]
        try:
            from PIL import Image
            im = Image.open(io.BytesIO(png))
            if box.get("width_px") and im.width != box["width_px"]:
                f = box["width_px"] / im.width
                im = im.resize((box["width_px"], max(1, round(im.height * f))))
                out = io.BytesIO()
                im.save(out, "PNG")
                png = out.getvalue()
        except Exception:
            pass
        return png

    # ---------------------------------------------------------------- the conversation
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
            if not self.flush_scheduled:
                self.flush_scheduled = True
                AppHelper.callLater(0.04, self._flush_ai)
        self._ui(add)
        if self.cap_tid != msg_id:
            self.cap_tid = msg_id
            self.cap_ai_text = ""
        self.cap_ai_text += text
        if text:
            self._caption(ai=_tail(self.cap_ai_text))

    def _flush_ai(self) -> None:
        self.flush_scheduled = False
        pending, self.pending_ai = self.pending_ai, {}
        for mid, txt in pending.items():
            self._chat_run(f"CO.ai({json.dumps(mid)},{json.dumps(txt)})" if txt else f"CO.aiStart({json.dumps(mid)})")

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
            self._ui(lambda: self._hide_captions_after(2.5))

    def history(self, items: list[dict[str, Any]]) -> None:
        self._chat_js(f"CO.history({json.dumps(items)})")

    def apply_settings(self, theme: str, text_size: int, user_name: str) -> None:
        self._chat_js(f"CO.settings({json.dumps(theme)},{json.dumps(user_name)})", "settings")
        self._ui(self._apply_layout)

    # captions over content
    def _caption(self, you: str | None = None, ai: str | None = None) -> None:
        def run() -> None:
            if you is not None:
                self.cap_you.setStringValue_(you)
                self.cap_you.setHidden_(not you)
            if ai is not None:
                self.cap_ai.setStringValue_(ai)
                self.cap_ai.setHidden_(not ai)
            show = self.mode == "content" and bool(self.cap_you.stringValue() or self.cap_ai.stringValue())
            self._layout_captions()
            self.cap_box.setHidden_(not show)
            self._hide_captions_after(8)
        self._ui(run)

    def _hide_captions_after(self, seconds: float) -> None:
        self.cap_hide_at = time.monotonic() + seconds

        def check() -> None:
            if time.monotonic() >= self.cap_hide_at - 0.05:
                self.cap_box.setHidden_(True)
        AppHelper.callLater(seconds, check)

    # ---------------------------------------------------------------- keys
    def _on_key_event(self, event: Any) -> Any:
        try:
            if self.mode != "chat":
                return event                  # keys go to the website (forms, sign-ins)
            flags = int(event.modifierFlags())
            if flags & (NSEventModifierFlagCommand | NSEventModifierFlagControl):
                return event                  # Cmd+H, Cmd+Q and other shortcuts
            code = int(event.keyCode())
            # A keyboard is optional: typing on the conversation screen sends a message.
            if code in (KEY_RETURN, KEY_ENTER):
                text, self.typed = self.typed.strip(), ""
                self._chat_run("CO.partial('')", "transient")
                if text and self.loop:
                    self.loop.call_soon_threadsafe(self.on_typed, text)
                return None
            if code == KEY_BACKSPACE:
                self.typed = self.typed[:-1]
            elif code == KEY_ESCAPE:
                self.typed = ""
            else:
                chars = "".join(c for c in str(event.characters() or "") if c.isprintable())
                if not chars:
                    return event
                self.typed += chars
            self._chat_run(f"CO.partial({json.dumps(self.typed)})", "transient")
            return None
        except Exception:
            log.exception("key handling")
            return event


def _tail(text: str, n: int = 170) -> str:
    t = " ".join(text.split())
    if len(t) <= n:
        return t
    cut = t[-n:]
    sp = cut.find(" ")
    return "…" + (cut[sp + 1:] if 0 <= sp < 30 else cut)


def run_app(paths: Paths, settings: Settings, controller_factory: Callable[[MacScreen], Any], windowed: bool = False) -> int:
    """Start AppKit on this thread and ClaudeOS's logic on a background asyncio loop."""
    app = NSApplication.sharedApplication()
    app.setActivationPolicy_(NSApplicationActivationPolicyRegular)
    _install_menu(app)
    screen = MacScreen(paths, settings, windowed=windowed)
    delegate = _AppDelegate.alloc().initWithScreen_(screen)
    app.setDelegate_(delegate)
    loop = asyncio.new_event_loop()
    t = threading.Thread(target=loop.run_forever, name="claudeos-loop", daemon=True)
    t.start()
    controller = controller_factory(screen)
    screen.bind(loop, on_media_key=controller.media_key, on_media_state=controller.media_state,
                on_typed=lambda text: loop.create_task(controller.handle(text)))
    screen.on_hidden = lambda: loop.call_soon_threadsafe(controller.went_aside)
    screen.on_shown = lambda: loop.call_soon_threadsafe(controller.wake)
    platform = getattr(controller, "platform", None)
    if platform is not None and hasattr(platform, "snapshot_provider"):
        platform.snapshot_provider = screen.snapshot_png

    def quit_cleanly(done: Callable[[], None]) -> None:
        """Stops ClaudeOS (saving the conversation, closing the microphone), then lets macOS quit the app.
        macOS ends the process itself, so nothing after the run loop runs: this is the clean-up."""
        def finished(_f: Any = None) -> None:
            try:
                loop.call_soon_threadsafe(loop.stop)
            except RuntimeError:
                pass
            if screen.relaunch_after_quit:
                _relaunch()
            logging.shutdown()
            for stream in (sys.stdout, sys.stderr):
                try:
                    stream.flush()
                except Exception:
                    pass
            done()
        try:
            asyncio.run_coroutine_threadsafe(controller.stop(), loop).add_done_callback(finished)
        except RuntimeError:          # the loop has already stopped
            finished()
    screen.on_quit = quit_cleanly

    def started(f: Any) -> None:
        exc = f.exception()
        if exc:
            log.error("ClaudeOS failed to start: %s", exc, exc_info=exc)
            AppHelper.callAfter(lambda: _app().terminate_(None))
    asyncio.run_coroutine_threadsafe(controller.start(), loop).add_done_callback(started)
    _bring_to_front(app, screen.win)
    AppHelper.runEventLoop(installInterrupt=True)
    return controller.exit_code or 0


def _relaunch() -> None:
    """Opens ClaudeOS again a moment after this copy has quit (used when its pages keep failing to draw)."""
    bundle = NSBundle.mainBundle().bundlePath()
    if not str(bundle).endswith(".app"):
        return
    subprocess.Popen(["/bin/sh", "-c", 'sleep 3; exec /usr/bin/open -n -a "$0"', str(bundle)],
                     start_new_session=True, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL)


def _bring_to_front(app: Any, win: Any) -> None:
    """Since macOS 14 an app can only ask to come to the front; ordering its window to the front still works,
    and asking to be active gives it the keyboard when macOS agrees."""
    win.orderFrontRegardless()
    win.makeKeyWindow()
    try:
        app.activate()                       # macOS 14 and later
    except AttributeError:
        app.activateIgnoringOtherApps_(True)
