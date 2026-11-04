"""Actions, at the right of the conversation (the sidebars show from the start):
a sixth of the screen wide, grouped by day; tapping an item scrolls back to the message that asked for it, and for
one from an earlier day, puts that day's messages back first (from the transcript). Typing stays in the message box.
An item that can be reverted shows Revert beside its pin, under the pointer; clicking it asks the computer, and the
item says "Reverting…" until the computer says it's done, then it's struck through.

Runs a real Chat Computer window (GTK + WebKit) on a virtual display, clicks with xdotool, and prints JSON.
Run:  xvfb-run -a -s "-screen 0 1600x900x24" python tests/screen_done_check.py [screenshot.png]
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
os.environ["CHATCOMPUTER_HOME"] = tempfile.mkdtemp(prefix="chatcomputer-done-")
os.environ["CHATCOMPUTER_W"], os.environ["CHATCOMPUTER_H"] = "1600", "900"

import gi  # noqa: E402

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
gi.require_version("GdkX11", "3.0")
from gi.repository import Gdk, GdkX11, GLib, Gtk  # noqa: E402,F401

from chatcomputer.config import Paths, Settings  # noqa: E402
from chatcomputer.done import History  # noqa: E402
from chatcomputer.ui.gtk_app import GtkScreen, choose_rendering  # noqa: E402

SHOT = sys.argv[1] if len(sys.argv) > 1 else None
paths = Paths().ensure()
now = time.time()
yesterday = now - 86400

# Yesterday's conversation, only in the transcript; today's, on the screen too.
with open(paths.transcript, "w", encoding="utf-8") as f:
    for i in range(6):
        f.write(json.dumps({"ts": yesterday + i * 60, "role": "user", "id": f"y{i}",
                            "text": f"Yesterday's question {i}"}) + "\n")
        f.write(json.dumps({"ts": yesterday + i * 60 + 5, "role": "assistant", "id": f"ya{i}",
                            "text": f"Yesterday's answer {i}."}) + "\n")
    for i in range(30):
        f.write(json.dumps({"ts": now - 3000 + i * 60, "role": "user", "id": f"u{i}",
                            "text": f"Question number {i}: what about the weather and the trains tomorrow?"}) + "\n")

choose_rendering()
screen = GtkScreen(paths, Settings(), windowed=True)
screen.apply_settings(0, "Sam", "Sonnet 5.5")
screen.mic(True)
loop = asyncio.new_event_loop()
threading.Thread(target=loop.run_forever, daemon=True).start()
screen.bind(loop, on_media_key=lambda k: None, on_media_state=lambda p: None, on_typed=lambda t: None)
history = History(paths)
asked: list = []


def on_goto(target, ts, before):
    asked.append([target, round(ts), before is not None])
    screen.earlier(history.earlier(target, ts, before), target, ts)


screen.on_goto = on_goto
items = []
for i in range(30):
    items.append({"role": "user", "text": f"Question number {i}: what about the weather and the trains tomorrow?",
                  "id": f"u{i}", "ts": now - 3000 + i * 60})
    items.append({"role": "assistant", "text": f"Answer {i}. " + "Sunny, and the trains run as usual. " * 3,
                  "ts": now - 3000 + i * 60 + 5})
screen.history(items)
screen.done_list([
    {"id": "d1", "ts": yesterday + 70, "text": "Opened youtube.com", "prompt_id": "y1", "prompt_ts": yesterday + 60},
    {"id": "d2", "ts": now - 2700, "text": "Searched the web for “trains on Sunday”", "prompt_id": "u5",
     "prompt_ts": now - 3000 + 5 * 60},
    {"id": "d3", "ts": now - 600, "text": "Saved notes.txt", "prompt_id": "u25", "prompt_ts": now - 3000 + 25 * 60,
     "revert": True},
])
screen.done({"id": "d4", "ts": now - 60, "text": "Set the volume to 40%", "prompt_id": "u29",
             "prompt_ts": now - 3000 + 29 * 60})
out: dict = {}
page_events: list = []
screen.on_page_event = page_events.append

STATE_JS = """(function () {
  var d = document.getElementById('done'), r = d.getBoundingClientRect(), p = document.scrollingElement;
  var col = document.getElementById('column').getBoundingClientRect(), box = document.getElementById('box').getBoundingClientRect();
  var found = document.querySelector('#log .found'), fr = found && found.getBoundingClientRect();
  var items = Array.prototype.map.call(document.querySelectorAll('#done-list > li'), function (li) {
    var b = li.getBoundingClientRect();
    var what = li.querySelector('.done-what'), when = li.querySelector('.done-when');
    return {cls: li.className.replace(' item', '').replace('item ', ''), text: (what || li).textContent,
            when: when ? when.textContent : '', x: Math.round(b.left + 40), y: Math.round(b.top + b.height / 2),
            // its time at the right of its words, on their first line, and at the item's right edge
            beside: !!(what && when) && when.getBoundingClientRect().left >= what.getBoundingClientRect().right - 1 &&
              Math.abs(when.getBoundingClientRect().top - what.getBoundingClientRect().top) < 3 &&
              b.right - when.getBoundingClientRect().right < 12};
  });
  return JSON.stringify({panel: document.body.classList.contains('panel'), display: getComputedStyle(d).display,
    full: document.body.classList.contains('full'),
    left: r.left, width: r.width, view: window.innerWidth, title: document.getElementById('done-title').textContent,
    items: items, column_right: col.right, box: [box.left, box.right],
    found: found ? {id: found.dataset.id, top: fr.top} : null, scroll: p.scrollTop,
    messages: document.querySelectorAll('#log > .you').length, events: (window.__ev || []).slice(-4),
    days: Array.prototype.map.call(document.querySelectorAll('#log > .day'), function (e) { return e.textContent; }),
    focused: document.activeElement === document.getElementById('input'),
    radius: getComputedStyle(d).borderTopLeftRadius, line: getComputedStyle(document.getElementById('done-list')).borderLeftStyle,
    rem: parseFloat(getComputedStyle(document.documentElement).fontSize),
    clock: [document.getElementById('clock-date').textContent, document.getElementById('clock-time').textContent]});
})()"""


def read(name: str, then) -> None:
    def done(view, res, *_):
        out[name] = json.loads(view.evaluate_javascript_finish(res).to_string())
        then()
    screen.chat.evaluate_javascript(STATE_JS, -1, None, None, None, done, None)


def click(x: int, y: int, then) -> None:
    wid = str(screen.win.get_window().get_xid())

    def work() -> None:
        subprocess.run(["xdotool", "mousemove", "--window", wid, str(x), str(y), "click", "1"], timeout=20)
        GLib.timeout_add(1200, lambda: then() and False)
    threading.Thread(target=work, daemon=True).start()


def item(text: str) -> dict:
    return next(i for i in out["start"]["items"] if i["text"].startswith(text))


EVENTS_JS = """window.__ev = []; ["mousedown", "click"].forEach(function (t) {
  document.addEventListener(t, function (e) { window.__ev.push(t + " " + (e.target.className || e.target.tagName)); }, true);
}); 1"""


def start() -> None:
    """The sidebars show from the start."""
    screen.chat.evaluate_javascript(EVENTS_JS, -1, None, None, None, None, None)
    read("first", lambda: read("start", lambda: click(item("Searched")["x"], item("Searched")["y"],
                                                      lambda: read("today", go_earlier))))


def go_earlier() -> None:
    it = item("Opened")
    click(it["x"], it["y"], lambda: read("earlier", hover_revert))


REVERT_JS = """(function () {
  var li = document.querySelector('#done-list li[data-id="d3"]'), b = li && li.querySelector('.revert');
  function at(e) {
    if (!e) return null;
    var r = e.getBoundingClientRect();
    return {x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2), left: r.left, right: r.right,
            visible: getComputedStyle(e).visibility};
  }
  var last = document.querySelector('#done-list li:last-child');
  return JSON.stringify({cls: li ? li.className : null, revert: at(b), pin: at(li && li.querySelector('.pin')),
    struck: li ? getComputedStyle(li.querySelector('.done-what')).textDecorationLine : null,
    when: li ? li.querySelector('.done-when').textContent : null,
    colors: li ? [getComputedStyle(li.querySelector('.done-what')).color, getComputedStyle(li).opacity] : null,
    with_revert: Array.prototype.map.call(document.querySelectorAll('#done-list .revert'), function (x) {
      return x.closest('li').dataset.id; }),
    last: last ? [last.dataset.id, last.className] : null});
})()"""


def read_revert(name: str, then) -> None:
    def done(view, res, *_):
        out[name] = json.loads(view.evaluate_javascript_finish(res).to_string())
        then()
    screen.chat.evaluate_javascript(REVERT_JS, -1, None, None, None, done, None)


def hover_revert() -> None:
    """Under the pointer, the item that can be reverted shows Revert, left of its pin."""
    wid = str(screen.win.get_window().get_xid())
    it = next(i for i in out["earlier"]["items"] if i["text"] == "Saved notes.txt")

    def work() -> None:
        subprocess.run(["xdotool", "mousemove", "--window", wid, str(it["x"]), str(it["y"])], timeout=20)
        GLib.timeout_add(600, lambda: read_revert("hover", click_revert) and False)
    threading.Thread(target=work, daemon=True).start()


def click_revert() -> None:
    b = out["hover"]["revert"]
    click(b["x"], b["y"], lambda: read_revert("reverting", answered))


def answered() -> None:
    """The computer's answer: that one reverted, and the revert listed after it."""
    out["page_events"] = [e for e in page_events if e.get("type") == "revert"]
    screen.done_states([{"id": "d3", "reverted": True}, {"id": "d5", "revert": True}])
    screen.done({"id": "d5", "ts": time.time(), "text": "Reverted “Saved notes.txt”", "revert": True})
    GLib.timeout_add(700, lambda: read_revert("reverted", shoot) and False)


def shoot() -> None:
    if SHOT:
        w = screen.win.get_window()
        Gdk.pixbuf_get_from_window(w, 0, 0, w.get_width(), w.get_height()).savev(SHOT, "png", [], [])
    out["asked"] = asked
    print(json.dumps(out), flush=True)
    Gtk.main_quit()


GLib.timeout_add(3000, lambda: start() and False)
GLib.timeout_add(60000, lambda: (print(json.dumps({"timeout": True, **out}), flush=True), Gtk.main_quit()) and False)
Gtk.main()
