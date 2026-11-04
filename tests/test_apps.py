"""What chatcomputer.ai/get offers (docs/get): the page, as Annotated's is (the name, what to get it with, what it
is; no video), and the apps it links to, made by scripts/build_apps.py from apps/. The Chrome extension is tried in
Chromium when Playwright is there.

On Windows and Mac, one Setup each (the Chat Computer app, apps/desktop, and for the Mac a line of Terminal that
installs it): the app's first page asks how you want Chat Computer, as a Chrome extension, as your browser, as an app
or as its own system, and the app does it (src-tauri/src/ways.rs). Its first page, and the app as a browser (tabs),
are tried in Chromium too (tests/app_ways_check.js), with a stand-in for the app; tests/test_desktop.py drives the
real app, built for Linux, through them."""
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

import build_apps  # noqa: E402

GET = ROOT / "docs" / "get"
DESKTOP = ROOT / "apps" / "desktop"
TAURI = DESKTOP / "src-tauri"


def test_what_get_offers_is_what_build_apps_makes():
    stale = [str(p.relative_to(ROOT)) for p, data in build_apps.files().items() if not p.exists() or p.read_bytes() != data]
    assert stale == [], f"run: python scripts/build_apps.py ({stale})"


def test_the_get_page_is_as_annotated_s_without_the_video():
    page = (GET / "index.html").read_text()
    # The name, then what to get it with, then what it is. No video, no picture logo (the words alone).
    assert re.search(r"<h1>Chat Computer</h1>", page) and '<h2 class="tagline">' in page
    assert page.index("<h1>") < page.index('class="get"') < page.index('class="tagline"')
    assert "<iframe" not in page and "<video" not in page and "<img" not in page and 'rel="icon"' not in page
    # Each download is there, and says how to install it; what isn't ready yet says so and links nowhere.
    links = dict(re.findall(r'<a class="add[^"]*" id="(\w+)" href="([^"]+)"', page))
    for name, href in links.items():
        if not href.startswith(("../", "http", "#")):
            assert (GET / href).is_file(), (name, href)
    assert links == {"chrome": "Chat-Computer-Chrome.zip", "windows": "Chat-Computer-Windows.exe", "mac": "#mac-setup",
                     "android": "Chat-Computer-Android.apk", "web": "../"}
    assert 'title="Downloads the extension. Unzip it, open chrome://extensions, turn on Developer mode, and choose Load unpacked."' in page
    # The apps aren't signed with paid certificates yet: each button says what's asked the first time.
    assert "choose More info, then Run anyway" in page and "Privacy &amp; Security, and choose Open Anyway" in page
    assert "allow installing from your browser" in page
    soon = re.findall(r'<span class="add soon" id="(\w+)" aria-disabled="true"', page)
    assert soon == ["iphone"] and 'title="Coming to the App Store">iPhone<' in page
    # Windows and Mac: one Setup each, which asks how you want Chat Computer; the Mac's a line for Terminal (shown when
    # Mac is chosen), with the disk image as the other way.
    for name in ("windows", "mac"):
        title = re.search(r'id="%s" href="[^"]+"(?: download)?\s+title="([^"]+)"' % name, page).group(1)
        assert "as a Chrome extension, as your browser, as an app, or as its own system" in title, name
    assert "<code>curl -fsSL https://chatcomputer.ai/get/mac.sh | bash</code>" in page and "#mac-setup:target { display: block; }" in page
    assert '<a id="mac-dmg" href="Chat-Computer-Mac.dmg" download>' in page and "M1, M2 or M3" in page
    # What each download is: Windows' installer (NSIS), a Mac disk image, an Android app.
    assert (GET / "Chat-Computer-Windows.exe").read_bytes()[:2] == b"MZ" and b"Nullsoft" in (GET / "Chat-Computer-Windows.exe").read_bytes()
    assert (GET / "Chat-Computer-Mac.dmg").read_bytes()[-512:][:4] == b"koly"
    with __import__("zipfile").ZipFile(GET / "Chat-Computer-Android.apk") as z:
        assert "AndroidManifest.xml" in z.namelist() and "classes.dex" in z.namelist()


