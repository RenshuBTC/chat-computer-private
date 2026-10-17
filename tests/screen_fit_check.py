"""The conversation keeps whole messages on screen while words stream in, and shows older ones again when
there is room.

Runs a real ClaudeOS window (GTK + WebKit) on a virtual display, fills the conversation, streams a long reply
into it, and prints JSON: for each check, what the page showed.
Run:  xvfb-run -a -s "-screen 0 1280x800x24" python tests/screen_fit_check.py
"""
import json
import os
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
os.environ["CLAUDEOS_HOME"] = tempfile.mkdtemp(prefix="claudeos-fit-")

from gi.repository import GLib, Gtk  # noqa: E402

from claudeos.config import Paths, Settings  # noqa: E402
from claudeos.ui.gtk_app import GtkScreen, choose_rendering  # noqa: E402

choose_rendering()
screen = GtkScreen(Paths().ensure(), Settings(), windowed=True)
screen.apply_settings(0, "Ren")
for i in range(20):
    screen.user_text(f"u{i}", f"Question number {i}: what about the weather and the trains tomorrow morning?", True)
    screen.ai_delta(f"a{i}", f"Answer {i}. " + "It will be sunny, and the trains run as usual. " * 4)
    screen.ai_done(f"a{i}")
out = {}

# What the page shows: for each message, whether it's hidden, and where its top is; and the room kept at the top.
STATE_JS = """JSON.stringify({room: parseFloat(getComputedStyle(document.body).fontSize) * 1.5,
  items: Array.prototype.map.call(document.getElementById('log').children, function (k) {
    return {gone: k.classList.contains('gone'), top: k.getBoundingClientRect().top}; })})"""


def read(key, then):
    def done(view, res, *_):
        out[key] = json.loads(view.evaluate_javascript_finish(res).to_string())
        then()
    screen.chat.evaluate_javascript(STATE_JS, -1, None, None, None, done, None)


words = iter(("The forecast says " + "clear skies and a light breeze from the sea all day long, " * 12).split(" "))


def stream():
    w = next(words, None)
    if w is None:
        screen.ai_done("live")
        GLib.timeout_add(400, lambda: read("streamed", refit) and False)
        return False
    screen.ai_delta("live", w + " ")
    return True


def refit():
    # Text a third smaller: older messages fit again and must come back.
    screen.chat.evaluate_javascript("CO.layout(14, 600)", -1, None, None, None, None, None)
    GLib.timeout_add(400, lambda: read("smaller", finish) and False)


def finish():
    print(json.dumps(out))
    Gtk.main_quit()


GLib.timeout_add(3000, lambda: read("filled", lambda: GLib.timeout_add(15, stream)) and False)
GLib.timeout_add(60000, lambda: (print(json.dumps({"timeout": True, **out})), Gtk.main_quit()) and False)
Gtk.main()
