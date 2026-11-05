"""The touchscreen in a real Chat Computer window (GTK + WebKit, on a virtual display): the window's side of the iPad's
gestures (ui/touch.py) and the conversation page's (chat.js).

A virtual display has no touchscreen, so the window's touches are made here: stand-ins for GTK's touch events,
given to the window's touch handling as GTK would give them, with what it hands on to GTK recorded instead of
delivered. The page's are DOM touch events made in the page (see PAGE_JS). Prints JSON.
Run:  xvfb-run -a -s "-screen 0 1600x900x24" python tests/screen_touch_check.py
"""
import asyncio
import json
import os
import sys
import tempfile
import threading
import time
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
os.environ["CHATCOMPUTER_HOME"] = tempfile.mkdtemp(prefix="chatcomputer-touch-")
os.environ["CHATCOMPUTER_W"], os.environ["CHATCOMPUTER_H"] = "1600", "900"

import gi  # noqa: E402

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, GLib, Gtk  # noqa: E402

from chatcomputer.config import Paths, Settings  # noqa: E402
from chatcomputer.ui.gtk_app import GtkScreen, choose_rendering  # noqa: E402

B, U, E, C = (Gdk.EventType.TOUCH_BEGIN, Gdk.EventType.TOUCH_UPDATE, Gdk.EventType.TOUCH_END,
              Gdk.EventType.TOUCH_CANCEL)
paths = Paths().ensure()
choose_rendering()
screen = GtkScreen(paths, Settings(), windowed=True)
screen.apply_settings(0, "Sam", "Sonnet 5.5")
loop = asyncio.new_event_loop()
threading.Thread(target=loop.run_forever, daemon=True).start()
screen.bind(loop, on_media_key=lambda k: None, on_media_state=lambda p: None, on_typed=lambda t: None)
now = time.time()
screen.history([{"role": "user", "id": "u1", "ts": now - 60, "text": "Hello there"},
                {"role": "assistant", "ts": now - 50, "text": "Hi Sam. The weather is lovely today."}])
screen.messages_list([{"id": "m1", "source": "email", "label": "Gmail", "who": "Jane", "text": "Lunch?", "ts": now - 30,
                       "unread": True}], [])
delivered: list = []
real_deliver = screen._deliver


def deliver(e):
    """What the window hands on to GTK: the stand-ins' recorded; real events (drawing, the window) delivered."""
    if isinstance(e, Touch):
        delivered.append([e.seq, e.touch.type.value_nick])
    else:
        real_deliver(e)
events, keys, keyboards, closed, edits = [], [], [], [], []
screen.on_page_event = events.append
screen.on_keyboard_action = keys.append
screen.on_keyboard = keyboards.append
screen.on_closed = lambda: closed.append(True)
screen.chat.execute_editing_command = edits.append
screen._deliver = deliver
site = Path(tempfile.mkdtemp()) / "site.html"
site.write_text("<!doctype html><title>A website</title><h1>A website</h1>")
out: dict = {}


class Touch:
    """A stand-in for one of GTK's touch events (what the window's touch handling uses of it)."""

    def __init__(self, seq, kind, x, y):
        self.seq, self.x, self.y = seq, x, y
        self.touch = SimpleNamespace(type=kind)

    def get_event_sequence(self):
        return self.seq

    def get_root_coords(self):
        gw = screen.win.get_window()
        ox, oy = gw.get_origin()[-2:]
        return True, self.x + ox, self.y + oy

    def copy(self):
        return Touch(self.seq, self.touch.type, self.x, self.y)


def ev(seq, kind, x, y):
    """One touch event; True if the page doesn't get it."""
    return screen._touch_event(Touch(seq, kind, x, y), kind)


def finger(seq, path):
    """A finger down at path[0], moved along it, lifted at its end: what the page didn't get, step by step."""
    taken = [ev(seq, B, *path[0])]
    taken += [ev(seq, U, x, y) for x, y in path[1:]]
    taken.append(ev(seq, E, *path[-1]))
    return taken


def window_tests() -> None:
    w, h = screen.win.get_allocated_width(), screen.win.get_allocated_height()
    out["size"] = [w, h]
    out["tap"] = finger(1, [(800, 450)])
    out["tap_delivered"] = list(delivered)
    delivered.clear()
    out["edge_tap"] = finger(2, [(800, h - 4)])
    out["edge_tap_delivered"] = list(delivered)       # held back, then given to the page
    delivered.clear()
    # Three fingers pinched closed: copy, in the conversation (the page had the first two: cancelled there).
    ev(10, B, 600, 300)
    ev(11, B, 900, 300)
    out["third"] = ev(12, B, 750, 600)
    out["cancelled"] = list(delivered)
    for s, (x, y) in ((10, (740, 420)), (11, (760, 420)), (12, (750, 460))):
        ev(s, U, x, y)
    for s, (x, y) in ((10, (740, 420)), (11, (760, 420)), (12, (750, 460))):
        ev(s, E, x, y)
    out["edits"] = list(edits)
    delivered.clear()
    # Home from the bottom edge, with a website open.
    asyncio.run_coroutine_threadsafe(screen.show_url(site.as_uri()), loop)
    GLib.timeout_add(2000, lambda: home(w, h) and False)


