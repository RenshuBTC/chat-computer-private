""" "/" in the message box searches this computer's files (chatcomputer/slash_search.py and the controller's side), and
the rest is done by asking: another microphone (the sound tool) or another model (chatcomputer_settings), applied
straight away."""
import asyncio
import json
import time
from types import SimpleNamespace

from chatcomputer import slash_search, tools as T
from chatcomputer.config import Paths, Settings
from chatcomputer.controller import Controller
from chatcomputer.journal import Journal
from chatcomputer.memory import Memory
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen


class FakeIndex:
    def __init__(self, rows, count=5):
        self.rows, self.n, self.asked = rows, count, []

    def search(self, query, limit=20):
        self.asked.append((query, limit))
        return self.rows

    def friendly(self, path):
        return path.replace(str(self.home), "~") if hasattr(self, "home") else path

    def count(self):
        return self.n


def _files(tmp_path):
    home = tmp_path / "home"
    (home / "Documents").mkdir(parents=True)
    (home / "Documents" / "report.pdf").write_bytes(b"%PDF")
    (home / "Documents" / "notes.txt").write_text("hello")
    paths = Paths(home / "ChatComputer").ensure()
    (paths.secrets / "accounts.json").write_text("{}")
    return home, paths


def test_slash_finds_files_on_this_computer(tmp_path):
    home, paths = _files(tmp_path)
    report = home / "Documents" / "report.pdf"
    idx = FakeIndex([{"path": str(report), "kind": "document"},
                     {"path": str(paths.secrets / "accounts.json"), "kind": "code"},
                     {"path": str(home / "gone.txt"), "kind": "document"}])
    idx.home = home
    items, note = slash_search.search(idx, "rep", paths)
    # Chat Computer's private files, and ones gone since the index looked, are left out.
    assert [(i["name"], i["where"], i["kind"]) for i in items] == [("report.pdf", "~/Documents", "document")]
    assert note == "" and idx.asked == [("rep", 16)]
    assert slash_search.search(FakeIndex([]), "zzq", paths) == ([], "Nothing on this computer matches “zzq”.")
    assert slash_search.search(FakeIndex([], count=0), "x", paths)[1].startswith("I'm still going through")
    assert slash_search.search(None, "x", paths)[1] == "Searching this computer's files is off."


def test_a_file_s_time_as_a_person_says_it():
    now = time.mktime((2026, 10, 19, 15, 0, 0, 0, 0, -1))
    assert slash_search.when_text(now - 3600, now) == "2:00 PM"
    assert slash_search.when_text(time.mktime((2026, 3, 4, 9, 0, 0, 0, 0, -1)), now) == "Mar 4"
    assert slash_search.when_text(time.mktime((2024, 3, 4, 9, 0, 0, 0, 0, -1)), now) == "Mar 4, 2024"


def _controller(tmp_path, monkeypatch):
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "ChatComputer"))
    screen = RecordingScreen()
    c = Controller(Paths().ensure(), Settings(user_name="Sam Lee"), FakePlatform(), screen, voice=False, index=False)
    return c, screen


def test_the_page_s_search_and_opening_what_it_found(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)
    notes = tmp_path / "notes.txt"
    notes.write_text("hello")
    c.index = FakeIndex([{"path": str(notes), "kind": "document"}])

    async def run():
        c.loop = asyncio.get_running_loop()
        c.page_event({"type": "search", "q": "notes", "n": 3})
        c.page_event({"type": "open_file", "path": str(notes)})
        c.page_event({"type": "open_file", "path": str(c.paths.secrets / "claude_token")})
        c.page_event({"type": "open_file", "path": str(tmp_path / "gone.txt")})
        await asyncio.sleep(0.3)
    asyncio.run(run())
    n, q, items, note = [e[1] for e in screen.events if e[0] == "search"][0]
    assert (n, q, [i["name"] for i in items], note) == (3, "notes", ["notes.txt"], "")
    assert ("show_html", "notes.txt") in screen.events and screen.mode == "content"
    notices = [e[1][0] for e in screen.events if e[0] == "notice"]
    assert any("stays private" in t for t in notices) and "That file isn't there any more." in notices


def test_another_microphone_by_asking(tmp_path):
    paths = Paths(tmp_path / "ChatComputer").ensure()
    reopened = []
    platform = FakePlatform()
    ctx = T.ToolContext(paths, Settings(), platform, Journal(paths), RecordingScreen(), Memory(paths),
                        on_mic_changed=lambda: reopened.append(True))
    T.build(ctx)
    said = call(ctx, "sound", {"action": "input", "value": "webcam"})
    assert said == "Listening with webcam now." and reopened == [True] and ("set_input", ("webcam",)) in platform.calls
    assert call(ctx, "sound", {"action": "input"}) == "Which microphone? Available: AirPods Pro"
    from chatcomputer.done import describe
    assert describe("mcp__chatcomputer__sound", {"action": "input", "value": "webcam"}, said) == \
        ("Listened with the microphone webcam", False)


def call(ctx, name, args):
    out = asyncio.run(ctx.tool_fns[name](args))
    return "\n".join(x["text"] for x in out["content"])


def test_another_model_from_the_next_message(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)
    switched = []

    async def switch():
        switched.append(True)
    c._switch_model_when_idle = switch
    c.brain = SimpleNamespace()
    c.model = SimpleNamespace()

    async def run():
        c.loop = asyncio.get_running_loop()
        c._settings_changed(["model"])
        c._settings_changed(["effort"])                   # (from the next question by itself)
        c._settings_changed(["model"], restart=False)
        c._settings_changed(["user_name"])
        await asyncio.sleep(0.05)
    asyncio.run(run())
    assert switched == [True]
    assert json.dumps(screen.events)                      # (nothing on the screen needs to change for effort)
