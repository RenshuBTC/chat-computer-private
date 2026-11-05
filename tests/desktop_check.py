"""The Windows and Mac app (apps/desktop), built for Linux and driven through WebDriver (tauri-driver, WebKit's
WebKitWebDriver), with a pretend home folder and a small website of its own on this computer. Prints JSON.

    python tests/desktop_check.py <the app's binary> [loopback]

The model: the engine that comes with the app (beside its program) serves the model that's here (a tiny one with
random weights, tests/data/tiny-model.ccm, in a folder of models and a catalogue of this check's own: a build for
testing takes CHATCOMPUTER_MODELS_DIR and CHATCOMPUTER_ENGINE_CATALOGUE), the page hears it's ready, "/" names it,
and something asked is answered by it; when the app closes, the engine ends with it.

How you want Chat Computer: the app starts on the page that asks (no way chosen yet), the two ways there (the app,
or its own system); the own system can't be chosen on Linux, and says why; the app chosen, the conversation. At the
end, "/" lists how you have Chat Computer, and its page shows the way chosen.

What it does, as the app's own page would: starts with the greeting; is told the computer's name; "/" finds files
in Documents and opens one (with a stand-in for the system's opener); refuses a file "/" didn't find, and a program;
shows a site in the middle, in the app's own view, which hears its title and where it is; a site there can't ask the
app for anything; the app's own page, sent elsewhere, comes back, and the address opens in the middle. With
"loopback", the page comes from the app's own server at 127.0.0.1 (as on a Mac), which answers only to that name.

Messages: Gmail, WhatsApp and Outlook connected from Messages' + (pages of this check's own, built as theirs are:
a build for testing points the services at them, CHATCOMPUTER_SERVICES_AT), each opened in the middle to sign in,
then put aside; what their reader reads comes into Messages; tapping one opens its service at that conversation; a
site in the middle can't hand over messages; a service disconnected goes from Messages.
"""
from __future__ import annotations

import hashlib
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

