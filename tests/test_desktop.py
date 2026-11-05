"""The Chat Computer app for Windows and Mac (apps/desktop): the website's page bundled into a window of its own
(scripts/build_apps.py desktop), with what a website can't do (any site in the middle, in the app's own view; "/"
finding the computer's files; the model, on this computer: Chat Computer's own engine, which comes with the app,
scripts/build_apps.py engine); and, the first time it starts, how you want Chat Computer (its page, start/, and
src/ways.rs: tests/test_apps.py checks those). Only the app's own page may ask the app for those: its capabilities
say so, and so does each command. Where it's built here for Linux (tauri build --debug --no-bundle) and WebDriver for
it is there (tauri-driver, WebKitWebDriver), it's tried for real (tests/desktop_check.py)."""
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
APP = ROOT / "apps" / "desktop"
TAURI = APP / "src-tauri"
sys.path.insert(0, str(ROOT / "scripts"))

import build_apps  # noqa: E402

COMMANDS = ["show_site", "place_site", "hide_site", "unhide_site", "close_site", "site_media", "search_files", "open_file", "about",
            "connect_service", "forget_service", "open_service_item", "model_status",
            "chatgpt_status", "chatgpt_sign_in", "chatgpt_sign_out", "chatgpt_ask",
            "way_status", "choose_way", "show_folder"]
# What a connected service's own pages may ask (what its reader read there), and nothing else.
SERVICE_COMMANDS = ["messages_found"]
SERVICES = ["gmail", "outlook", "telegram", "whatsapp", "wechat", "discord"]


def test_the_app_s_page_is_the_website_s(tmp_path, monkeypatch):
    monkeypatch.setattr(build_apps, "FRONTEND", tmp_path / "frontend")
    out = build_apps.desktop_frontend(lm=False)
    assert (out / "index.html").read_bytes() == (ROOT / "docs" / "index.html").read_bytes()
    for f in (ROOT / "docs" / "demo").rglob("*"):
        if f.is_file():
            assert (out / "demo" / f.relative_to(ROOT / "docs" / "demo")).read_bytes() == f.read_bytes(), f
    # Heart's code (the warmer voice), the same version as the website's (from jsDelivr there; with the app, from
    # npm, here): kokoro-js, and the ONNX Runtime its Transformers.js loads. (The model that answers isn't in the
    # page: it's the engine's, beside the app's program.)
    host = (ROOT / "docs" / "demo" / "host.js").read_text()
    assert list(build_apps.LM) == ["kokoro-js@1.2.1", "@huggingface/transformers@3.5.1"]
    assert 'library: "https://cdn.jsdelivr.net/npm/kokoro-js@1.2.1/dist/kokoro.web.js"' in host
    assert 'if (APP) HEART.library = LM_HERE + "kokoro.web.js";' in host and "if (LM_HERE && k.env) k.env.wasmPaths = LM_HERE;" in host


def test_only_the_app_s_own_page_may_ask_the_app():
    build = (TAURI / "build.rs").read_text()
    assert re.findall(r'"([a-z_]+)",', build) == COMMANDS + SERVICE_COMMANDS
    lib = (TAURI / "src" / "lib.rs").read_text()
    handlers = re.search(r"generate_handler!\[([^\]]+)\]", lib).group(1)
    assert sorted(x.strip() for x in handlers.split(",") if x.strip()) == sorted(COMMANDS + SERVICE_COMMANDS)
    # Each command checks it's the app's own page asking.
    for c in COMMANDS:
        body = re.search(r"fn " + c + r"\(([^)]*)\)[^{]*\{(.*?)\n\}", lib, re.S)
        assert body and "from_main(&webview)?" in body.group(2), c
    allowed = ["allow-" + c.replace("_", "-") for c in COMMANDS]
    main = json.loads((TAURI / "capabilities" / "main.json").read_text())
    assert main["webviews"] == ["main"] and "windows" not in main and "remote" not in main and main["permissions"] == allowed
    # Where the page comes from the app's own server (a Mac): that address alone, at its three ports.
    loop = json.loads((TAURI / "capabilities" / "loopback.json").read_text())
    ports = re.search(r"PORTS: \[u16; 3\] = \[(\d+), (\d+), (\d+)\]", (TAURI / "src" / "loopback.rs").read_text()).groups()
    assert loop["webviews"] == ["main"] and "windows" not in loop and loop["permissions"] == allowed
    assert loop["remote"]["urls"] == [f"http://127.0.0.1:{p}" for p in ports]
    # The services' views: on their own pages (https, their own sites), only what their reader read, and the command
    # checks that too (the view's name, and that it's at one of its service's own sites).
    msg = json.loads((TAURI / "capabilities" / "messages.json").read_text())
    assert msg["webviews"] == ["msg-" + s for s in SERVICES] and "windows" not in msg and msg["local"] is False
    assert msg["permissions"] == ["allow-messages-found"]
    assert all(re.fullmatch(r"https://[a-z0-9.]+\.(com|org)/\*", u) for u in msg["remote"]["urls"])
    found = re.search(r"fn messages_found\(([^)]*)\)[^{]*\{(.*?)\n\}", lib, re.S).group(2)
    assert "messages::of_view(webview.label())" in found and "messages::is_own_page(s, &url)" in found
    assert "from_main" not in found
    # A build for testing alone may point the services at pages on this computer.
    assert "if !cfg!(debug_assertions) {\n        return None;" in lib
    assert len(sorted((TAURI / "capabilities").glob("*.json"))) == 3
    # The site view: only web pages, a new window's link in its place, and the Mac's view as Safari.
    assert 'const SITE: &str = "site";' in lib and '.on_navigation(|u| matches!(u.scheme(), "http" | "https" | "about" | "blob"))' in lib
    assert "NewWindowResponse::Deny" in lib and "Version/26.0 Safari/605.1.15" in lib


