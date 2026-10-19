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

SONG_DONE = "Played “hello world” by Louie Zong on YouTube"


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
    # From elsewhere: YouTube's player (Louie Zong's "hello world", playing as the demo opens), a map when asked, and
    # Anthropic's API, only with the visitor's own key (Connect Claude): every request goes there, nowhere else.
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
    # It opens playing YouTube between the sidebars, from 0:03 (no sheet, no button to press, nothing above the
    # message box), and What's done says so; the cursor is in the box, under the dimmed hint.
    first = res["opening"]
    assert first["content"] and not first["sheet"] and first["toggles"] == ["true", "true", "true"]
    assert "youtube-nocookie.com/embed/Yw6u6YkTgQ4?autoplay=1" in first["src"] and "&start=3" in first["src"]
    assert first["strip"] == [] and first["last"] == ""
    assert first["messages"] == 7 and first["panel"] and first["done"][-1] == SONG_DONE and len(first["done"]) == 5
    assert first["typing_in"] == "input" and first["hint"] == "Chat computer anything or type \u201c/\u201d to search" and first["hint_weight"] == "400"
    # The two buttons at the right of the box.
    assert first["tools"] == ["Connect Claude", "Connect AirPods"] and first["tools_at"] == "box"
    # A click on the video turned the sound on (the player was told), and with the pointer off the video the keys
    # are the box's again.
    assert res["clicked_in"] == "demo-view" and res["clicked_commands"][:2] == ["unMute", "setVolume"]
    assert res["keys_back"] == "input"
    # "/": the list is in front of the video (the sound already on: nothing more told to the player).
    assert res["over"] and res["over_view"] == first["view"] and res["player_commands"] == res["clicked_commands"]
    # The video ended: the conversation's screen again, just the word logo.
    assert not res["ended"]["content"] and res["ended"]["view"] is None and res["ended"]["welcome"] == ["Chat Computer"]
    # The sidebars' switches: off (their icons empty) and on again, and the choice is kept.
    assert res["sides_off"]["sides"] == [False, False] and res["sides_off"]["toggles"][:2] == ["false", "false"]
    assert json.loads(res["kept"]) == {"left": False, "right": False, "bottom": True}
    assert res["sides_on"]["sides"] == [True, True] and res["sides_on"]["welcome"] == ["Chat Computer"]
    # Asked for music: YouTube again, inside the frame, and What's done says so.
    music = res["music"]
    assert music["content"] and music["last"] == "Playing “hello world” by Louie Zong."
    assert music["done"][-1] == SONG_DONE and "youtube-nocookie.com" in music["src"]
    # Above the message box: what was asked and Chat Computer's answer, in full.
    assert music["strip"] == ["play hello world", "Playing “hello world” by Louie Zong."]
    f, v = music["frame"], music["view"]
    assert f[0] < v[0] and f[1] < v[1] and v[0] + v[2] < f[0] + f[2] and v[1] + v[3] < f[1] + f[3]
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
    api = res["api"]
    assert len(api) == 3 and all(a["url"] == "https://api.anthropic.com/v1/messages" for a in api)
    assert all(a["key"] == "sk-ant-api03-demo-test-key" and a["direct"] == "true" and a["model"] == "claude-sonnet-5-5" for a in api)
    assert not api[0]["stream"] and api[1]["stream"] and "show_map" in api[1]["tools"] and "web_search" in api[1]["tools"]
    assert "tool_result" in api[2]["last"] and "toolu_1" in api[2]["last"]
    # Disconnecting: the key leaves the browser, and the written answers are back.
    assert res["claude_off_sheet"]["sheet_title"] == "Claude is connected"
    assert res["claude_off"]["tools"][0] == "Connect Claude" and res["stored_after"] is None
    # Connect AirPods says what it does before the microphone is asked for.
    assert res["talk_sheet"]["sheet"] and res["talk_sheet"]["sheet_title"] == "Talk through your AirPods"
    # A phone: no sidebars and no switches; it opens playing too, the buttons are inside the box, and it works.
    phone = res["phone_opening"]
    assert phone["content"] and phone["toggles"] == [False, False, False] and phone["tools_at"] == "row"
    assert not res["phone"]["panel"] and res["phone"]["last"].startswith("Hi! I'm Chat Computer")
