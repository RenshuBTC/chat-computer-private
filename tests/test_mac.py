"""ClaudeOS for Mac, the parts that run anywhere: the Mac engine's use of macOS's commands (with stand-ins for
them), its parsing of macOS's reports, the tools and the prompt as a Mac sees them."""
import asyncio
import json
import os
import plistlib

import pytest

from claudeos.brain.prompt import build_system_prompt
from claudeos.config import Paths, Settings
from claudeos.journal import Journal
from claudeos.memory import Memory
from claudeos.platform import mac as M
from claudeos.platform.base import CommandResult
from claudeos.platform.fake import FakePlatform
from claudeos.screen import RecordingScreen
from claudeos import tools as T

HARDWARE_PORTS = """
Hardware Port: Thunderbolt Bridge
Device: bridge0
Ethernet Address: N/A

Hardware Port: Wi-Fi
Device: en0
Ethernet Address: 3c:22:fb:00:11:22
"""

BLUETOOTH = {"SPBluetoothDataType": [{
    "controller_properties": {"controller_address": "F0:18:98:00:00:01", "controller_state": "attrib_on"},
    "device_connected": [{"Ren's AirPods Pro": {"device_address": "D4:6D:6D:97:62:9B", "device_minorType": "Headphones"}}],
    "device_not_connected": [{"Magic Keyboard": {"device_address": "A0-B1-C2-D3-E4-F5"}}],
}]}

AUDIO = {"SPAudioDataType": [{"_items": [
    {"_name": "MacBook Pro Speakers", "coreaudio_device_output": 2, "coreaudio_device_transport": "coreaudio_device_type_builtin"},
    {"_name": "MacBook Pro Microphone", "coreaudio_device_input": 1, "coreaudio_device_transport": "coreaudio_device_type_builtin"},
    {"_name": "Ren's AirPods Pro", "coreaudio_device_output": 2, "coreaudio_device_input": 1,
     "coreaudio_default_audio_output_device": "spaudio_yes", "coreaudio_default_audio_input_device": "spaudio_yes",
     "coreaudio_device_transport": "coreaudio_device_type_bluetooth"},
]}]}


class FakeMacCommands:
    """Plays macOS's command-line tools: records each call and answers like macOS would."""

    def __init__(self, answers=None):
        self.calls = []
        self.answers = {
            "networksetup -listallhardwareports": HARDWARE_PORTS,
            "networksetup -getairportpower en0": "Wi-Fi Power (en0): On",
            "networksetup -getairportnetwork en0": "Current Wi-Fi Network: Home 5G",
            "system_profiler SPBluetoothDataType -json": json.dumps(BLUETOOTH),
            "system_profiler SPAudioDataType -json": json.dumps(AUDIO),
            "osascript -e get volume settings": "output volume:40, input volume:75, alert volume:100, output muted:false",
            "sw_vers -productVersion": "15.6",
        }
        self.answers.update(answers or {})

    def __call__(self, cmd, timeout=20, input=None, env=None):
        line = " ".join(cmd)
        self.calls.append(line)
        return CommandResult(0, self.answers.get(line, ""), "")


def mac(tmp_path, answers=None):
    cmds = FakeMacCommands(answers)
    p = M.MacPlatform(runner=cmds, home=tmp_path)
    p.internet_ok = lambda: True
    p._corewlan_interface = lambda: None          # no Core WLAN off a Mac
    return p, cmds


def test_wifi_status_and_power(tmp_path):
    p, cmds = mac(tmp_path)
    st = p.wifi_status()
    assert st["wifi_device"] == "en0" and st["radio"] == "enabled" and st["connected_to"] == "Home 5G"
    assert p.wifi_radio(False) == "Wi-Fi turned off."
    assert "networksetup -setairportpower en0 off" in cmds.calls


def test_wifi_connect_reports_errors_macos_prints_on_stdout(tmp_path):
    p, cmds = mac(tmp_path, {"networksetup -setairportnetwork en0 Cafe wrongpass":
                             "Failed to join network Cafe.\nError: -3900  The operation couldn't be completed."})
    assert p.wifi_connect("Cafe", "wrongpass").startswith("Failed")
    assert p.wifi_connect("Home 5G", None) == "Connected to Home 5G."


