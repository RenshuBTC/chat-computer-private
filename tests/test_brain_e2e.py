"""End to end: the brain's own agent loop (brain/agent.py) talking to a stand-in for the model's server (the same
API), driving the real Chat Computer tools and permission gate."""
import asyncio
import json
import time
from pathlib import Path

import pytest

from mock_model import FakeModelService, MockModel, call, last_is_tool_result, last_user_text, tool_results

from chatcomputer import tools as T
from chatcomputer.brain.agent import Brain, BrainEvents
from chatcomputer.brain.permissions import PermissionGate
from chatcomputer.config import Paths, Settings, State
from chatcomputer.journal import Journal
from chatcomputer.memory import Memory
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen


class Rig:
    def __init__(self, home: Path, answers: list[tuple[str, str]], ready: bool = True):
        self.mock = MockModel()
        self.mock.start()
        self.paths = Paths(home / "ChatComputer").ensure()
        self.settings = Settings(user_name="Sam Lee")
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
        specs, policies, _ = T.build(self.ctx)
        self.gate = PermissionGate(self.paths, self.journal, confirm, policies)
        ev = BrainEvents(
            on_turn_start=lambda t: self.events.append(("start", t)),
            on_text=lambda t, d: self.events.append(("text", d)),
            on_activity=lambda t, a: self.events.append(("activity", a)),
            on_turn_end=lambda t, r: (self.events.append(("end", r)), self.done.set()),
            on_tool=lambda t, n, i, ok, r, u: self.events.append(("tool", n, ok)),
        )
        self.model = FakeModelService(self.mock, ready=ready)
        self.brain = Brain(self.paths, self.settings, self.state, self.model, specs, self.ctx.tool_fns, self.gate,
                           lambda: "You are Chat Computer (test).", ev, cwd=str(home))

    async def say(self, text: str, timeout: float = 60):
        self.done.clear()
        await self.brain.ask(text)
        await asyncio.wait_for(self.done.wait(), timeout)
        return [e for e in self.events if e[0] == "end"][-1][1]

    def text(self) -> str:
        return "".join(e[1] for e in self.events if e[0] == "text")


def script(mock: MockModel, steps: dict[str, list], after_tool: str = "Done."):
    def rule(req):
        if last_is_tool_result(req):
            return [after_tool]
        said = last_user_text(req)
        for key, parts in steps.items():
            if key in said:
                return parts
        return None
    mock.rules.append(rule)


@pytest.mark.timeout(60)
def test_voice_request_opens_a_website_and_streams_words(clean_env):
    async def go():
        rig = Rig(clean_env, [])
        script(rig.mock, {"open youtube": ["Opening YouTube.", call("show_website", url="youtube.com")]},
               after_tool="It's on screen now.")
        try:
            res = await rig.say("open youtube")
            assert res.ok, res
            assert rig.screen.events[0] == ("show_url", "https://youtube.com")
            assert ("activity", "Opening youtube.com") in rig.events
            assert "Opening YouTube." in rig.text() and "It's on screen now." in rig.text()
            assert rig.text() == "Opening YouTube. It's on screen now."   # carried on after the tool, with a space
            assert rig.asked == []                     # showing a website needs no permission
            assert ("tool", "mcp__chatcomputer__show_website", True) in rig.events
            # Every request carries Chat Computer's tools and the everyday ones, with the system prompt first.
            first = rig.mock.requests[0]
            names = [t["function"]["name"] for t in first["tools"]]
            assert "show_website" in names and "Bash" in names and names.index("show_website") < names.index("Bash")
            assert first["messages"][0] == {"role": "system", "content": "You are Chat Computer (test)."}
            # The tool's result went back with its call's id.
            second = rig.mock.requests[1]["messages"]
            assert second[-2]["tool_calls"][0]["function"]["name"] == "show_website"
            assert second[-1]["role"] == "tool" and second[-1]["tool_call_id"] == second[-2]["tool_calls"][0]["id"]
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(60)
def test_trash_needs_a_spoken_yes_and_can_be_undone(clean_env):
    async def go():
        rig = Rig(clean_env, [("yes", "yes")])
        f = clean_env / "old.zip"
        f.write_bytes(b"zip")
        script(rig.mock, {"delete": [call("trash_files", paths=[str(f)])]})
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


