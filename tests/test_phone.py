"""Messaging ClaudeOS from the Claude app on the phone (Claude Code's Remote Control).

The engine is replaced by a stand-in that behaves like Claude Code's stream: a turn typed on the
phone arrives without ClaudeOS asking anything. The real link to Anthropic's servers can only be
tried with a real account, so it isn't covered here."""
import asyncio
import json
import os
from types import SimpleNamespace

import pytest
from claude_agent_sdk import ResultMessage, SystemMessage, UserMessage
from claude_agent_sdk.types import StreamEvent

from claudeos.brain.agent import Brain, BrainEvents
from claudeos.brain.permissions import PermissionGate
from claudeos.config import Paths, Settings, State, has_login, login_valid, save_login
from claudeos.controller import Controller
from claudeos.journal import Journal
from claudeos.platform.fake import FakePlatform
from claudeos.screen import RecordingScreen

TOKEN = "sk-ant-oat01-" + "x" * 60
LOGIN = json.dumps({"claudeAiOauth": {"accessToken": "a" * 20, "refreshToken": "r" * 20, "expiresAt": 1}})


def text(t, sid="s1"):
    return StreamEvent(uuid="u", session_id=sid, event={"type": "content_block_delta",
                                                         "delta": {"type": "text_delta", "text": t}})


def result(sid="s1", aborted=False, error=False, status=None):
    return ResultMessage(subtype="error_during_execution" if error else "success", duration_ms=1, duration_api_ms=1,
                         is_error=error, num_turns=1, session_id=sid, api_error_status=status,
                         terminal_reason="aborted_streaming" if aborted else None)


class FakeEngine:
    """Stands in for ClaudeSDKClient: replies to what's asked here, and can start turns on its own."""

    def __init__(self, options):
        self.options = options
        self.q: asyncio.Queue = asyncio.Queue()
        self.sent: list[str] = []
        self.controls: list[dict] = []
        self.control_results: list = [{"session_id": "rc_123"}]
        self.reply = lambda prompt: [text("Hi there."), result()]
        self._query = SimpleNamespace(_send_control_request=self._control)

    async def connect(self):
        pass

    async def disconnect(self):
        self.q.put_nowait(None)

    async def query(self, prompt):
        self.sent.append(prompt)
        for m in self.reply(prompt):
            self.q.put_nowait(m)

    async def interrupt(self):
        self.q.put_nowait(result(aborted=True))

    async def receive_messages(self):
        while True:
            m = await self.q.get()
            if m is None:
                return
            yield m

    async def _control(self, req, timeout=60):
        self.controls.append(dict(req))
        r = self.control_results.pop(0) if self.control_results else {}
        if isinstance(r, Exception):
            raise r
        return r

    def emit(self, *msgs):
        for m in msgs:
            self.q.put_nowait(m)


class Rig:
    def __init__(self, tmp_path, *, login=True, token=TOKEN, phone=True):
        self.paths = Paths(tmp_path / "ClaudeOS").ensure()
        if login:
            save_login(self.paths, LOGIN)
        self.settings = Settings(phone_access=phone)
        self.events: list[tuple] = []
        self.engines: list[FakeEngine] = []

        def factory(options):
            e = FakeEngine(options)
            self.engines.append(e)
            return e

        ev = BrainEvents(
            on_turn_start=lambda t: self.events.append(("start", t)),
            on_text=lambda t, d: self.events.append(("text", t, d)),
            on_turn_end=lambda t, r: self.events.append(("end", t, r)),
            on_user_text=lambda t, s: self.events.append(("phone_text", t, s)),
            on_remote=lambda s, i: self.events.append(("remote", s, i)),
        )
        self.brain = Brain(self.paths, self.settings, State(self.paths), token, {}, None, lambda: "test", ev,
                           cwd=str(tmp_path), client_factory=factory)

    @property
    def engine(self) -> FakeEngine:
        return self.engines[-1]

    def kinds(self):
        return [e[0] for e in self.events]


