"""What Chat Computer asks when it starts (a new version of itself, then AirPods), and answers given in writing only
when the question was typed on this PC's keyboard."""
import asyncio
from pathlib import Path
from types import SimpleNamespace

import pytest

from chatcomputer import controller as C
from chatcomputer import update
from chatcomputer.config import Paths, Settings
from chatcomputer.controller import Controller
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen

AIRPODS = {"address": "AA:BB:CC:DD:EE:01", "name": "AirPods Pro"}


class Bluetooth(FakePlatform):
    """A PC whose Bluetooth knows `paired` devices; `windows_has` are paired in Windows (copied over by
    bt_import_windows_keys); `pairable` are in pairing mode nearby. Devices in their case don't connect."""

    def __init__(self, paired=(), windows_has=(), pairable=(), in_case=False):
        super().__init__()
        self.headset = False
        self.bt = {"powered": True, "adapter": True, "connected": [], "paired": [dict(d) for d in paired]}
        self.windows_has, self.pairable, self.in_case = list(windows_has), list(pairable), in_case

    def bt_status(self):
        return {k: (list(v) if isinstance(v, list) else v) for k, v in self.bt.items()}

    def bt_connect(self, address):
        self._c("bt_connect", address)
        for d in self.bt["paired"]:
            if d["address"] == address and not self.in_case and d not in self.bt["connected"]:
                self.bt["connected"].append(d)
        return "Connected."

    def bt_import_windows_keys(self):
        self._c("bt_import_windows_keys")
        self.bt["paired"] += [dict(d) for d in self.windows_has]
        return f"Copied {len(self.windows_has)} Bluetooth pairing(s) from Windows."

    def bt_scan(self, seconds=10):
        self._c("bt_scan", seconds)
        return [dict(d, paired=False) for d in self.pairable]

    def bt_pair(self, address, on_code=None):
        self._c("bt_pair", address)
        for d in self.pairable:
            if d["address"] == address:
                self.bt["paired"].append(dict(d))
                self.bt["connected"].append(dict(d))
        return "Paired and connected."


def _controller(tmp_path, monkeypatch, platform=None, **kw):
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "ChatComputer"))
    screen = RecordingScreen()
    c = Controller(Paths().ensure(), Settings(user_name="Sam Lee"), platform or FakePlatform(), screen, voice=False,
                   index=False, **kw)
    return c, screen


def _answering(c, *answers):
    """Questions get these answers, typed, in order. Returns what was asked: (question, typed)."""
    asked, left = [], list(answers)

    async def confirm(question, typed=None):
        asked.append((question, typed))
        a = left.pop(0) if left else "timeout"
        c._answer_typed = True
        if a in ("yes", "no"):
            return a, a
        return ("timeout", "") if a == "timeout" else ("unclear", a)
    c.confirm = confirm
    return asked


def _notices(screen):
    return [e[1][0] for e in screen.events if e[0] == "notice"]


def _calls(platform, name):
    return [a for n, a in platform.calls if n == name]


@pytest.fixture
def fast(monkeypatch):
    real_sleep = asyncio.sleep

    async def sleep(s, *a, **k):
        await real_sleep(0)
    monkeypatch.setattr(C.asyncio, "sleep", sleep)


def _run(c, coro_fn):
    async def run():
        c.loop = asyncio.get_running_loop()
        return await coro_fn()
    return asyncio.run(run())


def _offer(**kw):
    return update.Offer(**{"source": Path("/mnt/windows/Users/Sam Lee/Downloads/ChatComputer/ChatComputer"), "changed": 3,
                           "version": "0.3.0", "needs_setup": False, **kw})


# ------------------------------------------------------------------ a new version of Chat Computer
def test_a_new_version_is_offered_on_screen_and_installed(tmp_path, monkeypatch, fast):
    c, screen = _controller(tmp_path, monkeypatch)
    monkeypatch.setattr(update, "check", lambda: _offer())
    asked = _answering(c, "yes")
    assert _run(c, c._offer_update) is True
    question, typed = asked[0]
    assert "new version of Chat Computer (0.3.0)" in question and typed is True  # shown, not said
    assert _calls(c.platform, "update_app") == [()]
    assert c.exit_code == 3 and ("quit", None) in screen.events                # the session starts it again at once
    assert any("Updated" in n for n in _notices(screen))


def test_declining_the_new_version(tmp_path, monkeypatch, fast):
    c, screen = _controller(tmp_path, monkeypatch)
    monkeypatch.setattr(update, "check", lambda: _offer())
    _answering(c, "no")
    assert _run(c, c._offer_update) is False
    assert not _calls(c.platform, "update_app") and c.exit_code is None
    assert "Okay. I'll ask again the next time Chat Computer starts." in _notices(screen)