def _apk_certificate(apk: bytes) -> bytes:
    """The certificate an APK is signed with (APK Signature Scheme v2/v3: the signing block just before the zip's
    central directory)."""
    import struct
    eocd = apk.rfind(b"PK\x05\x06")
    cd = struct.unpack_from("<I", apk, eocd + 16)[0]
    assert apk[cd - 16:cd] == b"APK Sig Block 42"
    size = struct.unpack_from("<Q", apk, cd - 24)[0]
    at, end = cd - size, cd - 24                        # (the pairs: after the first size, before the second)
    while at < end:
        length, kind = struct.unpack_from("<QI", apk, at)
        value = apk[at + 12:at + 8 + length]
        at += 8 + length
        if kind in (0x7109871A, 0xF05368C0):                 # v2, v3
            p = 4 + 4                                            # signers, the first signer
            p += 4                                               # its signed data
            digests = struct.unpack_from("<I", value, p)[0]
            p += 4 + digests + 4                                 # past the digests, into the certificates
            n = struct.unpack_from("<I", value, p)[0]
            return value[p + 4:p + 4 + n]
    raise AssertionError("no v2 or v3 signature")


def test_the_android_app_is_the_one_chatcomputer_ai_vouches_for():
    """chatcomputer.ai says the Android app is its own (docs/.well-known/assetlinks.json): its package, and the
    fingerprint of the key it's signed with, which must be the downloadable app's, or Chrome shows an address bar."""
    import hashlib
    links = json.loads((ROOT / "docs" / ".well-known" / "assetlinks.json").read_text())
    target = links[0]["target"]
    assert links[0]["relation"] == ["delegate_permission/common.handle_all_urls"] and target["package_name"] == "ai.chatcomputer.app"
    cert = _apk_certificate((GET / "Chat-Computer-Android.apk").read_bytes())
    fingerprint = ":".join(f"{b:02X}" for b in hashlib.sha256(cert).digest())
    assert target["sha256_cert_fingerprints"] == [fingerprint]
    gradle = (ROOT / "apps" / "android" / "app" / "build.gradle.kts").read_text()
    assert 'applicationId = "ai.chatcomputer.app"' in gradle and "CHATCOMPUTER_KEYSTORE" in gradle
    manifest = (ROOT / "apps" / "android" / "app" / "src" / "main" / "AndroidManifest.xml").read_text()
    assert "com.google.androidbrowserhelper.trusted.LauncherActivity" in manifest and 'android:host="chatcomputer.ai"' in manifest
    strings = (ROOT / "apps" / "android" / "app" / "src" / "main" / "res" / "values" / "strings.xml").read_text()
    assert '\\"site\\": \\"https://chatcomputer.ai\\"' in strings and "https://chatcomputer.ai/</string>" in strings


def test_the_chrome_extension_is_small_and_asks_for_little():
    m = json.loads((ROOT / "apps" / "chrome" / "manifest.json").read_text())
    assert m["manifest_version"] == 3 and m["name"] == "Chat Computer"
    # Only the side panel: opening a tab needs no permission, and it reads no page.
    assert m["permissions"] == ["sidePanel"] and "host_permissions" not in m and "content_scripts" not in m
    assert m["side_panel"]["default_path"] == "panel.html" and m["background"] == {"service_worker": "background.js"}
    assert "icons" not in m and "default_icon" not in m["action"]        # (no picture logo: Chrome's own letter)
    panel = (ROOT / "apps" / "chrome" / "panel.html").read_text()
    assert re.findall(r"<script[^>]*>", panel) == ['<script src="panel.js">']
    assert 'src="https://chatcomputer.ai/?app=chrome"' in panel
    # The website in the panel may reach the Chat Computer app's model on this computer (Chrome asks once: its Local
    # Network Access, delegated to the frame by name, the current one and the one before it).
    assert 'allow="loopback-network; local-network-access;' in panel and "language-model" not in panel
    js = (ROOT / "apps" / "chrome" / "panel.js").read_text()
    # Only Chat Computer's own page can ask for a tab, and only for a web address.
    assert 'const SITE = "https://chatcomputer.ai";' in js and "e.origin !== SITE" in js
    assert 'url.protocol !== "https:" && url.protocol !== "http:"' in js
    with __import__("zipfile").ZipFile(GET / "Chat-Computer-Chrome.zip") as z:
        assert sorted(z.namelist()) == sorted("Chat Computer/" + n for n in build_apps.CHROME_FILES)


def _node_with_playwright():
    node = shutil.which("node")
    if not node:
        return None, None
    root = subprocess.run(["npm", "root", "-g"], capture_output=True, text=True).stdout.strip() if shutil.which("npm") else ""
    if not root or not (Path(root) / "playwright").exists():
        return None, None
    return node, root


