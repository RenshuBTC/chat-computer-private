"""Full Chat Computer run on a virtual display: real window, real WebKit, the real brain (its agent loop, against a
stand-in for the model's server), real tools, real speech models with a fake microphone.

Run:  xvfb-run -s "-screen 0 1920x1080x24" dbus-run-session python tests/ui_scenario.py OUTDIR
Writes screenshots and result.json to OUTDIR.
"""
import asyncio
import http.server
import json
import os
import socketserver
import sys
import threading
import time
import traceback
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent))

OUT = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/chatcomputer-ui")
OUT.mkdir(parents=True, exist_ok=True)
for k in list(os.environ):
    if k.lower() in ("https_proxy", "http_proxy"):
        os.environ.pop(k, None)
HOME = OUT / "home"
HOME.mkdir(exist_ok=True)
os.environ["HOME"] = str(HOME)
os.environ["CHATCOMPUTER_HOME"] = str(HOME / "ChatComputer")
os.environ.setdefault("CHATCOMPUTER_GPU", "0")
os.environ["NO_PROXY"] = "127.0.0.1,localhost"

from mock_model import FakeModelService, MockModel, call, last_is_tool_result, last_user_text, tool_results  # noqa: E402

# ------------------------------------------------------------------ a small local "website"
SITE = OUT / "site"
SITE.mkdir(exist_ok=True)
(SITE / "index.html").write_text("""<!doctype html><html><head><meta charset=utf-8><title>Test Video Site</title>
<style>body{font-family:sans-serif;background:#fafafa;margin:40px} .tile{display:inline-block;width:300px;height:170px;
background:#c33;color:#fff;margin:10px;padding:10px;font-size:22px;cursor:pointer}</style></head><body>
<h1>Test Video Site</h1><form onsubmit="document.getElementById('q').dataset.sent=1;document.title='Results: '+
document.getElementById('q').value;return false"><input id=q aria-label="Search" placeholder="Search videos">
<button>Search</button></form><div class=tile onclick="location.href='watch.html'">Calm piano music for focus</div>
<div class=tile>Lo-fi beats</div></body></html>""")
(SITE / "watch.html").write_text("""<!doctype html><html><head><meta charset=utf-8><title>Calm piano music - Test Video Site</title></head>
<body style="background:#111;color:#eee;font-family:sans-serif"><h2>Calm piano music for focus</h2><p>Now playing.</p></body></html>""")


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass


httpd = socketserver.TCPServer(("127.0.0.1", 0), lambda *a: Quiet(*a, directory=str(SITE)))
threading.Thread(target=httpd.serve_forever, daemon=True).start()
SITE_URL = f"http://127.0.0.1:{httpd.server_address[1]}/"

# ------------------------------------------------------------------ the scripted model
mock = MockModel()
mock.start()
step = {"n": 0}


def rule(req):
    said = last_user_text(req).split("\n")[-1]
    if last_is_tool_result(req):
        res = json.dumps(tool_results(req))
        if "Clicked" in res:
            return ["It's playing now."]
        if "Showing: Test Video Site" in res:
            return [call("click", text="Calm piano music")]
        if "Back to the conversation" in res:
            return ["Here we are. The piano video is still loaded, so just say play whenever you want "
                    "it back. Or tell me what else you'd like to do this evening: I can find a recipe, "
                    "check tomorrow's weather, or put on something else."]
        if "Showing the document" in res:
            return ["I've put the plan on the screen."]
        return ["Done."]
    if "piano" in said.lower():
        return ["Sure, opening the video site.", call("show_website", url=SITE_URL)]
    if "chat" in said.lower() or "conversation" in said.lower():
        return [call("show_conversation")]
    if "plan" in said.lower():
        return [call("show_document", title="Plan for Saturday", markdown=
                "## Morning\n- Run at **7am** by the river\n- Breakfast at Tiong Bahru\n\n## Afternoon\n"
                "| Time | What |\n|---|---|\n| 2pm | Library |\n| 5pm | Groceries |\n\nRemember to bring the *library card*.")]
    return ["Hello! I'm Chat Computer. Tell me what you'd like to do, and I'll handle it."]


