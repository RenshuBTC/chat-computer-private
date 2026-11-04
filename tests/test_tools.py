"""Chat Computer tools called directly (no Claude), against a recording screen and fake machine."""
import asyncio
import json
import os
import time
from pathlib import Path

import pytest

from chatcomputer import tools as T
from chatcomputer.config import Paths, Settings
from chatcomputer.files_index import FileIndex, parse_date, words_of
from chatcomputer.journal import Journal
from chatcomputer.memory import Memory
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen
from chatcomputer.ui import render


def make(tmp_path, clean=True):
    home = tmp_path / "home"
    home.mkdir(exist_ok=True)
    # (The trash's folder is one for the whole run: tests/conftest.py sets XDG_DATA_HOME once.)
    paths = Paths(home / "ChatComputer").ensure()
    win = tmp_path / "windows"
    (win / "Users" / "sammy" / "Documents").mkdir(parents=True)
    settings = Settings(windows_user="sammy", windows_mount=str(win))
    screen = RecordingScreen()
    ctx = T.ToolContext(paths, settings, FakePlatform(), Journal(paths), screen, Memory(paths), power_delay_s=0.05)
    _, policies, names = T.build(ctx)
    return ctx, ctx, policies, names


def call(ctx, name, args):
    """Invoke a tool as the brain does (ctx.tool_fns: what each tool does)."""
    out = asyncio.get_event_loop().run_until_complete(ctx.tool_fns[name](args))
    texts = [c["text"] for c in out["content"] if c.get("type") == "text"]
    return "\n".join(texts), bool(out.get("is_error"))


@pytest.fixture(autouse=True)
def loop():
    lp = asyncio.new_event_loop()
    asyncio.set_event_loop(lp)
    yield lp
    lp.close()


def test_windows_paths_are_understood(tmp_path):
    ctx, *_ = make(tmp_path)
    p = ctx.resolve(r"C:\Users\sammy\Documents\tax.pdf")
    assert p == str(tmp_path / "windows" / "Users" / "sammy" / "Documents" / "tax.pdf")
    assert ctx.resolve("~/x.txt") == os.path.join(os.path.expanduser("~"), "x.txt")


def test_a_path_said_without_the_home_folder_is_the_home_folders(tmp_path, monkeypatch):
    """evals/os: a small model wrote "/Downloads/Boarding pass SQ322.pdf" for the file search_files had found in the
    home folder's Downloads. Where only the home folder's is there, that's the one; a path that's there stays as it is."""
    ctx, *_ = make(tmp_path)
    home = tmp_path / "home"
    monkeypatch.setenv("HOME", str(home))
    (home / "Downloads").mkdir()
    (home / "Downloads" / "Boarding pass.pdf").write_text("x")
    assert ctx.resolve("/Downloads/Boarding pass.pdf") == str(home / "Downloads" / "Boarding pass.pdf")
    assert ctx.resolve("Downloads/Boarding pass.pdf") == str(home / "Downloads" / "Boarding pass.pdf")
    assert ctx.resolve("/Downloads/nothing.pdf") == "/Downloads/nothing.pdf"      # (nowhere: as said)
    assert ctx.resolve("/etc/hostname") == "/etc/hostname"


def test_a_wifi_network_said_loosely_is_the_one_in_range(tmp_path):
    """evals/os: "Connect to the CoffeeBean wifi" was joined as a network called "CoffeeBean wifi"."""
    ctx, *_ = make(tmp_path)
    ctx.platform.wifi_scan = lambda: [{"name": "CoffeeBean", "signal": 70, "secured": True, "connected": False},
                                      {"name": "HomeWiFi", "signal": 40, "secured": True, "connected": True}]
    out, err = call(ctx, "wifi", {"action": "connect", "network": "the CoffeeBean wifi", "password": "latte2026"})
    assert not err and out == "Connected to CoffeeBean."
    out, err = call(ctx, "wifi", {"action": "connect", "network": "Starbucks", "password": "x"})
    assert out == "Connected to Starbucks."       # (not in range: tried as said; the computer says if it isn't there)


def test_the_status_gives_each_disks_free_share(tmp_path):
    """evals/os: a small model said 40 GB free of 64 was 24%: the share comes worked out."""
    ctx, *_ = make(tmp_path)
    out, _ = call(ctx, "system_status", {})
    assert json.loads(out)["machine"]["disks"]["home"]["free_percent"] == 62


def test_show_website_normalizes_urls_and_searches(tmp_path):
    ctx, server, *_ = make(tmp_path)
    call(server, "show_website", {"url": "youtube.com"})
    call(server, "show_website", {"url": "best laksa in singapore"})
    urls = [e[1] for e in ctx.screen.events if e[0] == "show_url"]
    assert urls[0] == "https://youtube.com"
    assert urls[1].startswith("https://www.google.com/search?q=best+laksa")


def test_page_tools_need_something_on_screen(tmp_path):
    ctx, server, *_ = make(tmp_path)
    text, err = call(server, "click", {"text": "Play"})
    assert err and "Nothing is open" in text


