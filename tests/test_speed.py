"""Things that make Chat Computer quicker to start and to answer, kept from coming back slow."""
import asyncio
from types import SimpleNamespace
import subprocess
import sys
import time
from pathlib import Path

from chatcomputer.config import Paths, Settings
from chatcomputer.controller import Controller
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen

REPO = Path(__file__).resolve().parents[1]


def _controller(tmp_path, monkeypatch, platform=None):
    monkeypatch.setenv("HOME", str(tmp_path / "home"))
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "home" / "ChatComputer"))
    paths = Paths().ensure()
    return Controller(paths, Settings(user_name="Sam Lee"), platform or FakePlatform(), RecordingScreen(),
                      voice=False, index=False)


class SlowInternetCheck(FakePlatform):
    def __init__(self, online=True):
        super().__init__()
        self.online = online
        self.checks = 0

    def internet_ok(self):
        self.checks += 1
        time.sleep(1.0)            # a connection to Claude's servers on a slow network
        return self.online


def test_a_request_doesnt_wait_for_the_internet(tmp_path, monkeypatch):
    # The model is on the computer: a question goes to it without any check of the internet, online or not.
    c = _controller(tmp_path, monkeypatch, SlowInternetCheck(online=False))
    asked = []

    async def ask(text, typed=False):
        asked.append(text)
        return "t1"
    c.brain = SimpleNamespace(busy=False, last_activity=0.0, ask=ask, current_origin="here")
    c.model = SimpleNamespace(ready=True)

    async def run():
        c.loop = asyncio.get_running_loop()
        t0 = time.monotonic()
        await c.handle("what's 2 and 2", typed=True)
        return time.monotonic() - t0
    assert asyncio.run(run()) < 0.5 and len(asked) == 1 and c.platform.checks == 0


def test_the_window_doesnt_wait_for_the_brain():
    # The window module needs is_private from the permission gate: loading the gate doesn't load the brain.
    code = "import sys, chatcomputer.brain.permissions; print('chatcomputer.brain.agent' in sys.modules)"
    r = subprocess.run([sys.executable, "-c", code], cwd=REPO, capture_output=True, text=True, timeout=60)
    assert r.stdout.strip() == "False", r.stderr


def test_the_gate_answers_with_its_own_results(tmp_path):
    from chatcomputer.brain.permissions import PermissionGate, PermissionResultAllow, PermissionResultDeny
    from chatcomputer.journal import Journal
    paths = Paths(tmp_path / "ChatComputer").ensure()

    async def confirm(q):
        return ("no", "no")
    gate = PermissionGate(paths, Journal(paths), confirm, {})
    ok = asyncio.run(gate("WebSearch", {"query": "weather"}))
    no = asyncio.run(gate("Read", {"file_path": str(paths.secrets / "accounts.json")}))
    assert isinstance(ok, PermissionResultAllow) and isinstance(no, PermissionResultDeny)
    assert ok.behavior == "allow" and ok.updated_input == {"query": "weather"} and no.behavior == "deny"


def test_voice_models_load_side_by_side(tmp_path, monkeypatch):
    import chatcomputer.voice.stt as stt_mod
    import chatcomputer.voice.tts as tts_mod
    from chatcomputer.voice import models as M
    from chatcomputer.voice.audio import FakePlayer
    c = _controller(tmp_path, monkeypatch)
    c.player = FakePlayer()
    w = M.whisper_dir(c.paths, c.settings.stt_model)
    w.mkdir(parents=True, exist_ok=True)
    (w / "model.bin").write_bytes(b"x")

    class Slow:
        def __init__(self, *a, **k):
            time.sleep(0.8)
            self.kokoro = None

        def transcribe(self, audio, fast=False):
            return "", {}

        def synth(self, text):
            import numpy as np
            return np.zeros(0, dtype=np.float32)
    monkeypatch.setattr(stt_mod, "STT", Slow)
    monkeypatch.setattr(tts_mod, "TTS", Slow)
    c.mic_factory = lambda cb: type("Mic", (), {"open": lambda self: True, "is_open": True, "close": lambda self: None})()

    async def run():
        c.loop = asyncio.get_running_loop()
        t0 = time.monotonic()
        await c._start_voice()
        return time.monotonic() - t0
    took = asyncio.run(run())
    assert c.listener is not None and took < 1.4         # two 0.8 s loads, side by side


