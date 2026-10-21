"""What chatcomputer.ai/get offers (docs/get): the page, as Annotated's is (the name, what to get it with, what it
is; no video), and the apps it links to, made by scripts/build_apps.py from apps/. The Chrome extension is tried in
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

import build_apps  # noqa: E402

GET = ROOT / "docs" / "get"


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
        if not href.startswith(("../", "http")):
            assert (GET / href).is_file(), (name, href)
    assert links == {"chrome": "Chat-Computer-Chrome.zip", "windows": "Chat-Computer-Windows.exe", "mac": "Chat-Computer-Mac.dmg",
                     "android": "Chat-Computer-Android.apk", "web": "../"}
    assert 'title="Downloads the extension. Unzip it, open chrome://extensions, turn on Developer mode, and choose Load unpacked."' in page
    # The apps aren't signed with paid certificates yet: each button says what's asked the first time.
    assert "choose More info, then Run anyway" in page and "Privacy &amp; Security, and choose Open Anyway" in page
    assert "allow installing from your browser" in page
    soon = re.findall(r'<span class="add soon" id="(\w+)" aria-disabled="true"', page)
    assert soon == ["iphone"] and 'title="Coming to the App Store">iPhone<' in page
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
    assert o == {"url": "https://chatcomputer.ai/?app=chrome", "rem": "16px", "hint": "Type anything or “/”",
                 "greeting": "hello, world"}
    # A site's address typed: a real tab with it, at once, without a click.
    tabs = res["typed"]["tabs"]
    assert len(tabs) == 1 and re.match(r"https://(www\.)?bbc\.co\.uk/", tabs[0]) and res["typed"]["said"] == "Opened bbc.co.uk in a new tab."
    # Another page can't make it open tabs.
    assert res["forged"] is False