def test_wifi_name_hidden_by_macos_still_says_connected(tmp_path):
    p, _ = mac(tmp_path, {"networksetup -getairportnetwork en0": "You are not associated with an AirPort network.",
                          "ipconfig getifaddr en0": "192.168.1.23"})
    assert p.wifi_status()["connected_to"].startswith("(connected")


def test_bluetooth_and_sound_from_system_profiler(tmp_path):
    p, _ = mac(tmp_path)
    bt = p.bt_status()
    assert bt["powered"] is True
    assert bt["connected"] == [{"name": "Ren's AirPods Pro", "address": "D4:6D:6D:97:62:9B"}]
    assert {"name": "Magic Keyboard", "address": "A0:B1:C2:D3:E4:F5"} in bt["paired"]
    assert p.output_is_headset() and p.input_is_bluetooth()
    st = p.audio_status()
    assert st["volume"] == 40 and st["muted"] is False
    assert [o["name"] for o in st["outputs"] if o["default"]] == ["Ren's AirPods Pro"]


def test_volume_and_mute_through_applescript(tmp_path):
    p, cmds = mac(tmp_path)
    assert p.set_volume(None, 15) == "Volume is 55%."
    assert "osascript -e set volume output volume 55 without output muted" in cmds.calls
    assert p.set_volume(130) == "Volume is 100%."
    assert p.set_mute(None) == "Muted."               # it wasn't muted
    assert "osascript -e set volume output muted true" in cmds.calls


def test_switching_to_macos_steps_aside(tmp_path):
    p, cmds = mac(tmp_path)
    seen = []
    p.on_step_aside = lambda: seen.append("aside")
    assert "macOS is showing" in p.power("switch_to_macos")
    assert seen == ["aside"] and not cmds.calls            # no restart, no command


def test_what_starts_at_login(tmp_path):
    p, cmds = mac(tmp_path)
    assert "opens when you log in" in p.set_default_os("claudeos")
    plist = plistlib.loads((tmp_path / "Library/LaunchAgents/com.claudeos.start.plist").read_bytes())
    assert plist["ProgramArguments"] == ["/usr/bin/open", "-a", str(tmp_path / "Applications/ClaudeOS.app")]
    assert plist["RunAtLoad"] is True and "KeepAlive" not in plist      # quitting ClaudeOS is respected
    assert "starts in macOS" in p.set_default_os("other")
    assert not (tmp_path / "Library/LaunchAgents/com.claudeos.start.plist").exists()
    assert any(c.startswith("launchctl bootout gui/") for c in cmds.calls)


def test_software_without_homebrew_explains(tmp_path, monkeypatch):
    p, cmds = mac(tmp_path)
    monkeypatch.setattr(p, "_brew", lambda: None)
    assert "App Store" in p.install(["ffmpeg"])
    monkeypatch.setattr(p, "_brew", lambda: "/opt/homebrew/bin/brew")
    assert p.install(["ffmpeg"]) == "Installed ffmpeg."
    assert "/opt/homebrew/bin/brew install ffmpeg" in cmds.calls
    assert p.install(["x; rm -rf ~"]).startswith("Those don't look like")


def test_key_combos():
    assert M.parse_key_combo("cmd+f") == (3, 1 << 20)
    assert M.parse_key_combo("Return") == (36, 0)
    assert M.parse_key_combo("shift+Tab") == (48, 1 << 17)
    with pytest.raises(ValueError):
        M.parse_key_combo("hyper+q")


def test_mac_prompt():
    s = Settings(user_name="Ren")
    p = build_system_prompt(s, "", "/Users/ren", None, platform="mac")
    assert "switch_to_macos" in p and "Windows" not in p and "Linux" not in p and "/Users/ren" in p


@pytest.fixture
def loop():
    lp = asyncio.new_event_loop()
    asyncio.set_event_loop(lp)
    yield lp
    lp.close()


