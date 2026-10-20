"""The sidebars' switches: Messages (left) and Actions (right). The page says when one is switched; Chat Computer
keeps the choice and tells the page at the start, and Claude switches them when asked. There's no third switch:
nothing is under what's open but the message box."""
import asyncio
import json

import pytest
from test_tools import call, make

from chatcomputer.config import Paths, Settings
from chatcomputer.controller import Controller
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen


@pytest.fixture
def loop():
    lp = asyncio.new_event_loop()
    asyncio.set_event_loop(lp)
    yield lp
    lp.close()


def _controller(tmp_path, monkeypatch, **settings):
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "ChatComputer"))
    screen = RecordingScreen()
    c = Controller(Paths().ensure(), Settings(**settings), FakePlatform(), screen, voice=False, index=False)
    return c, screen


def test_both_sidebars_show_until_switched_off():
    s = Settings()
    assert (s.left_panel, s.right_panel) == (True, True)
    assert not hasattr(s, "bottom_panel")


def test_a_switch_pressed_on_the_screen_is_kept(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)
    c.page_event({"type": "panels", "left": False, "right": True})
    s = Settings.load(c.paths)
    assert (s.left_panel, s.right_panel) == (False, True)
    assert not any(e[0] == "panels" for e in screen.events)        # the page shows it already
    c.page_event({"type": "panels", "left": "yes", "right": False})  # only true or false counts
    s = Settings.load(c.paths)
    assert (s.left_panel, s.right_panel) == (False, False)


def test_the_page_is_told_which_sidebars_show(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch, right_panel=False)
    assert c._panels() == {"left": True, "right": False}
    c.settings.left_panel = False
    c._settings_changed(["left_panel"])
    assert ("panels", {"left": False, "right": False}) in screen.events


def test_the_old_bottom_switch_is_forgotten(tmp_path, monkeypatch):
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "ChatComputer"))
    paths = Paths().ensure()
    paths.settings_file.write_text(json.dumps({"left_panel": False, "bottom_panel": False}), encoding="utf-8")
    s = Settings.load(paths)
    assert s.left_panel is False and "bottom_panel" not in s.extra
    s.save(paths)
    assert "bottom_panel" not in json.loads(paths.settings_file.read_text(encoding="utf-8"))


def test_claude_switches_them_when_asked(tmp_path, loop):
    ctx, server, *_ = make(tmp_path)
    changed = []
    ctx.on_settings_changed = changed.append
    text, err = call(server, "chatcomputer_settings", {"action": "set", "key": "left_panel", "value": "false"})
    assert not err and ctx.settings.left_panel is False and changed == [["left_panel"]]
    text, err = call(server, "chatcomputer_settings", {"action": "set", "key": "bottom_panel", "value": False})
    assert err                                                       # not a setting any more
    text, err = call(server, "chatcomputer_settings", {"action": "get"})
    assert '"right_panel": true' in text and "bottom_panel" not in text
