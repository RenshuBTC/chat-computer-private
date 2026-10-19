"""End to end: the real Claude Code engine (bundled with the Agent SDK) talking to a mock
Claude API, driving the real ClaudeOS tools and permission gate."""
import asyncio
import time
from pathlib import Path

import pytest

from mock_anthropic import MockAnthropic, last_message_is_tool_result, last_user_text, text_block, tool_results, tool_use_block

from claudeos import tools as T
from claudeos.brain.agent import Brain, BrainEvents
from claudeos.brain.permissions import PermissionGate
from claudeos.config import Paths, Settings, State
from claudeos.journal import Journal
from claudeos.memory import Memory
from claudeos.platform.fake import FakePlatform
from claudeos.screen import RecordingScreen

TOKEN = "sk-ant-api03-" + "x" * 40


class Rig:
    def __init__(self, home: Path, answers: list[tuple[str, str]]):
        self.mock = MockAnthropic()
        self.mock.start()
        self.paths = Paths(home / "ClaudeOS").ensure()
        self.settings = Settings(user_name="Ren Shu")
        self.state = State(self.paths)
        self.screen = RecordingScreen()
        self.platform = FakePlatform()
        self.journal = Journal(self.paths)
        self.asked: list[str] = []
        self.answers = answers
        self.events: list[tuple] = []
        self.done = asyncio.Event()

        async def confirm(q):
            self.asked.append(q)
            return self.answers.pop(0) if self.answers else ("timeout", "")

        self.ctx = T.ToolContext(self.paths, self.settings, self.platform, self.journal, self.screen, Memory(self.paths),
                                 confirm=confirm)
        server, policies, _ = T.build(self.ctx)
        self.gate = PermissionGate(self.paths, self.journal, confirm, policies)
        ev = BrainEvents(
            on_turn_start=lambda t: self.events.append(("start", t)),
            on_text=lambda t, d: self.events.append(("text", d)),
            on_activity=lambda t, a: self.events.append(("activity", a)),
            on_turn_end=lambda t, r: (self.events.append(("end", r)), self.done.set()),
        )
        self.brain = Brain(self.paths, self.settings, self.state, TOKEN, server, self.gate,
                           lambda: "You are ClaudeOS (test).", ev,
                           extra_env={"ANTHROPIC_BASE_URL": self.mock.url}, cwd=str(home))

    async def say(self, text: str, timeout: float = 60):
        self.done.clear()
        await self.brain.ask(text)
        await asyncio.wait_for(self.done.wait(), timeout)
        return [e for e in self.events if e[0] == "end"][-1][1]

    def text(self) -> str:
        return "".join(e[1] for e in self.events if e[0] == "text")


def script(mock: MockAnthropic, steps: dict[str, list[dict]], after_tool: str = "Done."):
    def rule(req):
        if last_message_is_tool_result(req):
            return [text_block(after_tool)]
        said = last_user_text(req)
        for key, blocks in steps.items():
            if key in said:
                return blocks
        return None
    mock.rules.append(rule)


@pytest.mark.timeout(120)
def test_voice_request_opens_a_website_and_streams_words(clean_env):
    async def go():
        rig = Rig(clean_env, [])
        script(rig.mock, {"open youtube": [text_block("Opening YouTube."),
                                           tool_use_block("mcp__claudeos__show_website", {"url": "youtube.com"})]},
               after_tool="It's on screen now.")
        try:
            res = await rig.say("open youtube")
            assert res.ok, res
            assert rig.screen.events[0] == ("show_url", "https://youtube.com")
            assert ("activity", "Opening youtube.com") in rig.events
            assert "Opening YouTube." in rig.text() and "It's on screen now." in rig.text()
            assert rig.asked == []                     # showing a website needs no permission
            # Every request carries the ClaudeOS tools up front (no deferred tool search).
            names = [t.get("name") for t in rig.mock.requests[0]["tools"]]
            assert "mcp__claudeos__show_website" in names and "Bash" in names
            assert not any(n and n.startswith("mcp__claude-in-chrome") for n in names)
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(120)
def test_trash_needs_a_spoken_yes_and_can_be_undone(clean_env):
    async def go():
        rig = Rig(clean_env, [("yes", "yes")])
        f = clean_env / "old.zip"
        f.write_bytes(b"zip")
        script(rig.mock, {"delete": [tool_use_block("mcp__claudeos__trash_files", {"paths": [str(f)]})]})
        try:
            res = await rig.say("delete old zip")
            assert res.ok
            assert rig.asked == ["Move 1 item to the trash: old.zip?"]
            assert not f.exists()
            msgs = rig.journal.undo_last(1)
            assert f.exists(), msgs
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(120)
def test_saying_no_stops_a_risky_command_and_claude_hears_why(clean_env):
    async def go():
        rig = Rig(clean_env, [("no", "no don't, I still need it")])
        script(rig.mock, {"clean": [tool_use_block("Bash", {"command": "rm -r ~/Downloads/stuff", "description": "Delete the stuff folder"})]})
        try:
            await rig.say("clean my downloads")
            assert rig.asked == ["Can I run a command to delete the stuff folder?"]
            results = tool_results(rig.mock.requests[-1])
            assert results and "I still need it" in str(results[0])
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(120)
def test_conversation_resumes_after_restart(clean_env):
    async def go():
        rig = Rig(clean_env, [])
        try:
            await rig.say("my favourite colour is green")
            sid = rig.state.get("session_id")
            assert sid
            await rig.brain.close()
            # A new process: same state file → same conversation.
            rig.state = State(rig.paths)
            rig.brain.state = rig.state
            rig.brain.session_id = None
            await rig.say("what did I say?")
            convo = [m for m in rig.mock.requests[-1]["messages"] if m["role"] in ("user", "assistant")]
            assert any("favourite colour is green" in str(m) for m in convo)
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(120)
def test_speaking_over_claude_interrupts_the_running_task(clean_env):
    async def go():
        rig = Rig(clean_env, [("yes", "yes")])
        script(rig.mock, {"long job": [text_block("Starting."), tool_use_block("Bash", {"command": "sleep 20", "description": "Wait a while"})]})
        try:
            rig.done.clear()
            await rig.brain.ask("do the long job")
            await asyncio.sleep(3)
            assert rig.brain.busy
            t0 = time.time()
            await rig.brain.ask("actually stop and say hi")
            for _ in range(300):
                ends = [e[1] for e in rig.events if e[0] == "end"]
                if len(ends) >= 2:
                    break
                await asyncio.sleep(0.1)
            assert len(ends) == 2
            assert ends[0].interrupted
            assert time.time() - t0 < 15
            # The new words reach Claude in the very next request, after the interrupted tool's result.
            assert "say hi" in last_user_text(rig.mock.requests[-1])
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(180)
def test_a_model_the_api_doesnt_know_is_reported_as_such(clean_env):
    async def go():
        rig = Rig(clean_env, [])
        rig.settings.model = "claude-no-such-model-9"
        rig.mock.unknown_models.add("claude-no-such-model-9")
        try:
            res = await rig.say("hello", timeout=150)
            assert not res.ok and res.error_kind == "model", (res.error_kind, res.error_text)
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())
