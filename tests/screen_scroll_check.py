"""The conversation reads like a chat and scrolls like the Claude app: your messages on the right in grey bubbles,
Claude's on the left; what you send goes to the top of the screen, with the answer under it, and new words keep the
newest in view, unless you scrolled up to read something earlier.

Runs a real Chat Computer window (GTK + WebKit) on a virtual display, fills the conversation, streams a long reply
into it, scrolls up and streams more, then sends a new message; prints JSON: what the page showed at each step.
Run:  xvfb-run -a -s "-screen 0 1280x800x24" python tests/screen_scroll_check.py
"""
import json
import os
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
os.environ["CHATCOMPUTER_HOME"] = tempfile.mkdtemp(prefix="chatcomputer-scroll-")

from gi.repository import GLib, Gtk  # noqa: E402

from chatcomputer.config import Paths, Settings  # noqa: E402
from chatcomputer.ui.gtk_app import GtkScreen, choose_rendering  # noqa: E402

choose_rendering()
screen = GtkScreen(Paths().ensure(), Settings(), windowed=True)
screen.apply_settings(0, "Sam")
for i in range(20):
    screen.user_text(f"u{i}", f"Question number {i}: what about the weather and the trains tomorrow morning?", True)
    screen.ai_delta(f"a{i}", f"Answer {i}. " + "It will be sunny, and the trains run as usual. " * 4)
    screen.ai_done(f"a{i}")
out = {}

# Where the page is scrolled, and where the last message of each kind sits in the column.
STATE_JS = """(function () {
  var p = document.scrollingElement, col = document.getElementById('column').getBoundingClientRect();
  var you = document.querySelectorAll('#log > .you'), ai = document.querySelectorAll('#log > .ai');
  var u = you[you.length - 1], a = ai[ai.length - 1], ur = u.getBoundingClientRect(), ar = a.getBoundingClientRect();
  var cs = getComputedStyle(u);
  return JSON.stringify({top: p.scrollTop, bottom: p.scrollHeight - p.scrollTop - p.clientHeight,
    you_top: ur.top, margin: parseFloat(cs.scrollMarginTop), ai_below: ar.top >= ur.bottom,
    height: p.scrollHeight, view: p.clientHeight, messages: document.getElementById('log').children.length,
    you: {left: ur.left - col.left, right: col.right - ur.right, background: cs.backgroundColor, style: cs.fontStyle},
    ai: {left: ar.left - col.left, background: getComputedStyle(a).backgroundColor}});
})()"""


def read(key, then):
    def done(view, res, *_):
        out[key] = json.loads(view.evaluate_javascript_finish(res).to_string())
        then()
    screen.chat.evaluate_javascript(STATE_JS, -1, None, None, None, done, None)


def streamer(tid, text, then):
    words = iter(text.split(" "))

    def step():
        w = next(words, None)
        if w is None:
            screen.ai_done(tid)
            GLib.timeout_add(400, then)
            return False
        screen.ai_delta(tid, w + " ")
        return True
    return step


LONG = "The forecast says " + "clear skies and a light breeze from the sea all day long, " * 80    # past a screenful


def scroll_up():
    screen.chat.evaluate_javascript("window.scrollTo(0, 0)", -1, None, None, None, None, None)
    GLib.timeout_add(300, lambda: GLib.timeout_add(15, streamer("more", LONG, lambda: read("scrolled_up", ask) and False)) and False)
    return False


def ask():
    screen.user_text("new", "And on Sunday?", True)
    GLib.timeout_add(1200, lambda: read("asked", finish) and False)


def finish():
    print(json.dumps(out))
    Gtk.main_quit()


GLib.timeout_add(3000, lambda: read("filled", lambda: GLib.timeout_add(
    15, streamer("live", LONG, lambda: read("streamed", scroll_up) and False))) and False)
GLib.timeout_add(100000, lambda: (print(json.dumps({"timeout": True, **out})), Gtk.main_quit()) and False)
Gtk.main()