def test_move_copy_trash_and_undo(tmp_path):
    ctx, server, policies, _ = make(tmp_path)
    docs = tmp_path / "windows" / "Users" / "sammy" / "Documents"
    a = docs / "a.txt"
    a.write_text("A")
    # moving onto an existing file asks first
    (docs / "b.txt").write_text("B")
    q = policies["move_files"]({"items": [{"from": str(a), "to": str(docs / "b.txt")}]})
    assert q and "replace b.txt" in q
    assert policies["move_files"]({"items": [{"from": str(a), "to": str(docs / "c.txt")}]}) is None
    call(server, "move_files", {"items": [{"from": str(a), "to": str(docs / "c.txt")}]})
    assert not a.exists() and (docs / "c.txt").read_text() == "A"
    call(server, "copy_files", {"items": [{"from": str(docs / "c.txt"), "to": str(docs / "d.txt")}]})
    assert (docs / "d.txt").exists()
    assert "Move 1 item to the trash: d.txt?" == policies["trash_files"]({"paths": [str(docs / "d.txt")]})
    call(server, "trash_files", {"paths": [str(docs / "d.txt")]})
    assert not (docs / "d.txt").exists()
    text, _ = call(server, "undo_last_change", {"count": 3})
    assert (docs / "d.txt").exists() is False  # undoing the copy removes it again after restoring from trash
    assert a.exists() and (docs / "c.txt").exists() is False
    assert "moved" in text


def test_windows_system_files_cannot_be_trashed(tmp_path):
    ctx, server, policies, _ = make(tmp_path)
    win = tmp_path / "windows" / "Windows" / "System32"
    win.mkdir(parents=True)
    assert policies["trash_files"]({"paths": [str(win / "kernel32.dll")]}).startswith("DENY:")
    assert policies["trash_files"]({"paths": [str(ctx.paths.secrets / "accounts.json")]}).startswith("DENY:")
    assert policies["move_files"]({"items": [{"from": r"C:\Program Files\x", "to": "~/x"}]}).startswith("DENY:")
    # A folder of the person's own that happens to be called Recovery or Boot is theirs.
    q = policies["trash_files"]({"paths": [r"C:\Users\sammy\Documents\Recovery"]})
    assert q.startswith("Move 1 item to the trash"), q


def test_moving_a_folder_onto_one_of_the_same_name_changes_nothing(tmp_path):
    # It used to end up inside it (B/A/A), and undo then moved B/A, with what was in it before, out of B.
    ctx, server, policies, _ = make(tmp_path)
    home = tmp_path / "home"
    a, b = home / "A", home / "B"
    a.mkdir()
    (a / "new.txt").write_text("from A")
    (b / "A").mkdir(parents=True)
    (b / "A" / "old.txt").write_text("was in B/A")
    assert policies["move_files"]({"items": [{"from": str(a), "to": str(b)}]}) is None   # nothing gets replaced
    text, err = call(server, "move_files", {"items": [{"from": str(a), "to": str(b)}]})
    assert "already a folder" in text, text
    assert (a / "new.txt").exists() and sorted(p.name for p in (b / "A").iterdir()) == ["old.txt"]
    text, _ = call(server, "undo_last_change", {})
    assert (b / "A" / "old.txt").exists() and (a / "new.txt").exists()
    # A file onto a file still replaces it (after asking, backed up); a folder copied onto one adds to it.
    (a / "x.txt").write_text("1")
    (b / "A" / "x.txt").write_text("2")
    assert "replace" in policies["copy_files"]({"items": [{"from": str(a), "to": str(b)}]})
    call(server, "copy_files", {"items": [{"from": str(a), "to": str(b)}]})
    assert (b / "A" / "x.txt").read_text() == "1" and (b / "A" / "old.txt").exists()


def test_power_always_asks_and_runs_later(tmp_path, loop):
    ctx, server, policies, _ = make(tmp_path)
    assert policies["power"]({"action": "switch_to_windows"}).startswith("Restart into Windows?")
    call(server, "power", {"action": "restart"})
    loop.run_until_complete(asyncio.sleep(0.3))
    assert ("power", ("restart",)) in ctx.platform.calls


def test_the_logs_are_copied_before_the_computer_restarts(tmp_path, loop):
    # Chat Computer is ended at once by a restart: the copy on Windows' drive is made first (logcopy.py).
    ctx, server, policies, _ = make(tmp_path)
    order = []

    async def before():
        order.append("logs copied")
    ctx.before_power = before
    real = ctx.platform.power
    ctx.platform.power = lambda act: order.append("power") or real(act)
    call(server, "power", {"action": "switch_to_windows"})
    loop.run_until_complete(asyncio.sleep(0.3))
    assert order == ["logs copied", "power"]


def test_chatcomputer_own_files_cant_be_moved_or_trashed(tmp_path, monkeypatch):
    from chatcomputer import update
    ctx, server, policies, _ = make(tmp_path)
    src = Path(ctx.settings.windows_mount) / "Users/sammy/Downloads/ChatComputer"
    (src / "chatcomputer").mkdir(parents=True)
    note = tmp_path / "update-source"
    note.write_text("Users/sammy/Downloads/ChatComputer\n")
    monkeypatch.setattr(update, "UPDATE_SOURCE", note)
    monkeypatch.setattr(update, "WINDOWS_MOUNT", Path(ctx.settings.windows_mount))
    evil = tmp_path / "evil.py"
    evil.write_text("")
    for name, args in (("copy_files", {"items": [{"from": str(evil), "to": str(src / "chatcomputer/evil.py")}]}),
                       ("move_files", {"items": [{"from": str(src), "to": str(tmp_path / "elsewhere")}]}),
                       ("trash_files", {"paths": [str(src / "chatcomputer")]}),
                       ("trash_files", {"paths": [str(src.parent)]})):
        assert policies[name](args).startswith("DENY: Those are Chat Computer's own program files"), (name, args)
    # Copying them somewhere else is fine.
    assert not str(policies["copy_files"]({"items": [{"from": str(src), "to": str(tmp_path / "copy")}]})).startswith("DENY")