def test_how_it_s_made_and_signed():
    conf = json.loads((TAURI / "tauri.conf.json").read_text())
    assert conf["identifier"] == "ai.chatcomputer.desktop" and conf["productName"] == "Chat Computer"
    assert conf["build"]["frontendDist"] == "../frontend" and conf["app"]["withGlobalTauri"] is True
    assert conf["app"]["windows"] == []                      # (made in lib.rs: the page and the site's view)
    b = conf["bundle"]
    assert b["macOS"]["signingIdentity"] == "-"              # ad hoc: an Apple silicon Mac runs nothing unsigned
    assert b["windows"]["nsis"]["installMode"] == "currentUser"
    for icon in b["icon"]:
        assert (TAURI / icon).is_file(), icon
    plist = (TAURI / "Info.plist").read_text()
    for key in ("NSMicrophoneUsageDescription", "NSDocumentsFolderUsageDescription", "NSAllowsLocalNetworking",
                "NSAllowsArbitraryLoadsInWebContent"):
        assert key in plist, key
    flow = (ROOT / ".github" / "workflows" / "apps.yml").read_text()
    assert "windows-latest" in flow and "macos-latest" in flow and "universal-apple-darwin" in flow
    assert "Chat-Computer-Windows.exe" in flow and "Chat-Computer-Mac.dmg" in flow and "@tauri-apps/cli@2.12.3" in flow
    assert "python scripts/build_apps.py desktop" in flow and "cargo test --locked" in flow
    assert (TAURI / "Cargo.lock").is_file()
    # The model's engine comes with the app, beside its program (Tauri's externalBin, built for the app's target by
    # build_apps.py engine, before the app is built: binaries/ isn't kept in git).
    assert b["externalBin"] == ["binaries/chat-computer-model"]
    assert "python scripts/build_apps.py engine" in flow and flow.index("build_apps.py engine") < flow.index("cargo test --locked")
    assert "apps/desktop/src-tauri/binaries/" in (ROOT / ".gitignore").read_text()


def test_the_model_runs_on_this_computer_and_ends_with_the_app():
    """engine.rs: the engine beside the app's program, its processes holding nothing once the app has gone (each
    started with --exit-with-stdin, the app holding their standard input), getting the model ready at a lower
    priority; the models in the app's own folder, and another folder or catalogue only in a build for testing; the
    page told how it goes; started again should it stop by itself."""
    engine = (TAURI / "src" / "engine.rs").read_text()
    lib = (TAURI / "src" / "lib.rs").read_text()
    assert 'format!("chat-computer-model{}", std::env::consts::EXE_SUFFIX)' in engine
    assert '.arg("--exit-with-stdin").stdin(Stdio::piped())' in engine
    assert "BELOW_NORMAL_PRIORITY_CLASS" in engine and "CREATE_NO_WINDOW" in engine and '"/usr/bin/nice"' in engine
    assert engine.count("stdin.take()") == 3             # (models, serve and prepare: each held)
    assert 'app.path().app_local_data_dir()?.join("models")' in lib
    assert 'if cfg!(debug_assertions) {\n        if let Some(d) = std::env::var_os("CHATCOMPUTER_MODELS_DIR")' in lib
    assert 'std::env::var_os("CHATCOMPUTER_ENGINE_CATALOGUE").filter(|_| cfg!(debug_assertions))' in lib
    assert 'tell_main(&told, serde_json::json!({ "model": s }))' in lib and "stopping.stop();" in lib
    assert "fn watch(" in engine and "if again == 3" in engine
    model_status = re.search(r"fn model_status\(([^)]*)\)[^{]*\{(.*?)\n\}", lib, re.S).group(2)
    assert "from_main(&webview)?" in model_status