async def settle(n=20):
    for _ in range(n):
        await asyncio.sleep(0)


def test_login_file_checks(tmp_path):
    assert login_valid(LOGIN)
    assert not login_valid("{}") and not login_valid("not json") and not login_valid(json.dumps({"claudeAiOauth": {}}))
    p = Paths(tmp_path / "C").ensure()
    assert not has_login(p)
    with pytest.raises(ValueError):
        save_login(p, "{}")
    save_login(p, LOGIN)
    assert has_login(p) and (p.login_file.stat().st_mode & 0o777) == 0o600
    assert p.login_file == p.engine / ".credentials.json"


def test_sign_in_modes_and_engine_settings(tmp_path):
    r = Rig(tmp_path / "a")
    assert r.brain.auth_mode == "login"
    os.environ["CLAUDE_CODE_OAUTH_TOKEN"] = "sk-ant-oat01-from-elsewhere"
    env = r.brain._env()
    assert "CLAUDE_CODE_OAUTH_TOKEN" not in env and "ANTHROPIC_API_KEY" not in env
    assert "CLAUDE_CODE_OAUTH_TOKEN" not in os.environ                # it would win over the full sign-in
    assert "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC" not in env      # Remote Control needs feature flags
    assert env["DISABLE_TELEMETRY"] == "1" and env["CLAUDE_CONFIG_DIR"].endswith(".engine")
    # The full sign-in fails: back to the one-year token.
    assert r.brain.use_token_instead() and r.brain.auth_mode == "token"
    env = r.brain._env()
    assert env["CLAUDE_CODE_OAUTH_TOKEN"] == TOKEN and env["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"] == "1"
    assert not r.brain.use_token_instead()
    # Phone access off: the one-year token stays the main sign-in.
    assert Rig(tmp_path / "b", phone=False).brain.auth_mode == "token"
    # Only the full sign-in: use it.
    assert Rig(tmp_path / "c", token=None).brain.auth_mode == "login"
    # Neither.
    assert Rig(tmp_path / "d", login=False, token=None).brain.auth_mode == "none"


def test_remote_control_is_switched_on_after_connecting(tmp_path):
    async def run():
        r = Rig(tmp_path)
        await r.brain.connect()
        await settle()
        assert r.engine.controls[0]["subtype"] == "remote_control" and r.engine.controls[0]["enabled"] is True
        assert r.engine.controls[0]["name"].startswith("ClaudeOS")
        assert r.brain.remote == "on" and ("remote", "on", {"session_id": "rc_123"}) in r.events
        assert r.brain.state.get("remote_session_id") == "rc_123"
        # Next start: ask to keep the same session on the phone.
        await r.brain.close()
        await r.brain.connect()
        await settle()
        assert r.engine.controls[0]["reattach_session_id"] == "rc_123"
        await r.brain.close()
    asyncio.run(run())


def test_remote_control_retries_without_an_old_session_and_reports_failure(tmp_path, monkeypatch):
    from claudeos.brain import agent
    monkeypatch.setattr(agent, "REMOTE_RETRY_S", (0.0, 0.0))

    async def run():
        r = Rig(tmp_path)
        r.brain.state.set(remote_session_id="gone")
        scoped = Exception("Remote Control requires a full-scope login token")

        def factory(options):
            e = FakeEngine(options)
            e.control_results = [Exception("session not found"), scoped, scoped, scoped]
            r.engines.append(e)
            return e
        r.brain.client_factory = factory
        await r.brain.connect()
        await settle(200)
        # The old session first, then a new one; tried three times in all before giving up.
        assert [c.get("reattach_session_id") for c in r.engine.controls] == ["gone", None, None, None]
        assert r.brain.remote == "failed" and "full-scope" in r.brain.remote_detail
        assert r.events[-1][:2] == ("remote", "failed")
        assert [e for e in r.events if e[0] == "remote"] == [r.events[-1]]      # one report, at the end
        await r.brain.close()
    asyncio.run(run())