def test_a_switch_that_cant_be_set_up_is_reported_before_the_goodbye(tmp_path, loop):
    ctx, server, policies, _ = make(tmp_path)
    ctx.platform.prepare_power = lambda act: "I couldn't set Windows for the next start: no Windows Boot Manager"
    text, err = call(server, "power", {"action": "switch_to_windows"})
    loop.run_until_complete(asyncio.sleep(0.3))
    assert err and "Windows Boot Manager" in text
    assert not any(c[0] == "power" for c in ctx.platform.calls)          # nothing restarts
    # And when the restart itself fails afterwards, the screen says why (Claude has already said goodbye).
    ctx.platform.prepare_power = lambda act: None
    ctx.platform.power = lambda act: "Failed: Access denied"
    text, err = call(server, "power", {"action": "restart"})
    loop.run_until_complete(asyncio.sleep(0.3))
    assert not err and ("notice", ("Failed: Access denied", "warn")) in ctx.screen.events


def test_windows_can_start_by_default_and_back(tmp_path):
    ctx, server, policies, names = make(tmp_path)
    assert "start_by_default" in names
    assert policies["start_by_default"]({"system": "other"}) == (
        "Start Windows when the computer turns on instead of Chat Computer? Chat Computer stays installed.")
    assert policies["start_by_default"]({"system": "chatcomputer"}) == "Start Chat Computer when the computer turns on?"
    text, err = call(server, "start_by_default", {"system": "other"})
    assert not err and ("set_default_os", ("other",)) in ctx.platform.calls
    call(server, "start_by_default", {"system": "chatcomputer"})
    assert ctx.platform.default_os == "chatcomputer"


def test_settings_and_memory(tmp_path):
    ctx, server, *_ = make(tmp_path)
    changed = []
    ctx.on_settings_changed = changed.append
    text, err = call(server, "chatcomputer_settings", {"action": "set", "key": "speech_speed", "value": "1.2"})
    assert not err and ctx.settings.speech_speed == 1.2 and changed == [["speech_speed"]]
    assert json.loads(ctx.paths.settings_file.read_text())["speech_speed"] == 1.2
    text, err = call(server, "chatcomputer_settings", {"action": "set", "key": "windows_mount", "value": "/"})
    assert err
    # The screen is black on white only: there is no theme to change.
    text, err = call(server, "chatcomputer_settings", {"action": "set", "key": "theme", "value": "dark"})
    assert err and not hasattr(ctx.settings, "theme")
    call(server, "memory", {"action": "remember", "text": "Prefers Bitcoin charts in SGD"})
    assert "Bitcoin charts" in ctx.memory.for_prompt()
    call(server, "memory", {"action": "forget", "text": "bitcoin charts"})
    assert "Bitcoin" not in ctx.memory.for_prompt()


def test_show_page_can_be_saved_and_reopened(tmp_path):
    ctx, server, *_ = make(tmp_path)
    call(server, "show_page", {"title": "BTC chart", "html": "<h1>hi</h1>", "save_as": "Bitcoin chart"})
    assert (ctx.paths.pages / "bitcoin-chart.html").exists()
    text, _ = call(server, "saved_pages", {"action": "list"})
    assert "Bitcoin chart" in text
    call(server, "saved_pages", {"action": "open", "name": "bitcoin"})
    assert ctx.screen.events[-1][1].endswith("bitcoin-chart.html")


def test_file_index_finds_files_by_name_and_content(tmp_path):
    root = tmp_path / "Users" / "sammy"
    (root / "Documents").mkdir(parents=True)
    (root / "AppData" / "Local").mkdir(parents=True)
    (root / "Documents" / "Tax_Return_2025.txt").write_text("IRAS assessment for year of assessment 2025")
    (root / "Documents" / "notes.md").write_text("Buy durian for mum's birthday")
    (root / "AppData" / "Local" / "cache.txt").write_text("durian")
    old = root / "Documents" / "old.txt"
    old.write_text("x")
    os.utime(old, (time.time() - 400 * 86400, time.time() - 400 * 86400))
    idx = FileIndex(tmp_path / "idx.db", [("windows", str(root))], windows_root=str(tmp_path))
    assert idx.crawl_once() == 3          # AppData is skipped
    idx.extract_contents()
    hits = idx.search("tax return")
    assert hits and hits[0]["path"].endswith("Tax_Return_2025.txt")
    assert hits[0]["where"].startswith("C:\\Users\\sammy")
    hits = idx.search("durian")
    assert len(hits) == 1 and hits[0]["path"].endswith("notes.md") and "durian" in hits[0]["match"].lower()
    recent = idx.search("", after="30d")
    assert {Path(h["path"]).name for h in recent} == {"Tax_Return_2025.txt", "notes.md"}
    # deletions are noticed on the next scan
    (root / "Documents" / "notes.md").unlink()
    idx.crawl_once()
    assert idx.search("durian") == []


def test_words_and_dates():
    assert "Tax Return 2025" in words_of("Tax_Return-2025.pdf")
    assert parse_date("7d") < time.time() - 6 * 86400
    assert parse_date("2025-01-31") is not None and parse_date("nonsense") is None