@pytest.mark.skipif(_node_with_playwright()[0] is None, reason="node with Playwright not available")
def test_the_chrome_extension_works_in_chromium():
    node, modules = _node_with_playwright()
    r = subprocess.run([node, str(ROOT / "tests" / "chrome_check.js")], capture_output=True, text=True, timeout=180,
                       env={**os.environ, "NODE_PATH": modules})
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "failed" not in res, res["failed"]
    assert res["errors"] == []
    # Its side panel is Chat Computer, the website in its frame, at a side panel's text size (no Escape there).
    o = res["opening"]
    greeting = o.pop("greeting")
    assert o == {"url": "https://chatcomputer.ai/?app=chrome", "rem": "16px", "hint": "Type anything", "logo": "Chat Computer"}
    assert greeting and not greeting.startswith("and ")                       # (one line under the word logo)
    # A site's address typed: a real tab with it, at once, without a click.
    tabs = res["typed"]["tabs"]
    assert len(tabs) == 1 and re.match(r"https://(www\.)?bbc\.co\.uk/", tabs[0]) and res["typed"]["said"] == "Opened bbc.co.uk in a new tab."
    # Another page can't make it open tabs.
    assert res["forged"] is False


# ---------------------------------------------------------------------------------------------------------------------
# One Setup for Windows and one for Mac: the app, whose first page asks how you want Chat Computer.

def test_the_mac_s_setup_is_one_line_of_terminal():
    """mac.sh (curl -fsSL https://chatcomputer.ai/get/mac.sh | bash), made by build_apps.py from apps/desktop/get/mac.sh:
    one download, checked against the SHA-256 of the disk image beside it, put in Applications without an
    administrator's password, then opened (it asks how you want Chat Computer). A file curl fetches isn't marked as
    from the internet, so macOS doesn't ask for Open Anyway."""
    import hashlib
    sh = (GET / "mac.sh").read_text()
    sha = hashlib.sha256((GET / "Chat-Computer-Mac.dmg").read_bytes()).hexdigest()
    assert f"sha256='{sha}'" in sh and "url='https://chatcomputer.ai/get/Chat-Computer-Mac.dmg'" in sh
    assert "@" not in re.sub(r"#.*", "", sh)                                  # (nothing left to fill in)
    assert sh.startswith("#!/bin/bash\n") and "set -euo pipefail" in sh and "sudo" not in sh
    steps = ["curl -fL", "shasum -a 256 -c -", "hdiutil attach", "ditto ", "open \"$dest/$app\""]
    at = [sh.index(x) for x in steps]
    assert at == sorted(at), "download, check, mount, copy, open: in that order"
    assert 'if [ ! -w "$dest" ]; then dest="$HOME/Applications"' in sh and "trap cleanup EXIT" in sh
    r = subprocess.run(["bash", "-n", str(GET / "mac.sh")], capture_output=True, text=True)
    assert r.returncode == 0, r.stderr


def test_the_app_asks_how_you_want_chat_computer():
    """Its first page (apps/desktop/start, put beside the website's page as start/): the four ways, in Chat Computer's
    own look (theme.css); it asks the app how things are (way_status) and to do the way chosen (choose_way)."""
    page = (DESKTOP / "start" / "index.html").read_text()
    js = (DESKTOP / "start" / "start.js").read_text()
    css = (DESKTOP / "start" / "start.css").read_text()
    assert re.findall(r'data-way="(\w+)"', page) == ["chrome", "browser", "app", "os"]
    assert [n.strip() for n in re.findall(r'<span class="way-name">([^<]+)</span>', page)] == [
        "Chrome extension", "Browser", "App", "Its own system"]
    assert "<h2 id=\"ask\">How do you want Chat Computer?</h2>" in page and "You can change this later" in page
    assert '<link rel="stylesheet" href="../demo/theme.css">' in page and re.findall(r"<script[^>]*>", page) == ['<script src="start.js">']
    assert "<img" not in page and 'rel="icon"' not in page
    # Plainly: Chrome adds extensions by itself only from its Web Store, and won't let another app open its page.
    assert "Chrome adds extensions by itself only from its Web Store" in page and "chrome://extensions" in page
    assert "Developer mode" in page and "Load unpacked" in page
    for cmd in ("way_status", "choose_way", "make_default_browser", "show_folder"):
        assert f'app("{cmd}"' in js, cmd
    assert 'location.href = "../index.html"' in js and "if (way === \"os\" && !status.os.can)" in js
    assert "var(--ground)" not in css or "background: var(--ink); color: var(--ground)" in css
    assert "font-family" not in css.split("#start")[0]                    # (Lora: theme.css's, everywhere)
    # It's in the app's page: build_apps.py desktop puts it there.
    import build_apps
    assert build_apps.START == DESKTOP / "start"
    # "/" in the app lists it (last), and opens it to change the way.
    host = (ROOT / "docs" / "demo" / "host.js").read_text()
    assert 'if (id === "way") location.href = "start/index.html?change";' in host
    assert 'if (APP) list.push({ command: "way", icon: "i-chat", name: "How you have Chat Computer",' in host


