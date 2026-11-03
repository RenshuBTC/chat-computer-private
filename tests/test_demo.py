"""The browser demo in docs/ (for GitHub Pages): Chat Computer's own screen, with docs/demo/host.js standing in for the
computer. It must be the real screen as it is now (scripts/build_demo.py makes it from chatcomputer/ui/assets), keep
what's typed on this computer (in the browser, or with the Chat Computer app's model, on this computer too), and
work: tried in Chromium when Playwright is there."""
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


def test_what_s_typed_stays_on_this_computer():
    host = (ROOT / "docs" / "demo" / "host.js").read_text()
    for call in ("XMLHttpRequest", "sendBeacon", "WebSocket", "EventSource", "navigator.geolocation"):
        assert call not in host, call
    # What answers for real is Chat Computer's own model, on this computer: the app's engine at 127.0.0.1 (on the
    # website, where the app runs here and the visitor chose it); or, once the visitor signs in with an AI of their
    # own (the spark beside +), that AI, on their account: OpenRouter (its own sign-in), or their key for OpenAI,
    # Google's Gemini, OpenRouter or another that answers as OpenAI's chat completions do. No other.
    assert 'function localBase() { return "http://127.0.0.1:" + local.port; }' in host
    for gone in ("api.anthropic.com", "x-api-key", "sk-ant", "languagemodel", "gemma", "gemini nano", "connect claude"):
        assert gone not in host.lower(), gone
    assert 'openrouter: { name: "OpenRouter", base: "https://openrouter.ai/api/v1"' in host
    assert 'openai: { name: "OpenAI", base: "https://api.openai.com/v1"' in host
    assert 'base: "https://generativelanguage.googleapis.com/v1beta/openai"' in host
    assert 'url = ownBase() + "/chat/completions";' in host and 'url = localBase() + "/v1/chat/completions";' in host
    # From elsewhere: a map when asked, and YouTube's player for a video. For a place's map, weather or air,
    # Open-Meteo, sent only the place's name (then its latitude and longitude). What's looked up ("look it up", a
    # question of fact, the news, the model's web_search) goes to Wikipedia: only the words looked up. Google's own
    # pages show in the middle when asked for (Google, its search for what Wikipedia hasn't got, Google Maps'
    # directions: the addresses Google lets other pages show, igu=1 and output=embed). YouTube's search and BBC News
    # are only links, opened by the visitor in a new tab (open-meteo.com too: the air's source, named). Heart, the
    # warmer voice, when the visitor downloads it: kokoro-js, a fixed version, from jsDelivr, and its model from
    # Hugging Face (where the browser keeps its copy, by that address); it runs on the device, so nothing read aloud
    # is sent anywhere.
    assert set(re.findall(r"https?://[^/\"' ]+", host)) == {
        "http://127.0.0.1:", "https://www.youtube-nocookie.com", "https://www.openstreetmap.org",
        "https://geocoding-api.open-meteo.com", "https://api.open-meteo.com", "https://air-quality-api.open-meteo.com",
        "https://en.wikipedia.org", "https://open-meteo.com",
        "https://www.youtube.com", "https://www.google.com", "https://maps.google.com", "https://www.bbc.com",
        "https://cdn.jsdelivr.net", "https://huggingface.co", "https://icons.duckduckgo.com", "https://openrouter.ai",
        "https://api.openai.com", "https://generativelanguage.googleapis.com", "https://chatcomputer.ai"}
    assert 'library: "https://cdn.jsdelivr.net/npm/kokoro-js@1.2.1/dist/kokoro.web.js"' in host
    # Messages: each service's own icon (as its pages give it), through DuckDuckGo's icon service, sent only the
    # service's address.
    assert 'return site ? "https://icons.duckduckgo.com/ip3/" + site + ".ico" : "";' in host
    assert host.count("import(") == 1 and "import(HEART.library)" in host
    fetches = re.findall(r"fetch\(([^,)]+)", host)
    assert sorted(f.split("?")[0] for f in fetches if not f.startswith("localBase(")) == [
        '"https://api.open-meteo.com/v1/forecast', '"https://geocoding-api.open-meteo.com/v1/search',
        '"https://openrouter.ai/api/v1/auth/keys"', 'base + "/models"', "url", "url", "url"], fetches
    assert 'fetch(localBase() + "/health"' in host and len([f for f in fetches if f.startswith("localBase(")]) == 1
    assert 'fetch(url, { method: "POST", headers: headers, body: JSON.stringify(body) })' in host
    # (getJSON's, and Heart's big file, a piece at a time, from Hugging Face: only which bytes are asked for)
    assert 'pieceFetch(heartFile(), HEART_PARTS' in host
    assert host.count("pieceFetch(") == 2 and 'fetch(url, { headers: { Range: "bytes=" + from + "-" + to }' in host
    assert "huggingface.co/\" + HEART.model + \"/resolve/main/" in host
    assert [f for f in fetches if "open-meteo" in f][0].endswith("&name=\" + encodeURIComponent(name")
    # getJSON(url): Wikipedia's API (search, or a day's Current events), and the air where a place is.
    gets = re.findall(r"getJSON\(([^,)]+)", host)
    assert gets and all(g.startswith(("WIKI +", '"https://air-quality-api.open-meteo.com/v1/air-quality?latitude=" + p.lat'))
                        for g in gets if g != "url"), gets
    assert 'var WIKI = "https://en.wikipedia.org/w/api.php?' in host and "gsrsearch=\" + encodeURIComponent(q)" in host
    for site in ("https://www.youtube.com/", "https://www.bbc.com/"):
        for at in [m.start() for m in re.finditer(re.escape(site), host)]:
            line = host[host.rfind("\n", 0, at):host.find("\n", at)]
            before = host[host.rfind("\n", 0, host.rfind("\n", 0, at)):host.rfind("\n", 0, at)]
            # (in the app, a video is YouTube's own page, in the app's own view: never on the website)
            assert ("var url = " in line or "link(" in line or ("watch?v=" in line and "if (APP)" in before)
                    or "var yurl = " in line), line
    # (YouTube's search, asked for by the model: in the app, in the middle; on the website, a link)
    assert "if (APP) show({ src: yurl" in host and "else youtubeSearch(yq).act();" in host
    for site in ("https://www.google.com/", "https://maps.google.com/"):
        for at in [m.start() for m in re.finditer(re.escape(site), host)]:
            line = host[host.rfind("\n", 0, at):host.find("\n", at)]
            assert "igu" in line or "output=embed" in line, line
    # Only Google's and Wikipedia's addresses are shown for real (framed); any other site typed opens in a new tab,
    # with no way back to this page (a link where the browser stops the tab).
    assert "function framed(url)" in host and "wikipedia\\.org$" in host
    assert 'window.open("", "_blank")' in host and "tab.opener = null" in host
    assert "written in advance" in host and "Chat Computer is an independent project" in host


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
    # few letters at a time, fading in: hello, world, alone (the greeting under it is off for now, as asked).
    intro, first = res["intro"], res["opening"]
    assert intro["logo"] == "Chat Computer" and intro["said"] == 0
    assert res["intro_fading"] == {"fade": "0.5s", "said": 0}
    assert res["greeting_thinking"] == {"logo_gone": True, "words": ""}
    typing = res["greeting_typing"]
    assert "hello, world".startswith(typing["words"]) and typing["fading"]
    assert res["greeting"] == "hello, world" and res["visits"] == ["hello, world", "hello, world"]
    # (The greetings it would choose from, were it on, still go on from it: "and ...".)
    options = res["greeting_options"]
    assert "and greetings to you, whoever you are" in options["time"] and not any("{" in g for g in options["time"])
    assert all(g.startswith("and ") for g in options["time"] + options["here"]), options
    # Then: nothing open, no sheet, the cursor in the box under the dimmed hint; under the words, as in the Claude app,
    # + (add files) at the left, and at the right the microphone beside the arrow, each doing just that (no menus),
    # + and the microphone each in a circle of its own, as big as the arrow's.
    # "/" lists what can be used here (this computer's model, AirPods), then the files.
    assert not first["content"] and not first["sheet"] and first["view"] is None
    # The sidebars show from the start (where there's room for them), with their switches pressed.
    assert not first["full"] and first["sides"] == [True, True] and first["toggles"] == ["true", "true"] and not first["bottom_toggle"]
    ins = res["sides_in"]
    assert not ins["full"] and ins["sides"] == [True, True] and ins["toggles"] == ["true", "true"]
    assert first["first_ai"] == res["greeting"] and first["last"] == res["greeting"].replace("\n", "") and first["said"] == 1
    assert first["logo"] is None
    assert first["messages"] == 7 and first["panel"] and len(first["done"]) == 4
    assert first["typing_in"] == "input" and first["hint"] == "Type anything"
    assert first["hint_weight"] == "400" and first["buttons"] == ["add", "ai", "mic"]
    # In the box, under the words: +, then the spark (your own AI), at the left; the microphone by the send arrow.
    (add, ax, ay), (own, ox, oy), (mic, mx, my) = first["tools"]
    assert (add, own, mic) == ("Add files", "Sign in with your own AI", "Talk")
    assert ax < ox < ax + 50 < mx < first["send_at"][0] and abs(ay - my) < 1 and abs(ay - oy) < 1
    assert ay > first["input_top"] and abs(my - first["send_at"][1]) < 1           # on the row under the words
    (aw, ah, around, afill, _), (ow, oh, oround, ofill, _), (mw, mh, mround, mfill, mright) = first["tools_round"]
    assert (aw, ah, around, afill) == (ow, oh, oround, ofill) == (mw, mh, mround, mfill) == (32, 32, "50%", "rgb(65, 65, 65)")
    assert first["send_size"] == [32, 32]
    box_left, box_right = first["box_at"]
    assert ax - box_left < 16 and 0 < first["send_at"][0] - mright < 12 and box_right - first["send_at"][0] < 50
    assert res["slash"][:2] == ["Use this computer's model", "Connect AirPods"] and "Packing list.md" in res["slash"][2:]
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
    # No good answer: a few short lines, and the one thing that gets one (Chat Computer's own model: with the app).
    assert long["last"] == ("I can't answer that yet. My model comes with the Chat Computer app (coming soon). Where the app runs "
                            "on this computer, type / and choose Use this computer's model.")
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
    # This computer's model, where the Chat Computer app runs here: chosen while the app isn't running, the sheet
    # says so (and what it is, before anything is reached); with it running, it's used (said in Actions, kept for
    # later visits), and it answers and does things here (a map, as asked), its words streamed in.
    assert res["local_sheet"]["sheet"] and res["local_sheet"]["sheet_title"] == "Use this computer's model"
    assert "free, with no account, and what you type goes only to the app, on this computer" in res["local_sheet_text"]
    assert res["local_refused"] == {"sheet": True, "error": "The app isn't answering on this computer: is it running? "
                                                            "(Or, if your browser asked, allow it.)"}
    on = res["local_on"]
    assert not on["sheet"] and on["done"][-1] == "Using this computer's model (Qwen3.5 9B)"
    assert res["local_on_slash"][:2] == ["Stop using this computer's model", "Connect AirPods"]
    assert json.loads(res["local_stored"]) is True
    m = res["local_map"]
    assert m["last"] == "Here's Tokyo." and "openstreetmap.org/export/embed.html?bbox=139.55%2C35.55%2C139.95%2C35.8" in m["src"]
    assert m["done"][-1] == "Opened openstreetmap.org" and m["frame_title"] == "Map of Tokyo" and not m["column_shown"]
    # A video the model plays: YouTube's player between the sidebars, Actions says so, and when it ends the
    # conversation has the middle again.
    cv = res["local_video"]
    assert cv["content"] and "youtube-nocookie.com/embed/aBcDeFgHiJk?autoplay=1" in cv["src"] and cv["last"] == "Playing it."
    assert cv["done"][-1] == "Played “A test video” on YouTube" and cv["frame_title"] == "A test video"
    # The model turns it down: YouTube's player is told (it once wasn't: a second function of the same name took its
    # messages), and the video stays on the screen.
    q = res["local_quieter"]
    assert q["content"] and q["last"] == "Quieter now." and q["done"][-1] == "Set the volume to 30%" and "setVolume" in q["player"]
    assert not res["video_ended"]["content"] and res["video_ended"]["view"] is None
    # What the page asked the engine: OpenAI's chat completions, streamed, without thinking first (an answer at once),
    # with the page's tools; the system prompt the same all conversation (the engine reads only what's new: the time
    # is in each message instead); each tool's call and its result go back as they came, and the whole conversation
    # with the next question.
    asked = res["asked"]
    assert len(asked) == 6 and all((a["method"], a["path"], a["model"], a["stream"], a["think"], a["max_tokens"]) ==
                                   ("POST", "/v1/chat/completions", "chat-computer", True, False, 4096) for a in asked)
    assert asked[0]["tools"] == ["function " + n for n in ("play_youtube", "show_map", "show_page", "open_website", "open_message",
        "open_file", "make_timer", "set_volume", "close", "youtube_search", "play_piano", "weather", "web_search", "set_name", "set_place")]
    system = {a["messages"][0]["content"] for a in asked}
    assert len(system) == 1 and all(a["messages"][0]["role"] == "system" for a in asked)
    system = system.pop()
    assert system.startswith("You are Chat Computer: you run a whole computer for the person, with your own model, an open model "
                             "of Qwen's that runs on this computer")
    assert "Right now you're in Chat Computer's website" in system and "Jane Lim" in system and "It's " not in system
    assert re.fullmatch(r"\[\w+day \d{1,2} \w+ 20\d\d.*\] show me Tokyo", asked[0]["messages"][1]["content"])
    call, result = asked[1]["messages"][-2:]
    assert call == {"role": "assistant", "content": "", "tool_calls": [{"id": "call_1", "type": "function", "function": {
        "name": "show_map", "arguments": '{"place": "Tokyo", "west": 139.55, "south": 35.55, "east": 139.95, "north": 35.8}'}}]}
    assert result == {"role": "tool", "tool_call_id": "call_1", "content": "The map of Tokyo is on the screen."}
    assert [m["role"] for m in asked[2]["messages"]] == ["system", "user", "assistant", "tool", "assistant", "user"]
    assert asked[2]["messages"][4] == {"role": "assistant", "content": "Here's Tokyo."}
    assert asked[5]["messages"][-1]["content"] == "The volume is 30%."
    # A later visit: it's used again by itself (a fresh conversation); when the app stops, the page says so, the
    # written answers are back and "/" offers it again; Stop using it, and the browser forgets it.
    assert res["local_later_slash"][0] == "Stop using this computer's model"
    assert res["local_later"] == "Hello from Qwen."
    assert len(res["local_later_asked"]) == 1 and res["local_later_asked"][0][0][0] == "user"
    assert res["local_later_asked"][0][0][1].endswith("] hello there") and len(res["local_later_asked"][0]) == 1
    assert res["local_gone"] == "I can't reach the Chat Computer app on this computer any more: is it still running?"
    assert res["local_gone_slash"][0] == "Use this computer's model"
    assert res["local_gone_after"].startswith("Hi! I'm Chat Computer")
    stopped = res["local_stopped"]
    assert stopped["done"][-2:] == ["Using this computer's model (Qwen3.5 9B)", "Stopped using this computer's model"]
    assert stopped["slash"][0] == "Use this computer's model" and stopped["stored"] is None
    # The greeting is off for now: hello, world alone, in the evening and in the morning, and nothing is looked up
    # for it (no Open-Meteo); the greetings it would choose from for the time of day still have the name you go by.
    ev = res["greet_evening"]
    assert ev["said"] == "hello, world" and ev["meteo"] == [] and ev["now"]["here"] == []
    assert ev["now"]["time"] == ["and look who's back: Ren!", "and good evening to you, Ren", "and to you, Ren: how was your day?"]
    mo = res["greet_morning"]
    assert mo["said"] == "hello, world" and mo["now"]["here"] == []
    assert mo["now"]["time"] == ["and good morning to you", "and welcome", "and hey there to you", "and a happy Friday to you",
                                 "and hello to that Friday feeling", "and greetings to you, whoever you are"]
    assert res["greet_dawn"] == []
    # The name you go by: asked, given ("call me", in your words, capitalised), not taken from "call me back later",
    # and forgotten; where you live: found on the map (Penang, in your word for it), its weather in the greeting and
    # when you ask for the weather.
    assert res["name_asked"] == "You're Ren. (To change it, say \u201ccall me\u201d and the name you'd like.)"
    assert res["name_given"]["said"] == "Nice to meet you, Ren Shu! I'll call you that from now on."
    assert res["name_given"]["time"][1] == "and good evening to you, Ren Shu"
    assert not res["name_not"].startswith("Nice to meet you")
    assert res["place_given"]["said"] == "Got it: you live in Penang, Malaysia. Say \u201cweather\u201d for its forecast."
    assert res["place_given"]["here"] == []
    assert res["place_weather"].startswith("Today in Penang: light rain, 26\u00b0 to 31\u00b0")
    forgot = res["name_forgotten"]
    assert forgot["said"] == "Done: I've forgotten your name." and "and greetings to you, whoever you are" in forgot["time"]
    assert forgot["done"][-4:] == ["Set your name to Ren Shu", "Set your place to Penang", "Looked up the weather for Penang",
                                   "Forgot your name"]
    # Penang found once, and its forecast when asked for (nothing for a greeting).
    assert res["greet_meteo"] == ["geocoding-api penang", "api"]
    # Connect AirPods says what it does before the microphone is asked for.
    assert res["talk_sheet"]["sheet"] and res["talk_sheet"]["sheet_title"] == "Talk through your AirPods"
    # The microphone button listens at once (no sheet), the microphone becoming one bar that moves with what it hears
    # (Chromium's stand-in microphone: a tone); pressed again, it stops and lets go of the microphone.
    # Messages: each with its service's own icon, from DuckDuckGo's icon service (standing in), sent no page address;
    # where it can't be had (LinkedIn's, there), Chat Computer's own.
    assert res["msg_icons"] == [
        ["WhatsApp", "web.whatsapp.com.ico", "no-referrer"], ["Gmail", "mail.google.com.ico", "no-referrer"],
        ["Telegram", "web.telegram.org.ico", "no-referrer"], ["Instagram", "www.instagram.com.ico", "no-referrer"],
        ["Outlook", "outlook.live.com.ico", "no-referrer"], ["X", "x.com.ico", "no-referrer"], ["LinkedIn", "own", ""]], res["msg_icons"]
    assert res["icons_asked"] == ["mail.google.com.ico", "outlook.live.com.ico", "web.telegram.org.ico", "web.whatsapp.com.ico",
                                  "www.instagram.com.ico", "www.linkedin.com.ico", "x.com.ico"]
    # The sidebars' switches on the edges nearest the middle (Messages' at its right, Actions' at its left), each
    # centred on its sidebar's title's line; Messages' connect rows only once + is pressed.
    sw = res["switches"]
    assert len(set(sw["centres"])) == 1 and 0 < sw["left_gap"] < 20 and 0 < sw["right_gap"] < 20, sw
    assert sw["connect"] == "none"
    # Your own AI (the spark beside +): OpenRouter's own sign-in (PKCE: a SHA-256 challenge, the verifier given only
    # when trading the code for a key), back to this page, the code gone from its address; the newest free model that
    # uses tools chosen; then it answers, on that key, with the page's tools and none of the engine's own options, told
    # who it is; signed out, nothing kept. A key of one's own (OpenAI's): an empty one said so; then its newest chat model.
    assert res["own_menu"] == {"title": "Your own AI", "choices": ["Continue with OpenRouter", "Use a key"]}
    auth = res["own_auth"]
    assert auth["back"].startswith("file://") and auth["back"].endswith("/docs/index.html") and auth["method"] == "S256"
    assert auth["challenge"] == 43 and res["own_traded"] == {"code": "test-code", "method": "S256", "verifier": 43}
    own_in = res["own_in"]
    assert own_in["search"] == "" and own_in["title"] == "OpenRouter (new/free-model:free) answers. Press to change"
    assert own_in["kept"] == {"provider": "openrouter", "key": "test-openrouter-key", "model": "new/free-model:free"}
    (asked,) = res["own_asked"]
    assert asked["auth"] == "Bearer test-openrouter-key" and asked["title"] == "Chat Computer" and asked["model"] == "new/free-model:free"
    assert asked["stream"] and not asked["think"] and not asked["max"] and asked["tools"] > 5
    assert "Right now you think with new/free-model:free, through OpenRouter" in asked["system"]
    assert asked["last"]["role"] == "user" and asked["last"]["content"].endswith("hi there")
    assert res["own_menu_in"] == {"title": "Your own AI", "choices": ["Choose its model", "Sign out of OpenRouter"]}
    assert res["own_out"]["title"] == "Sign in with your own AI" and res["own_out"]["kept"] is None
    assert res["own_out"]["done"] == ["Signed in with OpenRouter (new/free-model:free)", "Signed out of OpenRouter"]
    assert res["own_empty"] == "Paste the key first."
    assert res["own_key"] == {"title": "OpenAI (gpt-9) answers. Press to change", "sheet": False, "type": "password"}   # (typed hidden)
    now = res["mic_now"]
    assert not now["sheet"] and now["pressed"] == "true" and now["title"] == "Stop listening"
    assert now["bar"] == "block" and now["icon"] == "none" and now["live"] == "live"
    assert now["low"] >= 0.18 and now["high"] > now["low"] + 0.2, now
    stopped = res["mic_stopped"]
    assert stopped == {"pressed": "false", "bar": "none", "live": "ended", "aborted": True}
    # In the app (a stand-in for it), the model comes with it. While it gets ready, the empty box says how it's going
    # (downloading, then taking its refusals out), and so do "/" and its sheet, and an answer it can't give yet; then
    # it answers (named in "/"), on the port the app says, with the app's own tools (the files on this computer:
    # found by the app, the model told what was found).
    prep = res["app_preparing"]
    assert prep["hint"] == "Getting my model ready: downloading it (1.2 of 9.0 GB, 13%)…" and prep["messages"] == 0
    assert res["app_slash"][:2] == ["Chat Computer's model", "Connect AirPods"]
    assert res["app_sheet"] == {"title": "Chat Computer's model", "ok": "OK", "text": "My model, Qwen3.5 9B, is getting ready on "
        "this computer: downloading it (1.2 of 9.0 GB, 13%). That happens once; after it, it answers from the start."}
    assert res["app_before"] == ("I can't answer that yet. My model is getting ready on this computer: downloading it (1.2 of 9.0 GB, "
                                 "13%). Ask me again once it's here.")
    assert res["app_uncensoring"] == "Getting my model ready: taking its refusals out, so it answers whatever you ask (25%)…"
    assert res["app_ready"]["hint"] == "Type anything" and res["app_ready_slash"][:2] == ["Qwen3.5 9B", "Connect AirPods"]
    assert res["app_answer"] == "It's in Documents."
    app_asked = res["app_asked"]
    assert len(app_asked) == 2 and [t.split()[1] for t in app_asked[0]["tools"]][-4:] == ["find_files", "open_file", "read_messages", "open_message"]
    assert "function open_message" in app_asked[0]["tools"] and app_asked[0]["tools"].count("function open_file") == 1
    assert "Right now you're in the Chat Computer app on their" in app_asked[0]["messages"][0]["content"]
    assert "Jane Lim" not in app_asked[0]["messages"][0]["content"]          # (no made-up messages in the app)
    found = app_asked[1]["messages"][-1]
    assert found["role"] == "tool" and json.loads(found["content"])[0]["path"] == "/home/ren/Documents/Taxes 2025.pdf"
    calls = [c for c in res["app_calls"] if c[0] != "search_files" or c[1]["query"]]
    assert calls == [["about", {}], ["model_status", {}], ["search_files", {"query": "tax"}]]
    # Read aloud's voice where the graphics chip can make Heart's (WebGPU): offered the first time Read aloud is pressed
    # (nothing said yet), with its size; closed, the browser's best voice reads the answer (Edge's natural Ava, not
    # Google's, the usual Samantha or a novelty voice), and on another visit it reads straight away, Heart in the list.
    assert res["voice_slash"][:3] == ["Use this computer's model", "Connect AirPods", "Download Heart"]
    vs = res["voice_sheet"]
    assert vs["title"] == "A warmer voice" and vs["ok"] == "Download it" and vs["spoken"] == 0
    assert "the one Chat Computer itself speaks with" in vs["text"] and "about 330 MB" in vs["text"]
    system = res["voice_system"]
    assert not system["sheet"] and system["heart"] == 0 and system["spoken"]
    assert {s["voice"] for s in system["spoken"]} == {"Microsoft Ava Online (Natural) - English (United States)"}
    assert system["answer"].startswith(system["spoken"][0]["text"]) and len(system["spoken"]) > 1    # (a sentence or so at a time)
    assert res["voice_again"]["sheet"] is False and res["voice_again"]["spoken"] > 0
    assert "Download Heart" in res["voice_again_slash"]
    # Chosen: its progress in the box and the list; downloaded in two pieces, each kept as it came: the page left
    # after the first, the next visit asks only for the second (cut off once, asked for again), puts them together
    # where Transformers.js finds the file, and throws the pieces away; started on the graphics chip from that store,
    # and then the answer read in Heart (af_heart), not by the browser's voice; Actions says so; Remove Heart in the list.
    dl = res["voice_downloading"]
    pct = re.fullmatch(r"Downloading Heart \((\d+)%\)…", dl["hint"])
    assert pct and "Downloading Heart (" + pct.group(1) + "%)" in dl["slash"]
    assert res["voice_ranges_first"] == ["bytes=0-16777215", "bytes=16777216-20971519"]
    assert res["voice_parts_kept"] == ["piece=0", "size"]
    h = res["voice_heart"]
    assert h["loads"] == [{"model": "onnx-community/Kokoro-82M-v1.0-ONNX", "dtype": "fp32", "device": "webgpu", "data": 20 * 1024 * 1024}]
    assert h["said"][0]["voice"] == "af_heart" and h["answer"].startswith(h["said"][0]["text"]) and h["spoken"] == 0
    assert "Turned on the Heart voice, on this device" in h["done"] and not h["hint"].startswith("Downloading")
    assert res["voice_ranges"] == ["bytes=16777216-20971519", "bytes=16777216-20971519"] and res["voice_parts_after"] == []
    assert "Remove Heart" in res["voice_on_slash"] and not any(n.startswith("Download Heart") for n in res["voice_on_slash"])
    # A later visit: started from the browser's copy (nothing downloaded), no sheet, in Heart; Remove Heart frees it.
    later = res["voice_later"]
    assert len(later["loads"]) == 1 and later["loads"][0]["data"] == 20 * 1024 * 1024 and later["spoken"] == 0 and not later["sheet"]
    assert res["voice_later_ranges"] == [] and res["voice_remove_sheet"] == "Remove Heart"
    gone = res["voice_removed"]
    assert "Download Heart" in gone["slash"] and gone["kept"] == 0 and gone["disposed"]
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
    # A phone: no sidebars and no switches; in the box the words, + and the microphone, and the arrow, and it works.
    phone = res["phone_opening"]
    assert phone["first_ai"] == "hello, world" and phone["logo"] is None and phone["toggles"] == [False, False]
    assert phone["buttons"] == ["add", "ai", "mic"]
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
    assert res["phone_select"] == {"reading": True, "text": "Hi! I'm Chat Computer: I run the whole c", "select": "text"}
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
    assert refused["hint"] == "Type anything" and refused["awake"] == 0
    # Revert, in Actions: on what can be taken back (yesterday's volume, the saved file), under the pointer, beside its
    # pin; not on what only looked or opened a site. The saved file taken away (struck through, "/" no longer lists
    # it, and the revert listed, revertable itself; typing still in the box); that revert reverted: the file back, and
    # the action revertable again. The volume; the text made bigger, back to its size; a timer closed. A timer
    # closed by its x has nothing left to take back: no Revert. Nothing went wrong on the way (no notices).
    start = {a["id"]: a for a in res["revert_start"]["acts"]}
    assert [start[d]["marks"] for d in ("d1", "d2", "d3", "d4")] == [[], ["revertable"], [], ["revertable"]]
    assert [start[d]["button"] for d in ("d1", "d2", "d3", "d4")] == [False, True, False, True]
    assert res["revert_start"]["hidden"] == "hidden" and res["revert_seen"] == "visible"
    gone = res["reverted_file"]
    d4 = next(a for a in gone["acts"] if a["id"] == "d4")
    assert d4["marks"] == ["reverted"] and d4["struck"] and not d4["button"] and gone["typing_in"] == "input"
    assert gone["acts"][-1]["text"] == "Reverted “Saved Packing list.md”" and gone["acts"][-1]["marks"] == ["revertable"]
    assert "Packing list.md" not in gone["files"] and "Beach.jpg" in gone["files"]
    back = res["unreverted_file"]
    by = {a["text"]: a for a in back["acts"]}
    assert by["Saved Packing list.md"]["marks"] == ["revertable"] and not by["Saved Packing list.md"]["struck"]
    assert by["Reverted “Saved Packing list.md”"]["marks"] == ["reverted"]
    assert back["acts"][-1]["text"] == "Reverted “Reverted “Saved Packing list.md””"
    assert back["acts"][-1]["marks"] == ["revertable"]
    assert back["files"].index("Packing list.md") == res["slash"].index("Packing list.md")
    vol = {a["id"]: a for a in res["reverted_volume"]}
    assert vol["d2"]["marks"] == ["reverted"] and res["reverted_volume"][-1]["text"] == "Reverted “Set the volume to 40%”"
    small, big, again = res["text_sizes"]
    assert float(big[:-2]) > float(small[:-2]) and again == small
    assert [(a["text"], a["marks"]) for a in res["reverted_size"]] == [
        ("Made the text bigger", ["reverted"]), ("Reverted “Made the text bigger”", ["revertable"])]
    assert res["timer_made"]["open"] and res["timer_made"]["act"]["marks"] == ["revertable"]
    t = res["timer_reverted"]
    assert not t["open"] and [(a["text"], a["marks"], a["button"]) for a in t["acts"]] == [
        ("Made a timer", ["reverted"], False), ("Reverted “Made a timer”", [], False)]
    assert res["timer_kept"]["marks"] == ["revertable"]
    assert res["timer_closed"]["text"] == "Made a timer" and res["timer_closed"]["marks"] == [] and not res["timer_closed"]["button"]
    assert res["revert_notices"] == []