mock.rules.append(rule)

# ------------------------------------------------------------------ Chat Computer
import chatcomputer.controller as controller_module  # noqa: E402
from chatcomputer.config import Paths, Settings  # noqa: E402
from chatcomputer.controller import Controller  # noqa: E402
from chatcomputer.platform.fake import FakePlatform  # noqa: E402
from chatcomputer.platform.linux import LinuxPlatform  # noqa: E402
from chatcomputer.ui.gtk_app import run_app  # noqa: E402
from chatcomputer.voice.audio import FakeMic, FakePlayer  # noqa: E402

paths = Paths().ensure()
from chatcomputer.journal import setup_logging  # noqa: E402
setup_logging(paths, verbose=True)
# The model's server is the stand-in: the controller's ModelService gives its port.
controller_module.ModelService = lambda *a, **k: FakeModelService(mock)
settings = Settings(user_name="Sam Lee", windows_user="sammy", stt_model=os.environ.get("CHATCOMPUTER_TEST_STT", "base.en"))
# Pretend-monitor size and viewing distance (the virtual display always reports 96 dpi).
settings.screen_inches = float(os.environ.get("UITEST_INCHES", "0") or 0)
settings.viewing_distance_cm = int(os.environ.get("UITEST_DISTANCE_CM", "0") or 0)
MODELS = Path(os.environ.get("CHATCOMPUTER_TEST_MODELS", str(Path.home() / ".cache" / "chatcomputer-test" / "models")))


class XPlatform(FakePlatform):
    """Fake machine, but real screen input and screenshots on the virtual display."""
    lin = LinuxPlatform()

    def screenshot_png(self):
        return self.lin.screenshot_png()

    def screenshot_jpeg(self, max_w, max_h):
        return self.lin.screenshot_jpeg(max_w, max_h)

    def click(self, x, y, button="left", clicks=1):
        self.lin.click(x, y, button, clicks)

    def type_text(self, text):
        self.lin.type_text(text)

    def key(self, combo, repeat=1):
        self.lin.key(combo, repeat)

    def screen_size(self):
        return self.lin.screen_size()


platform = XPlatform()
mics = []
player = FakePlayer()
stt = tts = None
if (MODELS / "kokoro-v1.0.int8.onnx").exists():
    from chatcomputer.voice.tts import TTS
    tts = TTS(str(MODELS / "kokoro-v1.0.int8.onnx"), str(MODELS / "voices-v1.0.bin"))
wd = MODELS / "whisper" / f"faster-whisper-{settings.stt_model}"
if (wd / "model.bin").exists():
    from chatcomputer.voice.stt import STT
    stt = STT(str(wd), "en", hint="Computer, Sam Lee, YouTube")

result = {"steps": [], "ok": False}
holder = {}


def factory(screen):
    c = Controller(paths, settings, platform, screen, voice=True, mic_factory=lambda cb: mics.append(FakeMic(cb)) or mics[-1],
                   player=player, stt=stt, tts=tts, index=True, greet=True)
    holder["c"] = c
    return c


def shot(name):
    png = LinuxPlatform().screenshot_png()
    (OUT / f"{name}.png").write_bytes(png)


def page_value(screen, js):
    """What a bit of script gives on the conversation page (as text)."""
    from gi.repository import GLib
    box, done_ev = {}, threading.Event()

    def done(view, res, *_):
        try:
            box["text"] = view.evaluate_javascript_finish(res).to_string()
        except Exception as exc:
            box["text"] = f"(error: {exc})"
        done_ev.set()
    GLib.idle_add(lambda: screen.chat.evaluate_javascript(js, -1, None, None, None, done, None) and False)
    done_ev.wait(10)
    return box.get("text", "")


def chat_log_text(screen):
    """The conversation as the page shows it."""
    return page_value(screen, "document.getElementById('log').innerText")


