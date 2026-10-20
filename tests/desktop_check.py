"""The Windows and Mac app (apps/desktop), built for Linux and driven through WebDriver (tauri-driver, WebKit's
WebKitWebDriver), with a pretend home folder and a small website of its own on this computer. Prints JSON.

    python tests/desktop_check.py <the app's binary> [loopback]

What it does, as the app's own page would: starts with the greeting; is told the computer's name; "/" finds files
in Documents and opens one (with a stand-in for the system's opener); refuses a file "/" didn't find, and a program;
shows a site in the middle, in the app's own view, which hears its title and where it is; a site there can't ask the
app for anything; the app's own page, sent elsewhere, comes back, and the address opens in the middle. With
"loopback", the page comes from the app's own server at 127.0.0.1 (as on a Mac), which answers only to that name.
"""
from __future__ import annotations

import http.server
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from pathlib import Path

from selenium import webdriver
from selenium.webdriver.common.options import ArgOptions

SITE_PORT = 8765
PAGES = {
    "/a.html": "<!doctype html><title>A test page</title><p>A test page</p><script>"
               "var t = window.__TAURI__ && window.__TAURI__.core;"
               "if (!t) document.title = 'A test page: no app';"
               "else t.invoke('search_files', {query: ''}).then(function () { document.title = 'A test page: the app answered'; },"
               " function () { document.title = 'A test page: the app said no'; });</script>",
    "/b.html": "<!doctype html><title>B page</title><p>B</p>",
}


class Site(http.server.BaseHTTPRequestHandler):
    def do_GET(self):  # noqa: N802
        body = PAGES.get(self.path.split("?")[0])
        self.send_response(200 if body else 404)
        self.send_header("Content-Type", "text/html")
        self.end_headers()
        self.wfile.write((body or "").encode())

    def log_message(self, *a):
        pass


def wait_port(port: int, s: float = 20) -> None:
    end = time.time() + s
    while time.time() < end:
        try:
            socket.create_connection(("127.0.0.1", port), 0.5).close()
            return
        except OSError:
            time.sleep(0.2)
    raise RuntimeError(f"nothing on port {port}")


