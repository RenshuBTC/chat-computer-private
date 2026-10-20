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
    # request goes, and YouTube's player for a video Claude plays. For a place's map, weather or air, Open-Meteo, sent
    # only the place's name (then its latitude and longitude). What's looked up (by the browser's AI, "look it up", a
    # question of fact, the news) goes to Wikipedia: only the words looked up. YouTube's search, Google's, Google Maps
    # and BBC News are only links, opened by the visitor in a new tab (open-meteo.com too: the air's source, named).
    assert set(re.findall(r"https?://[^/\"' ]+", host)) == {
        "https://www.youtube-nocookie.com", "https://www.openstreetmap.org", "https://api.anthropic.com",
        "https://geocoding-api.open-meteo.com", "https://api.open-meteo.com", "https://air-quality-api.open-meteo.com",
        "https://en.wikipedia.org", "https://open-meteo.com",
        "https://www.youtube.com", "https://www.google.com", "https://www.bbc.com"}
    fetches = re.findall(r"fetch\(([^,)]+)", host)
    assert sorted(f.split("?")[0] for f in fetches if not f.startswith("API +")) == [
        '"https://api.open-meteo.com/v1/forecast', '"https://geocoding-api.open-meteo.com/v1/search', "url"], fetches
    assert [f for f in fetches if "open-meteo" in f][0].endswith("&name=\" + encodeURIComponent(name")
    # getJSON(url): Wikipedia's API (search, or a day's Current events), and the air where a place is.
    gets = re.findall(r"getJSON\(([^,)]+)", host)
    assert gets and all(g.startswith(("WIKI +", '"https://air-quality-api.open-meteo.com/v1/air-quality?latitude=" + p.lat'))
                        for g in gets if g != "url"), gets
    assert 'var WIKI = "https://en.wikipedia.org/w/api.php?' in host and "gsrsearch=\" + encodeURIComponent(q)" in host
    for site in ("https://www.youtube.com/", "https://www.google.com/", "https://www.bbc.com/"):
        for at in [m.start() for m in re.finditer(re.escape(site), host)]:
            line = host[host.rfind("\n", 0, at):host.find("\n", at)]
            assert "var url = " in line or "link(" in line, line
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
    # It opens on the word logo and Chat Computer's first words, hello, world (nothing open, no sheet, no button to
    # press), the cursor in the box under the dimmed hint, and the two buttons at the box's right.
    first = res["opening"]
    assert not first["content"] and not first["sheet"] and first["view"] is None and first["toggles"] == ["true", "true", False]
    assert first["first_ai"] == first["last"] == "hello, world" and first["said"] == 1 and first["logo"] == "Chat Computer"
    assert first["messages"] == 7 and first["panel"] and len(first["done"]) == 4
    assert first["typing_in"] == "input" and first["hint"] == "Chat computer anything or type \u201c/\u201d to search"
    assert first["hint_weight"] == "400" and first["tools"] == ["Connect Claude", "Connect AirPods"] and first["tools_at"] == "box"
    # The sidebars' switches: off (their icons empty) and on again, and the choice is kept.
    assert res["sides_off"]["sides"] == [False, False] and res["sides_off"]["toggles"][:2] == ["false", "false"]
    assert json.loads(res["kept"]) == {"left": False, "right": False, "bottom": True}
    assert res["sides_on"]["sides"] == [True, True] and res["sides_on"]["first_ai"] == "hello, world"
    assert res["sides_on"]["logo"] == "Chat Computer"
    # Asked for music: soft piano made by the browser, between the sidebars, and Actions says so.
    music = res["music"]
    assert music["content"] and music["last"] == "Playing some calm piano." and music["done"][-1] == "Played some calm piano"
    # The first thing said took the word logo away; hello, world is still the conversation's first message.
    assert music["logo"] is None and music["first_ai"] == "hello, world"
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
    assert long["strip"][1].startswith("I can't answer that on this page without an AI") and long["strip"][1].endswith("Type / to search the files.")
    # Connect Claude: a key, checked with Anthropic; then Claude answers and does things here (a map, as asked).
    assert res["claude_sheet"]["sheet"] and res["claude_sheet"]["sheet_title"] == "Connect Claude"
    assert not res["claude_on"]["sheet"] and res["claude_on"]["tools"][0] == "Claude connected"
    assert json.loads(res["stored"]) == "sk-ant-api03-demo-test-key"
    m = res["claude_map"]
    assert m["last"] == "Here's Tokyo." and "openstreetmap.org/export/embed.html?bbox=139.55%2C35.55%2C139.95%2C35.8" in m["src"]
    assert m["done"][-1] == "Opened openstreetmap.org" and m["strip"] == ["show me Tokyo", "Here's Tokyo."]
    # A video Claude plays: YouTube's player between the sidebars, Actions says so, and when it ends the
    # conversation has the middle again.
    cv = res["claude_video"]
    assert cv["content"] and "youtube-nocookie.com/embed/aBcDeFgHiJk?autoplay=1" in cv["src"] and cv["last"] == "Playing it."
    assert cv["done"][-1] == "Played \u201cA test video\u201d on YouTube"
    assert not res["video_ended"]["content"] and res["video_ended"]["view"] is None
    api = res["api"]
    assert len(api) == 5 and all(a["url"] == "https://api.anthropic.com/v1/messages" for a in api)
    assert all(a["key"] == "sk-ant-api03-demo-test-key" and a["direct"] == "true" and a["model"] == "claude-opus-5-5" for a in api)
    assert not api[0]["stream"] and api[1]["stream"] and "show_map" in api[1]["tools"] and "web_search" in api[1]["tools"]
    assert "tool_result" in api[2]["last"] and "toolu_1" in api[2]["last"]
    # Opus 5.5 always thinks first, within max_tokens: room for it, at the API's own effort (nothing sent for either).
    assert all(a["max_tokens"] == 16000 and a["thinking"] is None and a["effort"] is None for a in api[1:])
    # Its thinking is only valid while the system prompt, the tools and the messages before it stay the same: the
    # system prompt is the same all conversation (the time is in each question instead), the turn's own thinking
    # goes back with the tool's result as it came, and a finished turn's is left out after.
    assert len({a["system"] for a in api[1:]}) == 1 and "It's " not in api[1]["system"]
    first_q = json.loads(api[1]["all"])[0]["content"]
    assert re.fullmatch(r"\[\w+day \d{1,2} \w+ 20\d\d.*\] show me Tokyo", first_q), first_q
    assert '"signature":"sig-toolu_1"' in api[2]["all"] and "sig-toolu_1" not in api[3]["all"] and "toolu_1" in api[3]["all"]
    assert '"signature":"sig-toolu_2"' in api[4]["all"]
    # Disconnecting: the key leaves the browser, and the written answers are back.
    assert res["claude_off_sheet"]["sheet_title"] == "Claude is connected"
    assert res["claude_off"]["tools"][0] == "Connect Claude" and res["stored_after"] is None
    # Connect AirPods says what it does before the microphone is asked for.
    assert res["talk_sheet"]["sheet"] and res["talk_sheet"]["sheet_title"] == "Talk through your AirPods"
    # Chrome's built-in AI, where Chrome has it: "Download Gemini" first; its sheet says what it is before anything is
    # downloaded; the download's progress on the button; then it answers what the written answers can't (a page on
    # the screen, said in Actions), from a session told what Chat Computer is, and the written answers still
    # answer what they can.
    assert res["ai_opening"]["tools"] == ["Download Gemini", "Connect Claude", "Connect AirPods"]
    assert res["ai_sheet"]["sheet"] and res["ai_sheet"]["sheet_title"] == "Download Gemini"
    assert not res["ai_downloading"]["sheet"] and res["ai_downloading"]["tools"][0] == "Downloading 50%"
    on = res["ai_on"]
    assert on["tools"] == ["Connect Claude", "Connect AirPods"] and on["done"][-1] == "Turned on Chrome's built-in AI"
    poem = res["ai_poem"]
    assert poem["content"] and poem["last"] == "Here's a short poem about the sea." and poem["done"][-1] == "Showed \u201cThe sea\u201d"
    assert "The tide comes in on silver feet." in res["ai_poem_view"] and "Written by Chrome's built-in AI" in res["ai_poem_view"]
    assert res["ai_music"]["last"] == "Playing some calm piano."
    calls = res["ai_calls"]
    assert len(calls["created"]) == 1 and calls["created"][0]["monitored"] and "Gemini Nano" in calls["created"][0]["system"]
    assert calls["prompts"] == [{"text": "write a short poem about the sea", "schema": True}]
    # A phone: no sidebars and no switches; the buttons are inside the box, and it works.
    phone = res["phone_opening"]
    assert phone["first_ai"] == "hello, world" and phone["logo"] == "Chat Computer" and phone["toggles"] == [False, False, False]
    assert phone["tools_at"] == "row"
    assert not res["phone"]["panel"] and res["phone"]["last"].startswith("Hi! I'm Chat Computer")
    # Music, and then asked about it ("turn the music down", "stop the music": music words too): turned down (the
    # same music, quieter, not started again), then stopped.
    assert res["phone_music"]["content"] and res["phone_music"]["last"] == "Playing some calm piano."
    down = res["phone_down"]
    assert down["content"] and down["last"] == "Volume is 40%." and down["done"][-1] == "Turned the volume down"
    assert res["phone_down_piano"] == {"same": True, "vol": 0.4}
    stopped = res["phone_stopped"]
    assert not stopped["content"] and stopped["view"] is None and stopped["last"] == "Closed it."