def home(w, h) -> None:
    out["before_home"] = screen.mode
    out["home"] = finger(3, [(800, h - 5), (800, h - 80), (800, h - 200)])
    out["after_home"] = screen.mode
    keyboard_tests(w, h)


def key_centre(label):
    k = next(k for k in screen.keyboard_layer.kb.keys if k.label == label)
    px, py = screen.keyboard_layer.panel[:2]
    return px + k.x + k.w / 2, py + k.y + k.h / 2


def keyboard_tests(w, h) -> None:
    screen.touch_keyboard(True)
    out["keyboard_up"] = screen.keyboard_layer.up
    out["keyboard_spec"] = keyboards[-1] is not None if keyboards else False
    q = key_centre("q")
    out["key_q"] = finger(20, [q])
    finger(21, [key_centre("w"), key_centre("e")])       # slid from w to e before lifting: e
    out["still_up"] = screen.keyboard_layer.up
    hide = next(k for k in screen.keyboard_layer.kb.keys if k.action == ("hide",))
    px, py = screen.keyboard_layer.panel[:2]
    finger(22, [(px + hide.x + hide.w / 2, py + hide.y + hide.h / 2)])
    out["hidden_by_key"] = not screen.keyboard_layer.up
    # A real keyboard's key puts it away; one Chat Computer typed (XTEST) doesn't.
    screen.touch_keyboard(True)

    def key_event(name):
        return SimpleNamespace(get_source_device=lambda: SimpleNamespace(get_name=lambda: name))
    screen._note_key(key_event("Virtual core XTEST keyboard"))
    out["up_after_xtest"] = screen.keyboard_layer.up
    screen._note_key(key_event("AT Translated Set 2 keyboard"))
    out["up_after_real_key"] = screen.keyboard_layer.up
    screen.touch_keyboard(True)
    out["refused_with_real_keyboard"] = not screen.keyboard_layer.up
    screen._real_key_at = -1e9
    GLib.timeout_add(300, lambda: page_tests() and False)


# WebKitGTK doesn't let a page make Touch objects ("Illegal constructor"), so the page's touch events are events of
# those names carrying the same fields (touches, changedTouches: identifier, target, clientX, clientY).
PAGE_JS = r"""(function () {
  function t(target, id, x, y) { return {identifier: id, target: target, clientX: x, clientY: y, pageX: x, pageY: y}; }
  function fire(type, target, touches, changed) {
    var e = new Event(type, {bubbles: true, cancelable: true});
    Object.defineProperty(e, "touches", {value: touches});
    Object.defineProperty(e, "targetTouches", {value: touches});
    Object.defineProperty(e, "changedTouches", {value: changed || touches});
    return target.dispatchEvent(e);
  }
  function centre(el) { var r = el.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; }
  window.__T = {t: t, fire: fire, centre: centre};
  var box = document.getElementById('input'), c = centre(box);
  fire('touchstart', box, [t(box, 1, c[0], c[1])]); fire('touchend', box, [], [t(box, 1, c[0], c[1])]);
  return 1;
})()"""

PINCH_JS = r"""(function () {
  var T = window.__T, log = document.getElementById('log'), c = T.centre(log), x = c[0], y = c[1];
  var a = T.t(log, 1, x - 50, y), b = T.t(log, 2, x + 50, y);
  T.fire('touchstart', log, [a, b]);
  var size0 = getComputedStyle(document.documentElement).fontSize;
  var a2 = T.t(log, 1, x - 75, y), b2 = T.t(log, 2, x + 75, y);
  var notPrevented = T.fire('touchmove', log, [a2, b2]);
  var size1 = getComputedStyle(document.documentElement).fontSize;
  T.fire('touchend', log, [], [a2, b2]);
  return JSON.stringify({size0: size0, size1: size1, zoomBlocked: !notPrevented});
})()"""

