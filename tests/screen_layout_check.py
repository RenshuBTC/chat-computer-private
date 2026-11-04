"""The screen's layout: both sidebars from the start: Messages at the left (newest first, pins, opening one) and
Actions at the right, against the screen's edges; something opened (a website) in the middle between them, in a
frame with its name and a
close button, from the top down to the message box, the conversation put away; "/" searching, its list in front (the
page ends above it); the sidebars' switches; the website has the keys (typing goes into its box), and Page Down in the
message box scrolls it; the x closes it; opened again, Escape from the page closes it too (the sidebars staying); and
Escape in the conversation leaves the sidebars be.

Runs a real Chat Computer window (GTK + WebKit) on a virtual display, clicks and presses keys with xdotool, prints JSON.
Run:  xvfb-run -a -s "-screen 0 1920x1080x24" python tests/screen_layout_check.py [shot-prefix]
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
os.environ["CHATCOMPUTER_HOME"] = tempfile.mkdtemp(prefix="chatcomputer-layout-")
os.environ["CHATCOMPUTER_W"], os.environ["CHATCOMPUTER_H"] = "1920", "1080"

import gi  # noqa: E402

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
gi.require_version("GdkX11", "3.0")
from gi.repository import Gdk, GdkX11, GLib, Gtk  # noqa: E402,F401

from chatcomputer.config import Paths, Settings  # noqa: E402
from chatcomputer.ui.gtk_app import GtkScreen, choose_rendering  # noqa: E402

SHOT = sys.argv[1] if len(sys.argv) > 1 else None
paths = Paths().ensure()
now = time.time()
choose_rendering()
screen = GtkScreen(paths, Settings(), windowed=True)
screen.apply_settings(0, "Sam", "Sonnet 5.5")
screen.mic(True)
loop = asyncio.new_event_loop()
threading.Thread(target=loop.run_forever, daemon=True).start()
screen.bind(loop, on_media_key=lambda k: None, on_media_state=lambda p: None, on_typed=lambda t: None)
events: list = []
closed: list = []
screen.on_page_event = lambda msg: events.append(msg)
screen.on_closed = lambda: closed.append(True)
screen.history([
    {"role": "user", "id": "u1", "ts": now - 600, "text": "Play some calm piano music on YouTube"},
    {"role": "assistant", "ts": now - 590, "text": "Playing a calm piano mix now."},
    {"role": "user", "id": "u2", "ts": now - 120, "text": "Will it rain tomorrow? And should I take the train or "
                                                         "drive to the airport if it does, with two big suitcases?"},
    {"role": "assistant", "ts": now - 110, "text": "Probably in the afternoon: a 70% chance of showers after 2 pm, "
     "mostly light.\n\nWith two big suitcases I'd drive: the train means two changes, and one of them has stairs "
     "and no lift.\n\n- Leave by 1 pm to beat the rain\n- Park in the long-stay car park\n- Book a space today"},
])
screen.done_list([{"id": "d1", "ts": now - 595, "text": "Searched youtube.com for “calm piano music”",
                   "prompt_id": "u1", "prompt_ts": now - 600}])
screen.messages_list([
    {"id": "m1", "source": "email", "label": "Gmail", "who": "Jane from the bank", "text": "Your statement is ready",
     "ts": now - 3000, "unread": True},
    {"id": "m2", "source": "whatsapp", "label": "WhatsApp", "who": "Mum", "text": "Dinner at 7?", "ts": now - 60,
     "unread": True},
    {"id": "m3", "source": "telegram", "label": "Telegram", "who": "Alex", "text": "Sent the files", "ts": now - 86400,
     "unread": False},
    {"id": "m4", "source": "instagram", "label": "Instagram", "who": "sam.k", "text": "Liked your photo",
     "ts": now - 7200, "unread": False, "pinned": now - 100},
], [{"id": "a1", "label": "Gmail"}])
site = Path(tempfile.mkdtemp()) / "site.html"
site.write_text("<!doctype html><title>A website</title><body style='margin:0'><h1>A website</h1>"
                "<input id=q style='position:absolute;left:20px;top:100px;width:300px;height:40px'>"
                "<div style='height:4000px'></div></body>")
out: dict = {}

STATE_JS = """(function () {
  var m = document.getElementById('messages').getBoundingClientRect(), d = document.getElementById('done').getBoundingClientRect();
  var f = document.getElementById('frame').getBoundingClientRect(), col = document.getElementById('column').getBoundingClientRect();
  function items(id) { return Array.prototype.map.call(document.querySelectorAll('#' + id + ' > li'), function (li) {
    var b = li.getBoundingClientRect(), who = li.querySelector('.msg-who'), use = li.querySelector('use');
    var text = li.querySelector('.msg-text');
    return {cls: li.className, who: who ? who.textContent : li.textContent, icon: use ? use.getAttribute('href') : '',
            x: Math.round(b.left + 30), y: Math.round(b.top + b.height / 2), pinx: Math.round(b.right - 18), piny: Math.round(b.top + 14),
            clipped: li.scrollHeight > li.clientHeight + 1, lines: text ? Math.round(text.getBoundingClientRect().height / parseFloat(getComputedStyle(text).lineHeight)) : 0,
            title: li.title};
  }); }
  var yous = document.querySelectorAll('#log .you'), lastYou = yous[yous.length - 1].getBoundingClientRect();
  var youEl = yous[yous.length - 1], aiEl = Array.prototype.slice.call(document.querySelectorAll('#log > .ai')).pop();
  var res = document.getElementById('results').getBoundingClientRect();
  var inp = document.getElementById('input').getBoundingClientRect();
  var line = parseFloat(getComputedStyle(document.body).lineHeight);
  function toggle(k) { var e = document.getElementById('toggle-' + k), r = e.getBoundingClientRect();
    return {x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2), pressed: e.getAttribute('aria-pressed'),
            shown: getComputedStyle(e).display !== 'none'}; }
  function shown(id) { return getComputedStyle(document.getElementById(id)).display !== 'none'; }
  var x = document.getElementById('frame-close').getBoundingClientRect(), box = document.getElementById('box').getBoundingClientRect();
  return JSON.stringify({content: document.body.classList.contains('content'), full: document.body.classList.contains('full'),
    toggles: {left: toggle('left'), right: toggle('right')}, bottom_toggle: !!document.getElementById('toggle-bottom'),
    sides: [shown('messages'), shown('done')], composer: shown('composer'), column_shown: shown('column'),
    frame_title: document.getElementById('frame-title').textContent, close: [Math.round(x.left + x.width / 2), Math.round(x.top + x.height / 2)],
    box: document.getElementById('input').value, box_rect: [box.left, box.top, box.right, box.bottom],
    hello: document.querySelectorAll('#log > .ai.hello').length,
    messages: [m.left, m.top, m.right, m.bottom], done: [d.left, d.top, d.right, d.bottom],
    frame: [f.left, f.top, f.right, f.bottom], column: [col.left, col.top, col.right, col.bottom],
    last_you: [lastYou.left, lastYou.top, lastYou.right, lastYou.bottom],
    radius: getComputedStyle(document.getElementById('messages')).borderTopLeftRadius,
    pinned: items('messages-pinned'), list: items('messages-list'), view: [innerWidth, innerHeight],
    line: line, you_h: youEl.getBoundingClientRect().height, ai_h: aiEl.getBoundingClientRect().height,
    ai_bottom: aiEl.getBoundingClientRect().bottom, more: document.querySelectorAll('.more').length,
    searching: document.body.classList.contains('searching'), results: [res.left, res.top, res.right, res.bottom],
    input: [Math.round(inp.left + 20), Math.round(inp.top + inp.height / 2)],
    hint: document.getElementById('input').value === '' ? document.getElementById('input').placeholder : null});
})()"""


def xdo(*args: str) -> None:
    subprocess.run(["xdotool", *args], check=False, timeout=20)


def wid() -> str:
    return str(screen.win.get_window().get_xid())


def content_box() -> list:
    v = screen.content
    if not v.get_visible():
        return []
    x, y = v.translate_coordinates(screen.win, 0, 0)
    a = v.get_allocation()
    return [x, y, x + a.width, y + a.height]


def read(name: str, then) -> None:
    def page(view, res, *_):
        try:
            out[name]["page"] = json.loads(view.evaluate_javascript_finish(res).to_string())
        except Exception as e:                 # nothing open
            out[name]["page"] = {"error": str(e)[:80]}
        then()

    def done(view, res, *_):
        out[name] = json.loads(view.evaluate_javascript_finish(res).to_string())
        out[name]["content_view"] = content_box()
        out[name]["mode"] = screen.mode
        out[name]["keys_in_page"] = screen.content.has_focus()
        screen.content.evaluate_javascript(
            "JSON.stringify({typed: (document.getElementById('q') || {}).value, y: Math.round(scrollY)})",
            -1, None, None, None, page, None)
    screen.chat.evaluate_javascript(STATE_JS, -1, None, None, None, done, None)


def shot(name: str) -> None:
    if SHOT:
        w = screen.win.get_window()
        Gdk.pixbuf_get_from_window(w, 0, 0, w.get_width(), w.get_height()).savev(f"{SHOT}-{name}.png", "png", [], [])


def later(ms: int, fn) -> None:
    GLib.timeout_add(ms, lambda: fn() and False)


def act(cmds: list, then, wait_ms: int = 900) -> None:
    def work() -> None:
        for c in cmds:
            xdo(*c)
        GLib.timeout_add(wait_ms, lambda: then() and False)
    threading.Thread(target=work, daemon=True).start()


def step_start() -> None:
    """The sidebars show from the start."""
    read("first", lambda: act([["windowfocus", "--sync", wid()]], lambda: read("start", step_pin)))


def step_pin() -> None:
    m2 = next(i for i in out["start"]["list"] if i["who"] == "Mum")
    # The pin shows under the pointer: move there, then click it.
    act([["mousemove", "--window", wid(), str(m2["x"]), str(m2["y"])], ["sleep", "0.3"],
         ["mousemove", "--window", wid(), str(m2["pinx"]), str(m2["piny"])], ["click", "1"]],
        lambda: read("pinned", step_open))


def step_open() -> None:
    m1 = next(i for i in out["pinned"]["list"] if i["who"] == "Jane from the bank")
    act([["mousemove", "--window", wid(), str(m1["x"]), str(m1["y"])], ["click", "1"]],
        lambda: step_content())


def step_content() -> None:
    out["events"] = [e for e in events]
    asyncio.run_coroutine_threadsafe(screen.show_url(site.as_uri()), loop)
    later(2500, lambda: (shot("content"), read("content", step_slash)))


def step_slash() -> None:
    """ "/" with the page open: eight files found (as the controller would send them), listed in front."""
    found = json.dumps([{"name": f"notes {i}.txt", "path": f"/home/notes{i}.txt", "kind": "document",
                         "where": "~/Documents", "when": "Today"} for i in range(1, 9)])      # taller than the strip

    def results() -> None:
        screen.chat.evaluate_javascript(f"CO.searchResults(1, '', {found}, '')", -1, None, None, None, None, None)
        later(600, lambda: (shot("slash"), read("slash", step_unslash)))
    box = out["content"]["input"]          # into the message box (a website that's open has the keys after it opens)
    act([["windowfocus", "--sync", wid()], ["mousemove", "--window", wid(), str(box[0]), str(box[1])], ["click", "1"],
         ["sleep", "0.3"], ["type", "/"]], results, wait_ms=700)


def step_unslash() -> None:
    act([["key", "Escape"]], lambda: read("unslashed", step_sides_off))


def click(name: str, then_name: str, then, *toggles: str) -> None:
    """Click the panels' switches named (positions from the state called `name`), then read `then_name`."""
    cmds = []
    for k in toggles:
        t = out[name]["toggles"][k]
        cmds += [["mousemove", "--window", wid(), str(t["x"]), str(t["y"])], ["click", "1"], ["sleep", "0.3"]]
    act(cmds, lambda: read(then_name, then))


