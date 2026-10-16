"""ClaudeOS on a real Mac's screen, with a scripted Claude (tests/mock_anthropic.py) in place of a Claude
account: the real window (AppKit and WebKit), the real Claude Code engine, the real tools, the real Mac layer
(claudeos/platform/mac.py) and the real speech models, with a pretend microphone.

Run it with ClaudeOS's own Python after setup (it has AppKit, WebKit and the speech packages, plus aiohttp for
the pretend Claude), from the repository:

    ~/ClaudeOS/runtime/bin/python tests/mac_ci/mac_scenario.py OUTDIR

It talks to ClaudeOS (by voice, then typed), opens a website and a document, goes to sleep and wakes, steps aside
to show macOS and comes back, rebuilds its conversation page, and at the end quits the way Command-Q does.
Screenshots and result.json go to OUTDIR. The exit status is ClaudeOS's own, after quitting: 0 means it quit
cleanly (result.json says whether each step worked). tests/mac_ci/run.sh runs this on GitHub's Macs.
"""
import asyncio
import http.server
import json
import os
import socketserver
import subprocess
import sys
import threading
import time
import traceback
from pathlib import Path

HERE = Path(__file__).resolve().parent
TESTS = HERE.parent
sys.path.insert(0, str(TESTS))
sys.path.insert(0, str(TESTS.parent))

OUT = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/claudeos-mac-scenario").resolve()
OUT.mkdir(parents=True, exist_ok=True)
# The speech models setup downloaded (before HOME changes below).
MODELS = Path(os.environ.get("CLAUDEOS_TEST_MODELS") or (Path.home() / "ClaudeOS" / "models"))
for k in list(os.environ):
    if k.startswith(("CLAUDE", "ANTHROPIC")) or k.lower() in ("https_proxy", "http_proxy"):
        os.environ.pop(k, None)
HOME = OUT / "home"
HOME.mkdir(exist_ok=True)
os.environ["HOME"] = str(HOME)
os.environ["CLAUDEOS_HOME"] = str(HOME / "ClaudeOS")
os.environ["NO_PROXY"] = "127.0.0.1,localhost"

from mock_anthropic import (MockAnthropic, last_message_is_tool_result, last_user_text, text_block,  # noqa: E402
                            tool_results, tool_use_block)

# ------------------------------------------------------------------ a small local "website"
SITE = OUT / "site"
SITE.mkdir(exist_ok=True)
(SITE / "index.html").write_text("""<!doctype html><html><head><meta charset=utf-8><title>Test Video Site</title>
<style>body{font-family:sans-serif;background:#fafafa;margin:40px} .tile{display:inline-block;width:300px;height:170px;
background:#c33;color:#fff;margin:10px;padding:10px;font-size:22px;cursor:pointer}</style></head><body>
<h1>Test Video Site</h1><div class=tile onclick="location.href='watch.html'">Calm piano music for focus</div>
<div class=tile>Lo-fi beats</div></body></html>""")
(SITE / "watch.html").write_text("""<!doctype html><html><head><meta charset=utf-8><title>Calm piano music - Test Video Site</title></head>
<body style="background:#111;color:#eee;font-family:sans-serif"><h2>Calm piano music for focus</h2><p>Now playing.</p></body></html>""")


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass


httpd = socketserver.TCPServer(("127.0.0.1", 0), lambda *a: Quiet(*a, directory=str(SITE)))
threading.Thread(target=httpd.serve_forever, daemon=True).start()
SITE_URL = f"http://127.0.0.1:{httpd.server_address[1]}/"

# ------------------------------------------------------------------ scripted Claude
mock = MockAnthropic()
mock.start()


def rule(req):
    said = last_user_text(req).split("\n")[-1].lower()
    if last_message_is_tool_result(req):
        res = json.dumps(tool_results(req))
        if "Showing: Test Video Site" in res:
            return [tool_use_block("mcp__claudeos__click", {"text": "Calm piano music"})]
        if "Clicked" in res:
            return [text_block("It's playing now.")]
        if "Showing the document" in res:
            return [text_block("I've put the plan on the screen.")]
        return [text_block("Done.")]
    if "piano" in said:
        return [text_block("Sure, opening the video site."),
                tool_use_block("mcp__claudeos__show_website", {"url": SITE_URL})]
    if "macos" in said or "mac os" in said:
        return [tool_use_block("mcp__claudeos__power", {"action": "switch_to_macos"})]
    if "chat" in said or "conversation" in said:
        return [tool_use_block("mcp__claudeos__show_conversation", {})]
    if "plan" in said:
        return [tool_use_block("mcp__claudeos__show_document", {"title": "Plan for Saturday", "markdown":
                "## Morning\n- Run at **7am** by the river\n- Breakfast at Tiong Bahru\n\n## Afternoon\n"
                "| Time | What |\n|---|---|\n| 2pm | Library |\n| 5pm | Groceries |\n\nRemember to bring the *library card*."})]
    return [text_block("Hello! I'm ClaudeOS. Tell me what you'd like to do, and I'll handle it.")]