SWIPE_JS = r"""(function () {
  var T = window.__T, li = document.querySelector('#messages-list .item'), c = T.centre(li);
  T.fire('touchstart', li, [T.t(li, 1, c[0], c[1])]);
  T.fire('touchmove', li, [T.t(li, 1, c[0] - 30, c[1] + 2)]);
  T.fire('touchmove', li, [T.t(li, 1, c[0] - 90, c[1] + 3)]);
  T.fire('touchend', li, [], [T.t(li, 1, c[0] - 90, c[1] + 3)]);
  var pin = li.querySelector('.pin');
  return JSON.stringify({swiped: li.classList.contains('swiped'), label: getComputedStyle(pin, '::after').content,
                         visible: getComputedStyle(pin).visibility, pinRect: T.centre(pin)});
})()"""

HOLD_START_JS = r"""(function () {
  var T = window.__T, m = document.querySelector('#log > .ai'), r = m.getBoundingClientRect();
  window.__held = [r.left + 20, r.top + r.height / 2];
  T.fire('touchstart', m, [T.t(m, 1, window.__held[0], window.__held[1])]);
  return 1;
})()"""

HOLD_END_JS = r"""(function () {
  var T = window.__T, m = document.querySelector('#log > .ai');
  var notPrevented = T.fire('touchend', m, [], [T.t(m, 1, window.__held[0], window.__held[1])]);
  return JSON.stringify({selected: String(window.getSelection()), tapPrevented: !notPrevented});
})()"""

DOUBLE_TAP_JS = r"""(function () {
  window.getSelection().removeAllRanges();
  var T = window.__T, m = document.querySelector('#log > .ai p') || document.querySelector('#log > .ai');
  var range = document.createRange(), node = m.firstChild || m, text = node.textContent, at = text.indexOf('weather');
  range.setStart(node, at + 2); range.setEnd(node, at + 3);
  var r = range.getBoundingClientRect(), x = r.left + r.width / 2, y = r.top + r.height / 2;
  for (var i = 0; i < 2; i++) { T.fire('touchstart', m, [T.t(m, 1, x, y)]); T.fire('touchend', m, [], [T.t(m, 1, x, y)]); }
  return 1;
})()"""

MESSAGES_JS = r"""(function () {
  var T = window.__T, log = document.getElementById('log'), out = {};
  function pull(name) {
    CO.showMessages();
    var shown = document.body.classList.contains('show-messages') && getComputedStyle(document.getElementById('messages')).display;
    var c = T.centre(log);
    T.fire('touchstart', log, [T.t(log, 1, c[0], c[1])]);
    T.fire('touchend', log, [], [T.t(log, 1, c[0], c[1])]);
    out[name] = {shown: shown, hiddenAfter: !document.body.classList.contains('show-messages')};
  }
  CO.toggleFull();                       // the sidebars put away (as a page's host can), the middle filling the screen
  pull('full');
  CO.toggleFull();
  document.body.classList.remove('panel');   // a screen too narrow for them
  pull('narrow');
  return JSON.stringify(out);
})()"""


def js(script, then):
    def done(view, res, *_):
        val = view.evaluate_javascript_finish(res)
        then(val.to_string() if val is not None else None)
    screen.chat.evaluate_javascript(script, -1, None, None, None, done, None)


def page_tests() -> None:
    def after_box(_):
        out["box_touch_keyboard"] = screen.keyboard_layer.up
        js(PINCH_JS, after_pinch)

    def after_pinch(res):
        out["pinch"] = json.loads(res)
        js(SWIPE_JS, after_swipe)

    def after_swipe(res):
        out["swipe"] = json.loads(res)
        out["keyboard_after_touch_elsewhere"] = screen.keyboard_layer.up      # a finger on a sidebar item put it away
        js(HOLD_START_JS, lambda _: GLib.timeout_add(800, lambda: js(HOLD_END_JS, after_hold) and False))

    def after_hold(res):
        out["hold"] = json.loads(res)
        js(DOUBLE_TAP_JS, lambda _: GLib.timeout_add(300, lambda: js("String(window.getSelection())", after_double)
                                                       and False))

    def after_double(res):
        out["double_tap"] = res
        js(MESSAGES_JS, after_messages)

    def after_messages(res):
        out["messages"] = json.loads(res)
        out["page_events"] = [e for e in events if e.get("type") in ("text_scale", "pin")]
        out["keys"] = keys
        out["closed"] = bool(closed)
        print(json.dumps(out), flush=True)
        Gtk.main_quit()
    js(PAGE_JS, lambda _: GLib.timeout_add(400, lambda: after_box(None) and False))


GLib.timeout_add(2500, lambda: window_tests() and False)
GLib.timeout_add(60000, lambda: (print(json.dumps({"timeout": True, **out}), flush=True), Gtk.main_quit()) and False)
Gtk.main()