def test_render_files(tmp_path):
    s = Settings()
    (tmp_path / "a.csv").write_text("name,amount\nrent,1200\n")
    kind, html = render.file_view(str(tmp_path / "a.csv"), s, Paths(tmp_path / "c"))
    assert kind == "html" and "rent" in html and "renderMarkdown" in html
    kind, html = render.file_view(str(tmp_path), s, Paths(tmp_path / "c"))
    assert "a.csv" in html
    from PIL import Image
    Image.new("RGB", (10, 10)).save(tmp_path / "p.png")
    kind, html = render.file_view(str(tmp_path / "p.png"), s, Paths(tmp_path / "c"))
    assert "<img" in html
    import docx
    d = docx.Document()
    d.add_heading("Quarterly plan", 1)
    d.add_paragraph("Ship Chat Computer.")
    d.save(tmp_path / "plan.docx")
    kind, html = render.file_view(str(tmp_path / "plan.docx"), s, Paths(tmp_path / "c"))
    assert "Quarterly plan" in html and "Ship Chat Computer." in html


def test_table_cells_with_line_breaks_or_bars_stay_in_their_row(tmp_path):
    # A spreadsheet cell with a line break in it (Alt+Enter in Excel), a header with a "|", a Word table the same:
    # each table row stays one Markdown row, so the table shows as a table.
    import docx
    import openpyxl
    wb = openpyxl.Workbook()
    ws = wb.active
    ws.append(["Item | code", "Notes"])
    ws.append(["Rent", "Due on the 1st\nPaid by transfer"])
    wb.save(tmp_path / "b.xlsx")
    md = render._xlsx_markdown(str(tmp_path / "b.xlsx"))
    lines = [ln for ln in md.splitlines() if ln.startswith("|")]
    assert lines == ["| Item \\| code | Notes |", "|---|---|", "| Rent | Due on the 1st Paid by transfer |"]
    d = docx.Document()
    t = d.add_table(rows=2, cols=2)
    t.cell(0, 0).text, t.cell(0, 1).text = "A|B", "C"
    t.cell(1, 0).text, t.cell(1, 1).text = "one\ntwo", "three"
    d.save(tmp_path / "t.docx")
    md = render._docx_markdown(str(tmp_path / "t.docx"))
    assert md.splitlines() == ["| A\\|B | C |", "|---|---|", "| one two | three |"]


def test_private_sign_in_files_stay_private(tmp_path):
    ctx, server, policies, _ = make(tmp_path)
    token = ctx.paths.secrets / "accounts.json"         # the messages accounts' sign-ins
    token.write_text('{"x": {"access_token": "secret"}}\n')
    desk = tmp_path / "home" / "Desktop"
    desk.mkdir()
    # A copy of a sign-in (or of a folder holding it) would be an ordinary file anyone could read.
    for src in (token, ctx.paths.engine / "conversations", ctx.paths.root, tmp_path / "home"):
        q = policies["copy_files"]({"items": [{"from": str(src), "to": str(desk)}]})
        assert q and q.startswith("DENY:"), src
        assert policies["move_files"]({"items": [{"from": str(src), "to": str(desk)}]}).startswith("DENY:"), src
        assert policies["trash_files"]({"paths": [str(src)]}).startswith("DENY:"), src
    assert policies["copy_files"]({"items": [{"from": str(desk), "to": str(ctx.paths.secrets)}]}).startswith("DENY:")
    # Nor shown on screen, where read_screen would hand it to the model.
    text, err = call(server, "show_file", {"path": str(ctx.paths.engine / ".credentials.json")})
    assert err and "private" in text
    text, err = call(server, "show_website", {"url": token.as_uri()})
    assert err and "show_file" in text
    assert not [e for e in ctx.screen.events if e[0] in ("show_url", "show_html")]


def test_the_screen_never_opens_a_private_file(tmp_path):
    gtk_app = pytest.importorskip("chatcomputer.ui.gtk_app")
    ctx, *_ = make(tmp_path)
    assert gtk_app.private_file_uri(ctx.paths, (ctx.paths.secrets / "accounts.json").as_uri())
    assert gtk_app.private_file_uri(ctx.paths, (ctx.paths.web / "cookies.sqlite").as_uri())
    assert gtk_app.private_file_uri(ctx.paths, "file:///proc/self/environ")
    assert not gtk_app.private_file_uri(ctx.paths, (ctx.paths.pages / "chart.html").as_uri())
    assert not gtk_app.private_file_uri(ctx.paths, "https://example.com/claude_token")


def test_undo_never_deletes_what_was_there_before(tmp_path, monkeypatch):
    import shutil as sh
    from chatcomputer import journal
    bin_ = tmp_path / "trash-bin"
    bin_.mkdir()
    # (send2trash picks its trash folder once, when first used, so look at what undo hands it instead.)
    monkeypatch.setattr(journal, "move_to_trash", lambda p: sh.move(p, bin_ / os.path.basename(p)))
    ctx, server, policies, _ = make(tmp_path)
    docs = tmp_path / "windows" / "Users" / "sammy" / "Documents"
    (docs / "Photos").mkdir()
    (docs / "Photos" / "mine.jpg").write_text("the person's own photo")
    new = tmp_path / "home" / "Photos"
    new.mkdir()
    (new / "trip.jpg").write_text("copied photo")
    # Copying a folder into one of the same name merges them: undo must leave the folder alone.
    call(server, "copy_files", {"items": [{"from": str(new), "to": str(docs)}]})
    assert (docs / "Photos" / "trip.jpg").exists()
    text, _ = call(server, "undo_last_change", {})
    assert (docs / "Photos" / "mine.jpg").read_text() == "the person's own photo", text
    assert "already there" in text
    # A file Claude created goes to the trash on undo, where it can still be found.
    created = tmp_path / "home" / "note.txt"
    ctx.journal.record_overwrite(str(created), None, "created")
    created.write_text("changed later")
    text, _ = call(server, "undo_last_change", {})
    assert not created.exists() and "trash" in text
    assert (bin_ / "note.txt").read_text() == "changed later"