mock.rules.append(rule)

# ------------------------------------------------------------------ ClaudeOS
from AppKit import NSApplication, NSScreen  # noqa: E402
from PyObjCTools import AppHelper  # noqa: E402

from claudeos.config import Paths, Settings, save_token  # noqa: E402
from claudeos.controller import Controller  # noqa: E402
from claudeos.journal import setup_logging  # noqa: E402
from claudeos.platform.mac import MacPlatform  # noqa: E402
from claudeos.ui.mac_app import run_app  # noqa: E402
from claudeos.voice.audio import FakeMic, FakePlayer  # noqa: E402

paths = Paths().ensure()
setup_logging(paths, verbose=True)
save_token(paths, "sk-ant-api03-" + "t" * 40)
settings = Settings(user_name="Ren Shu", stt_model=os.environ.get("CLAUDEOS_TEST_STT", "small.en"))
platform = MacPlatform()
mics = []
player = FakePlayer()
stt = tts = None
kokoro = MODELS / "kokoro"
if (kokoro / "kokoro-v1.0.int8.onnx").exists():
    from claudeos.voice.tts import TTS
    tts = TTS(str(kokoro / "kokoro-v1.0.int8.onnx"), str(kokoro / "voices-v1.0.bin"))
wd = MODELS / "whisper" / f"faster-whisper-{settings.stt_model}"
if (wd / "model.bin").exists():
    from claudeos.voice.stt import STT
    stt = STT(str(wd), "en", hint="Claude, Ren Shu, YouTube")
VOICE = stt is not None and tts is not None

result = {"ok": False, "steps": [], "voice_models": VOICE, "models_dir": str(MODELS)}
holder = {}


def factory(screen):
    c = Controller(paths, settings, platform, screen, voice=VOICE,
                   mic_factory=lambda cb: mics.append(FakeMic(cb)) or mics[-1],
                   player=player, stt=stt, tts=tts, index=True, extra_env={"ANTHROPIC_BASE_URL": mock.url}, greet=True)
    holder["c"] = c
    return c


def on_main(fn, timeout=15):
    """Runs fn on the main thread (AppKit's) and returns what it returns."""
    box, ev = {}, threading.Event()

    def run():
        try:
            box["v"] = fn()
        except Exception as exc:
            box["e"] = exc
        finally:
            ev.set()
    AppHelper.callAfter(run)
    if not ev.wait(timeout):
        raise TimeoutError("the main thread didn't answer")
    if "e" in box:
        raise box["e"]
    return box.get("v")


def shot(name):
    """The whole screen as a person sees it, and what ClaudeOS's window shows (WebKit's own picture)."""
    p = OUT / f"{name}.png"
    try:
        subprocess.run(["screencapture", "-x", "-t", "png", str(p)], timeout=30, capture_output=True)
    except Exception:
        pass
    try:
        png = holder["c"].screen.snapshot_png()
        if png:
            (OUT / f"{name}-window.png").write_bytes(png)
    except Exception:
        pass


def chat_text(screen):
    """The conversation as the page shows it."""
    box, ev = {}, threading.Event()

    def done(res, err):
        box["text"] = str(res) if res is not None else f"(error: {err})"
        ev.set()
    AppHelper.callAfter(lambda: screen.chat.evaluateJavaScript_completionHandler_(
        "document.getElementById('log').innerText", done))
    ev.wait(10)
    return box.get("text", "")


def wait_for(cond, seconds, step=0.5):
    end = time.time() + seconds
    while time.time() < end:
        try:
            if cond():
                return True
        except Exception:
            pass
        time.sleep(step)
    return False


def step(name, ok, **info):
    result["steps"].append({"step": name, "ok": bool(ok), **info})
    (OUT / "result.json").write_text(json.dumps(result, indent=1, default=str))


def speak_into_mic(text):
    from claudeos.voice.tts import _resample
    import numpy as np
    a = _resample(tts.synth(text), 24000, 16000)
    audio = np.concatenate([np.zeros(8000, np.float32), a, np.zeros(16000, np.float32)])
    mics[-1].feed(audio, realtime=True, speed=2.0)


def say(c, text, wait=10):
    asyncio.run_coroutine_threadsafe(c.handle(text), c.loop).result(wait)


def content(c):
    return asyncio.run_coroutine_threadsafe(c.screen.content_info(), c.loop).result(10)


