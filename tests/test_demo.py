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
    assert "demo/demo.css" not in page and not (ROOT / "docs" / "demo" / "demo.css").exists()     # no buttons of its own
    # No icon, anywhere: the tab shows the browser's own, and the word logo is the words alone.
    assert 'rel="icon"' not in page and "apple-touch-icon" not in page and "<img" not in page
    assert not any((ROOT / "docs" / n).exists() for n in ("favicon.ico", "favicon.svg", "apple-touch-icon.png"))
    assert not (ROOT / "chatcomputer" / "ui" / "assets" / "icon.svg").exists()
    # A phone: nothing zooms (and Safari doesn't zoom in on the box either); its keyboard's return key says Send.
    assert 'content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no' in page
    assert 'enterkeyhint="send"' in page


def test_what_s_typed_stays_in_the_browser_unless_claude_is_connected():
    host = (ROOT / "docs" / "demo" / "host.js").read_text()
    for call in ("XMLHttpRequest", "sendBeacon", "WebSocket", "EventSource", "navigator.geolocation"):
        assert call not in host, call
    # From elsewhere: a map when asked, and with the visitor's own key (Connect Claude) Anthropic's API, where every
    # request goes, and YouTube's player for a video Claude plays. For a place's map, weather or air, Open-Meteo, sent
    # only the place's name (then its latitude and longitude). What's looked up (by the browser's AI, "look it up", a
    # question of fact, the news) goes to Wikipedia: only the words looked up. Google's own pages show in the middle
    # when asked for (Google, its search for what Wikipedia hasn't got, Google Maps' directions: the addresses Google
    # lets other pages show, igu=1 and output=embed). YouTube's search and BBC News are only links, opened by the
    # visitor in a new tab (open-meteo.com too: the air's source, named). Gemma, when the visitor downloads it:
    # Transformers.js, a fixed version, from jsDelivr, and Google's model from Hugging Face (where the browser keeps
    # its copy, by that address); it runs on the device, so nothing typed is sent anywhere.
    assert set(re.findall(r"https?://[^/\"' ]+", host)) == {
        "https://www.youtube-nocookie.com", "https://www.openstreetmap.org", "https://api.anthropic.com",
        "https://geocoding-api.open-meteo.com", "https://api.open-meteo.com", "https://air-quality-api.open-meteo.com",
        "https://en.wikipedia.org", "https://open-meteo.com",
        "https://www.youtube.com", "https://www.google.com", "https://maps.google.com", "https://www.bbc.com",
        "https://cdn.jsdelivr.net", "https://huggingface.co"}
    assert 'var LM_LIBRARY = "https://cdn.jsdelivr.net/npm/@huggingface/transformers@4.3.1";' in host
    assert host.count("import(") == 1 and "import(LM_LIBRARY)" in host
    fetches = re.findall(r"fetch\(([^,)]+)", host)
    assert sorted(f.split("?")[0] for f in fetches if not f.startswith("API +")) == [
        '"https://api.open-meteo.com/v1/forecast', '"https://geocoding-api.open-meteo.com/v1/search', "url"], fetches
    assert [f for f in fetches if "open-meteo" in f][0].endswith("&name=\" + encodeURIComponent(name")
    # getJSON(url): Wikipedia's API (search, or a day's Current events), and the air where a place is.
    gets = re.findall(r"getJSON\(([^,)]+)", host)
    assert gets and all(g.startswith(("WIKI +", '"https://air-quality-api.open-meteo.com/v1/air-quality?latitude=" + p.lat'))
                        for g in gets if g != "url"), gets
    assert 'var WIKI = "https://en.wikipedia.org/w/api.php?' in host and "gsrsearch=\" + encodeURIComponent(q)" in host
    for site in ("https://www.youtube.com/", "https://www.bbc.com/"):
        for at in [m.start() for m in re.finditer(re.escape(site), host)]:
            line = host[host.rfind("\n", 0, at):host.find("\n", at)]
            assert "var url = " in line or "link(" in line, line
    for site in ("https://www.google.com/", "https://maps.google.com/"):
        for at in [m.start() for m in re.finditer(re.escape(site), host)]:
            line = host[host.rfind("\n", 0, at):host.find("\n", at)]
            assert "igu" in line or "output=embed" in line, line
    # Only Google's and Wikipedia's addresses are shown for real (framed); any other site typed opens in a new tab,
    # with no way back to this page (a link where the browser stops the tab).
    assert "function framed(url)" in host and "wikipedia\\.org$" in host
    assert 'window.open("", "_blank")' in host and "tab.opener = null" in host
    assert host.count("x-api-key") == 1 and "anthropic-dangerous-direct-browser-access" in host
    assert "not made by Anthropic" in host and "written in advance" in host
    # Claude Pro or Max sign-ins aren't for other apps: a sign-in token is turned away, and the sheet says why.
    assert "sk-ant-oat" in host and "Anthropic doesn't let other apps sign in with one" in host