def test_the_model_is_picked_for_the_computer_unless_they_chose_one(tmp_path):
    assert (Settings().model, Settings().effort) == ("auto", "medium")
    paths = Paths(tmp_path / "ChatComputer").ensure()
    # A Chat Computer from when Claude thought: Claude's model and effort mean nothing now, and its name isn't the
    # wake word any more; the rest stays.
    paths.settings_file.write_text(json.dumps({"model": "claude-opus-5-5", "effort": "xhigh", "speech_speed": 1.2,
                                               "defaults_version": 2, "phone_access": True,
                                               "wake_words": ["claude", "hey claude", "ok claude", "okay claude"]}))
    s = Settings.load(paths)
    assert (s.model, s.effort, s.speech_speed) == ("auto", "medium", 1.2)
    assert s.wake_words[0] == "computer" and "phone_access" not in s.extra
    s.save(paths)
    # Someone who picks a model and effort now keeps them.
    s.set("model", "qwen3.5-9b")
    s.set("effort", "off")
    s.save(paths)
    s = Settings.load(paths)
    assert (s.model, s.effort) == ("qwen3.5-9b", "off")
    # And wake words they chose themselves stay theirs.
    paths.settings_file.write_text(json.dumps({"wake_words": ["jarvis"], "defaults_version": 2}))
    assert Settings.load(paths).wake_words == ["jarvis"]


def test_settings_that_would_stop_the_model_are_refused(tmp_path):
    ctx, server, *_ = make(tmp_path)
    for key, value in (("effort", "extreme"), ("model", "claude-opus-5-5"), ("model", "../x"), ("stt_model", "../../x")):
        text, err = call(server, "chatcomputer_settings", {"action": "set", "key": key, "value": value})
        assert err, (key, text)
    assert ctx.settings.effort == Settings().effort and ctx.settings.model == Settings().model
    text, err = call(server, "chatcomputer_settings", {"action": "set", "key": "effort", "value": "high"})
    assert not err and ctx.settings.effort == "high"
    text, err = call(server, "chatcomputer_settings", {"action": "set", "key": "model", "value": "qwen3.5-4b"})
    assert not err and ctx.settings.model == "qwen3.5-4b"
    # Numbers that would break the voice, put Chat Computer back to sleep at once, or forget every conversation.
    for key, value in (("speech_speed", 3), ("speech_speed", "fast"), ("auto_sleep_minutes", -1),
                       ("fresh_session_after_idle_hours", 0)):
        text, err = call(server, "chatcomputer_settings", {"action": "set", "key": key, "value": value})
        assert err, (key, value, text)
    assert ctx.settings.speech_speed == Settings().speech_speed and ctx.settings.auto_sleep_minutes == 10
    text, err = call(server, "chatcomputer_settings", {"action": "set", "key": "speech_speed", "value": 1.5})
    assert not err and ctx.settings.speech_speed == 1.5


def test_a_settings_file_that_isnt_an_object_gives_the_defaults(tmp_path):
    from chatcomputer.config import State
    paths = Paths(tmp_path / "ChatComputer").ensure()
    paths.settings_file.write_text("[1, 2]")
    paths.state_file.write_text('"x"')
    assert Settings.load(paths) == Settings()
    assert State(paths).get("session_id") is None


def test_history_still_shows_when_a_line_was_cut_short(tmp_path):
    # One damaged line (the power went off mid-write) used to hide the whole history.
    from chatcomputer.controller import Controller
    paths = Paths(tmp_path / "ChatComputer").ensure()
    now = time.time()
    paths.transcript.write_text("\n".join([json.dumps({"ts": now, "role": "user", "text": "hello"}), '{"ts": 17',
                                           "5", json.dumps({"ts": now, "role": "assistant", "text": "hi"})]) + "\n")
    screen = RecordingScreen()
    Controller(paths, Settings(), FakePlatform(), screen, voice=False, index=False)._show_history()
    assert ("history", 2) in screen.events


def test_friendly_errors_say_how_far_the_model_is():
    from chatcomputer.brain.agent import TurnResult, friendly_error
    res = TurnResult(ok=False, error_kind="model_not_ready")
    assert friendly_error(res, "downloading it: 4.0 of 10.0 GB (40%)") == (
        "I'm still getting my model ready: downloading it: 4.0 of 10.0 GB (40%). Until then I can only help with "
        "Wi-Fi, volume and restarting.")
    assert "say that again" in friendly_error(TurnResult(ok=False, error_kind="engine"))


def test_command_output_that_isnt_utf8_is_still_read(tmp_path):
    from chatcomputer.platform.base import run
    r = run(["printf", "caf\\351 Wi-Fi\\n"])                # Latin-1 bytes
    assert r.ok and r.out.startswith("caf") and "Wi-Fi" in r.out
    d = tmp_path / "not-a-program"
    d.mkdir()
    assert run([str(d)]).code == 126                        # can't be run: reported, not raised


