"""The Chat Computer app for Windows and Mac (apps/desktop): the website's page bundled into a window of its own
(scripts/build_apps.py desktop), with what a website can't do (any site in the middle, in the app's own view; "/"
finding the computer's files). Only the app's own page may ask the app for those: its capabilities say so, and so
does each command. Where it's built here for Linux (tauri build --debug --no-bundle) and WebDriver for it is there
(tauri-driver, WebKitWebDriver), it's tried for real (tests/desktop_check.py)."""
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

COMMANDS = ["show_site", "place_site", "hide_site", "unhide_site", "close_site", "site_media", "search_files", "open_file", "about"]


def test_the_app_s_page_is_the_website_s(tmp_path, monkeypatch):
    monkeypatch.setattr(build_apps, "FRONTEND", tmp_path / "frontend")
    out = build_apps.desktop_frontend(lm=False)
    assert (out / "index.html").read_bytes() == (ROOT / "docs" / "index.html").read_bytes()
    for f in (ROOT / "docs" / "demo").rglob("*"):
        if f.is_file():
            assert (out / "demo" / f.relative_to(ROOT / "docs" / "demo")).read_bytes() == f.read_bytes(), f
    # Gemma's code, the same version as the website's (from jsDelivr there; with the app, from npm, here).
    host = (ROOT / "docs" / "demo" / "host.js").read_text()
    assert '"@huggingface/transformers@4.3.1"' in (ROOT / "scripts" / "build_apps.py").read_text()
    assert 'var LM_LIBRARY = "https://cdn.jsdelivr.net/npm/@huggingface/transformers@4.3.1";' in host
    assert 'if (APP) LM_LIBRARY = LM_HERE + "transformers.min.js";' in host
    assert list(build_apps.LM) == ["@huggingface/transformers@4.3.1", "onnxruntime-web@1.31.0-dev.20260914-8d85527a0"]


def test_only_the_app_s_own_page_may_ask_the_app():
    build = (TAURI / "build.rs").read_text()
    assert re.findall(r'"([a-z_]+)",', build) == COMMANDS
    lib = (TAURI / "src" / "lib.rs").read_text()
    handlers = re.search(r"generate_handler!\[([^\]]+)\]", lib).group(1)
    assert sorted(x.strip() for x in handlers.split(",") if x.strip()) == sorted(COMMANDS)
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


BINARY = TAURI / "target" / "debug" / "chat-computer"


def _can_drive():
    driver = shutil.which("tauri-driver") or os.path.expanduser("~/.cargo/bin/tauri-driver")
    return BINARY.exists() and os.path.exists(driver) and shutil.which("WebKitWebDriver") and shutil.which("Xvfb")


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
    assert res["greeting"] == "hello, world" and res["about"]["ok"]["system"] == "linux"
    # "/": in the usual folders, by name; opened with the system's opener; what "/" didn't find, and programs, not.
    assert sorted(res["found"]) == [["Taxes", "~/Documents", "folder"], ["Taxes 2025.pdf", "~/Documents/Taxes", "document"]]
    assert res["open_found"] == {"ok": "Taxes 2025.pdf"} and res["opened"] and res["opened"][-1].endswith("Taxes 2025.pdf")
    assert "Only a file found with" in res["open_unfound"]["no"]
    assert "Programs and scripts aren't opened from here" in res["open_program"]["no"]
    assert res["slash"] == "Beach.jpg"
    # A site in the middle, in the app's own view: what it's at and called come back; it can't ask the app.
    assert res["site_url"] == "http://127.0.0.1:8765/a.html"
    assert res["site_title"] in ("A test page: no app", "A test page: the app said no")
    assert res["content"] == [True, res["content"][1]] and res["content"][1].startswith("A test page")
    # The page sent elsewhere comes back, with that address in the middle; closed, the conversation again.
    assert res["back_home"] == res["page"] and res["sent_there"] == "B page" and res["closed"] == "yes"
    if loopback:
        assert res["other_name"] == 404 and res["own_name"] == 200