def test_remote_control_started_again_when_the_engine_didnt_know_the_account_yet(tmp_path, monkeypatch):
    # On the engine's first start it looks up the account in the background, and refuses Remote Control until it
    # has: a little later the same request works.
    from claudeos.brain import agent
    monkeypatch.setattr(agent, "REMOTE_RETRY_S", (0.0, 0.0))

    async def run():
        r = Rig(tmp_path)

        def factory(options):
            e = FakeEngine(options)
            e.control_results = [Exception("Unable to determine your organization for Remote Control eligibility."),
                                 {"session_id": "rc_9"}]
            r.engines.append(e)
            return e
        r.brain.client_factory = factory
        r.brain._engine_stderr("[bridge] eligibility: no_organization for sk-ant-oat01-abcdefgh\n")
        await r.brain.connect()
        await settle(200)
        assert len(r.engine.controls) == 2 and r.brain.remote == "on"
        assert r.brain.state.get("remote_session_id") == "rc_9"
        assert [e[:2] for e in r.events if e[0] == "remote"] == [("remote", "on")]
        assert r.brain.engine_tail() == "[bridge] eligibility: no_organization for sk-ant-(hidden)"
        await r.brain.close()
    asyncio.run(run())


def test_a_turn_typed_here_is_marked_as_typed(tmp_path):
    async def run():
        r = Rig(tmp_path, login=False)
        typed = await r.brain.ask("what's on today", typed=True)
        await r.brain.wait_idle()
        said = await r.brain.ask("and tomorrow?")
        await r.brain.wait_idle()
        assert r.brain.typed_here(typed) and r.brain.origin_of(typed) == "here"
        assert not r.brain.typed_here(said)
        await r.brain.close()
    asyncio.run(run())


def test_no_remote_control_with_the_token_or_when_turned_off(tmp_path):
    async def run():
        for rig in (Rig(tmp_path / "a", login=False), Rig(tmp_path / "b", phone=False)):
            await rig.brain.connect()
            await settle()
            assert rig.engine.controls == [] and rig.brain.remote == "off"
            await rig.brain.close()
    asyncio.run(run())


def test_a_turn_asked_here(tmp_path):
    async def run():
        r = Rig(tmp_path, login=False)
        tid = await r.brain.ask("[Mon · screen: conversation]\nhello")
        await r.brain.wait_idle()
        assert r.brain.origin_of(tid) == "here"
        assert r.kinds() == ["start", "text", "end"] and r.events[-1][2].ok
        assert r.engine.sent == ["[Mon · screen: conversation]\nhello"]
        await r.brain.close()
    asyncio.run(run())


def test_a_turn_typed_on_the_phone(tmp_path):
    async def run():
        r = Rig(tmp_path)
        await r.brain.connect()
        await settle()
        e = r.engine
        e.emit(SystemMessage(subtype="init", data={"session_id": "s9"}),
               UserMessage(content="what's the weather at home?", origin={"kind": "human"}))
        await settle()
        assert r.brain.busy and r.brain.current_origin == "phone"
        e.emit(text("Sunny, 31 degrees."), result(sid="s9"))
        await settle()
        assert not r.brain.busy
        starts = [ev for ev in r.events if ev[0] == "start"]
        tid = starts[0][1]
        assert r.brain.origin_of(tid) == "phone" and tid.startswith("p")
        assert ("phone_text", tid, "what's the weather at home?") in r.events
        assert ("text", tid, "Sunny, 31 degrees.") in r.events
        assert r.brain.session_id == "s9"
        await r.brain.close()
    asyncio.run(run())


def test_a_phone_turn_without_the_typed_words_still_counts_as_the_phone(tmp_path):
    async def run():
        r = Rig(tmp_path)
        await r.brain.connect()
        await settle()
        r.engine.emit(text("On it."), result())
        await settle()
        tid = [ev for ev in r.events if ev[0] == "start"][0][1]
        assert r.brain.origin_of(tid) == "phone" and r.kinds()[-1] == "end"
        await r.brain.close()
    asyncio.run(run())