def test_a_package_name_cant_turn_an_install_into_a_removal(monkeypatch):
    # apt-get reads "name-" as "remove name" (and a final "." is a pattern): such names never reach the root helper.
    from chatcomputer.platform import linux as L
    from chatcomputer.platform.base import CommandResult
    calls = []
    monkeypatch.setattr(L, "run", lambda cmd, **kw: calls.append(cmd) or CommandResult(0, '{"ok": true}', ""))
    p = L.LinuxPlatform()
    assert "don't look like package names" in p.install(["network-manager-"])
    assert "don't look like package names" in p.remove(["vlc."])
    assert calls == []
    assert p.install(["g++", "vlc"]).startswith("Installed")      # a real name may end in "+"
    assert calls == [["sudo", "-n", L.ROOT_HELPER, "apt-install", "g++", "vlc"]]


def test_copying_pairings_from_windows_says_what_was_copied(monkeypatch):
    # The root helper passes the importer's report on under "result"; nothing copied must not sound like success.
    from chatcomputer.platform import linux as L
    from chatcomputer.platform.base import CommandResult
    reports = {
        "two": {"ok": True, "count": 2, "imported": [{"device": "A1:B2:C3:D4:E5:F6", "action": "created"},
                                                     {"device": "C0:FF:EE:12:34:56", "action": "updated"}],
                "skipped_no_adapter_dir": [], "errors": []},
        "other adapter": {"ok": True, "count": 0, "imported": [], "errors": [],
                          "skipped_no_adapter_dir": [{"adapter": "0A:1B:2C:3D:4E:5F", "device": "A1:B2:C3:D4:E5:F6"}]},
        "none": {"ok": True, "controlset": "\\ControlSet001", "imported": [], "note": "no BTHPORT\\Parameters\\Keys"},
    }
    said = {}
    for name, report in reports.items():
        out = json.dumps({"ok": True, "result": report})
        monkeypatch.setattr(L, "run", lambda cmd, out=out, **kw: CommandResult(0, out + "\n", ""))
        said[name] = L.LinuxPlatform().bt_import_windows_keys()
    assert said["two"] == "Copied 2 Bluetooth pairings from Windows."
    assert "different Bluetooth adapter" in said["other adapter"] and "Copied" not in said["other adapter"]
    assert said["none"].startswith("Windows has no Bluetooth pairings")


FAKE_BLUETOOTHCTL = r"""
import sys, time
mode = sys.argv[1]
def say(text):
    sys.stdout.write(text)
    sys.stdout.flush()
for line in sys.stdin:
    cmd = line.strip()
    if cmd == "agent KeyboardDisplay":
        say("Agent registered\n")
    elif cmd == "default-agent":
        say("Default agent request successful\n")
    elif cmd.startswith("pair "):
        say("Attempting to pair with " + cmd[5:] + "\n")
        if mode == "show":       # the passkey, its digits coloured by how many are typed (as bluetoothctl does)
            say("\x1b[0;94m[agent]\x1b[0m Passkey: \x1b[1;30m12\x1b[1;37m3456\n\x1b[0m")
            time.sleep(0.2)
            say("\x1b[0;94m[agent]\x1b[0m Passkey: \x1b[1;30m1234\x1b[1;37m56\n\x1b[0m")
            say("[CHG] Device " + cmd[5:] + " Paired: yes\nPairing successful\n")
        elif mode == "confirm":
            say("[agent] Confirm passkey 654321 (yes/no): ")
        elif mode == "pin":
            say("[agent] Enter PIN code: ")
        else:
            say("Failed to pair: org.bluez.Error.AuthenticationFailed\n")
    elif cmd == "yes" and mode == "confirm":
        say("Pairing successful\n")
    elif cmd.isdigit() and mode == "pin":
        open(sys.argv[2], "w").write(cmd)
        say("Pairing successful\n")
    elif cmd == "quit":
        break
"""


@pytest.mark.parametrize("mode,paired,code", [("show", True, "123456"), ("confirm", True, "654321"),
                                              ("pin", True, None), ("refuse", False, None)])
def test_a_keyboard_pairs_with_a_code_shown_to_type_on_it(tmp_path, mode, paired, code):
    import sys
    from chatcomputer.platform import linux as L
    script = tmp_path / "bluetoothctl.py"
    script.write_text(FAKE_BLUETOOTHCTL)
    pin_file = tmp_path / "pin"
    codes = []
    ok, why = L.pair_with_agent("AA:BB:CC:DD:EE:03", codes.append, timeout=10,
                                cmd=(sys.executable, str(script), mode, str(pin_file)))
    assert ok is paired
    if mode == "pin":                          # a PIN made up here, shown, and given to the keyboard's pairing
        assert len(codes) == 1 and len(codes[0]) == 6 and pin_file.read_text() == codes[0]
    elif paired:
        assert codes == [code]                 # once, however often bluetoothctl repeats it
    else:
        assert codes == [] and "AuthenticationFailed" in why


def test_only_keyboards_pair_with_a_code(monkeypatch):
    from chatcomputer.platform import linux as L
    from chatcomputer.platform.base import CommandResult
    calls, agent = [], []
    icons = {"AA:BB:CC:DD:EE:03": "input-keyboard", "AA:BB:CC:DD:EE:04": "input-mouse"}

    def run(cmd, **kw):
        calls.append(cmd)
        if cmd[1] == "info":
            return CommandResult(0, f"Device {cmd[2]}\n\tName: Thing\n\tIcon: {icons[cmd[2]]}\n", "")
        return CommandResult(0, "", "")
    monkeypatch.setattr(L, "run", run)
    monkeypatch.setattr(L, "pair_with_agent", lambda a, on_code, **kw: agent.append(a) or (True, ""))
    p = L.LinuxPlatform()
    monkeypatch.setattr(p, "has_bluetooth", lambda *a: True)
    assert p.bt_pair("AA:BB:CC:DD:EE:03", print) == "Paired and connected."
    assert agent == ["AA:BB:CC:DD:EE:03"] and not any("NoInputNoOutput" in c for c in calls)
    assert p.bt_pair("AA:BB:CC:DD:EE:04") == "Paired and connected."
    assert agent == ["AA:BB:CC:DD:EE:03"] and any("NoInputNoOutput" in c for c in calls)
    monkeypatch.setattr(L, "pair_with_agent", lambda a, on_code, **kw: (False, "org.bluez.Error.AuthenticationFailed"))
    assert p.bt_pair("AA:BB:CC:DD:EE:03").startswith("Failed: org.bluez.Error.AuthenticationFailed")