def test_the_app_does_each_way():
    """src-tauri/src/ways.rs: the Chrome extension, as apps/chrome has it (the same files as the zip), put in a folder of
    the app's own and Chrome opened; the browser listed with Windows for this account only, and taken out again (by
    the app, when another way is chosen, and by the uninstaller); a Mac told the app opens web addresses (Info.plist);
    the own system, today's Chat Computer Setup, which comes with the app, run from a folder of its own."""
    import build_apps
    ways = (TAURI / "src" / "ways.rs").read_text()
    lib = (TAURI / "src" / "lib.rs").read_text()
    files = re.findall(r'\("([\w.]+)", include_str!\("\.\./\.\./\.\./chrome/([\w.]+)"\)\)', ways)
    assert [a for a, b in files] == [b for a, b in files] == build_apps.CHROME_FILES
    assert 'app_dir.join("Chrome extension")' in ways and 'c.arg("--new-window").arg(page);' in ways
    assert 'format!("https://chatcomputer.ai/get/chrome.html#folder={path}")' in ways
    # Chrome opens at the same steps, on chatcomputer.ai (its folder after #, which stays in the browser).
    steps = (GET / "chrome.html").read_text()
    assert "<title>Chat Computer for Chrome</title>" in steps and "chrome://extensions" in steps and "Load unpacked" in steps
    assert "/[#&]folder=([^&]*)/.exec(location.hash)" in steps and "fetch(" not in steps and "<img" not in steps
    # The browser: StartMenuInternet, Capabilities, RegisteredApplications and a URL ProgID, under HKEY_CURRENT_USER.
    for key in (r'k(&caps, "ApplicationName", BROWSER_NAME)', r'k(&format!(r"{caps}\URLAssociations"), "https", PROGID)',
                r'k(r"Software\RegisteredApplications", BROWSER_NAME, &caps)', r'format!("\"{exe}\" --url \"%1\"")'):
        assert key in ways, key
    assert "RegKey::predef(HKEY_CURRENT_USER)" in ways and "HKEY_LOCAL_MACHINE" not in ways.split("pub fn register_browser")[1].split("pub fn unregister_browser")[0]
    assert 'open::that_detached("ms-settings:defaultapps?registeredAppUser=Chat%20Computer")' in ways
    assert "fn LSSetDefaultHandlerForURLScheme(scheme: CFStringRef, bundle: CFStringRef) -> i32;" in ways
    hooks = (TAURI / "nsis" / "hooks.nsh").read_text()
    for gone in ('DeleteRegKey HKCU "Software\\Clients\\StartMenuInternet\\Chat Computer"',
                 'DeleteRegValue HKCU "Software\\RegisteredApplications" "Chat Computer"', 'DeleteRegKey HKCU "Software\\Classes\\ChatComputerURL"'):
        assert gone in hooks, gone
    assert "!macro NSIS_HOOK_POSTUNINSTALL" in hooks and "${If} $UpdateMode <> 1" in hooks
    assert "as a Chrome extension" in hooks and "MUI_FINISHPAGE_RUN_TEXT" in hooks
    conf = json.loads((TAURI / "tauri.conf.json").read_text())
    assert conf["bundle"]["windows"]["nsis"]["installerHooks"] == "nsis/hooks.nsh"
    import plistlib
    plist = plistlib.loads((TAURI / "Info.plist").read_bytes())
    assert plist["CFBundleURLTypes"] == [{"CFBundleURLName": "Web site address", "CFBundleURLSchemes": ["http", "https"]}]
    # Links from other apps: Windows starts the app again with one (one at a time: the running one gets it); a Mac
    # sends it to the app (RunEvent::Opened). Kept until the conversation's page asks (take_opening).
    cargo = (TAURI / "Cargo.toml").read_text()
    assert "[target.'cfg(windows)'.dependencies]\nwinreg = \"0.56\"\ntauri-plugin-single-instance = \"2.5\"" in cargo
    assert "builder.plugin(tauri_plugin_single_instance::init(" in lib and "if let tauri::RunEvent::Opened { urls } = &event" in lib
    assert "from_outside(app, ways::urls_in(&argv));" in lib
    # The own system: today's Chat Computer Setup (the repository's files it uses, as git has them), with the app
    # (Tauri's resources), run from a folder of its own: Windows' .cmd (it asks for permission and shows its plan
    # itself); a Mac's setup.sh in Terminal, on M1, M2 and M3 only.
    assert conf["bundle"]["resources"] == {"setup/": "setup/"}
    win, mac = build_apps.setup_files("windows"), build_apps.setup_files("mac")
    for f in ("windows/Chat Computer Setup.cmd", "windows/setup.ps1", "windows/lib/Installer.ps1", "chatcomputer/brain/models.json",
              "linux/install-chatcomputer.sh", "README.md"):
        assert f in win, f
    for f in ("mac/setup.sh", "mac/manifest.sh", "Chat Computer Setup.command", "linux/install-chatcomputer.sh", "README.md"):
        assert f in mac, f
    for skip in ("docs/", "apps/", "evals/", ".github/"):
        assert not any(f.startswith(skip) for f in win + mac), skip
    assert not any(f.startswith("mac/") for f in win) and not any(f.startswith("windows/") for f in mac)
    assert 'dir.join("windows").join("Chat Computer Setup.cmd")' in ways and "exec bash mac/setup.sh" in ways
    assert '.args(["-a", "Terminal"])' in ways and 'sysctl("machdep.cpu.brand_string")' in ways
    assert "apps/desktop/src-tauri/setup/" in (ROOT / ".gitignore").read_text()