# ------------------------------------------------------------------ the other system's drive, after the start
# The computer mounts Windows' drive alongside Chat Computer's start (it no longer holds the whole start back for the
# disks), so Chat Computer can come up first and must take the drive in when it arrives.
class JustStarted(FakePlatform):
    def just_started(self):
        return True


def _pc_controller(tmp_path, monkeypatch, mounted):
    import os
    mnt = tmp_path / "mnt"          # the empty mount point; Windows' folders appear once the drive is mounted
    mnt.mkdir()
    real = os.path.ismount
    monkeypatch.setattr(os.path, "ismount", lambda p: mounted["now"] if str(p) == str(mnt) else real(p))
    monkeypatch.setenv("HOME", str(tmp_path / "home"))
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "home" / "ChatComputer"))
    paths = Paths().ensure()
    s = Settings(user_name="Sam Lee", windows_user="sammy", windows_mount=str(mnt))
    return Controller(paths, s, JustStarted(), RecordingScreen(), voice=False, index=True), mnt


def test_windows_files_that_arrive_after_the_start_are_taken_in(tmp_path, monkeypatch):
    mounted = {"now": False}
    c, mnt = _pc_controller(tmp_path, monkeypatch, mounted)

    async def run():
        await c.start()
        assert c._other_files_pending() and not c._files_settled.is_set()
        assert [label for label, _ in c.index.roots] == ["home"]
        (mnt / "Users" / "sammy" / "Documents").mkdir(parents=True)
        mounted["now"] = True
        await asyncio.wait_for(c._files_settled.wait(), 3)
        await asyncio.sleep(0.2)
        c.brain = None
        await c.stop()
    asyncio.run(run())
    home = str(mnt / "Users" / "sammy")
    assert ("windows", home) in c.index.roots
    assert c._other_os_files() == home         # and the next conversation's system prompt says where they are


def test_the_engine_waits_a_moment_for_windows_files_at_the_start(tmp_path, monkeypatch):
    import types
    mounted = {"now": False}
    c, mnt = _pc_controller(tmp_path, monkeypatch, mounted)
    connected = []

    async def connect():
        connected.append((time.monotonic(), c._other_os_files()))

    async def run():
        c.loop = asyncio.get_running_loop()
        c._files_settled = asyncio.Event()
        c.brain = types.SimpleNamespace(connect=connect, add_dirs=[], client=None)
        task = asyncio.create_task(c._warm_up(wait_for_files=True))
        await asyncio.sleep(0.3)
        assert not connected
        (mnt / "Users" / "sammy").mkdir(parents=True)
        mounted["now"] = True
        c._files_settled.set()
        await task
    asyncio.run(run())
    assert connected and connected[0][1] == str(mnt / "Users" / "sammy") and c._engine_had_files


# ------------------------------------------------------------------ the file index
def _files_db(path):
    import sqlite3
    db = sqlite3.connect(str(path))
    try:
        return {r[0]: r[1:] for r in db.execute("SELECT path, seen, mtime FROM files")}
    finally:
        db.close()


def test_a_crawl_where_nothing_changed_writes_nothing(tmp_path):
    from chatcomputer.files_index import FileIndex
    root = tmp_path / "home"
    (root / "Documents").mkdir(parents=True)
    for i in range(50):
        (root / "Documents" / f"note{i}.txt").write_text(f"note {i}")
    idx = FileIndex(tmp_path / "files.db", [("home", str(root))])
    assert idx.crawl_once() == 50
    before = _files_db(tmp_path / "files.db")
    (root / "Documents" / "note7.txt").unlink()
    (root / "Documents" / "new.txt").write_text("new")
    assert idx.crawl_once() == 50
    after = _files_db(tmp_path / "files.db")
    # Unchanged files keep their rows as they were; the deleted one is gone, the new one is there.
    assert all(after[p] == before[p] for p in after if p in before)
    assert str(root / "Documents" / "note7.txt") not in after and str(root / "Documents" / "new.txt") in after
    assert [h["path"] for h in idx.search("note7")] == []


