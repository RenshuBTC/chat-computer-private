""" "/" in the message box, in a real Chat Computer window: typing "/" first makes the box search this computer (results
in a list above it, matching letters bold); the arrow keys choose, Enter opens one; with nothing found, the only
line asks Claude; Escape ends the search (clearing the box). Anything else typed is a message, as always, and right-clicking the box opens
no menu: Chat Computer is the conversation. Real key presses and clicks (xdotool) on a virtual display; Chat Computer's
answers (the search results) come from here, as the controller would send them. Prints JSON.
Run:  xvfb-run -a -s "-screen 0 1600x900x24" python tests/screen_search_check.py [shot-prefix]
"""
import asyncio
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
os.environ["CHATCOMPUTER_HOME"] = tempfile.mkdtemp(prefix="chatcomputer-search-")
os.environ["CHATCOMPUTER_W"], os.environ["CHATCOMPUTER_H"] = "1600", "900"

import gi  # noqa: E402

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
gi.require_version("GdkX11", "3.0")
from gi.repository import Gdk, GdkX11, GLib, Gtk  # noqa: E402,F401

from chatcomputer.config import Paths, Settings  # noqa: E402
from chatcomputer.ui.gtk_app import GtkScreen, choose_rendering  # noqa: E402

SHOT = sys.argv[1] if len(sys.argv) > 1 else None
paths = Paths().ensure()
REPORT = "/home/sam/Documents/report.pdf"
now = time.time()

choose_rendering()
screen = GtkScreen(paths, Settings(), windowed=True)
screen.apply_settings(0, "Sam", "Sonnet 5.5")
screen.mic(True)
loop = asyncio.new_event_loop()
threading.Thread(target=loop.run_forever, daemon=True).start()
typed: list = []
screen.bind(loop, on_media_key=lambda k: None, on_media_state=lambda p: None, on_typed=typed.append)
events: list = []


def page_event(msg: dict) -> None:
    """What the controller would answer (on its loop's thread)."""
    events.append({k: v for k, v in msg.items() if k != "n"})
    if msg.get("type") == "search":
        q = msg.get("q") or ""
        items = [{"name": "report.pdf", "path": REPORT, "kind": "document", "where": "~/Documents", "when": "Oct 3"}
                 ] if "rep" in q else []
        screen.search_results(msg.get("n"), q, items, "" if items else f"Nothing on this computer matches “{q}”.")


screen.on_page_event = page_event
screen.history([{"role": "user", "id": "u1", "ts": now - 60, "text": "Hello"},
                {"role": "assistant", "ts": now - 50, "text": "Hi Sam. What can I do?"}])
out: dict = {}

STATE_JS = """(function () {
  function rows(sel) { return Array.prototype.map.call(document.querySelectorAll(sel), function (b) {
    return {text: (b.querySelector('.res-text') || b).textContent, cls: b.className}; }); }
  var b = document.getElementById('box').getBoundingClientRect(), r = document.getElementById('results').getBoundingClientRect();
  return JSON.stringify({searching: document.body.classList.contains('searching'),
    menus: Array.prototype.filter.call(document.querySelectorAll('[role=menu], #menu, .mi'),
      function (m) { return getComputedStyle(m).display !== 'none'; }).length,     // shown (a phone's held-message menu is hidden)
    box: [Math.round(b.left), Math.round(b.top), Math.round(b.right), Math.round(b.bottom)],
    results_box: [Math.round(r.left), Math.round(r.top), Math.round(r.right), Math.round(r.bottom)],
    results: rows('#results .res'), note: (document.querySelector('#results .res-note') || {}).textContent || '',
    bold: Array.prototype.map.call(document.querySelectorAll('#results b'), function (e) { return e.textContent; }),
    icon: getComputedStyle(document.getElementById('search-icon')).display,
    value: document.getElementById('input').value, focused: document.activeElement === document.getElementById('input')});
})()"""


def xdo(*args: str) -> None:
    subprocess.run(["xdotool", *args], check=False, timeout=20)


def wid() -> str:
    return str(screen.win.get_window().get_xid())


def read(name: str, then) -> None:
    def done(view, res, *_):
        out[name] = json.loads(view.evaluate_javascript_finish(res).to_string())
        then()
    screen.chat.evaluate_javascript(STATE_JS, -1, None, None, None, done, None)


def shot(name: str) -> None:
    if SHOT:
        w = screen.win.get_window()
        Gdk.pixbuf_get_from_window(w, 0, 0, w.get_width(), w.get_height()).savev(f"{SHOT}-{name}.png", "png", [], [])


def act(cmds: list, then, wait_ms: int = 800) -> None:
    def work() -> None:
        for c in cmds:
            xdo(*c)
        GLib.timeout_add(wait_ms, lambda: then() and False)
    threading.Thread(target=work, daemon=True).start()


def steps() -> None:
    b = out["start"]["box"]
    cx, cy = (b[0] + b[2]) // 2, (b[1] + b[3]) // 2
    plan = [
        ("right_click", [["windowfocus", "--sync", wid()], ["mousemove", "--window", wid(), str(cx), str(cy)],
                         ["click", "3"]]),
        ("search", [["type", "--delay", "60", "/rep"]]),
        ("search_down", [["key", "Down"]]),
        ("search_up", [["key", "Up"]]),
        ("opened", [["key", "Return"]]),
        ("nothing", [["type", "--delay", "40", "/zzq"]]),
        ("cleared", [["key", "Escape"]]),
        ("message", [["type", "--delay", "40", "hello there"], ["key", "Return"]]),
    ]

    def run(i: int) -> None:
        if i == len(plan):
            finish()
            return
        name, cmds = plan[i]
        act(cmds, lambda: read(name, lambda: (shot(name) if name == "search" else None, run(i + 1))))
    run(0)


def finish() -> None:
    out["events"] = events
    out["typed"] = typed
    print(json.dumps(out), flush=True)
    Gtk.main_quit()


GLib.timeout_add(3000, lambda: read("start", steps) and False)
GLib.timeout_add(90000, lambda: (print(json.dumps({"timeout": True, **out, "events": events}), flush=True),
                                 Gtk.main_quit()) and False)
Gtk.main()
