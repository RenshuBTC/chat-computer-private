"""What's done (chatcomputer/done.py): what Chat Computer did for the person, in plain words, each linked to the message
that asked for it; shown in a panel at the right of the conversation and read by recent_actions."""
import asyncio
import json
import time
from types import SimpleNamespace

from claude_agent_sdk import (AssistantMessage, ServerToolResultBlock, ServerToolUseBlock, ToolResultBlock, ToolUseBlock,
                              UserMessage)

from chatcomputer import tools as T
from chatcomputer.brain.agent import Brain, BrainEvents
from chatcomputer.config import Paths, Settings, model_label
from chatcomputer.controller import Controller
from chatcomputer.done import History, describe
from chatcomputer.journal import Journal
from chatcomputer.memory import Memory
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen

OURS = "mcp__chatcomputer__"


def test_what_was_done_is_said_in_plain_words():
    assert describe(OURS + "show_website", {"url": "https://www.youtube.com/watch?v=abc"}) == ("Opened youtube.com", False)
    assert describe(OURS + "show_website", {"url": "https://www.google.com/search?q=laksa+near+me"}) == \
        ("Searched google.com for “laksa near me”", False)
    assert describe(OURS + "show_website", {"url": "https://www.youtube.com/results?search_query=lofi"}) == \
        ("Searched youtube.com for “lofi”", False)
    assert describe("WebSearch", {"query": "trains on Sunday"}) == ("Searched the web for “trains on Sunday”", False)
    assert describe("Write", {"file_path": "/home/ren/notes.txt"}) == ("Saved notes.txt", False)
    assert describe("Edit", {"file_path": "/home/ren/plan.md"}) == ("Edited plan.md", False)
    assert describe(OURS + "move_files", {"items": [{"from": "/a/b.pdf", "to": "/c"}]}) == ("Moved b.pdf", False)
    assert describe(OURS + "trash_files", {"paths": ["/a/1", "/a/2", "/a/3", "/a/4"]}) == \
        ("Moved 1, 2, 3 and 1 more to the trash", False)
    assert describe(OURS + "wifi", {"action": "connect", "network": "Home"}, "Connected to Home.") == \
        ("Joined the Wi-Fi network Home", False)
    assert describe(OURS + "sound", {"action": "set_volume", "value": 40}, "Volume is 40%.") == \
        ("Set the volume to 40%", False)
    assert describe(OURS + "software", {"action": "install", "packages": ["vlc"]}, "Installed vlc.") == \
        ("Installed vlc", False)
    assert describe(OURS + "bluetooth", {"action": "airpods"}, "Your AirPods Pro are connected.") == \
        ("Connected your AirPods", False)
    assert describe("Bash", {"command": "apt-get install -y ffmpeg", "description": "Install ffmpeg"}) == \
        ("Install ffmpeg", False)
    # Steps that only looked are kept for recent_actions, not listed.
    assert describe("Bash", {"command": "ls ~/Downloads", "description": "List the downloads"}) == \
        ("List the downloads", True)
    assert describe("Read", {"file_path": "/x/y.txt"}) == ("Read y.txt", True)
    assert describe(OURS + "click", {"text": "Subscribe"})[1] is True
    assert describe(OURS + "type_text", {"text": "hello", "submit": True}) == \
        ("Typed “hello” and pressed Enter", False)


def test_failures_and_looking_arent_kept():
    # Chat Computer's own tools say in words when something didn't work.
    assert describe(OURS + "wifi", {"action": "connect", "network": "Home"}, "Failed: Secrets were required") is None
    assert describe(OURS + "sound", {"action": "output", "value": "TV"}, "I couldn't find that sound output.") is None
    assert describe(OURS + "software", {"action": "install", "packages": ["x"]}, "Install failed: no such package") is None
    assert describe(OURS + "bluetooth", {"action": "airpods"}, "Your AirPods are already connected.") is None
    for name, inp in (("read_screen", {}), ("system_status", {}), ("wifi", {"action": "status"}),
                      ("memory", {"action": "list"}), ("confirm_with_user", {"question": "Go?"}),
                      ("stop_listening", {}), ("chatcomputer_settings", {"action": "get"}), ("recent_actions", {})):
        assert describe(OURS + name, inp, "fine") is None, name