def step_sides_off() -> None:
    click("unslashed", "sides_off", step_sides_on, "left", "right")


def step_sides_on() -> None:
    out["panel_events"] = [e for e in events if e.get("type") == "panels"]
    click("sides_off", "sides_on", step_type_in_page, "left", "right")


def step_type_in_page() -> None:
    """A click on the website's box, and typing goes there (not into the message box)."""
    v = out["sides_on"]["content_view"]
    act([["windowfocus", "--sync", wid()], ["mousemove", "--window", wid(), str(v[0] + 60), str(v[1] + 120)],
         ["click", "1"], ["sleep", "0.3"], ["type", "--delay", "60", "abc"]], lambda: read("typed_in_page", step_page_down))


def step_page_down() -> None:
    """Page Down in the (empty) message box scrolls what's open."""
    box = out["typed_in_page"]["input"]
    act([["mousemove", "--window", wid(), str(box[0]), str(box[1])], ["click", "1"], ["sleep", "0.3"], ["key", "Page_Down"]],
        lambda: read("paged", step_close))


def step_close() -> None:
    x = out["paged"]["close"]
    act([["mousemove", "--window", wid(), str(x[0]), str(x[1])], ["click", "1"]],
        lambda: (shot("chat"), read("closed", step_reopen)))


def step_reopen() -> None:
    """The website again, and a click in it: Escape from the page closes it, as the x does."""
    asyncio.run_coroutine_threadsafe(screen.show_url(site.as_uri()), loop)
    later(2500, lambda: read("reopened", step_page_escape))


def step_page_escape() -> None:
    v = out["reopened"]["content_view"]
    act([["windowfocus", "--sync", wid()], ["mousemove", "--window", wid(), str(v[0] + 60), str(v[1] + 120)],
         ["click", "1"], ["sleep", "0.3"], ["key", "Escape"]], lambda: read("page_escaped", step_chat_escape))


def step_chat_escape() -> None:
    act([["windowfocus", "--sync", wid()], ["key", "Escape"]], lambda: read("chat_escaped", finish))


def finish() -> None:
    out["closed_called"] = bool(closed)
    print(json.dumps(out), flush=True)
    Gtk.main_quit()


later(3000, step_start)
GLib.timeout_add(90000, lambda: (print(json.dumps({"timeout": True, **out}), flush=True), Gtk.main_quit()) and False)
Gtk.main()