def test_a_request_instead_of_an_answer_is_handled(tmp_path, monkeypatch, fast):
    c, screen = _controller(tmp_path, monkeypatch)
    monkeypatch.setattr(update, "check", lambda: _offer())
    _answering(c, "what is the weather like today")
    handled = []

    async def handle(text, typed=False):
        handled.append((text, typed))
    c.handle = handle

    async def go():
        r = await c._offer_update()
        await asyncio.sleep(0)
        return r
    assert _run(c, go) is False
    assert handled == [("what is the weather like today", True)] and not _calls(c.platform, "update_app")


def test_a_version_that_needs_setup_is_only_mentioned(tmp_path, monkeypatch, fast):
    c, screen = _controller(tmp_path, monkeypatch)
    monkeypatch.setattr(update, "check", lambda: _offer(needs_setup=True))
    asked = _answering(c)
    assert _run(c, c._offer_update) is False
    assert asked == [] and any("run Chat Computer Setup again" in n for n in _notices(screen))


def test_an_update_that_fails_says_why_and_ChatComputer_carries_on(tmp_path, monkeypatch, fast):
    c, screen = _controller(tmp_path, monkeypatch)
    c.platform.update_result = {"ok": False, "error": "the new version has something an update can't bring: x.pth"}
    monkeypatch.setattr(update, "check", lambda: _offer())
    _answering(c, "yes")
    assert _run(c, c._offer_update) is False
    assert c.exit_code is None and ("quit", None) not in screen.events
    assert any("x.pth" in str(e) and "warn" in str(e) for e in screen.events if e[0] == "notice")


def test_nothing_asked_without_another_version(tmp_path, monkeypatch, fast):
    c, screen = _controller(tmp_path, monkeypatch)
    monkeypatch.setattr(update, "check", lambda: None)
    asked = _answering(c)
    assert _run(c, c._offer_update) is False and asked == []


def test_an_update_that_started_well_is_kept(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch, offer_updates=True)
    marker = tmp_path / "update-pending"
    monkeypatch.setattr(update, "UPDATE_PENDING", marker)
    _run(c, c._keep_update)
    assert not _calls(c.platform, "update_done")           # nothing new to keep
    marker.write_text("1\n")
    _run(c, c._keep_update)
    assert _calls(c.platform, "update_done") == [()]


# ------------------------------------------------------------------ the questions at the start, in order
def test_the_update_comes_first_and_yes_skips_the_airpods(tmp_path, monkeypatch, fast):
    c, screen = _controller(tmp_path, monkeypatch, platform=Bluetooth(), offer_updates=True, offer_headphones=True)
    monkeypatch.setattr(update, "source_there", lambda: True)
    monkeypatch.setattr(update, "check", lambda: _offer())
    asked = _answering(c, "yes")
    _run(c, c._start_questions)
    assert len(asked) == 1 and "new version" in asked[0][0] and c.exit_code == 3


def test_airpods_first_while_windows_drive_is_still_coming(tmp_path, monkeypatch, fast):
    c, screen = _controller(tmp_path, monkeypatch, platform=Bluetooth(), offer_updates=True, offer_headphones=True)
    drive = {"there": False}
    monkeypatch.setattr(update, "source_there", lambda: drive["there"])
    order = []

    def check():
        order.append("looked for an update")
        return None
    monkeypatch.setattr(update, "check", check)
    asked = _answering(c, "no")
    real = c.confirm

    async def confirm(question, typed=None):
        order.append(question)
        drive["there"] = True
        return await real(question, typed)
    c.confirm = confirm
    _run(c, c._start_questions)
    assert order[0].startswith("Connect your AirPods?") and order[-1] == "looked for an update"
    assert len(asked) == 1


# ------------------------------------------------------------------ AirPods at the start
def test_no_question_when_sound_already_plays_on_headphones(tmp_path, monkeypatch, fast):
    p = Bluetooth()
    p.headset = True
    c, screen = _controller(tmp_path, monkeypatch, platform=p)
    asked = _answering(c)
    _run(c, c._offer_airpods)
    assert asked == [] and not _calls(p, "bt_connect")


def test_known_airpods_out_of_their_case_just_connect(tmp_path, monkeypatch, fast):
    p = Bluetooth(paired=[AIRPODS])
    c, screen = _controller(tmp_path, monkeypatch, platform=p)
    asked = _answering(c)
    _run(c, c._offer_airpods)
    assert asked == [] and "Your AirPods Pro are connected." in _notices(screen)