BINARY = TAURI / "target" / "debug" / "chat-computer"


def _can_drive():
    # (desktop_check.py drives the app with Selenium, which the runtime's requirements don't have.)
    import importlib.util
    driver = shutil.which("tauri-driver") or os.path.expanduser("~/.cargo/bin/tauri-driver")
    return (BINARY.exists() and os.path.exists(driver) and shutil.which("WebKitWebDriver") and shutil.which("Xvfb")
            and importlib.util.find_spec("selenium") is not None)


@pytest.mark.skipif(not _can_drive(), reason="the app isn't built here, or WebDriver for it isn't there")
@pytest.mark.parametrize("loopback", [False, True])
def test_the_app_works(loopback):
    r = subprocess.run([sys.executable, str(ROOT / "tests" / "desktop_check.py"), str(BINARY)] + (["loopback"] if loopback else []),
                       capture_output=True, text=True, timeout=240)
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "failed" not in res, res
    # The app's own page: from Tauri's scheme, or (as on a Mac) the app's own server.
    assert res["page"] == ("http://127.0.0.1:42851" if loopback else "tauri://localhost") and res["secure"] is True
    assert res["greeting"] and not res["greeting"].startswith("and ") and res["about"]["ok"]["system"] == "linux"
    # The model: the one here (a tiny one, of random weights) served by the engine beside the app, on this computer;
    # the page told, "/" names it, and it answers what's asked (words of no sense: its weights are random); when the
    # app closes, the engine ends with it.
    m = res["model"]
    assert (m["state"], m["label"], m["preparing"]) == ("ready", "Tiny", "") and 8765 <= m["port"] < 8775
    assert res["model_slash"] == "Tiny"
    answer = res["model_answer"]
    assert answer and not answer.startswith(("My model stopped", "My model couldn't", "I can't")), answer
    assert res["engine_left"] == 0
    # "/": in the usual folders, by name; opened with the system's opener; what "/" didn't find, and programs, not.
    assert sorted(res["found"]) == [["Taxes", "~/Documents", "folder"], ["Taxes 2025.pdf", "~/Documents/Taxes", "document"]]
    assert res["open_found"] == {"ok": "Taxes 2025.pdf"} and res["opened"] and res["opened"][-1].endswith("Taxes 2025.pdf")
    assert "Only a file found with" in res["open_unfound"]["no"]
    assert "Programs and scripts aren't opened from here" in res["open_program"]["no"]
    assert res["slash"] == "Beach.jpg"
    # A site in the middle, in the app's own view: what it's at and called come back; it can't ask the app.
    assert res["site_url"] == "http://127.0.0.1:8790/a.html"
    assert res["site_title"] in ("A test page: no app", "A test page: the app said no")
    assert res["content"] == [True, res["content"][1]] and res["content"][1].startswith("A test page")
    # The page sent elsewhere comes back, with that address in the middle; closed, the conversation again.
    assert res["back_home"] == res["page"] and res["sent_there"] == "B page" and res["closed"] == "yes"
    # Messages: Gmail, WhatsApp and Outlook (this check's own pages, built as theirs are) connected from + in
    # Messages (one panel of everything the app connects, by kind); their conversations together,
    # newest first; one tapped opens its service at that conversation (and is read); a site in the middle can't hand
    # over messages; Outlook disconnected, its go.
    assert res["choices"] == ["Open Gmail", "Outlook", "Telegram", "WhatsApp", "WeChat", "Discord", "Disconnect Gmail"]
    assert res["accounts"] == ["gmail", "whatsapp", "outlook"]
    got = {m[0]: m[1:] for m in res["messages"]}
    assert got == {"Mum": ["WhatsApp: Dinner at 7 tonight?", True], "Alex": ["WhatsApp: Sent you the slides", False],
                   "Jane Lim": ["Gmail: Lunch on Friday? – 12:30 at the usual place works for me.", True],
                   "City Library": ["Gmail: Books due – Your books are due back on Thursday.", False],
                   "Sam Lee": ["Outlook: Trip photos – Here they are, all 40.", True]}
    assert res["tapped"] == "opened: Mum" and res["c_title"] == "C page: the app said no"
    after = {m[0]: m[1:] for m in res["after_disconnect"]}
    assert "Sam Lee" not in after and after["Mum"] == ["WhatsApp: Dinner at 7 tonight?", False] and len(after) == 4
    # How you want Chat Computer: asked first, the two ways (the own system not from Linux, saying why); the app
    # chosen, the app it is, and "/" lists how you have it.
    assert res["first_page"] == "/start/index.html"
    assert res["ways"] == [["app", False], ["os", True]]
    st = res["way_status"]
    assert st["way"] is None and st["os"]["can"] is False and st["os"]["system"] == "linux"
    assert "installs from Windows or from macOS" in res["os"] and "installs from Windows or from macOS" in res["os_choose"]["no"]
    assert res["about_way"] == "app"
    assert res["way_slash"] == "How you have Chat Computer" and res["way_now"] == "app"
    if loopback:
        assert res["other_name"] == 404 and res["own_name"] == 200