def test_no_two_functions_share_a_name():
    """chat.js and host.js are each one big function: a second function of the same name in it silently replaces the
    first everywhere in it (the "/" list's command() once took the YouTube player's place, so a video turned down
    stayed as loud)."""
    for f in (ROOT / "chatcomputer" / "ui" / "assets" / "chat.js", ROOT / "docs" / "demo" / "host.js"):
        names = re.findall(r"^  function (\w+)\s*\(", f.read_text(), re.M)
        assert len(names) > 20 and sorted({n for n in names if names.count(n) > 1}) == [], f.name


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
    # It opens on the word logo alone; after a second it fades away, over half a second, and then Chat Computer's
    # first words come in as its answers do, as if just typed: a moment's thinking (the logo gone), then the words a
    # few letters at a time, fading in. First hello, world, and under it a greeting, as the Claude app's (one of those
    # for now: no name or place known here), another on another visit.
    intro, first = res["intro"], res["opening"]
    assert intro["logo"] == "Chat Computer" and intro["said"] == 0
    assert res["intro_fading"] == {"fade": "0.5s", "said": 0}
    assert res["greeting_thinking"] == {"logo_gone": True, "words": ""}
    typing = res["greeting_typing"]
    assert "hello, world".startswith(typing["words"]) and typing["fading"]
    hello, greeting = res["greeting"].split("\n")
    options = res["greeting_options"]
    assert hello == "hello, world" and greeting in options["time"] + options["here"], (greeting, options)
    assert "and greetings to you, whoever you are" in options["time"] and not any("{" in g for g in options["time"])
    # One greeting, not two: what's under hello, world goes on from it ("and ...").
    assert all(g.startswith("and ") for g in options["time"] + options["here"]), options
    one, two = res["visits"]
    assert one.startswith("hello, world\n") and two.startswith("hello, world\n") and one != two
    # Then: nothing open, no sheet, no button to press, the cursor in the box under the dimmed hint, and nothing else
    # in or beside the box but its arrow: "/" lists what can be connected (Claude, AirPods), then the files.
    assert not first["content"] and not first["sheet"] and first["view"] is None
    # No sidebars to start with (the middle fills the screen), though there's room for them; Escape brings them in.
    assert first["full"] and first["sides"] == [False, False] and first["toggles"] == [False, False] and not first["bottom_toggle"]
    ins = res["sides_in"]
    assert not ins["full"] and ins["sides"] == [True, True] and ins["toggles"] == ["true", "true"]
    assert first["first_ai"] == res["greeting"] and first["last"] == res["greeting"].replace("\n", "") and first["said"] == 1
    assert first["logo"] is None
    assert first["messages"] == 7 and first["panel"] and len(first["done"]) == 4
    assert first["typing_in"] == "input" and first["hint"] == "Type anything or \u201c/\u201d or press \u201cEsc\u201d"
    assert first["hint_weight"] == "400" and first["buttons"] == 0
    assert res["slash"][:2] == ["Connect Claude", "Connect AirPods"] and "Packing list.md" in res["slash"][2:]
    # The sidebars' switches: off (their icons empty) and on again, and the choice is kept.
    assert res["sides_off"]["sides"] == [False, False] and res["sides_off"]["toggles"][:2] == ["false", "false"]
    assert json.loads(res["kept"]) == {"left": False, "right": False}
    assert res["sides_on"]["sides"] == [True, True] and res["sides_on"]["first_ai"] == res["greeting"]
    assert res["sides_on"]["logo"] is None
    # Asked for music: soft piano made by the browser, in the middle in the conversation's place (named at the top
    # of its frame, with the message box under it), and Actions says so.
    music = res["music"]
    assert music["content"] and music["last"] == "Playing some calm piano." and music["done"][-1] == "Played some calm piano"
    assert not music["column_shown"] and music["frame_title"] == "Now playing" and music["composer"]
    # The first words are still the conversation's first message.
    assert music["logo"] is None and music["first_ai"] == res["greeting"]
    f, v = music["frame"], music["view"]
    assert f[0] < v[0] and f[1] + 30 < v[1] and v[0] + v[2] < f[0] + f[2] and v[1] + v[3] < f[1] + f[3]
    # "/": the list is in front of what's open; Escape ends the search (and only that).
    assert res["over"] and res["over_view"] == v
    assert res["unslashed"]["content"] and not res["unslashed"]["full"] and res["unslashed"]["view"] == v
    # Escape: the middle fills the screen (the sidebars and their switches gone, the message box still under it),
    # and Escape again brings them back.
    full = res["full"]
    assert full["full"] and full["sides"] == [False, False] and full["toggles"] == [False, False] and full["composer"]
    assert full["view"][0] < 40 and full["view"][2] > 1500 and full["view"][1] == v[1] and full["view"][3] == v[3]
    assert not res["full_back"]["full"] and res["full_back"]["view"] == v and res["full_back"]["sides"] == [True, True]
    # The x closes it: the conversation is back.
    assert not res["closed"]["content"] and res["closed"]["view"] is None and res["closed"]["column_shown"]
    # "/": the demo's files; Enter opens one.
    assert res["search"]["results"][0].startswith("Packing list.md")
    assert res["opened"]["content"] and res["opened_view"].startswith("Packing list")
    assert res["opened"]["frame_title"] == "Packing list"
    # A message opens as the whole conversation with its sender, in the middle (not the conversation with Chat
    # Computer); a timer made on the spot.
    assert "12:30 at the usual place" in res["jane"] and "Are you free for lunch on Friday?" in res["jane"]
    jane = res["jane_state"]
    assert jane["content"] and not jane["column_shown"] and jane["frame_title"] == "Jane Lim"
    assert res["timer"].startswith("Timer 5:00") and res["after_timer"]["done"][-1] == "Made a timer"
    # Asked something else while the timer is open, and answered only in words: the timer is put away and the
    # conversation has the middle again, the answer in it in full (no "more").
    long = res["fallback"]
    assert not long["content"] and long["column_shown"] and long["view"] is None and long["more"] == 0
    assert long["last"].startswith("I can't answer that on this page without an AI") and long["last"].endswith("Type / to search the files.")
    # (Here the browser has no AI of its own and can't run Gemma, no WebGPU: it says so.)
    assert "This browser can't run a free AI on this device (that needs WebGPU: Safari on iOS 26" in long["last"]
    # Google, for real, in the middle: its own address for being shown in another page; a click on its box, and
    # typing goes there, not into the message box.
    g = res["google"]
    assert g["content"] and g["src"] == "https://www.google.com/webhp?igu=1" and g["frame_title"] == "Google"
    assert g["done"][-1] == "Opened google.com" and res["google_typed"] == {"frame": "cats", "box": ""}
    # Any other address typed, as in a browser's address bar: a site that won't show inside another page opens at
    # once in a new tab, cut off from this page; a YouTube video's address plays in YouTube's player in the middle.
    yt, bbc = res["typed_sites"]
    assert yt["tab"] == {"url": "https://youtube.com/", "opener": True} and yt["last"].startswith("Opened YouTube in a new tab.")
    assert yt["done"] == "Opened youtube.com in a new tab" and not yt["content"]
    assert bbc["tab"] == {"url": "https://bbc.co.uk/", "opener": True} and bbc["done"] == "Opened bbc.co.uk in a new tab"
    video = res["typed_video"]
    assert video["content"] and video["src"].startswith("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?") and "&start=65" in video["src"]
    assert video["done"][-1] == "Played a video on YouTube"
    # Connect Claude: a key, checked with Anthropic; then Claude answers and does things here (a map, as asked).
    assert res["claude_sheet"]["sheet"] and res["claude_sheet"]["sheet_title"] == "Connect Claude"
    assert not res["claude_on"]["sheet"] and res["claude_on_slash"][0] == "Disconnect Claude"
    assert json.loads(res["stored"]) == "sk-ant-api03-demo-test-key"
    m = res["claude_map"]
    assert m["last"] == "Here's Tokyo." and "openstreetmap.org/export/embed.html?bbox=139.55%2C35.55%2C139.95%2C35.8" in m["src"]
    assert m["done"][-1] == "Opened openstreetmap.org" and m["frame_title"] == "Map of Tokyo" and not m["column_shown"]
    # A video Claude plays: YouTube's player between the sidebars, Actions says so, and when it ends the
    # conversation has the middle again.
    cv = res["claude_video"]
    assert cv["content"] and "youtube-nocookie.com/embed/aBcDeFgHiJk?autoplay=1" in cv["src"] and cv["last"] == "Playing it."
    assert cv["done"][-1] == "Played \u201cA test video\u201d on YouTube" and cv["frame_title"] == "A test video"
    # Claude turns it down: YouTube's player is told (it once wasn't: a second function of the same name took its
    # messages), and the video stays on the screen.
    # The greeting, as the Claude app's: Claude's own greetings for the time of day and the day of the week, with the
    # name you go by; and for where you are (your time zone's city, Singapore) and its weather (Open-Meteo: only the
    # place's name, then its position, kept half an hour: a second visit asks again for neither).
    ev = res["greet_evening"]
    assert ev["now"]["time"] == ["and look who's back: Ren!", "and good evening to you, Ren", "and to you, Ren: how was your day?"]
    assert ev["now"]["here"] == ["and good evening to you in Singapore, Ren", "and hello, Singapore! How was your day, Ren?",
                                 "and to you in rainy Singapore, Ren: stay dry", "and hello, rainy Singapore! A cozy evening in, Ren?",
                                 "and hello, Singapore! A warm night, Ren"]
    hello, greeting = ev["said"].split("\n")
    assert hello == "hello, world" and greeting in ev["now"]["time"] + ev["now"]["here"]
    assert ev["meteo"] == ["geocoding-api Singapore", "api"]
    mo = res["greet_morning"]
    assert mo["now"]["time"] == ["and good morning to you", "and welcome", "and hey there to you", "and a happy Friday to you",
                                 "and hello to that Friday feeling", "and greetings to you, whoever you are"]
    assert mo["now"]["here"] == ["and good morning to you in Singapore", "and hello, Singapore! How's your morning?",
                                 "and hello, sunny Singapore! Lovely morning", "and to you under blue skies in Singapore"]
    assert mo["said"].split("\n")[1] in mo["now"]["time"] + mo["now"]["here"]
    # Not light yet at a quarter past six: a morning's greetings, not a night's ("A warm night"), nor a day's ("grey skies").
    assert res["greet_dawn"] == ["and good morning to you in Singapore", "and hello, Singapore! How's your morning?",
                                 "and hello, cloudy Singapore! How's your morning?"]
    # The name you go by: asked, given ("call me", in your words, capitalised), not taken from "call me back later",
    # and forgotten; where you live: found on the map (Penang, in your word for it), its weather in the greeting and
    # when you ask for the weather.
    assert res["name_asked"] == "You're Ren. (To change it, say \u201ccall me\u201d and the name you'd like.)"
    assert res["name_given"]["said"] == "Nice to meet you, Ren Shu! I'll greet you by name from now on."
    assert res["name_given"]["time"][1] == "and good evening to you, Ren Shu"
    assert not res["name_not"].startswith("Nice to meet you")
    assert res["place_given"]["said"].startswith("Got it: you live in Penang, Malaysia.")
    assert res["place_given"]["here"][:3] == ["and good evening to you in Penang, Ren Shu", "and hello, Penang! How was your day, Ren Shu?",
                                              "and to you in rainy Penang, Ren Shu: stay dry"]
    assert res["place_weather"].startswith("Today in Penang: light rain, 26\u00b0 to 31\u00b0")
    forgot = res["name_forgotten"]
    assert forgot["said"] == "Done: I've forgotten your name." and "and greetings to you, whoever you are" in forgot["time"]
    assert forgot["done"][-4:] == ["Set your name to Ren Shu", "Set your place to Penang", "Looked up the weather for Penang",
                                   "Forgot your name"]
    # Penang found once; its weather looked up for the greeting, then kept (the forecast asked for itself).
    assert res["greet_meteo"] == ["geocoding-api Singapore", "api", "geocoding-api penang", "api", "api"]
    q = res["claude_quieter"]
    assert q["content"] and q["last"] == "Quieter now." and q["done"][-1] == "Set the volume to 30%" and "setVolume" in q["player"]
    assert not res["video_ended"]["content"] and res["video_ended"]["view"] is None
    api = res["api"]
    assert len(api) == 7 and all(a["url"] == "https://api.anthropic.com/v1/messages" for a in api)
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
    assert res["claude_off_slash"][0] == "Connect Claude" and res["stored_after"] is None
    # Connect AirPods says what it does before the microphone is asked for.
    assert res["talk_sheet"]["sheet"] and res["talk_sheet"]["sheet_title"] == "Talk through your AirPods"
    # Chrome's built-in AI, where Chrome has it: "Download Gemini" in the "/" list; its sheet says what it is before
    # anything is downloaded; the download's progress in the empty box (and the list); then it answers what the
    # written answers can't (a page on the screen, said in Actions), from a session told what Chat Computer is, and the
    # written answers still answer what they can.
    assert res["ai_opening"][:3] == ["Connect Claude", "Connect AirPods", "Download Gemini"]
    assert res["ai_sheet"]["sheet"] and res["ai_sheet"]["sheet_title"] == "Download Gemini"
    assert not res["ai_downloading"]["sheet"] and res["ai_downloading"]["hint"] == "Downloading Gemini (50%)\u2026"
    assert res["ai_downloading_slash"][2] == "Downloading Gemini (50%)"
    on = res["ai_on"]
    assert on["hint"] == "Type anything or \u201c/\u201d or press \u201cEsc\u201d" and on["done"][-1] == "Turned on Chrome's built-in AI"
    assert res["ai_on_slash"][:2] == ["Connect Claude", "Connect AirPods"] and "Download Gemini" not in res["ai_on_slash"]
    poem = res["ai_poem"]
    assert poem["content"] and poem["last"] == "Here's a short poem about the sea." and poem["done"][-1] == "Showed \u201cThe sea\u201d"
    assert "The tide comes in on silver feet." in res["ai_poem_view"] and "Written by Chrome's built-in AI" in res["ai_poem_view"]
    assert res["ai_music"]["last"] == "Playing some calm piano."
    calls = res["ai_calls"]
    assert len(calls["created"]) == 1 and calls["created"][0]["monitored"] and "Gemini Nano" in calls["created"][0]["system"]
    assert calls["prompts"] == [{"text": "write a short poem about the sea", "schema": True}]
    # Google's Gemma where the browser has no AI of its own but can use the graphics chip (WebGPU; a stand-in for
    # Transformers.js): "Download Gemma" in the "/" list, and the written answers say so meanwhile; its sheet says what
    # it is (and how big) before anything's downloaded; the download's progress in the empty box and the list.
    assert res["lm_opening"][:3] == ["Connect Claude", "Connect AirPods", "Download Gemma"]
    assert "type / and choose “Download Gemma”" in res["lm_before"]
    sheet = res["lm_sheet"]
    assert sheet["sheet"] and sheet["sheet_title"] == "Download Gemma"
    assert res["lm_downloading"]["hint"] == "Downloading Gemma (49%)…" and res["lm_downloading_slash"][2] == "Downloading Gemma (49%)"
    # Then it's on (said in Actions), Remove Gemma in the list, and it answers what the written answers can't, its
    # words streamed in, told who it is; the written answers still answer what they can, and what was said (theirs
    # too) goes with the next question.
    on = res["lm_on"]
    assert on["hint"] == "Type anything or “/” or press “Esc”" and on["done"][-1] == "Turned on Gemma, on this device"
    assert res["lm_on_slash"][:3] == ["Connect Claude", "Connect AirPods", "Remove Gemma"] and "Download Gemma" not in res["lm_on_slash"]
    assert res["lm_haiku"]["last"] == "Dark cup in my hands, the day starts with you." and res["lm_music"] == "Playing some calm piano."
    calls = res["lm_calls"]
    assert calls["loads"] == [{"task": "text-generation", "model": "onnx-community/gemma-3-1b-it-ONNX", "device": "webgpu",
                               "dtype": "q4f16", "kept": False}]
    first_q, second_q = calls["asked"]
    assert first_q[0][0] == "system" and first_q[0][1].startswith("You are Gemma, a small AI made by Google")
    assert first_q[1:] == [["user", "write a haiku about coffee"]]
    assert second_q[1:] == [["user", "write a haiku about coffee"], ["assistant", "Dark cup in my hands, the day starts with you."],
                            ["user", "play some piano music"], ["assistant", "Playing some calm piano."], ["user", "make it longer"]]
    # A later visit: it's here (Remove Gemma; nothing to download), and starts when it's first needed, from the
    # browser's copy (nothing downloaded).
    assert res["lm_later_slash"][2] == "Remove Gemma" and "Download Gemma" not in res["lm_later_slash"]
    assert res["lm_starting"] == "Starting Gemma…"
    assert res["lm_later"]["last"] == "Dark cup in my hands, the day starts with you." and [x["kept"] for x in res["lm_later"]["loads"]] == [True]
    # The page stopped while it was starting (a phone out of memory): it isn't started by itself again; it's offered,
    # saying why, and starts from the browser's copy when chosen. Remove Gemma frees the room: Download Gemma again.
    assert res["lm_stopped_slash"][2] == "Start Gemma"
    st = res["lm_stopped_sheet"]
    assert st["title"] == "Start Gemma" and st["ok"] == "Start it" and "Last time, this page stopped while Gemma was starting" in st["text"]
    assert [x["kept"] for x in res["lm_restarted"]] == [True]
    assert res["lm_remove_sheet"]["sheet_title"] == "Remove Gemma"
    gone = res["lm_removed"]
    assert gone["slash"][2] == "Download Gemma" and gone["disposed"] and gone["kept"] == 0
    # Under each message, as in the Claude app, and different under each: yours (when, ask again, edit, copy) and the
    # answer's (copy, read aloud, good, bad, ask again, when); the latest answer's row shows. Its words faded in.
    assert res["fading"] and set(res["fading"]) == {"fresh"}
    assert res["rows"] == [
        {"kind": "mine", "last": False, "when": "just now", "acts": ["Ask again", "Edit", "Copy"], "shown": "0", "after": "you"},
        {"kind": "theirs", "last": True, "when": "just now", "acts": ["Copy", "Read aloud", "Good answer", "Bad answer", "Ask again"],
         "shown": "1", "after": "ai"}]
    assert res["rows_hello"] == 0                     # (something said straight away: no first words before it)
    # The text's size asked for in words goes the way asked: "Increase font size" bigger (it went smaller before);
    # "font" alone asks which way, changing nothing; "decrease" and "smaller" smaller; what the font is, named; and
    # back to the usual size.
    sz = res["sizes"]
    assert sz["increase"]["last"] == "Made the text bigger." and sz["increase"]["rem"] > sz["start"]
    assert sz["font"]["last"].startswith("Bigger or smaller?") and sz["font"]["rem"] == sz["increase"]["rem"]
    assert sz["decrease"]["last"] == "Made the text smaller." and sz["decrease"]["rem"] < sz["font"]["rem"]
    assert sz["smaller"]["last"] == "Made the text smaller." and sz["smaller"]["rem"] < sz["decrease"]["rem"]
    assert sz["which"]["last"].startswith("It's Atkinson Hyperlegible Next") and sz["which"]["rem"] == sz["smaller"]["rem"]
    assert sz["normal"]["last"] == "Put the text back to its usual size." and sz["normal"]["rem"] == sz["start"]
    fb = res["feedback"]
    assert fb["on"] == ["good"] and [(k["good"], k["on"], k["prompt"]) for k in fb["kept"]] == [(True, True, "is it free")]
    assert res["edit"] == {"box": "is it free", "focused": "input"}
    assert res["retry"] == {"yous": ["is it free", "is it free"], "last": 1}
    # A phone: no sidebars and no switches; nothing in the box but the words and the arrow, and it works.
    phone = res["phone_opening"]
    assert phone["first_ai"].startswith("hello, world\n") and phone["logo"] is None and phone["toggles"] == [False, False]
    assert phone["buttons"] == 0
    assert not res["phone"]["panel"] and res["phone"]["last"].startswith("Hi! I'm Chat Computer")
    # As the Claude app is on a phone (measured on an iPhone): text a eighteenth of the screen's width (390px: 21.7px),
    # never under 16px, so the browser doesn't zoom in on the box; the box 0.77rem from the screen's sides and 4.77rem
    # tall, its words above, 0.58rem in past its line, and the send arrow under them, 8px from its right; nothing past
    # the screen's edges; the keyboard only for a touch on the box.
    lay = res["phone_layout"]
    assert lay["phone"] and lay["rem"] == 21.7 and lay["input_px"] >= 16
    assert lay["box_left"] == 17 and abs(lay["width"] - lay["box_w"] - 33.4) <= 1 and abs(lay["box_h"] - 4.77) < 0.1
    assert lay["send_under"] and lay["send_right"] == 8 and lay["words_left"] == 15
    assert lay["scroll_w"] <= lay["width"] and lay["typing_in"] != "input" and res["phone_tapped"] == "input"
    # The keyboard never covers the box: what it covers lifts the box to sit just above it (Safari on an iPhone
    # doesn't make the page smaller for it), with that much more room at the conversation's end; where Safari has
    # scrolled what's on the screen down to the box already, it isn't lifted; with the keyboard down, it's back.
    kbd = res["phone_keyboard"]
    assert kbd["up"] == {"box": kbd["up"]["seen"], "seen": kbd["up"]["seen"], "room": 300, "keyboard": True}
    assert kbd["scrolled"]["box"] == kbd["scrolled"]["seen"]
    assert kbd["down"] == {"box": kbd["down"]["seen"], "seen": kbd["down"]["seen"], "room": 0, "keyboard": False}
    # Sent: the keyboard goes down (the box isn't being typed in), as in ChatGPT's app.
    assert res["phone"]["typing_in"] != "input"
    # Holding a message: its menu (yours: Copy, Select text, Edit; the answer's: Copy, Select text, Read aloud; Send
    # to too where the browser can share), over the screen dimmed but not blurred (the menu is frosted glass), and
    # not the phone's own selecting. Edit puts it in the box, to type in; Select text shows it alone, its words
    # selectable; a tap beside it or on it puts the menu away.
    assert res["phone_hold_you"] == {"holding": True, "held": "you enter held", "items": ["Copy", "Select text", "Edit"], "selected": ""}
    assert res["phone_edit"] == {"holding": False, "box": "hello", "typing_in": "input"}
    assert res["phone_hold_ai"]["items"] == ["Copy", "Select text", "Read aloud"] and "held" in res["phone_hold_ai"]["held"]
    assert res["phone_select"] == {"reading": True, "text": "Hi! I'm Chat Computer: Claude, running t", "select": "text"}
    assert res["phone_put_away"] is False and res["phone_put_away_too"] is False
    assert res["phone_shade"]["blur"] == "none" and res["phone_shade"]["menu_blur"].startswith("blur(")
    # The buttons under messages with a finger: with the keyboard up, Read aloud and Good still do what they say (the
    # keyboard went down at once, before, and the page moved under the finger), and the keyboard goes down after;
    # an earlier answer's row, shown by a tap and put away by another, isn't left showing.
    assert res["thumb_keyboard"] is True
    spoken = res["thumb_speak"]["spoken"]                # (said in pieces of a sentence or so)
    assert spoken >= 1 and res["thumb_speak"] == {"spoken": spoken, "keyboard": False, "on": []}
    assert res["thumb_good"] == {"spoken": spoken, "keyboard": False, "on": ["good"]}
    assert res["thumb_twice"] == {"shown": False, "opacity": "0"}
    # Two fingers pinched: nothing zooms, and the text keeps its size.
    assert res["phone_pinched"] == {"rem": 21.7, "zoom": 1, "scroll_w": 390}
    # Music, and then asked about it ("turn the music down", "stop the music": music words too): turned down (the
    # same music, quieter, not started again), then stopped.
    assert res["phone_music"]["content"] and res["phone_music"]["last"] == "Playing some calm piano."
    down = res["phone_down"]
    assert down["content"] and down["last"] == "Volume is 40%." and down["done"][-1] == "Turned the volume down"
    assert res["phone_down_piano"] == {"same": True, "vol": 0.4}
    stopped = res["phone_stopped"]
    assert not stopped["content"] and stopped["view"] is None and stopped["last"] == "Closed it."
    # Connect AirPods on an iPhone (stand-ins for Safari's listening and speaking): one thing heard at a time (Safari's
    # listening on and on can go quiet for good), started with the tap, which also lets the page speak (a silent
    # word); the screen kept on; what's heard, never called final, said once it stops changing; the answer read out,
    # then listening again; a thing heard twice over asked once; refused, what to switch on, and the screen let go.
    talk = res["talk_phone"]
    assert talk == {"continuous": False, "spoken": [{"text": " ", "volume": 0}], "awake": 1, "hint": "AirPods on: talk, or type"}
    assert res["talk_hearing"] == "what time is it"
    heard = res["talk_answered"]
    assert heard["asked"] == "what time is it" and heard["read_out"] == heard["answer"] and heard["answer"].startswith("It's ")
    assert heard["first_ended"]
    assert res["talk_twice"] == "What time is it in Tokyo"
    refused = res["talk_refused"]
    assert refused["title"] == "Talking stopped" and "Siri or Dictation" in refused["text"]
    assert refused["hint"] == "Type anything or \u201c/\u201d" and refused["awake"] == 0      # (no Escape on a phone)
