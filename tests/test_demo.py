"""The browser demo in docs/ (for GitHub Pages): Chat Computer's own screen, with docs/demo/host.js standing in for the
computer. It must be the real screen as it is now (scripts/build_demo.py makes it from chatcomputer/ui/assets), keep
what's typed in the browser unless the visitor connects Claude (then it goes only to Anthropic), and work: tried in
Chromium when Playwright is there."""
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import build_demo  # noqa: E402


def test_the_demo_is_chatcomputer_s_real_screen_as_it_is_now():
    stale = [str(p.relative_to(ROOT)) for p, data in build_demo.files().items()
             if not p.exists() or p.read_bytes() != data]
    assert stale == [], f"run: python scripts/build_demo.py ({stale})"
    page = (ROOT / "docs" / "index.html").read_text()
    # The demo's host comes before Chat Computer's own scripts, which are the real ones; its own look after theirs.
    assert page.index("demo/host.js") < page.index("demo/md.js") < page.index("demo/chat.js")
    assert page.index("demo/chat.css") < page.index("demo/demo.css")


def test_what_s_typed_stays_in_the_browser_unless_claude_is_connected():
    host = (ROOT / "docs" / "demo" / "host.js").read_text()
    for call in ("XMLHttpRequest", "sendBeacon", "WebSocket", "EventSource", "navigator.geolocation"):
        assert call not in host, call
    # From elsewhere: a map when asked, and with the visitor's own key (Connect Claude) Anthropic's API, where every
    # request goes, and YouTube's player for a video Claude plays.
    assert set(re.findall(r"https?://[^/\"' ]+", host)) == {
        "https://www.youtube-nocookie.com", "https://www.openstreetmap.org", "https://api.anthropic.com"}
    fetches = re.findall(r"fetch\(([^,)]+)", host)
    assert fetches and all(f.startswith("API +") for f in fetches), fetches
    assert host.count("x-api-key") == 1 and "anthropic-dangerous-direct-browser-access" in host
    assert "not made by Anthropic" in host and "written in advance" in host
    # Claude Pro or Max sign-ins aren't for other apps: a sign-in token is turned away, and the sheet says why.
    assert "sk-ant-oat" in host and "Anthropic doesn't let other apps sign in with one" in host


def _node_with_playwright():
    node = shutil.which("node")
    if not node:
        return None, None
    root = subprocess.run(["npm", "root", "-g"], capture_output=True, text=True).stdout.strip() if shutil.which("npm") else ""
    if not root or not (Path(root) / "playwright").exists():
        return None, None
    return node, root