def main() -> None:
    binary, loopback = os.path.abspath(sys.argv[1]), "loopback" in sys.argv[2:]
    out: dict = {"loopback": loopback}
    work = Path(tempfile.mkdtemp(prefix="cc-desktop-"))
    home = work / "home"
    for f in ["Documents/Taxes/Taxes 2025.pdf", "Documents/Resume.docx", "Downloads/setup.exe", "Pictures/Beach.jpg"]:
        (home / f).parent.mkdir(parents=True, exist_ok=True)
        (home / f).write_bytes(b"x")
    (home / ".config").mkdir()
    (home / ".config" / "user-dirs.dirs").write_text("".join(f'XDG_{k}_DIR="$HOME/{v}"\n' for k, v in [
        ("DESKTOP", "Desktop"), ("DOCUMENTS", "Documents"), ("DOWNLOAD", "Downloads"), ("PICTURES", "Pictures"),
        ("MUSIC", "Music"), ("VIDEOS", "Videos")]))
    # The system's opener, standing in: what it was asked to open.
    bin_dir = work / "bin"
    bin_dir.mkdir()
    opener = bin_dir / "xdg-open"
    opener.write_text(f"#!/bin/sh\necho \"$1\" >> {work / 'opened.txt'}\n")
    opener.chmod(0o755)
    env = {**os.environ, "HOME": str(home), "XDG_CONFIG_HOME": str(home / ".config"), "PATH": f"{bin_dir}:{os.environ['PATH']}",
           "XDG_DATA_HOME": str(home / ".local/share"), "XDG_CACHE_HOME": str(home / ".cache")}
    for k in ("http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"):
        env.pop(k, None)
    env["no_proxy"] = env["NO_PROXY"] = "127.0.0.1,localhost"
    if loopback:
        env["CHATCOMPUTER_LOOPBACK"] = "1"
    xvfb = None
    if not os.environ.get("DISPLAY"):
        xvfb = subprocess.Popen(["Xvfb", ":97", "-screen", "0", "1440x900x24", "-nolisten", "tcp"], stderr=subprocess.DEVNULL)
        env["DISPLAY"] = ":97"
        time.sleep(1)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", SITE_PORT), Site)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    driver_bin = shutil.which("tauri-driver") or os.path.expanduser("~/.cargo/bin/tauri-driver")
    td = subprocess.Popen([driver_bin, "--port", "4545", "--native-port", "4546"], env=env,
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    drv = None
    try:
        wait_port(4545)
        opts = ArgOptions()
        opts.set_capability("tauri:options", {"application": binary})
        opts.set_capability("browserName", "wry")
        drv = webdriver.Remote("http://127.0.0.1:4545", options=opts)
        drv.set_script_timeout(20)
        js = drv.execute_script
        js_async = drv.execute_async_script

        def until(cond: str, s: float = 15):
            end = time.time() + s
            while time.time() < end:
                v = js("return (" + cond + ")")
                if v:
                    return v
                time.sleep(0.2)
            raise RuntimeError("never: " + cond)

        def invoke(cmd: str, args: dict | None = None):
            return js_async("var done = arguments[arguments.length - 1]; window.__TAURI__.core.invoke(arguments[0], arguments[1])"
                            ".then(function (v) { done({ ok: v }); }, function (e) { done({ no: String(e) }); });", cmd, args or {})

        def record():
            js("window.__seen = window.__seen || []; if (!window.__recording) { window.__recording = true;"
               " var was = window.chatcomputerApp; window.chatcomputerApp = function (m) { window.__seen.push(m); was(m); }; }")

        # The app's own page, from the app: the greeting.
        until("window.CO && document.querySelector('#log > .ai.hello') && document.querySelector('#log > .ai.hello')._said")
        out["page"] = js("return location.origin")
        out["secure"] = js("return window.isSecureContext")          # (WebGPU, the microphone and the cache need it)
        out["greeting"] = js("return document.querySelector('#log > .ai.hello')._said.split('\\n')[0]")
        out["about"] = invoke("about")
        # "/": the files in the pretend home folder; one opened; what wasn't found, and a program, refused.
        found = invoke("search_files", {"query": "taxes"})
        out["found"] = [(h["name"], h["where"], h["kind"]) for h in found.get("ok", [])] if "ok" in found else found
        pdf = next((h["path"] for h in found.get("ok", []) if h["name"].endswith(".pdf")), "")
        out["open_found"] = invoke("open_file", {"path": pdf})
        time.sleep(1)
        out["opened"] = (work / "opened.txt").read_text().splitlines() if (work / "opened.txt").exists() else []
        out["open_unfound"] = invoke("open_file", {"path": str(home / "Documents" / "Resume.docx")})
        exe = invoke("search_files", {"query": "setup"}).get("ok", [])
        out["open_program"] = invoke("open_file", {"path": exe[0]["path"]}) if exe else "not found"
        out["slash"] = None
        js("var i = document.getElementById('input'); i.focus(); i.value = '/beach'; i.dispatchEvent(new Event('input', {bubbles: true}));")
        try:
            out["slash"] = until("Array.from(document.querySelectorAll('#results .res .res-name')).map(function (e) { return e.textContent; })"
                                 ".filter(function (t) { return /Beach/.test(t); })[0]")
        finally:
            js("var i = document.getElementById('input'); i.value = ''; i.dispatchEvent(new Event('input', {bubbles: true}));")
        # A site in the middle, in the app's own view: its title and where it is come back; it can't ask the app.
        record()
        js("window.chatcomputerApp({ open: 'http://127.0.0.1:%d/a.html' })" % SITE_PORT)
        out["site_title"] = until("(window.__seen || []).map(function (m) { return m.title; }).filter(function (t) { return /: /.test(t || ''); })[0]")
        out["site_url"] = until("(window.__seen || []).map(function (m) { return m.url; }).filter(Boolean)[0]")
        out["content"] = js("return [document.body.classList.contains('content'), document.getElementById('frame-title').textContent]")
        if os.environ.get("CHATCOMPUTER_SHOT"):                  # the screen, with the site in its frame
            time.sleep(1)
            subprocess.run(["import", "-display", env["DISPLAY"], "-window", "root", os.environ["CHATCOMPUTER_SHOT"]], check=False)
        # The app's own page sent elsewhere: back, and the address in the middle.
        js("location.href = 'http://127.0.0.1:%d/b.html'" % SITE_PORT)
        time.sleep(4)
        until("window.CO && document.getElementById('input')", 20)
        out["back_home"] = js("return location.origin")
        out["sent_there"] = until("document.body.classList.contains('content') && document.getElementById('frame-title').textContent", 15)
        # Closed: the page has the middle again.
        js("document.getElementById('frame-close').click()")
        out["closed"] = until("!document.body.classList.contains('content') && 'yes'", 10)
        if loopback:
            port = int(out["page"].rsplit(":", 1)[1])
            req = urllib.request.Request(f"http://127.0.0.1:{port}/index.html", headers={"Host": "evil.example"})
            try:
                out["other_name"] = urllib.request.urlopen(req, timeout=5).status
            except urllib.error.HTTPError as e:
                out["other_name"] = e.code
            out["own_name"] = urllib.request.urlopen(f"http://127.0.0.1:{port}/index.html", timeout=5).status
    except Exception as e:  # noqa: BLE001
        out["failed"] = repr(e)[:1500]
    finally:
        if drv:
            try:
                drv.quit()
            except Exception:  # noqa: BLE001
                pass
        td.terminate()
        server.shutdown()
        if xvfb:
            xvfb.terminate()
        shutil.rmtree(work, ignore_errors=True)
    print(json.dumps(out))


if __name__ == "__main__":
    main()