def test_turns_the_engine_starts_itself_belong_here(tmp_path):
    async def run():
        r = Rig(tmp_path)
        await r.brain.connect()
        await settle()
        r.engine.emit(UserMessage(content="<task-notification>download finished</task-notification>",
                                  origin={"kind": "task-notification"}), text("The download finished."), result())
        await settle()
        tid = [ev for ev in r.events if ev[0] == "start"][0][1]
        assert r.brain.origin_of(tid) == "here" and "phone_text" not in r.kinds()
        await r.brain.close()
    asyncio.run(run())


def test_speaking_here_interrupts_a_phone_turn(tmp_path):
    async def run():
        r = Rig(tmp_path)
        await r.brain.connect()
        await settle()
        r.engine.emit(UserMessage(content="sort my downloads", origin={"kind": "human"}), text("Sorting"))
        await settle()
        assert r.brain.current_origin == "phone"
        tid = await r.brain.ask("[Mon · screen: conversation]\nstop, play music instead")
        await r.brain.wait_idle()
        ends = [ev for ev in r.events if ev[0] == "end"]
        assert ends[0][2].interrupted and r.brain.origin_of(ends[0][1]) == "phone"
        assert ends[1][1] == tid and ends[1][2].ok and r.brain.origin_of(tid) == "here"
        await r.brain.close()
    asyncio.run(run())


def test_an_old_silent_phone_turn_doesnt_block(tmp_path, monkeypatch):
    async def run():
        r = Rig(tmp_path)
        await r.brain.connect()
        await settle()
        r.engine.emit(UserMessage(content="hi", origin={"kind": "human"}))
        await settle()
        r.brain._cur.last -= 3600
        r.engine.emit(UserMessage(content="hello again", origin={"kind": "human"}), text("Hi!"), result())
        await settle()
        ends = [ev for ev in r.events if ev[0] == "end"]
        assert len(ends) == 2 and ends[0][2].interrupted and ends[1][2].ok
        await r.brain.close()
    asyncio.run(run())


def test_engine_stopping_ends_the_turn_and_reconnects_next_time(tmp_path):
    async def run():
        r = Rig(tmp_path, login=False)
        r.engines.clear()
        await r.brain.connect()
        r.engine.reply = lambda p: []
        await r.brain.ask("x")
        r.engine.q.put_nowait(None)          # the engine's output ends
        await r.brain.wait_idle()
        assert r.events[-1][0] == "end" and r.events[-1][2].interrupted
        await settle()
        assert r.brain.client is None
        r.brain.client_factory = lambda o: (r.engines.append(FakeEngine(o)) or r.engines[-1])
        await r.brain.ask("again")
        await r.brain.wait_idle()
        assert r.events[-1][2].ok and len(r.engines) == 2
        await r.brain.close()
    asyncio.run(run())


# ------------------------------------------------------------------ approvals asked on the phone
def test_risky_actions_from_the_phone_are_asked_in_the_chat(tmp_path):
    async def run():
        paths = Paths(tmp_path / "ClaudeOS").ensure()
        mode = {"origin": "phone"}
        spoken = []

        async def confirm(q):
            if mode["origin"] == "phone":
                return "phone", ""
            spoken.append(q)
            return "yes", "yes"
        gate = PermissionGate(paths, Journal(paths), confirm, {})
        cmd = {"command": "rm ~/Downloads/old.zip", "description": "Delete the old zip"}

        first = await gate("Bash", cmd)
        assert first.behavior == "deny" and "phone" in first.message and "Delete the old zip".lower() in first.message.lower()
        assert spoken == []
        # Changing the description doesn't matter; the command does.
        gate.phone_reply("what?")                     # not an answer: still waiting
        assert (await gate("Bash", dict(cmd))).behavior == "deny"
        gate.phone_reply("yes go ahead")
        again = await gate("Bash", dict(cmd, description="Remove it"))
        assert again.behavior == "allow"
        # Only once.
        assert (await gate("Bash", cmd)).behavior == "deny"
        # A different command needs its own yes.
        gate.phone_reply("yes")
        assert (await gate("Bash", {"command": "rm ~/Downloads/other.zip"})).behavior == "deny"
        # "No" clears the question.
        gate.phone_reply("no")
        gate.phone_reply("yes")       # too late: nothing is being asked any more
        assert (await gate("Bash", {"command": "rm ~/Downloads/other.zip"})).behavior == "deny"
        # Here at the PC it's spoken as before.
        mode["origin"] = "here"
        assert (await gate("Bash", {"command": "rm ~/x"})).behavior == "allow" and spoken
    asyncio.run(run())