def speak_into_mic(text):
    from chatcomputer.voice.tts import _resample
    import numpy as np
    a = _resample(tts.synth(text), 24000, 16000)
    audio = np.concatenate([np.zeros(8000, np.float32), a, np.zeros(16000, np.float32)])
    mics[-1].feed(audio, realtime=True, speed=2.0)


def scenario():
    try:
        time.sleep(6)
        c = holder["c"]
        loop = c.loop
        shot("01-start")
        result["steps"].append("started")
        result["layout"] = c.screen.layout_info()

        # 1. Talk (real speech through the fake mic → VAD → Whisper).
        speak_into_mic("Play some calm piano music.")
        deadline = time.time() + 60
        while time.time() < deadline:
            if "It's playing now" in json.dumps(mock.requests[-1:]):
                break
            if c.brain and not c.brain.busy and len(mock.requests) >= 3:
                break
            time.sleep(0.5)
        time.sleep(2.5)
        shot("02-website")
        info = asyncio.run_coroutine_threadsafe(c.screen.content_info(), loop).result(10)
        result["steps"].append({"after_voice": info, "requests": len(mock.requests),
                                # (the first request is the brain's warm-up, which says only ".")
                                "heard": [last_user_text(r)[-60:] for r in mock.requests[1:2]]})

        # 2. Back to the conversation (typed, like an optional keyboard).
        asyncio.run_coroutine_threadsafe(c.handle("go back to the chat"), loop).result(10)
        time.sleep(4)
        shot("03-conversation")

        # 3. A document on screen.
        asyncio.run_coroutine_threadsafe(c.handle("make me a plan for saturday"), loop).result(10)
        time.sleep(5)
        shot("04-document")
        info2 = asyncio.run_coroutine_threadsafe(c.screen.content_info(), loop).result(10)
        result["steps"].append({"document": info2})

        # 4. Sleep and wake.
        asyncio.run_coroutine_threadsafe(c.handle("show the chat"), loop).result(10)
        time.sleep(3)
        asyncio.run_coroutine_threadsafe(c.handle("go to sleep"), loop).result(40)   # waits for speech to end
        time.sleep(4)
        shot("05-sleeping")
        result["steps"].append({"asleep": c.asleep})
        asyncio.run_coroutine_threadsafe(c.handle("Computer"), loop).result(10)
        time.sleep(1)
        result["steps"].append({"awake_after_wake_word": not c.asleep})

        # 5. The conversation page's renderer crashes: the page comes back as it was.
        from gi.repository import GLib, WebKit2
        before = chat_log_text(c.screen)
        GLib.idle_add(lambda: c.screen.chat.terminate_web_process() and False)
        time.sleep(1)
        GLib.idle_add(lambda: c.screen._on_renderer_stopped(
            c.screen.chat, WebKit2.WebProcessTerminationReason.CRASHED) and False)
        time.sleep(8)
        after = chat_log_text(c.screen)
        shot("06-after-renderer-crash")
        # (The page comes back with the conversation's messages; the greeting it started with isn't one of them.)
        rebuilt = "Play some calm piano music" in after and before.strip().endswith(after.strip())
        result["steps"].append({"rebuilt_after_renderer_crash": rebuilt,
                                "drawing_with_processor": os.environ.get("WEBKIT_SKIA_ENABLE_CPU_RENDERING") == "1",
                                **({} if rebuilt else {"before": before[-800:], "after": after[-800:]})})
        result["spoken_chunks"] = len(player.played)
        result["tool_results"] = [json.dumps(tool_results(r))[:300] for r in mock.requests if last_is_tool_result(r)]
        result["ok"] = True
    except Exception:
        result["error"] = traceback.format_exc()
    finally:
        (OUT / "result.json").write_text(json.dumps(result, indent=1, default=str))
        from gi.repository import GLib, Gtk
        GLib.idle_add(Gtk.main_quit)


import subprocess, shutil
if shutil.which("openbox"):
    subprocess.Popen(["openbox"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1)
threading.Thread(target=scenario, daemon=True).start()
code = run_app(paths, settings, factory, windowed=False)
mock.stop()
print(json.dumps(result, indent=1, default=str))
