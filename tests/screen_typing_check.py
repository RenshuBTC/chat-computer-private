"""The message box on the conversation screen: what's typed goes into it (a real text box, typed into with real
key presses), Shift+Enter starts a new line and the box grows, Enter sends, typing after a click elsewhere still
goes into it, Escape leaves the sidebars (they show from the start) and what's typed be, twice,
and the page reports how long letters took to reach the screen.

Runs a real Chat Computer window (GTK + WebKit) on a virtual display, types with xdotool, and prints JSON.
Run:  xvfb-run -a -s "-screen 0 1280x800x24" python tests/screen_typing_check.py
"""
import asyncio
import json
import os
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
os.environ["CHATCOMPUTER_HOME"] = tempfile.mkdtemp(prefix="chatcomputer-typing-")

import gi  # noqa: E402

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
gi.require_version("GdkX11", "3.0")
from gi.repository import GdkX11, GLib, Gtk  # noqa: E402,F401

from chatcomputer.config import Paths, Settings  # noqa: E402
from chatcomputer.ui.gtk_app import GtkScreen, choose_rendering  # noqa: E402

choose_rendering()
screen = GtkScreen(Paths().ensure(), Settings(), windowed=True)
screen.apply_settings(0, "Sam", "Sonnet 5.5")
screen.mic(True)
loop = asyncio.new_event_loop()
threading.Thread(target=loop.run_forever, daemon=True).start()
sent: list[str] = []
screen.bind(loop, on_media_key=lambda k: None, on_media_state=lambda p: None, on_typed=sent.append)
out: dict = {}

PAGE_JS = """(function(){var i=document.getElementById('input'), b=document.getElementById('box');
  var c=document.getElementById('composer').getBoundingClientRect(), r=b.getBoundingClientRect();
  return JSON.stringify({value: i.value, focused: document.activeElement === i,
    drafting: document.body.classList.contains('drafting'), full: document.body.classList.contains('full'),
    send: getComputedStyle(document.getElementById('send')).backgroundColor,
    box_h: r.height, box_bottom: window.innerHeight - r.bottom,
    below_box: c.bottom - r.bottom, above_box: r.top - c.top,
    wake: getComputedStyle(document.getElementById('wake')).display, hint: i.value === '' ? i.placeholder : null,
    panel: document.body.classList.contains('panel'),
    words: document.getElementById('composer').innerText.replace(/\\s+/g, ' ').trim()});})()"""


def xdo(*args: str) -> None:
    subprocess.run(["xdotool", *args], check=False, timeout=20)


def read(name: str, then) -> None:
    def done(view, res, *_):
        out[name] = json.loads(view.evaluate_javascript_finish(res).to_string())
        then()
    screen.chat.evaluate_javascript(PAGE_JS, -1, None, None, None, done, None)


def later(ms: int, fn) -> None:
    GLib.timeout_add(ms, lambda: fn() and False)


def steps():
    """Each step types (in a thread: xdotool waits while it types) and then reads the page."""
    wid = str(screen.win.get_window().get_xid())
    plan = [
        ("empty", []),
        ("typed", [["windowfocus", "--sync", wid], ["type", "--delay", "40", "hi there"]]),
        ("two_lines", [["key", "shift+Return"], ["type", "--delay", "40", "second line"]]),
        ("sent", [["key", "Return"]]),
        ("after_blur", [["type", "--delay", "40", "abc"]]),
        ("escaped", [["key", "Escape"]]),
        ("escaped_again", [["key", "Escape"]]),
    ]

    def run(i: int) -> None:
        if i == len(plan):
            later(3500, finish)          # the page reports letters' times every 3 seconds
            return
        name, cmds = plan[i]
        if name == "after_blur":
            screen.chat.evaluate_javascript("document.activeElement.blur(); 1", -1, None, None, None, None, None)

        def work() -> None:
            for c in cmds:
                xdo(*c)
            GLib.timeout_add(500, lambda: read(name, lambda: run(i + 1)) and False)
        threading.Thread(target=work, daemon=True).start()
    run(0)


def finish() -> None:
    out["sent_text"] = list(sent)
    out["typed_ms"] = list(screen._speed["keys"])
    print(json.dumps(out))
    Gtk.main_quit()


GLib.timeout_add(3000, lambda: steps() and False)
GLib.timeout_add(60000, lambda: (print(json.dumps({"timeout": True, **out})), Gtk.main_quit()) and False)
Gtk.main()