def test_the_pairing_from_windows_is_used_without_asking(tmp_path, monkeypatch, fast):
    # Chat Computer Setup told the person Chat Computer would take over Windows' pairing: it isn't asked about again.
    p = Bluetooth(windows_has=[AIRPODS])
    c, screen = _controller(tmp_path, monkeypatch, platform=p)
    asked = _answering(c)
    _run(c, c._offer_airpods)
    assert asked == [] and _calls(p, "bt_import_windows_keys") == [()] and not _calls(p, "bt_pair")
    assert "Your AirPods Pro are connected." in _notices(screen)


def test_airpods_in_their_case_connect_when_taken_out(tmp_path, monkeypatch, fast):
    class InCase(Bluetooth):
        def bt_connect(self, address):
            if len(_calls(self, "bt_connect")) >= 2:
                self.in_case = False                # taken out after a moment
            return super().bt_connect(address)
    p = InCase(paired=[AIRPODS], in_case=True)
    c, screen = _controller(tmp_path, monkeypatch, platform=p)
    asked = _answering(c)

    async def go():
        await c._offer_airpods()
        while "Your AirPods Pro are connected." not in _notices(screen):
            await asyncio.sleep(0)
    _run(c, go)
    assert asked == []
    notices = _notices(screen)
    assert notices.index("Take your AirPods out of their case: they'll connect by themselves.") < \
        notices.index("Your AirPods Pro are connected.")


def test_known_airpods_that_never_come_out_are_given_up_on_quietly(tmp_path, monkeypatch, fast):
    monkeypatch.setattr(C, "AIRPODS_WAIT_S", 0.05)
    p = Bluetooth(paired=[AIRPODS], in_case=True)
    c, screen = _controller(tmp_path, monkeypatch, platform=p)
    asked = _answering(c)

    async def go():
        await c._offer_airpods()
        while not any("didn't connect" in n for n in _notices(screen)):
            await asyncio.sleep(0.01)
    _run(c, go)
    assert asked == [] and not _calls(p, "bt_pair")


def test_airpods_copied_over_without_a_name_are_found_by_connecting(tmp_path, monkeypatch, fast):
    # Pairings copied from Windows before names came along: BlueZ lists them by their address until it connects.
    unnamed = {"address": AIRPODS["address"], "name": AIRPODS["address"].replace(":", "-")}

    class Unnamed(Bluetooth):
        def bt_connect(self, address):
            self._c("bt_connect", address)
            if address == AIRPODS["address"]:
                self.bt["connected"].append(dict(AIRPODS))     # connected: BlueZ learns the name
            return "Connected."
    p = Unnamed(windows_has=[unnamed])
    c, screen = _controller(tmp_path, monkeypatch, platform=p)
    asked = _answering(c)
    _run(c, c._offer_airpods)
    assert asked == [] and "Your AirPods Pro are connected." in _notices(screen)


def test_asked_then_paired_here_when_windows_has_none(tmp_path, monkeypatch, fast):
    p = Bluetooth(pairable=[AIRPODS])
    c, screen = _controller(tmp_path, monkeypatch, platform=p)
    asked = _answering(c, "yes")
    _run(c, c._offer_airpods)
    assert asked == [("Connect your AirPods?", True)]
    assert any("hold the button on the back of the case" in n for n in _notices(screen))
    assert _calls(p, "bt_pair") == [(AIRPODS["address"],)]
    assert "Your AirPods Pro are connected." in _notices(screen)


def test_airpods_not_found(tmp_path, monkeypatch, fast):
    monkeypatch.setattr(C, "AIRPODS_PAIR_S", 0.05)
    p = Bluetooth()
    c, screen = _controller(tmp_path, monkeypatch, platform=p)
    _answering(c, "yes")
    _run(c, c._offer_airpods)
    assert any("couldn't find the AirPods" in n for n in _notices(screen))


def test_airpods_declined(tmp_path, monkeypatch, fast):
    p = Bluetooth()
    c, screen = _controller(tmp_path, monkeypatch, platform=p)
    _answering(c, "no")
    _run(c, c._offer_airpods)
    assert not _calls(p, "bt_pair")
    assert any("connect my AirPods" in n for n in _notices(screen))