# ------------------------------------------------------------------ the screen and the voice
class FakeSpeaker:
    def __init__(self):
        self.calls = []
        self.speaking = False
        self.tts = None

    def __getattr__(self, name):
        return lambda *a, **k: self.calls.append((name, a))


def _controller(tmp_path, monkeypatch):
    monkeypatch.setenv("CLAUDEOS_HOME", str(tmp_path / "ClaudeOS"))
    paths = Paths().ensure()
    screen = RecordingScreen()
    c = Controller(paths, Settings(user_name="Ren Shu"), FakePlatform(), screen, voice=False, index=False)
    return c, screen, paths


def test_phone_turns_are_shown_but_not_spoken(tmp_path, monkeypatch):
    c, screen, paths = _controller(tmp_path, monkeypatch)
    sp = FakeSpeaker()
    c.speaker = sp
    c.brain = SimpleNamespace(origin_of=lambda tid: "phone" if tid.startswith("p") else "here",
                              typed_here=lambda tid: False, current_origin="here", use_token_instead=lambda: False)
    c.gate = SimpleNamespace(phone_reply=lambda t: None)
    c.ctx = SimpleNamespace(rate_limit={})

    async def run():
        c.loop = asyncio.get_running_loop()
        c._on_turn_start("p1")
        c._on_user_text("p1", "turn on the lights")
        c._on_text("p1", "Done.")
        c._on_turn_end("p1", SimpleNamespace(ok=True, interrupted=False, error_kind=None))
        assert sp.calls == []
        shown = [e for e in screen.events if e[0] == "user"]
        assert shown and shown[-1][1][1] == "turn on the lights"      # typed looks like spoken
        assert ("ai", ("p1", "Done.")) in screen.events
        c._on_turn_start("t2")
        c._on_text("t2", "Hello.")
        assert ("feed", ("Hello.",)) in sp.calls
    asyncio.run(run())


def test_confirm_goes_to_the_phone_during_a_phone_turn(tmp_path, monkeypatch):
    c, screen, paths = _controller(tmp_path, monkeypatch)
    c.brain = SimpleNamespace(current_origin="phone")

    async def run():
        c.loop = asyncio.get_running_loop()
        assert await c.confirm("Delete the file?") == ("phone", "")
    asyncio.run(run())
    assert any("Waiting for your answer in the Claude app" in str(e) for e in screen.events)


def test_full_sign_in_left_on_windows_is_taken_and_removed(tmp_path, monkeypatch):
    c, screen, paths = _controller(tmp_path, monkeypatch)
    win = tmp_path / "Users" / "rensh"
    (win / "ClaudeOS").mkdir(parents=True)
    (win / "ClaudeOS" / "claude_login.json").write_text("﻿" + LOGIN, encoding="utf-8")
    c._import_signin(str(win / "ClaudeOS"))
    assert has_login(paths) and not (win / "ClaudeOS" / "claude_login.json").exists()
    assert any("Claude app on your phone" in str(e) for e in screen.events)
    # Something that isn't a sign-in is ignored (and removed).
    (win / "ClaudeOS" / "claude_login.json").write_text("{}", encoding="utf-8")
    paths.login_file.unlink()
    c._import_signin(str(win / "ClaudeOS"))
    assert not has_login(paths)