def test_tools_as_a_mac_sees_them(tmp_path, loop):
    class MacFake(FakePlatform):
        name = "mac"
        other_os = "macOS"
        setup_place = "ClaudeOS Setup on this Mac"
    paths = Paths(tmp_path / "ClaudeOS").ensure()
    ctx = T.ToolContext(paths, Settings(), MacFake(), Journal(paths), RecordingScreen(), Memory(paths), power_delay_s=0.05)
    server, policies, names = T.build(ctx)
    assert policies["power"]({"action": "switch_to_macos"}) is None          # stepping aside needs no yes
    assert policies["power"]({"action": "shutdown"}) == "Shut down the computer?"
    assert policies["start_by_default"]({"system": "other"}).startswith("Start macOS when the computer turns on")
    tool_list = server["instance"].get_request_handler("tools/list")
    listed = loop.run_until_complete(tool_list.handler(None, tool_list.params_type()))
    by_name = {t.name: t for t in listed.tools}
    assert by_name["power"].input_schema["properties"]["action"]["enum"] == ["shutdown", "restart", "sleep", "switch_to_macos"]
    assert "import_from_windows" not in by_name["bluetooth"].input_schema["properties"]["action"]["enum"]
    assert "Windows" not in by_name["search_files"].description


def test_a_full_sign_in_kept_in_the_keychain_counts(tmp_path, monkeypatch):
    import subprocess
    from claudeos import config as C
    paths = Paths(tmp_path / "ClaudeOS").ensure()
    monkeypatch.setattr(C, "engine_cli", lambda: "/runtime/claude")
    seen = {}

    def runner(cmd, **kw):
        seen["cmd"], seen["env"] = cmd, kw["env"]
        return subprocess.CompletedProcess(cmd, 0, json.dumps({"loggedIn": True, "authMethod": "claude.ai"}), "")
    monkeypatch.setenv("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-oat01-x")
    C._KEYCHAIN_LOGIN.clear()
    assert C.keychain_login(paths, runner=runner)
    assert seen["cmd"] == ["/runtime/claude", "auth", "status", "--json"]
    assert seen["env"]["CLAUDE_CONFIG_DIR"] == str(paths.engine) and "CLAUDE_CODE_OAUTH_TOKEN" not in seen["env"]

    def token_only(cmd, **kw):
        return subprocess.CompletedProcess(cmd, 0, json.dumps({"loggedIn": True, "authMethod": "oauth_token"}), "")
    C._KEYCHAIN_LOGIN.clear()
    assert not C.keychain_login(paths, runner=token_only)        # a one-year token isn't a full sign-in
    C._KEYCHAIN_LOGIN.clear()


def test_hiding_by_hand_listens_only_for_the_wake_word(tmp_path, monkeypatch):
    from claudeos.controller import Controller
    monkeypatch.setenv("CLAUDEOS_HOME", str(tmp_path / "ClaudeOS"))
    screen = RecordingScreen()
    c = Controller(Paths().ensure(), Settings(user_name="Ren"), FakePlatform(), screen, voice=False, index=False)

    async def run():
        c.loop = asyncio.get_running_loop()
        c.went_aside()                                   # Command-H
        assert c.asleep and ("state", "sleeping") in screen.events
        await c.handle("what's the weather")              # someone else in the room: ignored
        assert not any(e[0] == "user" for e in screen.events)
        c.wake()                                         # clicked in the Dock
        assert not c.asleep and screen.events[-1] == ("state", "listening")
    asyncio.run(run())


def test_the_file_index_leaves_out_apps_and_their_data_on_a_mac(tmp_path, monkeypatch):
    from claudeos import files_index as FI
    home = tmp_path / "home"
    for f in ["Documents/tax 2025.pdf", "Library/Containers/com.other.app/Data/x.txt",
              "Applications/Foo.app/Contents/Info.plist", "Pictures/Photos Library.photoslibrary/originals/1.jpg",
              "Pictures/beach.jpg", "Projects/Library/notes.txt"]:
        (home / f).parent.mkdir(parents=True, exist_ok=True)
        (home / f).write_text("x")
    monkeypatch.setattr(FI.sys, "platform", "darwin")
    idx = FI.FileIndex(tmp_path / "files.db", [("home", str(home))])
    idx.crawl_once()
    found = sorted(os.path.relpath(r["path"], os.path.realpath(home)) for r in idx.search("", limit=100))
    # A folder called Library deeper down is someone's own folder, and is kept.
    assert found == ["Documents/tax 2025.pdf", "Pictures/beach.jpg", "Projects/Library/notes.txt"]
    assert not FI.dataless(str(home / "Pictures/beach.jpg"))