@pytest.mark.timeout(60)
def test_saying_no_stops_a_risky_command_and_the_model_hears_why(clean_env):
    async def go():
        rig = Rig(clean_env, [("no", "no don't, I still need it")])
        target = clean_env / "Downloads" / "stuff"
        target.mkdir(parents=True)
        script(rig.mock, {"clean": [call("Bash", command=f"rm -r {target}", description="Delete the stuff folder")]})
        try:
            await rig.say("clean my downloads")
            assert rig.asked == ["Can I run a command to delete the stuff folder?"]
            results = tool_results(rig.mock.requests[-1])
            assert results and "I still need it" in results[0]
            assert target.exists()
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(60)
def test_a_command_runs_and_its_output_comes_back(clean_env):
    async def go():
        rig = Rig(clean_env, [])
        (clean_env / "notes.txt").write_text("one\ntwo\n")
        script(rig.mock, {"count": [call("Bash", command="wc -l notes.txt", description="Count the lines")]})
        try:
            res = await rig.say("count the lines in my notes")
            assert res.ok
            assert rig.asked == []                     # only reads: no question
            assert "2 notes.txt" in tool_results(rig.mock.requests[-1])[0]
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(60)
def test_conversation_resumes_after_restart(clean_env):
    async def go():
        rig = Rig(clean_env, [])
        try:
            await rig.say("my favourite colour is green")
            sid = rig.state.get("session_id")
            assert sid
            await rig.brain.close()
            # A new process: the same state file, so the same conversation, with the same system prompt.
            rig.state = State(rig.paths)
            rig.brain.state = rig.state
            rig.brain.session_id = None
            rig.brain.conversation = None
            await rig.say("what did I say?")
            msgs = rig.mock.requests[-1]["messages"]
            assert any("favourite colour is green" in str(m.get("content")) for m in msgs)
            assert msgs[0]["content"] == "You are Chat Computer (test)."
            assert rig.brain.session_id == sid
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(60)
def test_speaking_over_it_interrupts_the_running_task(clean_env):
    async def go():
        rig = Rig(clean_env, [("yes", "yes")])
        script(rig.mock, {"long job": ["Starting.", call("Bash", command="sleep 20", description="Wait a while")]})
        try:
            rig.done.clear()
            await rig.brain.ask("do the long job")
            await asyncio.sleep(2)
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
            assert time.time() - t0 < 10
            # The new words reach the model in the very next request, after a note that it was interrupted.
            msgs = rig.mock.requests[-1]["messages"]
            assert msgs[-1]["content"] == "actually stop and say hi"
            assert "[Request interrupted by user]" in [m.get("content") for m in msgs]
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(60)
def test_a_model_still_being_got_ready_says_so(clean_env):
    async def go():
        rig = Rig(clean_env, [], ready=False)
        try:
            res = await rig.say("hello")
            assert not res.ok and res.error_kind == "model_not_ready"
            assert "40%" in res.error_text
            assert rig.model.prepared == 1             # and it's being got ready
            assert rig.mock.requests == []
            # The question isn't kept (it's asked again once the model is ready).
            conv = json.loads((rig.paths.engine / "conversations" / f"{rig.brain.session_id}.json").read_text())
            assert conv["messages"] == []
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(60)
def test_pictures_are_said_to_be_pictures(clean_env):
    async def go():
        rig = Rig(clean_env, [])
        script(rig.mock, {"look": [call("look_at_screen")]})
        try:
            await rig.say("look at the screen")
            assert "reads text only" in tool_results(rig.mock.requests[-1])[0]
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(60)
def test_the_same_tool_asked_for_again_and_again_stops(clean_env):
    """A small model can ask for the same tool, with the same input, round after round (evals/os: Qwen3.5 0.8B showed
    the same folder thirty times until the turn ran out of time). Asked again, it isn't run again: its answer is given
    back with a word to try something else or answer; a second step in a row asking for nothing new ends the turn,
    saying so."""
    async def go():
        rig = Rig(clean_env, [])
        (clean_env / "notes.txt").write_text("one\ntwo\n")
        same = call("Bash", command="wc -l notes.txt", description="Count the lines")
        rig.mock.rules.append(lambda req: [same])          # every time, whatever it's told
        try:
            res = await rig.say("count the lines in my notes")
            ran = [e for e in rig.events if e[0] == "tool"]
            assert len(ran) == 1                           # run once, not again and again
            assert not res.ok and res.error_kind == "circles"
            told = tool_results(rig.mock.requests[1])
            assert told and "2 notes.txt" in told[-1]
            nudged = tool_results(rig.mock.requests[2])
            assert nudged[-1].startswith("Already done") and "try something else" in nudged[-1]
            assert len(rig.mock.requests) == 3             # once nudged, the same again: stopped
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(60)
def test_a_tool_s_own_answer_is_said_when_the_model_says_nothing(clean_env):
    """A tool that knows the answer itself gives it (`say`): it's said if the model ends without a word, or goes round
    in circles after it (evals/os: told there's no dark mode, Qwen3.5 0.8B asked for the same again until stopped).
    Anything done after it may change the answer: then it isn't said."""
    async def go():
        rig = Rig(clean_env, [])
        same = call("display", action="dark_mode")
        rig.mock.rules.append(lambda req: [same])          # every time, whatever it's told
        try:
            res = await rig.say("turn on dark mode")
            assert res.ok and res.error_kind is None and len(rig.mock.requests) == 3
            assert res.text.startswith("There's no dark or light mode") and rig.text() == res.text
        finally:
            await rig.brain.close()
            rig.mock.stop()
        rig = Rig(clean_env, [])
        (clean_env / "notes.txt").write_text("one\ntwo\n")
        script(rig.mock, {"count": [call("display", action="dark_mode"),
                                    call("Bash", command="wc -l notes.txt", description="Count the lines")],
                          "dark": [call("display", action="dark_mode")]}, after_tool="")
        try:
            res = await rig.say("dark mode please")
            assert res.ok and res.text.startswith("There's no dark or light mode")
            res = await rig.say("dark mode, and count the lines in my notes")
            assert res.ok and res.text == "Done."
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())


@pytest.mark.timeout(60)
def test_a_turn_that_does_something_and_says_nothing_says_so(clean_env):
    """A small model can do what was asked and end without a word (evals/os: the tax letter shown, nothing said): the
    reply says it's done, rather than nothing."""
    async def go():
        rig = Rig(clean_env, [])
        (clean_env / "notes.txt").write_text("one\ntwo\n")
        script(rig.mock, {"count": [call("Bash", command="wc -l notes.txt", description="Count the lines")]}, after_tool="")
        try:
            res = await rig.say("count the lines in my notes")
            assert res.ok and res.text == "Done." and rig.text() == "Done."
        finally:
            await rig.brain.close()
            rig.mock.stop()
    asyncio.run(go())