def test_connect_my_airpods_in_one_step(tmp_path, monkeypatch, fast):
    # Asked for later ("connect my AirPods"): one call does whatever it takes.
    p = Bluetooth(windows_has=[AIRPODS])
    c, screen = _controller(tmp_path, monkeypatch, platform=p)
    assert _run(c, c.connect_airpods) == "Your AirPods Pro are connected."
    assert _run(c, c.connect_airpods) == "Your AirPods Pro are already connected."
    p2 = Bluetooth(pairable=[AIRPODS])
    c2, screen2 = _controller(tmp_path / "2", monkeypatch, platform=p2)
    c2._say = lambda text: None
    assert _run(c2, c2.connect_airpods) == "Your AirPods Pro are connected."
    assert _calls(p2, "bt_pair") == [(AIRPODS["address"],)]


def test_no_question_without_bluetooth(tmp_path, monkeypatch, fast):
    p = Bluetooth()
    p.bt["adapter"] = False
    c, screen = _controller(tmp_path, monkeypatch, platform=p)
    asked = _answering(c)
    _run(c, c._offer_airpods)
    assert asked == []


# ------------------------------------------------------------------ typed here: answered in writing
class FakeSpeaker:
    def __init__(self):
        self.calls = []
        self.speaking = False

    def __getattr__(self, name):
        if name == "until_quiet":
            async def quiet(*a):
                return None
            return quiet
        return lambda *a, **k: self.calls.append((name, a))


def test_replies_to_typed_questions_are_shown_not_spoken(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)
    sp = c.speaker = FakeSpeaker()
    typed = {"t1"}
    c.brain = SimpleNamespace(origin_of=lambda tid: "here", typed_here=lambda tid: tid in typed,
                              current_origin="here", use_token_instead=lambda: False)

    async def run():
        c.loop = asyncio.get_running_loop()
        c._on_turn_start("t1")
        c._on_text("t1", "Here's the weather.")
        assert sp.calls == [] and ("ai", ("t1", "Here's the weather.")) in screen.events
        assert c._typing_now()
        c._announce("Type 482916 on the keyboard.")       # mid-tool news: shown only, too
        assert sp.calls == []
        c._on_turn_start("t2")                                # said out loud: spoken back
        c._on_text("t2", "Sunny.")
        assert ("feed", ("Sunny.",)) in sp.calls
    asyncio.run(run())


def test_a_typed_answer_in_words_brings_the_conversation_back(tmp_path, monkeypatch):
    """Something open fills the middle, with the conversation put away. A question typed here and answered only in
    words brings the conversation back to show the answer (music left playing); one whose turn worked on what's
    open (scrolled it) leaves it there, and so does a spoken one, whose answer is heard."""
    c, screen = _controller(tmp_path, monkeypatch)
    c.speaker = FakeSpeaker()
    typed = {"t1", "t2"}
    c.brain = SimpleNamespace(origin_of=lambda tid: "here", typed_here=lambda tid: tid in typed,
                              current_origin="here", use_token_instead=lambda: False)

    async def turn(tid, tools):
        c._on_turn_start(tid)
        c._on_text(tid, "Paris is the capital of France.")
        c._on_turn_end(tid, SimpleNamespace(ok=True, interrupted=False, error_kind=None, tools=tools))
        await asyncio.sleep(0.01)
        return screen.mode

    async def run():
        c.loop = asyncio.get_running_loop()
        await screen.show_url("https://www.google.com/")
        assert await turn("t2", ["mcp__chatcomputer__scroll"]) == "content"     # typed, but it scrolled the page
        assert await turn("t3", []) == "content"                                # said out loud: heard
        assert await turn("t1", ["WebSearch"]) == "chat" and screen.events[-1] == ("hide", None)
    asyncio.run(run())


def test_the_tools_that_work_on_what_s_open_are_claude_s_own():
    src = (Path(C.__file__).parent / "tools" / "__init__.py").read_text(encoding="utf-8")
    import re
    assert C.SCREEN_TOOLS <= set(re.findall(r'@reg\("([a-z_]+)"', src))


def test_typed_text_reaches_the_model_as_typed(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)
    asks = []

    async def ask(text, typed=False):
        asks.append((text, typed))
    c.brain = SimpleNamespace(busy=False, last_activity=0.0, ask=ask, current_origin="here")
    c.model = SimpleNamespace(ready=True)

    async def run():
        c.loop = asyncio.get_running_loop()
        c.asleep = True                       # typing wakes Chat Computer by itself: no wake word to type
        await c.handle("what's on today", typed=True)
    asyncio.run(run())
    assert asks and asks[-1][1] is True and not c.asleep
    # The model is told it was typed, so it answers like a chat message (see prompt.py).
    note, words = asks[-1][0].split("\n", 1)
    assert note.startswith("[") and note.endswith(" · typed]") and words == "what's on today"


