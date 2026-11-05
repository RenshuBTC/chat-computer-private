"""What chatcomputer.ai/get offers (docs/get): the page, as Annotated's is (the name, what to get it with, what it
is; no video), and the apps it links to, made by scripts/build_apps.py from apps/.

On Windows and Mac, one Setup each (the Chat Computer app, apps/desktop, and for the Mac a line of Terminal that
installs it): the app's first page asks how you want Chat Computer, as an app or as its own system, and the app does
it (src-tauri/src/ways.rs)."""
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
    assert links == {"windows": "Chat-Computer-Windows.exe", "mac": "#mac-setup", "web": "../"}
    # The apps aren't signed with paid certificates yet: each button says what's asked the first time.
    assert "choose More info, then Run anyway" in page and "Privacy &amp; Security, and choose Open Anyway" in page
    assert "Android" not in page and "iPhone" not in page and "chrome://extensions" not in page
    # Windows and Mac: one Setup each, which asks how you want Chat Computer; the Mac's a line for Terminal (shown when
    # Mac is chosen), with the disk image as the other way. (The Mac's disk image here is still the app from before
    # its first page asked: the page says so, until GitHub's Macs build the new one.)
    title = re.search(r'id="windows" href="[^"]+" download\s+title="([^"]+)"', page).group(1)
    assert "as an app, or as its own system next to Windows" in title
    assert "<code>curl -fsSL https://chatcomputer.ai/get/mac.sh | bash</code>" in page and "#mac-setup:target { display: block; }" in page
    assert '<a id="mac-dmg" href="Chat-Computer-Mac.dmg" download>' in page and "M1, M2 or M3" in page
    mac_title = re.search(r'id="mac" href="#mac-setup"\s+title="([^"]+)"', page).group(1)
    assert "asks" not in mac_title and '<p id="mac-next">Its next version asks how you want Chat Computer' in page
    # What each download is: Windows' installer (NSIS), a Mac disk image.
    assert (GET / "Chat-Computer-Windows.exe").read_bytes()[:2] == b"MZ" and b"Nullsoft" in (GET / "Chat-Computer-Windows.exe").read_bytes()
    assert (GET / "Chat-Computer-Mac.dmg").read_bytes()[-512:][:4] == b"koly"


def _node_with_playwright():
    node = shutil.which("node")
    if not node:
        return None, None
    root = subprocess.run(["npm", "root", "-g"], capture_output=True, text=True).stdout.strip() if shutil.which("npm") else ""
    if not root or not (Path(root) / "playwright").exists():
        return None, None
    return node, root


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
    """Its first page (apps/desktop/start, put beside the website's page as start/): the two ways, in Chat Computer's
    own look (theme.css); it asks the app how things are (way_status) and to do the way chosen (choose_way)."""
    page = (DESKTOP / "start" / "index.html").read_text()
    js = (DESKTOP / "start" / "start.js").read_text()
    css = (DESKTOP / "start" / "start.css").read_text()
    assert re.findall(r'data-way="(\w+)"', page) == ["app", "os"]
    assert [n.strip() for n in re.findall(r'<span class="way-name">([^<]+)</span>', page)] == ["App", "Its own system"]
    assert "<h2 id=\"ask\">How do you want Chat Computer?</h2>" in page and "You can change this later" in page
    assert '<link rel="stylesheet" href="../demo/theme.css">' in page and re.findall(r"<script[^>]*>", page) == ['<script src="start.js">']
    assert "<img" not in page and 'rel="icon"' not in page
    # The own system's steps, short: Windows' permission, Enter, a restart.
    assert "choose <b>Yes</b>" in page and "Press <b>Enter</b>" in page and "Restart the PC" in page
    assert "Chrome" not in page and "browser" not in page.lower()
    for cmd in ("way_status", "choose_way", "show_folder"):
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
    """src-tauri/src/ways.rs: the app, at once; the own system: Chat Computer Setup, which comes with the app, run from a
    folder of its own (on a PC, Chat Computer's own system's .cmd; on a Mac, setup.sh in Terminal)."""
    import build_apps
    ways = (TAURI / "src" / "ways.rs").read_text()
    lib = (TAURI / "src" / "lib.rs").read_text()
    assert "pub enum Way {\n    App,\n    Os,\n}" in ways
    for gone in ("Chrome", "register_browser", "open_tab", "take_opening", "make_default_browser", "single_instance"):
        assert gone not in ways and gone not in lib, gone
    conf = json.loads((TAURI / "tauri.conf.json").read_text())
    assert conf["bundle"]["windows"]["nsis"]["installerHooks"] == "nsis/hooks.nsh"
    assert "as an app,$\\r$\\n    or as its own system, next to Windows." in (TAURI / "nsis" / "hooks.nsh").read_text()
    import plistlib
    assert "CFBundleURLTypes" not in plistlib.loads((TAURI / "Info.plist").read_bytes())
    # The own system: Chat Computer Setup (the repository's files it uses, as git has them), with the app (Tauri's
    # resources), run from a folder of its own.
    assert conf["bundle"]["resources"] == {"setup/": "setup/"}
    win, mac = build_apps.setup_files("windows"), build_apps.setup_files("mac")
    for f in ("windows/Chat Computer Own System.cmd", "windows/own-os.ps1", "windows/lib/OwnOs.ps1", "os/release/chatcomputer.efi",
              "README.md"):
        assert f in win, f
    for f in ("mac/setup.sh", "mac/manifest.sh", "Chat Computer Setup.command", "linux/install-chatcomputer.sh", "README.md"):
        assert f in mac, f
    for skip in ("docs/", "apps/", "evals/", ".github/"):
        assert not any(f.startswith(skip) for f in win + mac), skip
    assert not any(f.startswith("mac/") for f in win) and not any(f.startswith("windows/") for f in mac)
    assert 'dir.join("windows").join("Chat Computer Own System.cmd")' in ways and "exec bash mac/setup.sh" in ways
    assert '.args(["-a", "Terminal"])' in ways and 'sysctl("machdep.cpu.brand_string")' in ways
    assert "apps/desktop/src-tauri/setup/" in (ROOT / ".gitignore").read_text()