@pytest.mark.skipif(_node_with_playwright()[0] is None, reason="node with Playwright not available")
def test_the_app_s_first_page_and_its_tabs_work_in_chromium(tmp_path, monkeypatch):
    import build_apps
    monkeypatch.setattr(build_apps, "FRONTEND", tmp_path / "frontend")
    frontend = build_apps.desktop_frontend(lm=False)
    node, modules = _node_with_playwright()
    r = subprocess.run([node, str(ROOT / "tests" / "app_ways_check.js"), str(frontend)], capture_output=True, text=True, timeout=300,
                       env={**os.environ, "NODE_PATH": modules})
    lines = [x for x in r.stdout.splitlines() if x.startswith("{")]
    assert lines, (r.stdout[-2000:], r.stderr[-2000:])
    res = json.loads(lines[-1])
    assert "failed" not in res, res["failed"]
    assert res["errors"] == []
    # The first start: the four ways, in Chat Computer's look; the own system in a PC's words.
    first = res["first"]
    assert first["panels"] == ["choose"] and first["ask"] == "How do you want Chat Computer?"
    assert [(w["way"], w["now"], w["cannot"]) for w in first["ways"]] == [("chrome", False, False), ("browser", False, False),
                                                                        ("app", False, False), ("os", False, False)]
    assert first["ways"][3]["what"] == "Chat Computer in place of Windows when the PC starts. Windows stays, one restart away."
    assert first["font"].startswith("Lora") and first["ground"] == "rgb(25, 25, 25)"
    # The app: the conversation at once, as before (no tabs; an address typed, the one site in the middle); "/" lists
    # how you have it, last; opened again, the app is marked as now.
    assert res["app"] == {"calls": ["about", "model_status", "take_opening"], "tabs": None}
    assert res["app_slash"][-1] == "How you have Chat Computer"
    assert res["app_site"] == [["show_site", "https://bbc.co.uk/"]]
    assert res["change"] == {"ways": ["app"]}
    # The Chrome extension: its folder's path copied in the click, Chrome open, the steps (Chrome's own page by its
    # address: no app can open it), in Windows' words; chrome://extensions copied; the folder shown.
    c = res["chrome"]
    assert c["panels"] == ["way-chrome"] and c["open"] and c["copied_note"] and not c["no_chrome"]
    assert c["folder"] == "C:\\Users\\Ren\\AppData\\Local\\ai.chatcomputer.desktop\\Chrome extension"
    assert c["copied"] == [c["folder"], "chrome://extensions"]
    assert c["steps"][0].startswith("In Chrome's address bar, type chrome://extensions") and "Developer mode" in c["steps"][1]
    assert "Load unpacked" in c["steps"][2] and c["folder"] in c["steps"][2]
    assert c["how"][0].startswith("In the window that opens, click the address bar") and "Command" not in " ".join(c["how"])
    assert c["calls"] == [["choose_way", {"way": "chrome"}], ["show_folder", {"which": "extension"}]]
    assert res["change_again"] == ["chrome"]
    # The browser: asked to be the default (Windows' Settings, where the person chooses it), then told it is.
    b = res["browser"]
    assert b["ask"] and "Settings opens at Chat Computer" in b["words"] and "macOS" not in b["words"]
    assert b["asked"] == "Settings is open: choose Chat Computer there." and b["yes"].startswith("It's your default browser")
    # The own system, on a PC: Chat Computer Setup started, in Windows' words (permission, its plan, nothing changed
    # until y), its folder.
    o = res["os"]
    assert o["started"] and "choose Yes" in o["words"] and "until you answer y" in o["words"] and "Terminal" not in o["words"]
    assert o["folder"].endswith("Chat Computer Setup")
    # Started again on the extension: at its steps, nothing done by itself.
    assert res["again"] == {"panels": ["way-chrome"], "open": False, "chose": 0}
    # A Mac with M4: the own system says why it can't, and the app isn't asked; the folder's steps in a Mac's words.
    m = res["mac"]
    assert m["os"]["cannot"] and m["os"]["what"].startswith("This Mac has an Apple M4") and m["error"] == m["os"]["what"]
    assert m["asked"] == [] and m["panels"] == ["choose"] and m["how"][0].startswith("In the window that opens, press Command-Shift-G")
    # The browser: the link from another app, in the first tab; the row above the middle, the frame under it, a tab
    # with no bar of its own.
    t = res["tabs"]
    assert t["outside"]["tabs"] == [[0, "Chat Computer", False], [1, "example.org", True]] and t["outside"]["content"]
    assert t["opened"][1]["tab"] == 1 and t["opened"][1]["url"] == "https://example.org/from-another-app"
    assert t["outside"]["bar"] == "none" and t["outside"]["frame_top"] > t["outside"]["bottom"] and t["layout"]["tabbed"]
    assert t["layout"]["hole"]["y"] > t["outside"]["bottom"] and t["layout"]["hole"]["w"] > 0
    # An address typed: a new tab (and its title from the app).
    assert [x[0] for x in t["typed"]["strip"]["tabs"]] == [0, 1, 2] and t["typed"]["strip"]["tabs"][2][2]
    assert t["typed"]["opened"][1]["tab"] == 2 and t["typed"]["opened"][1]["url"] == "https://bbc.co.uk/"
    # A tab chosen again, back and forward in it; the conversation's tab (the tab put away, still open); +.
    assert t["picked"]["strip"]["tabs"][1][2] and t["picked"]["strip"]["tabs"][2][1] == "BBC - Home"
    assert t["picked"]["shown"][1]["tab"] == 1 and t["picked"]["went"] == [{"tab": 1, "forward": False}, {"tab": 1, "forward": True}]
    assert t["home"]["strip"]["tabs"][0][2] and not t["home"]["strip"]["content"] and len(t["home"]["strip"]["tabs"]) == 3
    assert t["home"]["hid"] >= 1 and t["home"]["column"] > t["home"]["strip"]["bottom"] and t["home"]["strip"]["back"]
    assert t["new_tab"] == {"focused": "input", "hint": "Type a web address, or ask anything"}
    # Chat Computer's model opens a site: a tab of its own; another: the same tab (what it played, replaced).
    assert t["model_first"]["opened"][1]["tab"] == 3 and t["model_again"]["opened"][1]["tab"] == 3
    assert t["model_again"]["opened"][1]["url"].endswith("bbbbbbbbbbb") and len(t["model_again"]["strip"]["tabs"]) == 4
    # Escape: the conversation, the tab still open. A tab's x: closed, the one beside it shown. Ctrl+T: a new tab.
    assert len(t["escape"]["strip"]["tabs"]) == 4 and t["escape"]["closed"] == 0 and t["escape"]["strip"]["tabs"][0][2]
    assert t["closed"]["closed"][1] == {"tab": 2} and [x[0] for x in t["closed"]["strip"]["tabs"]] == [0, 1, 3]
    assert t["closed"]["strip"]["tabs"][2][2] and t["closed"]["shown"][1]["tab"] == 3
    assert t["ctrl_t"] == [0]