def test_pairing_a_keyboard_shows_and_says_its_code(tmp_path):
    ctx, server, *_ = make(tmp_path)
    said = []
    ctx.announce = said.append
    text, err = call(server, "bluetooth", {"action": "pair", "device": "Magic Keyboard"})
    assert not err and "Paired" in text
    assert said == ["Type 4 8 2 9 1 6 on the keyboard, then press Enter."]


def test_the_on_screen_keyboard_tool(tmp_path):
    ctx, server, *_ = make(tmp_path)
    text, err = call(server, "on_screen_keyboard", {"show": True})
    assert err and "no on-screen keyboard" in text
    asked = []

    async def keyboard(show):
        asked.append(show)
        return "The keyboard is up." if show else "The keyboard is away."
    ctx.keyboard = keyboard
    assert call(server, "on_screen_keyboard", {"show": True}) == ("The keyboard is up.", False)
    assert call(server, "on_screen_keyboard", {"show": False}) == ("The keyboard is away.", False)
    assert asked == [True, False]


def test_the_index_skips_chatcomputer_own_folders_not_the_persons_of_the_same_name(tmp_path):
    home = tmp_path / "home"
    paths = Paths(home / "ChatComputer").ensure()
    (paths.web / "Cookies").write_text("x")
    (paths.models / "voice.onnx").write_text("x")
    for d in ("web", "models", "index"):
        (home / "Projects" / d).mkdir(parents=True)
        (home / "Projects" / d / f"mine-{d}.txt").write_text("x")
    # A name that isn't UTF-8 used to stop the whole scan there.
    os.makedirs(os.fsencode(home / "Old"), exist_ok=True)
    with open(os.fsencode(home / "Old") + b"/caf\xe9.txt", "w") as f:
        f.write("x")
    (home / "Old" / "fine.txt").write_text("x")
    idx = FileIndex(tmp_path / "files.db", [("home", str(home))],
                    skip=[paths.secrets, paths.engine, paths.web, paths.index, paths.models, paths.logs, paths.undo])
    idx.crawl_once()
    names = {r["path"].rsplit("/", 1)[-1] for r in idx.search("", limit=100)}
    assert {"mine-web.txt", "mine-models.txt", "mine-index.txt", "fine.txt"} <= names, names
    assert not {"Cookies", "voice.onnx"} & names
    # A folder filter is exactly that folder: "_" in its name is a letter, not "any letter".
    (home / "my_docs").mkdir()
    (home / "my_docs" / "a.txt").write_text("x")
    (home / "myXdocs").mkdir()
    (home / "myXdocs" / "b.txt").write_text("x")
    idx.crawl_once()
    assert [r["path"].rsplit("/", 1)[-1] for r in idx.search("", folder=str(home / "my_docs"))] == ["a.txt"]


def test_an_interrupted_scan_keeps_what_it_didnt_reach(tmp_path, monkeypatch):
    from chatcomputer import files_index as FI
    docs = tmp_path / "docs"
    docs.mkdir()
    for i in range(5):
        (docs / f"f{i}.txt").write_text("x")
    idx = FileIndex(tmp_path / "files.db", [("home", str(docs))])
    assert idx.crawl_once() == 5
    monkeypatch.setattr(FI, "MAX_FILES", 2)                  # the next scan is cut short
    idx.crawl_once()
    assert idx.count() == 5


def test_screen_settings_stay_readable(tmp_path):
    # A text size of 3 px (or 5000) would leave nothing to read; 0 still means automatic.
    ctx, server, *_ = make(tmp_path)
    for key, bad in (("text_size", 3), ("text_size", 5000), ("text_scale", 10), ("viewing_distance_cm", 5),
                     ("screen_inches", -2), ("web_zoom", 9), ("text_scale", "big")):
        text, err = call(server, "chatcomputer_settings", {"action": "set", "key": key, "value": bad})
        assert err, (key, bad)
    for key, good in (("text_size", 32), ("text_size", 0), ("text_scale", 1.15), ("viewing_distance_cm", 250),
                      ("screen_inches", 0), ("screen_inches", 55), ("web_zoom", 1.5)):
        text, err = call(server, "chatcomputer_settings", {"action": "set", "key": key, "value": good})
        assert not err, (key, good, text)
    s = ctx.settings
    assert (s.text_size, s.text_scale, s.viewing_distance_cm, s.screen_inches, s.web_zoom) == (0, 1.15, 250, 55.0, 1.5)