def test_engine_is_started_again_so_the_phone_can_reach_it(tmp_path, monkeypatch):
    c, screen, paths = _controller(tmp_path, monkeypatch)
    warmed = []

    async def warm():
        warmed.append(1)
    c._warm_up = warm
    c.brain = SimpleNamespace(client=None, busy=False, auth_mode="login")

    async def run():
        await c._keep_phone_reachable()
        await c._keep_phone_reachable()          # not again within a minute
        assert warmed == [1]
        c._phone_retry_at -= 61
        c.brain.client = object()                # running: nothing to do
        await c._keep_phone_reachable()
        c.brain.client = None
        c.brain.auth_mode = "token"              # no phone without the full sign-in
        await c._keep_phone_reachable()
        assert warmed == [1]
        c.brain.auth_mode = "login"
        c.settings.phone_access = False
        await c._keep_phone_reachable()
        assert warmed == [1]
    asyncio.run(run())


def test_sign_ins_left_on_windows_are_never_indexed(tmp_path):
    from claudeos.files_index import FileIndex
    win = tmp_path / "Users" / "rensh"
    (win / "ClaudeOS").mkdir(parents=True)
    (win / "ClaudeOS" / "claude_login.json").write_text(LOGIN)
    (win / "ClaudeOS" / "claude_token").write_text(TOKEN)
    (win / "notes.json").write_text('{"shopping": "milk"}')
    idx = FileIndex(tmp_path / "files.db", [("windows", str(win))])
    names = {os.path.basename(e.path) for e in idx._walk(str(win))}
    assert names == {"notes.json"}


def test_typing_while_asleep_shows_the_conversation_but_the_microphone_stays_asleep(tmp_path, monkeypatch):
    c, screen, paths = _controller(tmp_path, monkeypatch)
    c.brain = SimpleNamespace(origin_of=lambda tid: "phone" if tid.startswith("p") else "here",
                              current_origin="phone", use_token_instead=lambda: False, busy=False)
    c.gate = SimpleNamespace(phone_reply=lambda t: None)
    c.ctx = SimpleNamespace(rate_limit={})
    c.asleep = True

    async def run():
        c.loop = asyncio.get_running_loop()
        c._on_turn_start("p7")
        c._on_user_text("p7", "what's the weather?")
        c._on_turn_end("p7", SimpleNamespace(ok=True, interrupted=False, error_kind=None))
    asyncio.run(run())
    states = [e[1] for e in screen.events if e[0] == "state"]
    assert states == ["thinking", "resting"] and c.asleep          # shown, but still waiting for "Claude"
    lines = [json.loads(l) for l in paths.transcript.read_text().splitlines()]
    assert lines[0] == {**lines[0], "role": "user", "text": "what's the weather?", "via": "phone"}


def test_an_unavailable_model_goes_back_to_the_usual_one_and_asks_again(tmp_path, monkeypatch):
    c, screen, paths = _controller(tmp_path, monkeypatch)
    c.settings.model = "claude-no-such-model-9"
    asked = []

    async def close():
        asked.append("closed")

    async def ask(text):
        asked.append(text)
    c.brain = SimpleNamespace(origin_of=lambda tid: "here", current_origin="here", use_token_instead=lambda: False,
                              close=close, ask=ask, last_activity=0.0)
    c.gate = SimpleNamespace(phone_reply=lambda t: None)
    c.ctx = SimpleNamespace(rate_limit={})
    c.last_asked = "what's the time?"

    async def run():
        c.loop = asyncio.get_running_loop()
        c._on_turn_start("t1")
        c._on_turn_end("t1", SimpleNamespace(ok=False, interrupted=False, error_kind="model"))
        for _ in range(50):
            if len(asked) >= 2:
                break
            await asyncio.sleep(0.02)
    asyncio.run(run())
    assert c.settings.model == Settings().model
    assert Settings.load(paths).model == Settings().model                 # kept for the next start
    assert asked[0] == "closed" and asked[1].endswith("what's the time?")   # asked again, on the usual model
    assert any("isn't available" in e[1][0] for e in screen.events if e[0] == "notice")