def test_a_web_address_typed_alone_opens_its_site_at_once(tmp_path, monkeypatch):
    # As a browser's address bar: any site's address, alone (or after "open", "go to"...), with or without https://
    # and a path; a file's name (notes.md) or a question about a site isn't one.
    from chatcomputer import answers
    assert answers.typed_address("youtube.com") == "https://youtube.com"
    assert answers.typed_address("open bbc.co.uk please") == "https://bbc.co.uk"
    assert answers.typed_address("YouTube.com.") == "https://YouTube.com"
    assert answers.typed_address("https://www.youtube.com/watch?v=dQw4w9WgXcQ") == "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
    assert answers.typed_address("go to github.com/anthropics") == "https://github.com/anthropics"
    for other in ("notes.md", "open main.rs", "what is youtube.com", "e.g.", "3.14", "hello", "youtube", "google.com is slow"):
        assert answers.typed_address(other) is None, other
    # It opens at once, in the middle, without a turn of the model's, and Actions says so (linked to the message).
    c, screen = _controller(tmp_path, monkeypatch)
    asks = []

    async def ask(text, typed=False):
        asks.append(text)
    c.brain = SimpleNamespace(busy=False, last_activity=0.0, ask=ask, current_origin="here")
    c.model = SimpleNamespace(ready=True)

    async def run():
        c.loop = asyncio.get_running_loop()
        await c.handle("youtube.com", typed=True)
    asyncio.run(run())
    assert asks == [] and ("show_url", "https://youtube.com") in screen.events
    done = [e for k, e in screen.events if k == "done"]
    assert done and done[-1]["text"] == "Opened youtube.com" and done[-1]["prompt_id"].startswith("u")


def test_the_note_says_typed_only_when_it_was():
    from chatcomputer.brain.prompt import build_system_prompt, turn_prefix
    assert turn_prefix({"mode": "chat"}, None).endswith("screen: conversation]\n")
    assert turn_prefix({"mode": "chat"}, None, typed=True).endswith("screen: conversation · typed]\n")
    assert '· typed]' in build_system_prompt(Settings(), "", "/home/sam", None)


def test_questions_about_typed_requests_are_shown_not_said(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)
    said = []
    c._say = said.append
    c.speaker = FakeSpeaker()
    c.listener = SimpleNamespace(mode="normal")
    c.mic = SimpleNamespace(is_open=True)                # the microphone hears, but the request was typed

    async def run():
        c.loop = asyncio.get_running_loop()
        c.confirm_lock = asyncio.Lock()
        task = asyncio.create_task(c.confirm("Delete the file?", typed=True))
        while c.pending_confirm is None:
            await asyncio.sleep(0)
        await c.handle("yes", typed=True)
        verdict = await task
        assert verdict == ("yes", "yes") and said == []
        shown = "".join(t for k, v in screen.events if k == "ai" for _, t in [v])
        assert "Delete the file? (Type yes or no.)" in shown
        # Asked about something said out loud: said out loud too.
        task = asyncio.create_task(c.confirm("Restart now?", typed=False))
        while c.pending_confirm is None:
            await asyncio.sleep(0)
        await c.handle("no")
        assert await task == ("no", "no") and said == ["Restart now?"]
    asyncio.run(run())


def test_without_a_microphone_questions_say_to_type(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)

    async def run():
        c.loop = asyncio.get_running_loop()
        c.confirm_lock = asyncio.Lock()
        task = asyncio.create_task(c.confirm("Restart now?"))
        while c.pending_confirm is None:
            await asyncio.sleep(0)
        await c.handle("no", typed=True)
        assert await task == ("no", "no")
    asyncio.run(run())
    shown = "".join(t for k, v in screen.events if k == "ai" for _, t in [v])
    assert "Restart now? (Type yes or no.)" in shown


# ------------------------------------------------------------------ the logs, before the computer restarts
def test_logs_are_copied_before_a_restart(tmp_path, monkeypatch, fast):
    c, screen = _controller(tmp_path, monkeypatch)
    copied = []

    async def copy():
        copied.append(True)
    c._copy_logs = copy
    _answering(c, "yes")

    async def run():
        c.loop = asyncio.get_running_loop()
        await c._offline_command("switch to windows", signed_out=True, typed=True)
    asyncio.run(run())
    assert copied == [True] and _calls(c.platform, "power") == [("switch_to_windows",)]
