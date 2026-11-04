"""The lock screen on a real Chat Computer window: it covers everything (the conversation, the sidebars, what's open),
the PIN typed with real key presses (or tapped on its keypad) goes to Chat Computer as the "unlock" page event and
nowhere else, typing while locked sends nothing to Claude, and unlocking brings back what was open.

Runs a real Chat Computer window (GTK + WebKit) on a virtual display, types with xdotool, and prints JSON.
Run:  xvfb-run -a -s "-screen 0 1280x800x24" python tests/screen_lock_check.py [screenshot.png]
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
os.environ["CHATCOMPUTER_HOME"] = tempfile.mkdtemp(prefix="chatcomputer-lock-")

import gi  # noqa: E402

gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
gi.require_version("GdkX11", "3.0")
from gi.repository import GdkX11, GLib, Gtk  # noqa: E402,F401

from chatcomputer.config import Paths, Settings  # noqa: E402
from chatcomputer.ui.gtk_app import GtkScreen, choose_rendering  # noqa: E402

choose_rendering()
screen = GtkScreen(Paths().ensure(), Settings(), windowed=True)
screen.apply_settings(0, "Sam", "Opus 5.5")
loop = asyncio.new_event_loop()
threading.Thread(target=loop.run_forever, daemon=True).start()
sent: list[str] = []
events: list[dict] = []
screen.bind(loop, on_media_key=lambda k: None, on_media_state=lambda p: None, on_typed=sent.append)
screen.on_page_event = events.append
out: dict = {}
SHOT = sys.argv[1] if len(sys.argv) > 1 else ""

PAGE_JS = """(function(){var b=document.body, l=document.getElementById('lock'), p=document.getElementById('lock-pin');
  function vis(id){var e=document.getElementById(id); return e ? getComputedStyle(e).visibility !== 'hidden' &&
    getComputedStyle(e).display !== 'none' : null;}
  return JSON.stringify({locked: b.classList.contains('locked'), lock_shown: getComputedStyle(l).display !== 'none',
    column: vis('column'), messages: vis('messages'), composer: vis('composer'),
    pin_focused: document.activeElement === p, pin: p.value, error: document.getElementById('lock-error').textContent,
    time: document.getElementById('lock-time').textContent,
    box: document.getElementById('input').value});})()"""


def xdo(*args: str) -> None:
    subprocess.run(["xdotool", *args], check=False, timeout=20)


def read(name: str, then) -> None:
    def done(view, res, *_):
        out[name] = json.loads(view.evaluate_javascript_finish(res).to_string())
        then()
    screen.chat.evaluate_javascript(PAGE_JS, -1, None, None, None, done, None)


def js(script: str) -> None:
    screen.chat.evaluate_javascript(script, -1, None, None, None, None, None)


def steps():
    wid = str(screen.win.get_window().get_xid())
    plan = [
        ("before", lambda: None, []),
        ("locked", lambda: screen.lock_screen({"note": "Type your PIN to unlock."}),
         [["windowfocus", "--sync", wid]]),
        # Typing while locked: the letters can't go anywhere but the PIN box, which takes only digits, and Enter
        # sends them to be checked, never to Claude.
        ("typed", lambda: None, [["type", "--delay", "40", "hello 24"], ["key", "Return"]]),
        ("pad", lambda: js("['1','3','5','del','7'].forEach(function(k){"
                           "document.querySelector('#lock-pad [data-key=\"'+k+'\"]').click();})"), []),
        ("wrong", lambda: screen.lock_screen({"error": "That's not the PIN."}), []),
        ("unlocked", lambda: screen.lock_screen(None), []),
    ]

    def run(i: int) -> None:
        if i == len(plan):
            finish()
            return
        name, action, cmds = plan[i]
        action()

        def work() -> None:
            for c in cmds:
                xdo(*c)
            GLib.timeout_add(700, lambda: read(name, lambda: shot(name, lambda: run(i + 1))) and False)
        threading.Thread(target=work, daemon=True).start()
    run(0)


def shot(name: str, then) -> None:
    if SHOT and name == "pad":
        try:
            import mss
            import mss.tools
            with mss.MSS() if hasattr(mss, "MSS") else mss.mss() as m:
                g = m.grab(m.monitors[0])
                mss.tools.to_png(g.rgb, g.size, output=SHOT)
        except Exception as exc:
            out["shot_error"] = str(exc)
    then()


def finish() -> None:
    out["sent_text"] = list(sent)
    out["events"] = events
    print(json.dumps(out))
    Gtk.main_quit()


GLib.timeout_add(3000, lambda: steps() and False)
GLib.timeout_add(60000, lambda: (print(json.dumps({"timeout": True, **out})), Gtk.main_quit()) and False)
Gtk.main()