def test_the_list_keeps_what_was_done_and_finds_it(tmp_path):
    h = History(Paths(tmp_path).ensure())
    h.add("Opened youtube.com", OURS + "show_website", {"url": "https://youtube.com"}, prompt=("u1", "play lofi", 1.0))
    h.add("Read notes.txt", "Read", {"file_path": "/home/ren/notes.txt"}, minor=True)
    h.add("Saved notes.txt", "Write", {"file_path": "/home/ren/notes.txt"}, prompt=("u2", "write my list", 2.0))
    shown = h.recent()
    assert [e["text"] for e in shown] == ["Opened youtube.com", "Saved notes.txt"]          # not the minor step
    assert shown[0]["prompt_id"] == "u1" and shown[0]["prompt_ts"] == 1.0 and set(shown[0]) == {
        "id", "ts", "text", "prompt_id", "prompt_ts"}
    rows = h.find()
    assert len(rows) == 2 and rows[-1].endswith("Saved notes.txt (asked: “write my list”)")
    # Words are looked for in what was done, its details (the file's path) and the message; minor steps too.
    assert len(h.find("notes")) == 2 and len(h.find("home ren notes txt")) == 2 and h.find("lofi")[0].count("youtube")
    assert h.find("nothing like this") == []
    # Older than asked for: left out.
    old = json.loads(h.file.read_text().splitlines()[0])
    old["ts"] -= 10 * 86400
    h.file.write_text("\n".join([json.dumps(old)] + h.file.read_text().splitlines()[1:]) + "\n")
    assert [r.split("  ")[1] for r in h.find(days=7)] == ["Saved notes.txt (asked: “write my list”)"]
    assert len(h.find(days=30)) == 2


def test_an_earlier_message_is_found_in_the_transcript(tmp_path):
    paths = Paths(tmp_path).ensure()
    lines = []
    for i in range(10):
        lines.append({"ts": 100.0 + i * 10, "role": "user", "text": f"q{i}", **({"id": f"u{i}"} if i >= 5 else {})})
        lines.append({"ts": 105.0 + i * 10, "role": "assistant", "text": f"a{i}"})
    paths.transcript.write_text("\n".join(json.dumps(x) for x in lines) + "\n{broken\n")
    h = History(paths)
    # By id, up to where the screen's messages start.
    got = h.earlier("u6", 0, 180.0)
    assert [m["text"] for m in got] == ["q6", "a6", "q7", "a7"] and got[0]["id"] == "u6"
    # Without an id (older transcripts): the last message sent by then.
    assert [m["text"] for m in h.earlier(None, 133.0, 150.0)] == ["q3", "a3", "q4", "a4"]
    # Too many: the ones in the middle are left out, and a line says how many.
    got = h.earlier("u5", 0, None, limit=4)
    assert [m["text"] for m in got] == ["q5", "a5", "6 more messages", "q9", "a9"] and got[2]["role"] == "gap"
    assert History(Paths(tmp_path / "none").ensure()).earlier("x", 0, None) == []


def test_the_brain_reports_tool_calls_when_they_finish():
    got = []
    brain = Brain(Paths(), Settings(), SimpleNamespace(get=lambda k, d=None: d, set=lambda **k: None), None, {}, None,
                  lambda: "", BrainEvents(on_tool=lambda *a: got.append(a)))
    brain._cur = brain._new_turn("here")
    tid = brain._cur.tid
    brain._route(AssistantMessage(content=[
        ToolUseBlock(id="t1", name="WebSearch", input={"query": "trains"}),
        ToolUseBlock(id="t2", name="mcp__chatcomputer__wifi", input={"action": "connect", "network": "Home"}),
        ServerToolUseBlock(id="s1", name="web_search", input={"query": "laksa"})], model="m"))
    assert got == []                            # nothing finished yet
    brain._route(UserMessage(content=[ToolResultBlock(tool_use_id="t1", content="results", is_error=False),
                                      ToolResultBlock(tool_use_id="t2", content=[{"type": "text", "text": "Failed: no"}],
                                                      is_error=True)],
                             tool_use_result={"x": 1}))
    brain._route(AssistantMessage(content=[ServerToolResultBlock(tool_use_id="s1", content={"type": "web_search_result"})],
                                  model="m"))
    assert got == [(tid, "WebSearch", {"query": "trains"}, True, "results"),
                   (tid, "mcp__chatcomputer__wifi", {"action": "connect", "network": "Home"}, False, "Failed: no"),
                   (tid, "web_search", {"query": "laksa"}, True, "")]
    brain._route(UserMessage(content=[ToolResultBlock(tool_use_id="t1", content="again")], tool_use_result={}))
    assert len(got) == 3                        # each once


def _controller(tmp_path, monkeypatch):
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "ChatComputer"))
    screen = RecordingScreen()
    c = Controller(Paths().ensure(), Settings(user_name="Ren Shu"), FakePlatform(), screen, voice=False, index=False)
    return c, screen