def test_a_folder_added_later_is_searched_at_once(tmp_path):
    from chatcomputer.files_index import FileIndex
    home, win = tmp_path / "home", tmp_path / "win"
    home.mkdir()
    (win / "Documents").mkdir(parents=True)
    (win / "Documents" / "Budget_2026.xlsx").write_bytes(b"x")
    idx = FileIndex(tmp_path / "files.db", [("home", str(home))])
    idx.start(interval_s=3600, first_delay_s=3600)        # nothing would happen for an hour...
    try:
        idx.add_root("windows", str(win))                  # ...but a new folder is crawled now
        deadline = time.time() + 10
        while time.time() < deadline and not idx.search("budget"):
            time.sleep(0.05)
        assert [Path(h["path"]).name for h in idx.search("budget")] == ["Budget_2026.xlsx"]
    finally:
        idx.stop()


# ------------------------------------------------------------------ while Chat Computer talks
class CountingHeadset(FakePlatform):
    def __init__(self):
        super().__init__()
        self.checks = 0

    def output_is_headset(self):
        self.checks += 1
        time.sleep(0.05)            # wpctl: about a quarter of a 32 ms frame, and more on a busy computer
        return True


def test_the_headphones_check_doesnt_run_on_every_frame(tmp_path, monkeypatch):
    # The listener asks on every 32 ms frame while Chat Computer talks. It used to run wpctl each time, on the
    # thread that also hears and shows everything.
    c = _controller(tmp_path, monkeypatch, CountingHeadset())

    async def run():
        c.loop = asyncio.get_running_loop()
        t0 = time.monotonic()
        first = [c._barge_in_allowed() for _ in range(100)]      # three seconds of frames
        took = time.monotonic() - t0
        await asyncio.sleep(0.2)                                  # the check, in the background
        return first, took, c._barge_in_allowed()
    first, took, later = asyncio.run(run())
    assert took < 0.02 and c.platform.checks == 1
    assert first == [False] * 100 and later is True         # loudspeakers until known; then headphones


def test_barge_in_on_and_off_dont_check_at_all(tmp_path, monkeypatch):
    c = _controller(tmp_path, monkeypatch, CountingHeadset())
    c.settings.barge_in = "on"
    assert c._barge_in_allowed() is True
    c.settings.barge_in = "off"
    assert c._barge_in_allowed() is False and c.platform.checks == 0


# ------------------------------------------------------------------ the transcript
def test_history_reads_only_the_end_of_a_long_transcript(tmp_path, monkeypatch):
    import json
    import chatcomputer.controller as C
    c = _controller(tmp_path, monkeypatch)
    now = time.time()
    old = json.dumps({"ts": now, "role": "user", "text": "x" * 500}) + "\n"
    recent = [json.dumps({"ts": now, "role": "assistant" if i % 2 else "user", "text": f"message {i}"}) + "\n"
              for i in range(12)]
    c.paths.transcript.write_text(old * 4000 + "".join(recent), encoding="utf-8")    # 2 MB of conversations
    read = []
    real_open = open

    def counting_open(*a, **k):
        f = real_open(*a, **k)
        if str(a[0]) == str(c.paths.transcript):
            real_read = f.read
            f.read = lambda *ra: read.append(len(data := real_read(*ra))) or data
        return f
    monkeypatch.setattr("builtins.open", counting_open)
    c._show_history()
    shown = [e for e in c.screen.events if e[0] == "history"]
    assert shown == [("history", 12)] and sum(read) <= C.HISTORY_TAIL_BYTES


# ------------------------------------------------------------------ tools
def _tools(tmp_path, platform):
    from chatcomputer import tools as T
    from chatcomputer.journal import Journal
    from chatcomputer.memory import Memory
    home = tmp_path / "home"
    home.mkdir(exist_ok=True)
    # (The trash's folder is one for the whole run: tests/conftest.py sets XDG_DATA_HOME once, before any test.)
    paths = Paths(home / "ChatComputer").ensure()
    ctx = T.ToolContext(paths, Settings(windows_mount=str(tmp_path / "win")), platform, Journal(paths), RecordingScreen(),
                        Memory(paths))
    T.build(ctx)
    return ctx, ctx


def _call(ctx, name, args):
    return asyncio.run(ctx.tool_fns[name](args))