def test_a_search_between_the_wrong_dates_says_what_the_words_find(tmp_path):
    """evals/os: a small model looked for "the tax letter I got in March" from March 2023 and found nothing: a search
    that finds nothing between its dates (or of its kind) gives what its words alone find, saying so."""
    ctx, *_ = make(tmp_path)

    class Index:
        status = {"state": "idle"}
        roots: list = []

        def search(self, query, kind, after, before, folder, limit):
            return [] if after or kind else [{"path": "/home/sam/Documents/Tax letter March 2026.pdf"}]
    ctx.index = Index()
    out, err = call(ctx, "search_files", {"query": "tax", "modified_after": "2023-03-01"})
    assert not err and "Tax letter March 2026" in out and "Nothing between those dates" in out
    out, _ = call(ctx, "search_files", {"query": "tax"})
    assert "Nothing" not in out


def test_the_drive_list_says_whats_on_each_drive(tmp_path):
    """evals/os: "what's on my USB stick?" was answered with the drive's size: the list says what's on it."""
    ctx, *_ = make(tmp_path)
    stick = tmp_path / "SANDISK"
    (stick / "Photos").mkdir(parents=True)
    (stick / "Recipes.docx").write_text("x")
    (stick / ".hidden").write_text("x")
    ctx.platform.drives = lambda: [{"name": "SANDISK", "kind": "USB drive", "size_gb": 32.0, "mounted": True, "path": str(stick)}]
    out, err = call(ctx, "drives", {"action": "list"})
    assert not err and json.loads(out)[0]["on_it"] == ["Photos/", "Recipes.docx"]
    # Opened, it says so too (evals/os: a small model opened it after the list, then asked again until stopped).
    ctx.platform.open_drive = lambda name: str(stick)
    out, err = call(ctx, "drives", {"action": "open", "drive": "SANDISK"})
    assert not err and out == f"It's open: its files are in {stick}. On it: Photos/, Recipes.docx"
    (stick / "Recipes.docx").unlink()
    (stick / "Photos").rmdir()
    out, _ = call(ctx, "drives", {"action": "open", "drive": "SANDISK"})
    assert out.endswith(". It's empty")


def test_files_are_never_moved_into_a_temporary_folder(tmp_path, monkeypatch):
    """evals/os: asked what's on a USB stick, a small model moved its photos and file into /tmp (emptied when the
    computer restarts). A copy there is fine: the files stay where they were."""
    import re
    ctx, *_ = make(tmp_path)
    temp = tmp_path / "temp"
    temp.mkdir()
    monkeypatch.setattr(T, "TEMPORARY", re.compile("^" + re.escape(os.path.realpath(temp)) + "(/|$)"))
    stick = tmp_path / "SANDISK"
    (stick / "Photos").mkdir(parents=True)
    (stick / "Photos" / "IMG_1001.jpg").write_text("x")
    (stick / "Recipes.docx").write_text("x")
    out, _ = call(ctx, "move_files", {"items": [{"from": str(stick / "Photos"), "to": str(temp / "Photos")},
                                                {"from": str(stick / "Recipes.docx"), "to": str(temp)}]})
    assert out.count("not moved:") == 2 and "a temporary folder, emptied when the computer restarts" in out
    assert (stick / "Photos" / "IMG_1001.jpg").exists() and (stick / "Recipes.docx").exists() and not any(temp.iterdir())
    out, _ = call(ctx, "copy_files", {"items": [{"from": str(stick / "Recipes.docx"), "to": str(temp)}]})
    assert out.startswith("copied") and (stick / "Recipes.docx").exists() and (temp / "Recipes.docx").exists()
    out, _ = call(ctx, "move_files", {"items": [{"from": str(temp / "Recipes.docx"), "to": str(temp / "Old recipes.docx")}]})
    assert out.startswith("moved")                       # within the temporary folder: nothing of theirs is lost


def test_dark_mode_is_answered_not_turned_up(tmp_path):
    """evals/os: "Turn on dark mode" turned the brightness up to 100% and said dark mode was on."""
    ctx, *_ = make(tmp_path)
    out, err = call(ctx, "display", {"action": "dark_mode"})
    assert not err and "always dark" in out and "no dark or light mode" in out
    out, _ = call(ctx, "display", {"action": "brightness", "value": 100, "mode": "dark"})
    assert "always dark" in out and not [c for c in ctx.platform.calls if c[0] == "set_brightness"]
    # Whichever action the model picks for it (evals/os: the brightness up, or the night light on, then "Done.").
    ctx.asked = lambda: "Turn on dark mode"
    for a in ({"action": "brightness", "value": 100, "mode": "on"}, {"action": "night_light", "mode": "on"},
              {"action": "brighter"}):
        out, _ = call(ctx, "display", a)
        assert "always dark" in out and "Nothing was changed" in out
    # With its own answer, said if the model says nothing (brain/agent.py).
    said = asyncio.get_event_loop().run_until_complete(ctx.tool_fns["display"]({"action": "dark_mode"}))["say"]
    assert said.startswith("There's no dark or light mode") and "dim it or turn on the night light" in said
    assert "Tell them" not in out          # (words to pass on as they are: the model passed that on too)
    assert not [c for c in ctx.platform.calls if c[0] in ("set_brightness", "set_night_light")]
    out, _ = call(ctx, "display", {"action": "status"})
    assert "always dark" in out                      # (what was asked isn't the status: told so again)
    # Asked for the screen itself, it's done.
    for words in ("Light mode is too bright, dim it", "It's too dark, make it brighter", "Turn on the night light"):
        ctx.asked = lambda w=words: w
        out, _ = call(ctx, "display", {"action": "dimmer"})
        assert "always dark" not in out
    assert [c for c in ctx.platform.calls if c[0] == "set_brightness"]