SITE_PORT = 8790
TINY = Path(__file__).resolve().parent / "data" / "tiny-model.ccm"
PAGES = {
    "/a.html": "<!doctype html><title>A test page</title><p>A test page</p><script>"
               "var t = window.__TAURI__ && window.__TAURI__.core;"
               "if (!t) document.title = 'A test page: no app';"
               "else t.invoke('search_files', {query: ''}).then(function () { document.title = 'A test page: the app answered'; },"
               " function () { document.title = 'A test page: the app said no'; });</script>",
    "/b.html": "<!doctype html><title>B page</title><p>B</p>",
    "/d.html": "<!doctype html><title>D page</title><p>D</p>",
    # A site that tries to hand over messages, as only a connected service's own pages may.
    "/c.html": "<!doctype html><title>C page</title><script>"
               "window.__TAURI__.core.invoke('messages_found', {items: [{id: 'x', who: 'Mallory', text: 'hi'}]})"
               ".then(function () { document.title = 'C page: the app took them'; },"
               " function () { document.title = 'C page: the app said no'; });</script>",
    # The services, built as theirs are (their lists' parts named as on their pages); a tap on one shows in the title.
    "/gmail.html": "<!doctype html><title>Inbox (2) - me@example.com - Gmail</title><table><tbody>"
                   "<tr class='zA zE'><td class='yX'><div class='yW'><span email='jane@example.com' name='Jane Lim'>Jane Lim</span>"
                   "</div></td><td><span class='bog'><span data-thread-id='#thread-f:1'>Lunch on Friday?</span></span>"
                   "<span class='y2'> - 12:30 at the usual place works for me.</span></td>"
                   "<td class='xW'><span title=''>9:05 AM</span></td></tr>"
                   "<tr class='zA yO'><td class='yX'><div class='yW'><span email='lib@example.org' name='City Library'>City Library"
                   "</span></div></td><td><span class='bog'><span data-thread-id='#thread-f:2'>Books due</span></span>"
                   "<span class='y2'> - Your books are due back on Thursday.</span></td><td class='xW'><span>Yesterday</span></td></tr>"
                   "</tbody></table><script>document.addEventListener('click', function (e) {"
                   " var r = e.target.closest('tr.zA'); if (r) document.title = 'opened: ' + r.querySelector('[name]').getAttribute('name'); });"
                   "</script>",
    "/whatsapp.html": "<!doctype html><title>(1) WhatsApp</title><div id='pane-side'><div aria-label='Chat list' role='grid'>"
                      "<div role='listitem'><div role='gridcell'><span dir='auto' title='Mum'>Mum</span><div>10:15</div>"
                      "<span title='Dinner at 7 tonight?'>Dinner at 7 tonight?</span><span aria-label='1 unread message'>1</span>"
                      "</div></div><div role='listitem'><div role='gridcell'><span dir='auto' title='Alex'>Alex</span><div>Mon</div>"
                      "<span title='Sent you the slides'>Sent you the slides</span></div></div></div></div>"
                      "<script>document.addEventListener('mousedown', function (e) {"
                      " var r = e.target.closest('[role=listitem]'); if (r) document.title = 'opened: ' + r.querySelector('span[title]').title; });"
                      "</script>",
    "/outlook.html": "<!doctype html><title>Mail - Outlook</title><div role='listbox'>"
                     "<div role='option' data-convid='c1' aria-label='Unread, Sam Lee, Trip photos'>"
                     "<div>Sam Lee</div><div>Trip photos</div><div>8:30 AM</div><div>Here they are, all 40.</div></div></div>",
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


def _exe(pid: str) -> str:
    try:
        return os.path.realpath(f"/proc/{pid}/exe")
    except OSError:
        return ""


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
    env["CHATCOMPUTER_SERVICES_AT"] = f"http://127.0.0.1:{SITE_PORT}/"      # (the services: this check's own pages)
    # The models: a tiny one, here already (nothing to download), in a catalogue of its own.
    models = work / "models"
    models.mkdir()
    shutil.copy2(TINY, models / "tiny.ccm")
    (work / "models.json").write_text(json.dumps({"models": [{
        "name": "tiny", "label": "Tiny", "repo": "Qwen/Tiny", "revision": "0" * 40, "memory_gb": 0, "bits": 8,
        "files": [{"name": "model.safetensors", "size": 1, "sha256": hashlib.sha256(b"x").hexdigest()}]}]}))
    env["CHATCOMPUTER_MODELS_DIR"] = str(models)
    env["CHATCOMPUTER_ENGINE_CATALOGUE"] = str(work / "models.json")
    engine = os.path.realpath(os.path.join(os.path.dirname(binary), "chat-computer-model"))
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

        # How you want Chat Computer: asked first (nothing chosen yet), the two ways.
        until("document.querySelector('#choose') && !document.getElementById('choose').hidden")
        out["first_page"] = js("return location.pathname")
        out["ways"] = js("return Array.from(document.querySelectorAll('.way')).map(function (b) { return [b.getAttribute('data-way'),"
                         " b.classList.contains('cannot')]; })")
        out["way_status"] = invoke("way_status").get("ok")
        # The own system: not from Linux, and it says why (nothing kept).
        js("document.querySelector('.way[data-way=\"os\"]').click()")
        out["os"] = until("document.getElementById('error').textContent")
        out["os_choose"] = invoke("choose_way", {"way": "os"})
        # The app: the conversation.
        js("document.querySelector('.way[data-way=\"app\"]').click()")
        until("location.pathname === '/index.html' && window.CO && document.getElementById('input')", 20)
        out["about_way"] = invoke("about").get("ok", {}).get("way")
        # The app's own page, from the app: the greeting.
        until("window.CO && document.getElementById('greeting').classList.contains('shown')")
        out["page"] = js("return location.origin")
        out["secure"] = js("return window.isSecureContext")          # (WebGPU, the microphone and the cache need it)
        out["greeting"] = js("return document.getElementById('greeting').textContent")
        out["about"] = invoke("about")
        # The model: served by the engine beside the app (the one here), the page told; "/" names it.
        end = time.time() + 90
        while time.time() < end:
            out["model"] = invoke("model_status").get("ok") or {}
            if out["model"].get("state") in ("ready", "failed", "none"):
                break
            time.sleep(0.5)
        until("document.getElementById('input').placeholder === 'Type anything'", 10)
        # (Searched by its name: "/" alone would list the files too, and a file listed may be opened.)
        js("var i = document.getElementById('input'); i.focus(); i.value = '/tiny'; i.dispatchEvent(new Event('input', {bubbles: true}));")
        try:
            out["model_slash"] = until("(document.querySelector('#results .res .res-name') || {}).textContent")
        finally:
            js("var i = document.getElementById('input'); i.value = ''; i.dispatchEvent(new Event('input', {bubbles: true}));")
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
        # Messages: Gmail, WhatsApp and Outlook connected from + in Messages (one panel of everything that can be
        # connected), each in the middle to sign in (here, already in), then out of sight, read; a site in the middle
        # can't hand over messages.
        def kind(group=None):
            js("document.getElementById('add-account').click()")
        def connect(name):
            kind()
            until("document.body.classList.contains('sheet') && document.querySelector('#sheet-fields .choices button')")
            js("var want = arguments[0]; Array.from(document.querySelectorAll('#sheet-fields .choices button')).filter(function (b) {"
               " return b.textContent === want; })[0].click()", name)
            until("document.body.classList.contains('content') && document.getElementById('frame-title').textContent === arguments[0]"
                  .replace("arguments[0]", json.dumps(name)), 15)
        msgs = "Array.from(document.querySelectorAll('#messages-list li.msg')).map(function (li) { return [li.querySelector('.msg-who')" \
               ".textContent, li.querySelector('.msg-text').textContent, li.classList.contains('unread')]; })"
        connect("Gmail")
        out["choices"] = None
        js("document.getElementById('frame-close').click()")
        kind("email")
        out["choices"] = until("document.body.classList.contains('sheet') && Array.from(document.querySelectorAll('#sheet-fields .choices "
                               "button')).map(function (b) { return b.textContent; })")
        js("document.getElementById('sheet-cancel').click()")
        connect("WhatsApp")
        js("document.getElementById('frame-close').click()")
        connect("Outlook")
        js("document.getElementById('frame-close').click()")
        out["messages"] = until(msgs + ".length >= 5 && " + msgs, 30)
        out["accounts"] = js("return JSON.parse(localStorage.getItem('chatcomputer-app-services'))")
        if os.environ.get("CHATCOMPUTER_SHOT_MESSAGES"):         # the screen, with Messages from the three
            time.sleep(1)
            subprocess.run(["import", "-display", env["DISPLAY"], "-window", "root", os.environ["CHATCOMPUTER_SHOT_MESSAGES"]], check=False)
        # Tapping one: its service in the middle, at that conversation.
        record()
        js("Array.from(document.querySelectorAll('#messages-list li.msg')).filter(function (li) {"
           " return li.querySelector('.msg-who').textContent === 'Mum'; })[0].click()")
        out["tapped"] = until("document.body.classList.contains('content') && /opened/.test(document.getElementById('frame-title')"
                              ".textContent) && document.getElementById('frame-title').textContent", 15)
        js("document.getElementById('frame-close').click()")
        js("window.chatcomputerApp({ open: 'http://127.0.0.1:%d/c.html' })" % SITE_PORT)
        out["c_title"] = until("(window.__seen || []).map(function (m) { return m.title; }).filter(function (t) { return /^C page: /.test(t || ''); })[0]")
        js("document.getElementById('frame-close').click()")
        # Disconnected: Outlook's go from Messages.
        kind("email")
        until("document.querySelector('#sheet-fields .choices button')")
        js("Array.from(document.querySelectorAll('#sheet-fields .choices button')).filter(function (b) {"
           " return b.textContent === 'Disconnect Outlook'; })[0].click()")
        out["after_disconnect"] = until("!" + msgs + ".some(function (m) { return m[0] === 'Sam Lee'; }) && " + msgs, 10)
        # Asked something, the model answers (last: its weights are random, and so may be what it does).
        js("document.getElementById('frame-close').click()")
        said = js("return document.querySelectorAll('#log > .ai').length")
        js("var i = document.getElementById('input'); i.value = 'hi'; i.dispatchEvent(new Event('input', {bubbles: true}));"
           "document.getElementById('send').click();")
        until("document.querySelectorAll('#log > .ai').length > %d && !document.querySelector('#log > .ai.thinking')" % said, 30)
        last, same, end = None, 0, time.time() + 60          # (its words stream in: until they stop coming)
        while time.time() < end and same < 8:
            now = js("return Array.from(document.querySelectorAll('#log > .ai')).pop().textContent")
            same, last = (same + 1 if now == last else 0), now
            time.sleep(0.25)
        out["model_answer"] = last
        # How you have Chat Computer: "/" lists it, and its page shows the way chosen.
        js("var i = document.getElementById('input'); i.focus(); i.value = '/how you have'; i.dispatchEvent(new Event('input', {bubbles: true}));")
        out["way_slash"] = until("(document.querySelector('#results .res .res-name') || {}).textContent")
        js("document.querySelector('#results .res').click()")
        until("location.pathname === '/start/index.html' && !document.getElementById('choose').hidden && document.querySelector('.way.now')", 20)
        out["way_now"] = js("return document.querySelector('.way.now').getAttribute('data-way')")
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
        # The engine ends with the app.
        end = time.time() + 10
        while time.time() < end:
            out["engine_left"] = sum(1 for d in os.listdir("/proc") if d.isdigit() and _exe(d) == engine)
            if not out["engine_left"]:
                break
            time.sleep(0.25)
        server.shutdown()
        if xvfb:
            xvfb.terminate()
        shutil.rmtree(work, ignore_errors=True)
    print(json.dumps(out))


if __name__ == "__main__":
    main()