@pytest.mark.skipif(_node_with_playwright()[0] is None, reason="node with Playwright not available")
def test_the_demo_works_in_a_browser():
    node, modules = _node_with_playwright()
    r = subprocess.run([node, str(ROOT / "tests" / "demo_check.js")], capture_output=True, text=True, timeout=240,
                       env={**os.environ, "NODE_PATH": modules})
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "failed" not in res, res["failed"]
    assert res["errors"] == []
    # It opens with Chat Computer's first words, hello, world (nothing open, no sheet, no button to press), the
    # cursor in the box under the dimmed hint, and the two buttons at the box's right.
    first = res["opening"]
    assert not first["content"] and not first["sheet"] and first["view"] is None and first["toggles"] == ["true", "true", False]
    assert first["first_ai"] == first["last"] == "hello, world" and first["said"] == 1
    assert first["messages"] == 7 and first["panel"] and len(first["done"]) == 4
    assert first["typing_in"] == "input" and first["hint"] == "Chat computer anything or type \u201c/\u201d to search"
    assert first["hint_weight"] == "400" and first["tools"] == ["Connect Claude", "Connect AirPods"] and first["tools_at"] == "box"
    # The sidebars' switches: off (their icons empty) and on again, and the choice is kept.
    assert res["sides_off"]["sides"] == [False, False] and res["sides_off"]["toggles"][:2] == ["false", "false"]
    assert json.loads(res["kept"]) == {"left": False, "right": False, "bottom": True}
    assert res["sides_on"]["sides"] == [True, True] and res["sides_on"]["first_ai"] == "hello, world"
    # Asked for music: soft piano made by the browser, between the sidebars, and What's done says so.
    music = res["music"]
    assert music["content"] and music["last"] == "Playing some calm piano." and music["done"][-1] == "Played some calm piano"
    # Above the message box: what was asked and Chat Computer's answer, in full.
    assert music["strip"] == ["play some piano music", "Playing some calm piano."]
    f, v = music["frame"], music["view"]
    assert f[0] < v[0] and f[1] < v[1] and v[0] + v[2] < f[0] + f[2] and v[1] + v[3] < f[1] + f[3]
    # "/": the list is in front of what's open.
    assert res["over"] and res["over_view"] == v
    # F1: the whole window, and back.
    assert res["f1"]["view"] == [0, 0, 1600, 900] and res["f1_back"]["view"] == v
    # The conversation's switch: the page reaches down to it; typing brings the conversation back.
    off, back = res["bottom_off"], res["bottom_back"]
    assert not off["composer"] and off["toggles"][2] == "false" and off["view"][3] > v[3] + 100
    assert back["composer"] and back["toggles"][2] == "true" and back["view"] == v
    assert not res["escaped"]["content"] and res["escaped"]["view"] is None
    # "/": the demo's files; Enter opens one.
    assert res["search"]["results"][0].startswith("Packing list.md")
    assert res["opened"]["content"] and res["opened_view"].startswith("Packing list")
    # A message opens as the conversation with its sender; a timer made on the spot.
    assert "12:30 at the usual place" in res["jane"] and res["timer"].startswith("Timer 5:00")
    assert res["after_timer"]["done"][-1] == "Made a timer"
    # A long answer while something is open: both messages in full above the box (no "more").
    long = res["fallback"]
    assert long["more"] == 0 and long["strip"][0] == "what's the meaning of life?"
    assert long["strip"][1].startswith("In this demo I only know") and long["strip"][1].endswith("Type / to search the files.")
    # Connect Claude: a key, checked with Anthropic; then Claude answers and does things here (a map, as asked).
    assert res["claude_sheet"]["sheet"] and res["claude_sheet"]["sheet_title"] == "Connect Claude"
    assert not res["claude_on"]["sheet"] and res["claude_on"]["tools"][0] == "Claude connected"
    assert json.loads(res["stored"]) == "sk-ant-api03-demo-test-key"
    m = res["claude_map"]
    assert m["last"] == "Here's Tokyo." and "openstreetmap.org/export/embed.html?bbox=139.55%2C35.55%2C139.95%2C35.8" in m["src"]
    assert m["done"][-1] == "Opened openstreetmap.org" and m["strip"] == ["show me Tokyo", "Here's Tokyo."]
    # A video Claude plays: YouTube's player between the sidebars, What's done says so, and when it ends the
    # conversation has the middle again.
    cv = res["claude_video"]
    assert cv["content"] and "youtube-nocookie.com/embed/aBcDeFgHiJk?autoplay=1" in cv["src"] and cv["last"] == "Playing it."
    assert cv["done"][-1] == "Played \u201cA test video\u201d on YouTube"
    assert not res["video_ended"]["content"] and res["video_ended"]["view"] is None
    api = res["api"]
    assert len(api) == 5 and all(a["url"] == "https://api.anthropic.com/v1/messages" for a in api)
    assert all(a["key"] == "sk-ant-api03-demo-test-key" and a["direct"] == "true" and a["model"] == "claude-sonnet-5-5" for a in api)
    assert not api[0]["stream"] and api[1]["stream"] and "show_map" in api[1]["tools"] and "web_search" in api[1]["tools"]
    assert "tool_result" in api[2]["last"] and "toolu_1" in api[2]["last"]
    # Disconnecting: the key leaves the browser, and the written answers are back.
    assert res["claude_off_sheet"]["sheet_title"] == "Claude is connected"
    assert res["claude_off"]["tools"][0] == "Connect Claude" and res["stored_after"] is None
    # Connect AirPods says what it does before the microphone is asked for.
    assert res["talk_sheet"]["sheet"] and res["talk_sheet"]["sheet_title"] == "Talk through your AirPods"
    # A phone: no sidebars and no switches; the buttons are inside the box, and it works.
    phone = res["phone_opening"]
    assert phone["first_ai"] == "hello, world" and phone["toggles"] == [False, False, False]
    assert phone["tools_at"] == "row"
    assert not res["phone"]["panel"] and res["phone"]["last"].startswith("Hi! I'm Chat Computer")
