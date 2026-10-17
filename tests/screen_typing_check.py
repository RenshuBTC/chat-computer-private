"""Typing on the conversation screen: each letter shows at once, the page says when it was on the screen, and
Enter sends what was typed.

Runs a real ClaudeOS window (GTK + WebKit) on a virtual display, types with real key events, and prints JSON:
what the page showed, how long each letter took to be drawn, and what was sent.
Run:  xvfb-run -a -s "-screen 0 1280x800x24" python tests/screen_typing_check.py
"""
import asyncio
import json
import os
import sys
import tempfile
import threading
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
os.environ["CLAUDEOS_HOME"] = tempfile.mkdtemp(prefix="claudeos-typing-")

import gi  # noqa: E402

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
from gi.repository import Gdk, GLib, Gtk  # noqa: E402

from claudeos.config import Paths, Settings  # noqa: E402
from claudeos.ui.gtk_app import GtkScreen, choose_rendering  # noqa: E402

choose_rendering()
screen = GtkScreen(Paths().ensure(), Settings(), windowed=True)
screen.apply_settings(0, "Ren")
loop = asyncio.new_event_loop()
threading.Thread(target=loop.run_forever, daemon=True).start()
sent: list[str] = []
screen.bind(loop, on_media_key=lambda k: None, on_media_state=lambda p: None, on_typed=sent.append)
out: dict = {}

PAGE_JS = """JSON.stringify({live: document.getElementById('live-text').textContent,
  typing: document.body.classList.contains('typing'), hearing: document.body.classList.contains('hearing'),
  caret: getComputedStyle(document.getElementById('caret')).animationName})"""


def key(name: str) -> bool:
    ev = Gdk.Event.new(Gdk.EventType.KEY_PRESS)
    ev.keyval = Gdk.keyval_from_name(name)
    ev.state = 0
    return screen._on_key(screen.win, ev)


def read(name: str, then) -> None:
    def done(view, res, *_):
        out[name] = json.loads(view.evaluate_javascript_finish(res).to_string())
        then()
    screen.chat.evaluate_javascript(PAGE_JS, -1, None, None, None, done, None)


def type_letters() -> bool:
    for ch in "hi there":
        key("space" if ch == " " else ch)
    GLib.timeout_add(800, lambda: read("typed", press_enter) and False)
    return False


def press_enter() -> None:
    out["painted_ms"] = list(screen._speed["keys"])
    key("Return")
    GLib.timeout_add(600, lambda: read("sent", finish) and False)


def finish() -> None:
    out["sent_text"] = list(sent)
    print(json.dumps(out))
    Gtk.main_quit()


GLib.timeout_add(3000, type_letters)
GLib.timeout_add(60000, lambda: (print(json.dumps({"timeout": True, **out})), Gtk.main_quit()) and False)
Gtk.main()
