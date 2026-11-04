"""The conversation page comes back as it was after its renderer process stops.

Runs a real Chat Computer window (GTK + WebKit) on a virtual display, puts a short conversation on it, stops the
page's renderer the way a crash would, and prints JSON: the conversation text before and after.
Run:  xvfb-run -a python tests/screen_recovery_check.py
"""
import json
import os
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
os.environ["CHATCOMPUTER_HOME"] = tempfile.mkdtemp(prefix="chatcomputer-recovery-")

from gi.repository import GLib, Gtk, WebKit2  # noqa: E402

from chatcomputer.config import Paths, Settings  # noqa: E402
from chatcomputer.ui.gtk_app import GtkScreen, choose_rendering  # noqa: E402

choose_rendering()
screen = GtkScreen(Paths().ensure(), Settings(), windowed=True)
screen.apply_settings(0, "Sam")
screen.user_text("u1", "What's the weather tomorrow?", True)
screen.ai_delta("a1", "Sunny, ")
screen.ai_delta("a1", "31 degrees.")
screen.ai_done("a1")
screen.notice("The Claude app on your phone can reach me.")
screen.state("listening")
out = {}


def read_log(key, then):
    def done(view, res, *_):
        try:
            out[key] = view.evaluate_javascript_finish(res).to_string()
        except Exception as exc:
            out[key] = f"(error: {exc})"
        then()
    screen.chat.evaluate_javascript("document.getElementById('log').innerText", -1, None, None, None, done, None)


def crash():
    screen.chat.terminate_web_process()     # the process really goes away...
    # ...and WebKit reports it the way it reports a crash.
    GLib.timeout_add(500, lambda: screen._on_renderer_stopped(
        screen.chat, WebKit2.WebProcessTerminationReason.CRASHED) and False)
    # Read it again once the page is back (it says so: chat_ready), however long that takes on a busy machine.
    waited = {"ms": 0}

    def back() -> bool:
        waited["ms"] += 200
        if (screen.chat_ready and waited["ms"] >= 1500) or waited["ms"] >= 20000:
            GLib.timeout_add(500, lambda: read_log("after", finish) and False)
            return False
        return True
    GLib.timeout_add(1000, lambda: GLib.timeout_add(200, back) and False)


def finish():
    out["ready_again"] = screen.chat_ready
    out["drawing_with_processor"] = os.environ.get("WEBKIT_SKIA_ENABLE_CPU_RENDERING") == "1"
    print(json.dumps(out))
    Gtk.main_quit()


GLib.timeout_add(4000, lambda: read_log("before", crash) and False)
GLib.timeout_add(30000, lambda: (print(json.dumps({"timeout": True, **out})), Gtk.main_quit()) and False)
Gtk.main()