def _node_with_playwright():
    node = shutil.which("node")
    root = subprocess.run(["npm", "root", "-g"], capture_output=True, text=True).stdout.strip() if shutil.which("npm") else ""
    return (node, root) if node and root and (Path(root) / "playwright").exists() else (None, None)


@pytest.mark.skipif(_node_with_playwright()[0] is None, reason="node with Playwright not available")
def test_the_reader_reads_each_service_s_list():
    """Messages' reader (reader.js) in Chromium, the engine of Windows' WebView2, on pages built as each service's
    list is, at each service's own address (tests/reader_check.js)."""
    node, modules = _node_with_playwright()
    r = subprocess.run([node, str(ROOT / "tests" / "reader_check.js")], capture_output=True, text=True, timeout=180,
                       env={**os.environ, "NODE_PATH": modules})
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "failed" not in res, res["failed"]
    assert res["errors"] == []
    # Each service: who, what (an email's subject and start; a chat's last message), when, unread or not; a time
    # read wherever one is shown; handed to the app as messages_found; the first opened as a tap on it would.
    expect = {
        "gmail": [["Jane Lim", "Lunch on Friday? – 12:30 at the usual place works for me.", "9:05 AM", True],
                  ["Ann, Bo", "Plans – See you there", "Oct 28", False]],
        "outlook": [["Sam Lee", "Trip photos – Here they are, all 40.", "8:30 AM", True],
                    ["City Council", "Bin day – Collections move to Friday.", "2026-10-30", False]],
        "telegram": [["Alex", "Sent you the slides for Monday.", "Yesterday", True], ["Chess club", "Lee: Game at 6?", "Sat", False]],
        "whatsapp": [["Mum", "Dinner at 7 tonight? Bring the photos!", "10:15", True], ["Dad", "Call me back", "11/2/2026", False]],
        "wechat": [["李明", "明天见", "下午3:20", True], ["王芳", "好的", "昨天", False]],
        "discord": [["Priya", "New message", "", True], ["Tom", "Direct message", "", False]],
        # A list whose parts aren't named as expected (its service changed its page): read by its shape.
        "changed": [["Kim", "On my way", "9:41", False], ["Joe", "Thanks!", "Yesterday", False], ["Eve", "See the doc", "Tue", False]],
    }
    for k, items in expect.items():
        got = res[k]
        assert [i[:4] for i in got["items"]] == items, k
        assert all(i[4] == bool(i[2]) for i in got["items"]), k                 # (a time wherever one is shown)
        assert got["command"] == "messages_found" and got["opened"] is True, k
        assert got["title_after"].startswith("opened "), k
        assert all(re.fullmatch(r"[a-z]+:[0-9a-z]+", i) for i in got["ids"]) and len(set(got["ids"])) == len(got["ids"]), k
    assert [res[k]["unread_in_title"] for k in ("gmail", "whatsapp", "discord", "outlook")] == [2, 1, 3, None]
    # Nothing listed (signed out, or still loading): the count in the page's title alone.
    assert res["title_only"]["items"] == [] and res["title_only"]["unread_in_title"] == 12
    # The times it understands (now: Tuesday 3 November 2026, 14:00, in London, its dates day first).
    when = dict((k, v) for k, v in res["when"])
    assert when == {
        "10:15": "Tue Nov 03 2026 10:15", "9:05 AM": "Tue Nov 03 2026 09:05", "3:20 PM": "Mon Nov 02 2026 15:20",
        "16:45": "Mon Nov 02 2026 16:45", "下午3:20": "Mon Nov 02 2026 15:20", "上午9:00": "Tue Nov 03 2026 09:00",
        "Yesterday": "Mon Nov 02 2026 12:00", "昨天": "Mon Nov 02 2026 12:00", "Sat": "Sat Oct 31 2026 12:00",
        "星期一": "Mon Nov 02 2026 12:00", "Oct 28": "Wed Oct 28 2026 12:00", "2 Nov": "Mon Nov 02 2026 12:00",
        "11/2/2026": "Wed Feb 11 2026 12:00", "2026-10-30": "Fri Oct 30 2026 12:00", "10月30日": "Fri Oct 30 2026 12:00",
        "5m": "Tue Nov 03 2026 13:55", "2h": "Tue Nov 03 2026 12:00", "just now": "Tue Nov 03 2026 14:00",
        "sometime": None, "": None, "9:05 AM + its date": "Mon Nov 02 2026 09:05"}