def test_sound_follows_airpods_connecting_on_a_mac(tmp_path, monkeypatch):
    """PortAudio reads the Mac's devices only when it starts: when the default devices change, the microphone
    and speaker are closed, PortAudio restarts, and the microphone opens again on the new default."""
    from claudeos.controller import Controller
    from claudeos.voice import audio as A
    monkeypatch.setenv("CLAUDEOS_HOME", str(tmp_path / "ClaudeOS"))
    c = Controller(Paths().ensure(), Settings(user_name="Ren"), FakePlatform(), RecordingScreen(), voice=False,
                   index=False)
    steps = []

    class Mic:
        is_open = True
        def close(self): steps.append("mic closed"); Mic.is_open = False
        def open(self): steps.append("mic opened"); Mic.is_open = True; return True

    class Player:
        def reset(self): steps.append("speaker closed")
    c.mic, c.player = Mic(), Player()
    devices = iter([(10, 20), (10, 20), (55, 56)])        # AirPods become the default input and output
    monkeypatch.setattr(A, "default_devices", lambda: next(devices, (55, 56)))
    monkeypatch.setattr(A, "restart_portaudio", lambda: steps.append("PortAudio restarted"))

    async def run():
        c.loop = asyncio.get_running_loop()
        task = asyncio.create_task(c._follow_audio_devices(every_s=0.01))
        for _ in range(200):
            await asyncio.sleep(0.01)
            if "mic opened" in steps:
                break
        task.cancel()
    asyncio.run(run())
    assert steps == ["mic closed", "speaker closed", "PortAudio restarted", "mic opened"]
    assert A.default_devices.__name__ == "<lambda>" and A.restart_portaudio.__name__ == "<lambda>"


# ------------------------------------------------------------------ WebKit's blocks (decision handlers)
def test_blocks_pyobjc_can_call_are_called_the_usual_way():
    from claudeos.ui.mac_blocks import call_block
    calls = []
    assert call_block(lambda *a: calls.append(a), b"v@?q", 2) == "pyobjc"
    assert calls == [(2,)]


def test_webkit_blocks_are_called_directly_when_pyobjc_cant():
    """WebKit's decision handlers come without a signature PyObjC can read; on a real Mac calling one as it
    was made ClaudeOS quit at its first page ("cannot call block without a signature")."""
    import ctypes

    from claudeos.ui.mac_blocks import _BlockLayout, call_block

    class Block:                      # what PyObjC hands over for such a block
        def __call__(self, *args):
            raise TypeError("cannot call block without a signature")

    seen = []
    invoke = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_long)(lambda blk, v: seen.append((blk, v)))
    layout = _BlockLayout(invoke=ctypes.cast(invoke, ctypes.c_void_p).value)
    addr = ctypes.addressof(layout)
    assert call_block(Block(), b"v@?q", 2, object_id=lambda o: addr) == "direct"
    assert seen == [(addr, 2)]                       # the block itself first, then the NSInteger
    # An object goes as its Objective-C pointer.
    got = []
    invoke2 = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_void_p)(lambda blk, obj: got.append(obj))
    layout2 = _BlockLayout(invoke=ctypes.cast(invoke2, ctypes.c_void_p).value)
    url = object()
    call_block(Block(), b"v@?@", url, object_id=lambda o: 0x1234 if o is url else ctypes.addressof(layout2))
    assert got == [0x1234]


def test_other_errors_from_a_block_are_not_hidden():
    from claudeos.ui.mac_blocks import call_block

    def block(*args):
        raise TypeError("expected 1 argument")
    with pytest.raises(TypeError, match="expected 1 argument"):
        call_block(block, b"v@?q", 1, 2)
