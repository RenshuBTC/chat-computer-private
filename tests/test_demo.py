"""The browser demo in docs/ (for GitHub Pages): ClaudeOS's own screen, with docs/demo/host.js standing in for the
computer. It must be the real screen as it is now (scripts/build_demo.py makes it from claudeos/ui/assets), keep
what's typed in the browser, and work: tried in Chromium when Playwright is there."""
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


def test_the_demo_is_claudeos_s_real_screen_as_it_is_now():
    stale = [str(p.relative_to(ROOT)) for p, data in build_demo.files().items()
             if not p.exists() or p.read_bytes() != data]
    assert stale == [], f"run: python scripts/build_demo.py ({stale})"
    page = (ROOT / "docs" / "index.html").read_text()
    # The demo's host comes before ClaudeOS's own scripts, which are the real ones.
    assert page.index("demo/host.js") < page.index("demo/md.js") < page.index("demo/chat.js")


def test_nothing_typed_in_the_demo_leaves_the_browser():
    host = (ROOT / "docs" / "demo" / "host.js").read_text()
    for call in ("fetch(", "XMLHttpRequest", "sendBeacon", "WebSocket", "EventSource", "navigator.geolocation"):
        assert call not in host, call
    # The one thing from elsewhere: the map, when asked for one.
    assert set(re.findall(r"https?://[^/\"' ]+", host)) == {"https://www.openstreetmap.org"}
    assert "not made by Anthropic" in host


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
    r = subprocess.run([node, str(ROOT / "tests" / "demo_check.js")], capture_output=True, text=True, timeout=150,
                       env={**os.environ, "NODE_PATH": modules})
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "failed" not in res, res["failed"]
    assert res["errors"] == []
    # The intro, then the screen as ClaudeOS shows it: Messages, What's done, the conversation.
    assert res["intro"] == "ClaudeOS, in your browser" and not res["start"]["sheet"]
    assert res["start"]["messages"] == 7 and len(res["start"]["done"]) == 4 and res["start"]["panel"]
    # Asked for music: it opens between the sidebars, inside the frame, and What's done says so.
    music = res["music"]
    assert music["content"] and music["last"] == "Playing some calm piano." and music["done"][-1] == "Played some calm piano"
    f, v = music["frame"], music["view"]
    assert f[0] < v[0] and f[1] < v[1] and v[0] + v[2] < f[0] + f[2] and v[1] + v[3] < f[1] + f[3]
    assert "Calm piano" in res["music_view"]
    # F1: the whole window, and back; Escape: closed.
    assert res["f1"]["view"] == [0, 0, 1600, 900] and res["f1_back"]["view"] == v
    assert not res["escaped"]["content"] and res["escaped"]["view"] is None
    # "/": the demo's files; Enter opens one.
    assert res["search"]["results"][0].startswith("Packing list.md")
    assert res["opened"]["content"] and res["opened_view"].startswith("Packing list")
    # A message opens as the conversation with its sender; a timer made on the spot; anything else: what to try.
    assert "12:30 at the usual place" in res["jane"] and res["timer"].startswith("Timer 5:00")
    assert res["after_timer"]["done"][-1] == "Made a timer" and res["fallback"].startswith("In this demo I only know")
    # A phone: no sidebars, the conversation works.
    assert not res["phone"]["panel"] and res["phone"]["last"].startswith("Hi! I'm ClaudeOS")
