"""Chat Computer on an Apple silicon Mac: Linux (Fedora Asahi Remix) started instead of macOS. What differs from a PC:
going back to macOS is a restart, and macOS's files come from the drive Chat Computer Setup sends them to."""
import asyncio
from types import SimpleNamespace
import os

from chatcomputer import tools as T
from chatcomputer.brain.prompt import build_system_prompt
from chatcomputer.config import Paths, Settings
from chatcomputer.controller import Controller
from chatcomputer.journal import Journal
from chatcomputer.memory import Memory
from chatcomputer.platform import linux as L
from chatcomputer.platform.base import CommandResult
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen

class FakeMac(FakePlatform):
    name = "mac"
    other_os = "macOS"
    setup_place = "Chat Computer Setup in macOS"


def test_apple_silicon_is_recognised_from_the_device_tree(tmp_path):
    f = tmp_path / "compatible"
    f.write_bytes(b"apple,j313\0apple,t8103\0apple,arm-platform\0")
    assert L.is_apple_silicon(str(f))
    f.write_bytes(b"linux,dummy-virt\0")
    assert not L.is_apple_silicon(str(f))
    assert not L.is_apple_silicon(str(tmp_path / "missing"))


def test_switching_to_macos_sets_the_next_start_then_restarts(monkeypatch):
    calls = []

    def fake_run(cmd, timeout=20, input=None, env=None):
        calls.append(cmd)
        if cmd[:3] == ["sudo", "-n", L.ROOT_HELPER]:
            return CommandResult(0, '{"ok":true,"how":"asahi-bless"}\n', "")
        return CommandResult(0, "", "")
    monkeypatch.setattr(L, "run", fake_run)
    p = L.MacPlatform()
    assert p.power("switch_to_macos") == "Restarting into macOS."
    assert calls == [["sudo", "-n", L.ROOT_HELPER, "boot-macos-next"], ["systemctl", "reboot"]]
    calls.clear()
    assert "macOS now starts" in p.set_default_os("other")
    assert calls == [["sudo", "-n", L.ROOT_HELPER, "boot-default", "macos"]]
    calls.clear()
    assert p.set_default_os("chatcomputer") == "Done: Chat Computer starts when the Mac turns on."
    assert calls == [["sudo", "-n", L.ROOT_HELPER, "boot-default", "chatcomputer"]]
    assert "no Windows" in p.power("switch_to_windows")


def test_a_failed_switch_says_so_and_does_not_restart(monkeypatch):
    calls = []

    def fake_run(cmd, timeout=20, input=None, env=None):
        calls.append(cmd)
        return CommandResult(1, '{"ok":false,"error":"asahi-bless could not find macOS"}\n', "")
    monkeypatch.setattr(L, "run", fake_run)
    out = L.MacPlatform().power("switch_to_macos")
    assert out.startswith("I couldn't set macOS for the next start") and "asahi-bless" in out
    assert ["systemctl", "reboot"] not in calls


def _mac_controller(tmp_path, monkeypatch):
    home = tmp_path / "home"
    (home / "Mac").mkdir(parents=True)
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(home / "ChatComputer"))
    paths = Paths().ensure()
    screen = RecordingScreen()
    c = Controller(paths, Settings(user_name="Sam Lee"), FakeMac(), screen, voice=False, index=False)
    return c, screen, paths, home


def test_on_a_mac_the_files_from_macos_are_the_other_systems_files(tmp_path, monkeypatch):
    c, _, _, home = _mac_controller(tmp_path, monkeypatch)
    assert c._other_os_files() == str(home / "Mac")
    prompt = c._system_prompt()
    assert str(home / "Mac") in prompt and "drive called ChatComp" in prompt
    assert "switch_to_macos" in prompt and "Fedora" in prompt and "Windows" not in prompt


def test_mac_without_the_drive_has_no_other_files(tmp_path, monkeypatch):
    c, _, _, home = _mac_controller(tmp_path, monkeypatch)
    os.rmdir(home / "Mac")
    assert c._other_os_files() is None
    assert "drive called ChatComp" not in c._system_prompt()


def test_offline_switch_to_macos_asks_then_restarts(tmp_path, monkeypatch):
    c, screen, _, _ = _mac_controller(tmp_path, monkeypatch)
    did = []
    c.platform.power = lambda action: did.append(action) or "Restarting into macOS."

    async def yes(q, *a, **k):
        did.append(("asked", q))
        return ("yes", "")

    async def no_wait(seconds):
        return None

    async def run():
        c.loop = asyncio.get_running_loop()
        c.confirm = yes
        monkeypatch.setattr(asyncio, "sleep", no_wait)
        await c._offline_command("switch to macOS", signed_out=True)
    asyncio.run(run())
    assert did == [("asked", "Restart into macOS?"), "switch_to_macos"]


def test_tools_on_a_mac(tmp_path):
    home = tmp_path / "home"
    home.mkdir()
    # (The trash's folder is one for the whole run: tests/conftest.py sets XDG_DATA_HOME once, before any test.)
    paths = Paths(home / "ChatComputer").ensure()
    ctx = T.ToolContext(paths, Settings(), FakeMac(), Journal(paths), RecordingScreen(), Memory(paths), power_delay_s=0.05)
    specs, policies, names = T.build(ctx)
    tools = {t["name"]: SimpleNamespace(**t) for t in specs}
    power = tools["power"]
    assert power.input_schema["properties"]["action"]["enum"] == ["shutdown", "restart", "sleep", "switch_to_macos"]
    assert policies["power"]({"action": "switch_to_macos"}).startswith("Restart into macOS?")
    assert "Fedora" in tools["software"].description and "Ubuntu" not in tools["software"].description
    assert policies["software"]({"action": "install", "packages": ["vlc"]}).endswith("Fedora's software library.")
    assert "import_from_windows" not in tools["bluetooth"].input_schema["properties"]["action"]["enum"]
    assert "files from macOS" in tools["system_status"].description
    assert "macOS" in tools["start_by_default"].description


def test_mac_prompt_tells_claude_where_files_are_and_how_to_go_back():
    class S:
        user_name = "Sam Lee"
        windows_user = ""
        timezone = "Asia/Singapore"
    p = build_system_prompt(S(), "", "/home/sammy", "/home/sammy/Mac", platform="mac")
    assert "The folders they brought over from macOS are at /home/sammy/Mac" in p
    assert "can't be opened from here" in p
    assert "restarts into it" in p and "Remove Chat Computer" in p
    assert "Apple silicon Mac running Chat Computer (Fedora Asahi Remix underneath)" in p