def test_whats_done_is_linked_to_the_message_that_asked(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)
    asked = []

    async def ask(text, typed=False):
        asked.append(text)
        return "t7"
    c.brain = SimpleNamespace(busy=False, last_activity=0.0, ask=ask, current_origin="here")
    c._online_now = lambda: asyncio.sleep(0, True)

    async def run():
        c.loop = asyncio.get_running_loop()
        await c.handle("play some lofi on youtube", typed=True)
        c._on_tool("t7", OURS + "show_website", {"url": "https://www.youtube.com/results?search_query=lofi"}, True,
                   "Opened")
        c._on_tool("t7", OURS + "read_screen", {}, True, "...")                # looking: not kept
        c._on_tool("t7", "WebSearch", {"query": "x"}, False, "error")          # failed: not kept
        c._on_tool("t7", "Read", {"file_path": "/a/b.txt"}, True, "...")       # minor: kept, not shown
    asyncio.run(run())
    uid, text, _ = [e[1] for e in screen.events if e[0] == "user"][-1]
    done = [e[1] for e in screen.events if e[0] == "done"]
    assert text == "play some lofi on youtube" and len(done) == 1
    assert done[0]["text"] == "Searched youtube.com for “lofi”" and done[0]["prompt_id"] == uid
    # The transcript has that message with the same id and time, so it can be found again on another day.
    said = [json.loads(x) for x in c.paths.transcript.read_text().splitlines()]
    assert said[-1]["id"] == uid and said[-1]["ts"] == done[0]["prompt_ts"]
    assert [e["text"] for e in c.history.recent()] == ["Searched youtube.com for “lofi”"]
    assert len(c.history.find("b.txt")) == 1


def test_something_typed_on_the_phone_is_the_message_asked_there(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)
    c.gate = SimpleNamespace(phone_reply=lambda t: None)
    c._on_user_text("p3", "turn the volume down")
    c._on_tool("p3", OURS + "sound", {"action": "volume_down"}, True, "Volume is 30%.")
    uid = [e[1] for e in screen.events if e[0] == "user"][-1][0]
    done = [e[1] for e in screen.events if e[0] == "done"]
    assert done[0]["text"] == "Turned the volume down" and done[0]["prompt_id"] == uid


def test_the_screen_gets_whats_done_and_can_go_back(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)
    c.history.add("Opened youtube.com", prompt=("u1", "play lofi", time.time() - 86400))
    c.paths.transcript.write_text(json.dumps({"ts": time.time() - 86400, "role": "user", "id": "u1",
                                              "text": "play lofi"}) + "\n")

    async def run():
        c.loop = asyncio.get_running_loop()
        await c._show_done()
        await c.go_back_to("u1", time.time() - 86400, None)
    asyncio.run(run())
    assert ("done_list", ["Opened youtube.com"]) in screen.events
    assert ("earlier", ([("user", "play lofi")], "u1")) in screen.events


def test_recent_actions_reads_whats_done(tmp_path):
    paths = Paths(tmp_path / "ChatComputer").ensure()
    h = History(paths)
    h.add("Installed vlc", OURS + "software", {"action": "install", "packages": ["vlc"]},
          prompt=("u1", "install vlc", 1.0))
    ctx = T.ToolContext(paths, Settings(), FakePlatform(), Journal(paths), RecordingScreen(), Memory(paths), history=h)
    server, _, _ = T.build(ctx)
    entry = server["instance"].get_request_handler("tools/call")

    def call(args):
        out = asyncio.run(entry.handler(None, entry.params_type(name="recent_actions", arguments=args)))
        return "\n".join(x.text for x in out.content)
    assert call({}).endswith("Installed vlc (asked: “install vlc”)")
    assert call({"query": "vlc"}).endswith("Installed vlc (asked: “install vlc”)")
    assert call({"query": "chrome"}) == "Nothing done in that time with those words."


def test_the_model_is_named_as_the_claude_app_names_it():
    assert model_label("claude-sonnet-5-5") == "Sonnet 5.5"
    assert model_label("claude-opus-5-5") == "Opus 5.5"
    assert model_label("claude-fable-5-1") == "Fable 5.1"
    assert model_label("claude-opus-4-20250514") == "Opus 4"
    assert model_label("opus") == "Opus"
    assert model_label("claude-sonnet-5-5[1m]") == "Sonnet 5.5"
    assert model_label("") == ""


def test_words_after_a_tool_call_are_not_glued_to_the_words_before():
    # Claude writes "Checking the drive." then calls a tool, then "It's open." in a new block of text: on the screen
    # and out loud that's two sentences, not "drive.It's open".
    from claude_agent_sdk.types import StreamEvent

    shown = []
    brain = Brain(Paths(), Settings(), SimpleNamespace(get=lambda k, d=None: d, set=lambda **k: None), None, {}, None,
                  lambda: "", BrainEvents(on_text=lambda tid, t: shown.append(t)))
    brain._cur = brain._new_turn("here")

    def ev(e):
        brain._route(StreamEvent(uuid="u", session_id="s", event=e))
    for block in ("Checking the drive.", "It's open.", " Your photos are on it.", "Done\n", "Bye"):
        ev({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})
        ev({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": block}})
    ev({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "t", "name": "x"}})
    assert brain._cur.res.text == "Checking the drive. It's open. Your photos are on it. Done\nBye"
    assert "".join(shown) == brain._cur.res.text