class SlowParts(FakePlatform):
    def _slow(self, value):
        time.sleep(0.4)
        return value

    def internet_ok(self):
        return self._slow(True)

    def system_info(self):
        return self._slow({"time": "now"})

    def wifi_status(self):
        return self._slow({"connected_to": "Home"})

    def bt_status(self):
        return self._slow({"adapter": True, "paired": []})

    def audio_status(self):
        return self._slow({"outputs": []})

    def busiest(self, n=5):                 # measured over half a second on a real computer
        return self._slow({"cpu_percent": 3.0, "programs": []})

    def camera_status(self):
        return self._slow({"cameras": [], "in_use_by": []})

    def windows_status(self):
        raise RuntimeError("no helper")


def test_the_status_check_asks_every_part_side_by_side(tmp_path):
    import json
    ctx, server = _tools(tmp_path, SlowParts())
    t0 = time.monotonic()
    out = _call(server, "system_status", {})
    took = time.monotonic() - t0
    st = json.loads(out["content"][0]["text"])
    assert took < 1.2                       # seven parts of 0.4 s each: 2.8 s one after another
    assert st["internet"] is True and st["wifi"] == {"connected_to": "Home"}
    assert st["windows_drive"] == "unavailable (no helper)"
    assert list(st)[:8] == ["internet", "machine", "busy", "wifi", "bluetooth", "sound", "camera", "windows_drive"]


def test_volume_zero_is_silence_not_the_default(tmp_path):
    ctx, server = _tools(tmp_path, FakePlatform())
    _call(server, "sound", {"action": "set_volume", "value": 0})
    assert ctx.platform.volume == 0
    _call(server, "sound", {"action": "set_volume", "value": "35%"})
    assert ctx.platform.volume == 35
    _call(server, "sound", {"action": "volume_down", "value": "5"})
    assert ctx.platform.volume == 30


def test_a_look_at_the_screen_comes_from_the_platforms_jpeg(tmp_path):
    class Shot(FakePlatform):
        def screenshot_jpeg(self, max_w, max_h):
            assert (max_w, max_h) == (1280, 800)
            return b"\xff\xd8jpeg", 3.0
    ctx, server = _tools(tmp_path, Shot())
    out = _call(server, "look_at_screen", {})
    assert out["content"][0]["type"] == "image" and ctx.shot_scale == 3.0
    _call(server, "click", {"x": 100, "y": 50})
    assert ("click", (300, 150, "left", 1)) in ctx.platform.calls


def test_the_base_jpeg_is_made_from_the_png(tmp_path):
    p = FakePlatform()
    p.size = (2560, 1440)
    jpg, scale = p.screenshot_jpeg(1280, 800)
    assert jpg[:2] == b"\xff\xd8" and scale == 2.0


# ------------------------------------------------------------------ Bluetooth
def test_a_computer_without_bluetooth_answers_at_once(tmp_path, monkeypatch):
    # Without an adapter bluetoothd doesn't run, and each bluetoothctl command waited 8 s for it.
    from chatcomputer.platform import linux as L
    p = L.LinuxPlatform()
    monkeypatch.setattr(L, "run", lambda *a, **k: (_ for _ in ()).throw(AssertionError(f"ran {a[0]}")))
    sysfs = tmp_path / "bluetooth"
    monkeypatch.setattr(p, "has_bluetooth", lambda sysfs=str(sysfs): L.LinuxPlatform.has_bluetooth(p, sysfs))
    assert p.bt_status() == {"adapter": False, "powered": False, "connected": [], "paired": []}
    import pytest
    for fn in (lambda: p.bt_scan(5), lambda: p.bt_connect("AA:BB:CC:DD:EE:FF"), lambda: p.bt_pair("AA:BB:CC:DD:EE:FF")):
        with pytest.raises(RuntimeError, match="no Bluetooth"):
            fn()
    sysfs.mkdir()
    assert not p.has_bluetooth(str(sysfs))           # the kernel's folder, but no adapter in it
    (sysfs / "hci0").mkdir()
    assert p.has_bluetooth(str(sysfs))