def scenario():
    try:
        if not wait_for(lambda: "c" in holder and holder["c"].loop is not None, 30):
            raise RuntimeError("ClaudeOS didn't start")
        c = holder["c"]
        app = NSApplication.sharedApplication()

        # 1. Started: the window fills the screen, in front, and Claude's greeting is on it.
        greeted = wait_for(lambda: len(mock.requests) >= 1 and "Hello" in chat_text(c.screen), 90, 1)
        time.sleep(2)
        shot("01-start")
        geo = on_main(lambda: {"screen": tuple(NSScreen.mainScreen().frame().size),
                               "window": tuple(c.screen.win.frame().size),
                               "scale": float(NSScreen.mainScreen().backingScaleFactor()),
                               "visible": bool(c.screen.win.isVisible()), "active": bool(app.isActive())})
        step("starts full screen with Claude's greeting", greeted and geo["visible"] and geo["window"] == geo["screen"],
             greeted=greeted, geometry=geo, layout=c.screen.layout_info(), requests=len(mock.requests))

        # 2. Talking: real speech through the pretend microphone, Silero, Whisper; Claude opens a website and
        #    clicks in it.
        if VOICE and mics:
            before = len(mock.requests)
            speak_into_mic("Play some calm piano music.")
            wait_for(lambda: "It's playing now" in json.dumps(mock.requests[-1:]) or
                     (c.brain and not c.brain.busy and len(mock.requests) >= before + 3), 120, 1)
            time.sleep(2.5)
            heard = [last_user_text(r)[-80:] for r in mock.requests[before:before + 1]]
        else:
            heard = []
            say(c, "Play some calm piano music.")
            wait_for(lambda: "It's playing now" in json.dumps(mock.requests[-1:]), 60, 1)
            time.sleep(2.5)
        shot("02-website")
        info = content(c)
        step("voice: opens the website and clicks in it", info.get("mode") == "content" and
             "Calm piano music" in str(info.get("title")), content=info, heard=heard, voice=VOICE and bool(mics))

        # 3. Typed (like the phone): back to the conversation.
        say(c, "go back to the chat")
        wait_for(lambda: content(c).get("mode") == "chat", 30)
        time.sleep(1.5)
        shot("03-conversation")
        step("back to the conversation", content(c).get("mode") == "chat")

        # 4. A document on screen.
        say(c, "make me a plan for saturday")
        wait_for(lambda: content(c).get("title") == "Plan for Saturday", 40)
        time.sleep(2)
        shot("04-document")
        step("shows a document", content(c).get("title") == "Plan for Saturday", content=content(c))

        # 5. Sleep and wake.
        say(c, "show the chat")
        time.sleep(3)
        say(c, "go to sleep", 60)
        wait_for(lambda: c.asleep, 30)
        time.sleep(2)
        shot("05-sleeping")
        asleep = c.asleep
        say(c, "Claude")
        time.sleep(1)
        step("sleeps and wakes to its name", asleep and not c.asleep)

        # 6. "Switch to macOS": ClaudeOS hides and the Mac's desktop shows; its name brings it back.
        say(c, "switch to macOS")
        hidden = wait_for(lambda: on_main(lambda: bool(app.isHidden())), 30)
        time.sleep(2)
        shot("06-macos-showing")
        aside = c.asleep
        say(c, "Claude")
        back = wait_for(lambda: on_main(lambda: not app.isHidden() and bool(c.screen.win.isVisible())), 30)
        time.sleep(2)
        shot("07-back")
        step("steps aside to show macOS, and comes back", hidden and aside and back, hidden=hidden,
             listening_for_name_only=aside, back=back)

        # 7. The conversation page's renderer stops (WebKit reports that when its process dies): the page is
        #    loaded again with the conversation as it was.
        before = chat_text(c.screen)
        AppHelper.callAfter(lambda: c.screen._on_renderer_stopped(c.screen.chat))
        wait_for(lambda: c.screen.chat_ready and not c.screen._reloading, 20)
        time.sleep(3)
        after = chat_text(c.screen)
        shot("08-after-page-reload")
        step("rebuilds the conversation after its page stops", bool(before.strip()) and after.strip() == before.strip())

        result["spoken_chunks"] = len(player.played)
        result["tool_results"] = [json.dumps(tool_results(r))[:300] for r in mock.requests if last_message_is_tool_result(r)]
        result["ok"] = all(s["ok"] for s in result["steps"])
    except Exception:
        result["error"] = traceback.format_exc()
    finally:
        result["quit_asked_at"] = time.time()
        (OUT / "result.json").write_text(json.dumps(result, indent=1, default=str))
        # 8. Quit the way Command-Q does: macOS asks ClaudeOS, which stops cleanly, then lets it go.
        AppHelper.callAfter(lambda: NSApplication.sharedApplication().terminate_(None))


threading.Thread(target=scenario, daemon=True).start()
code = run_app(paths, settings, factory, windowed=False)
mock.stop()
sys.exit(code)