# ------------------------------------------------------------------ the trash
def test_the_newest_trashed_copy_is_found_without_reading_the_whole_trash(tmp_path, monkeypatch):
    from chatcomputer import journal
    share = tmp_path / "share"
    trash = share / "Trash"
    (trash / "info").mkdir(parents=True)
    (trash / "files").mkdir()
    monkeypatch.setenv("XDG_DATA_HOME", str(share))

    def put(name, original, date):
        (trash / "files" / name).write_text(name)
        (trash / "info" / f"{name}.trashinfo").write_text(f"[Trash Info]\nPath={original}\nDeletionDate={date}\n")
    for i in range(300):
        put(f"note {i}.txt", str(tmp_path / f"note {i}.txt"), "2026-10-01T10:00:00")
    budget = str(tmp_path / "Budget.xlsx")
    put("Budget.xlsx", budget, "2026-10-17T16:00:00")
    time.sleep(0.02)
    put("Budget.2.xlsx", budget, "2026-10-17T16:00:00")    # trashed again within the second (GLib's name for it)
    reads = []
    real = Path.read_text
    monkeypatch.setattr(Path, "read_text", lambda self, *a, **k: reads.append(self.name) or real(self, *a, **k))
    assert journal.find_in_trash(budget) == str(trash / "files" / "Budget.2.xlsx")     # the newer, for undo
    assert sorted(reads) == ["Budget.2.xlsx.trashinfo", "Budget.xlsx.trashinfo"]
    # One named some other way is still found, by reading the rest.
    put("renamed.xlsx", str(tmp_path / "Other.xlsx"), "2026-10-17T16:00:01")
    assert journal.find_in_trash(str(tmp_path / "Other.xlsx")) == str(trash / "files" / "renamed.xlsx")


# ------------------------------------------------------------------ PDFs
def test_a_pdf_shows_as_pages_and_says_when_there_are_more(tmp_path, monkeypatch):
    import os
    import shutil
    import pytest
    from PIL import Image
    from chatcomputer.ui import render
    if not (shutil.which("pdftoppm") and shutil.which("pdfinfo")):
        pytest.skip("poppler-utils isn't installed")
    monkeypatch.setattr(render, "PDF_PAGES", 3)
    pdf = tmp_path / "Report.pdf"
    pages = [Image.new("RGB", (600, 800), "white") for _ in range(5)]
    pages[0].save(pdf, save_all=True, append_images=pages[1:])
    paths = Paths(tmp_path / "ChatComputer").ensure()
    root = paths.web / "pdf"
    old = root / "0123456789abcdef"            # pages of a PDF last shown a month ago
    old.mkdir(parents=True)
    (old / "p-1.jpg").write_bytes(b"x")
    month_ago = time.time() - 30 * 86400
    os.utime(old, (month_ago, month_ago))
    kind, page = render.file_view(str(pdf), Settings(), paths)
    assert kind == "html" and page.count("<img ") == 3 and ".jpg" in page
    assert "the first 3 of its 5 pages" in page
    kept = [d.name for d in root.iterdir()]
    assert len(kept) == 1 and not kept[0].endswith(".part")        # the old pages are gone; no half-made folder
    assert render.file_view(str(pdf), Settings(), paths) == (kind, page)     # again, from what was rendered


# ------------------------------------------------------------------ reading documents for the index
def test_documents_are_read_in_another_process(tmp_path):
    # In Chat Computer's own process the (pure Python) readers made its speech skip.
    import os
    from chatcomputer.files_index import TextReader
    doc = tmp_path / "notes.txt"
    doc.write_text("budget for 2026")
    r = TextReader()
    try:
        assert r.read(str(doc)) == "budget for 2026"
        assert r.proc.pid != os.getpid() and r.proc.is_alive()
    finally:
        r.stop()
    assert r.proc is None


def test_a_document_that_hangs_the_reader_doesnt_stop_the_index(tmp_path):
    import os
    import threading
    from chatcomputer.files_index import TextReader
    stuck = tmp_path / "stuck.txt"          # reading it never ends (like a file on a drive that stopped answering)
    os.mkfifo(stuck)
    doc = tmp_path / "notes.txt"
    doc.write_text("budget for 2026")
    r = TextReader(timeout_s=1.5)
    try:
        t0 = time.monotonic()
        assert r.read(str(stuck)) == ""
        assert time.monotonic() - t0 < 6
        assert r.read(str(doc)) == "budget for 2026"            # a new reader for the next one
        # Stopping Chat Computer doesn't wait out a document either.
        cancel = threading.Event()
        r2 = TextReader(timeout_s=60, cancel=cancel)
        threading.Timer(0.5, cancel.set).start()
        t0 = time.monotonic()
        assert r2.read(str(stuck)) == "" and time.monotonic() - t0 < 6
    finally:
        r.stop()
