"""Updating Chat Computer from the folder Chat Computer Setup ran from (update.py), the copy of the logs on Windows' drive
(logcopy.py), and what keeps the model from changing Chat Computer's own program files."""
import asyncio
import json
import os

import pytest

from chatcomputer import logcopy, update
from chatcomputer.brain import permissions as P
from chatcomputer.config import Paths
from chatcomputer.journal import Journal
from chatcomputer.ui.gtk_app import displays_report, graphics_report

SOURCE = "Users/Sam Lee/Downloads/ChatComputer/ChatComputer"


def _app(root, version="1", agent="x = 1\n", requirements="numpy==1\nsoxr==1\n"):
    (root / "chatcomputer/brain").mkdir(parents=True, exist_ok=True)
    (root / "linux/session").mkdir(parents=True, exist_ok=True)
    (root / "chatcomputer/__init__.py").write_text(f'__version__ = "{version}"\n')
    (root / "chatcomputer/brain/agent.py").write_text(agent)
    (root / "linux/session/chatcomputer-session").write_text("#!/bin/bash\n")
    (root / "requirements-linux.txt").write_text(requirements)
    return root


@pytest.fixture
def setup(tmp_path):
    """An installed Chat Computer, Windows' drive with the folder Chat Computer Setup ran from, and Setup's note of it."""
    app = _app(tmp_path / "opt/app")
    mount = tmp_path / "win"
    src = _app(mount / SOURCE)
    note = tmp_path / "update-source"
    note.write_text(SOURCE + "\n", encoding="utf-8")
    return app, mount, src, note


# ------------------------------------------------------------------ where updates come from
def test_the_folder_setup_ran_from(tmp_path, setup):
    app, mount, src, note = setup
    assert update.source_dir(note, mount) == src
    note.write_text(SOURCE + "\r\n", encoding="utf-8")         # written on Windows
    assert update.source_dir(note, mount) == src
    for bad in ("/etc", "Users/../../etc", "Users/./x", "Users/a\x01b", "", "\n"):
        note.write_text(bad, encoding="utf-8")
        assert update.source_dir(note, mount) is None, repr(bad)
    assert update.source_dir(tmp_path / "missing", mount) is None


def test_nothing_offered_when_both_are_the_same(setup):
    app, mount, src, note = setup
    (src / "chatcomputer/__pycache__").mkdir()
    (src / "chatcomputer/__pycache__/agent.cpython-312.pyc").write_bytes(b"\0")  # compiled files don't count
    assert update.check(note, mount, app) is None
    assert update.source_there(note, mount)


def test_a_new_version_is_offered(setup):
    app, mount, src, note = setup
    _app(src, version="1.1", agent="x = 2\n")
    (src / "chatcomputer/new.py").write_text("")
    (app / "linux/session/old").write_text("")
    offer = update.check(note, mount, app)
    assert offer is not None and offer.source == src and offer.version == "1.1"
    assert offer.changed == 4 and not offer.needs_setup      # __init__, agent, new.py, and old gone


def test_a_version_needing_other_python_packages_needs_setup(setup):
    app, mount, src, note = setup
    for other in ("numpy==2\nsoxr==1\n", "numpy==1\nsoxr==1\nmore==1\n"):
        _app(src, agent="x = 2\n", requirements=other)
        offer = update.check(note, mount, app)
        assert offer is not None and offer.needs_setup, other


def test_a_version_needing_fewer_python_packages_is_an_update(setup):
    # What it needs is installed already (the package left out stays, unused). Comments and spacing don't count.
    app, mount, src, note = setup
    _app(src, agent="x = 2\n", requirements="# the runtime\r\nnumpy==1    # arrays\r\n\r\n")
    offer = update.check(note, mount, app)
    assert offer is not None and not offer.needs_setup
    assert update.packages(src / "requirements-linux.txt") == {"numpy==1"}


def test_no_offer_without_the_folder(tmp_path, setup):
    app, mount, src, note = setup
    note.write_text("Users/Nobody/ChatComputer\n")
    assert update.check(note, mount, app) is None and not update.source_there(note, mount)
    assert update.check(tmp_path / "missing", mount, app) is None


def test_pending(tmp_path):
    marker = tmp_path / "update-pending"
    assert not update.pending(marker)
    marker.write_text("1\n")
    assert update.pending(marker)


# ------------------------------------------------------------------ Claude can't change Chat Computer's own files
@pytest.fixture
def program(tmp_path, setup, monkeypatch):
    app, mount, src, note = setup
    monkeypatch.setattr(update, "UPDATE_SOURCE", note)
    monkeypatch.setattr(update, "WINDOWS_MOUNT", mount)
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "home/ChatComputer"))
    monkeypatch.setenv("HOME", str(tmp_path / "home"))
    (tmp_path / "home").mkdir()
    return src


def test_chatcomputer_own_files_are_program_files(program):
    src = program
    assert P.is_program("/opt/chatcomputer/app/chatcomputer/controller.py")
    assert P.is_program(str(src / "chatcomputer/brain/permissions.py"))
    assert P.is_program(str(src).replace("ChatComputer/ChatComputer", "chatcomputer/CHATCOMPUTER"))  # Windows ignores case
    assert not P.is_program(str(src.parent / "notes.txt"))
    assert P.holds_program(str(src.parent)) and not P.holds_program(str(src / "chatcomputer"))


def test_claude_cant_edit_or_change_them(program, tmp_path):
    src = program
    paths = Paths().ensure()

    async def run():
        async def no(question):
            raise AssertionError("asked: " + question)
        gate = P.PermissionGate(paths, Journal(paths), no)
        for tool, inp in (("Write", {"file_path": str(src / "chatcomputer/brain/permissions.py"), "content": ""}),
                          ("Edit", {"file_path": "/opt/chatcomputer/app/chatcomputer/controller.py"}),
                          ("Bash", {"command": f'cp ~/x.py "{src}/chatcomputer/x.py"'}),
                          ("Bash", {"command": f"cd {str(src).replace(' ', chr(92) + ' ')} && git pull"}),
                          ("Bash", {"command": f'cd "{src.parent}" && rm -r ChatComputer'}),
                          ("Bash", {"command": f'echo hi > "{src}/chatcomputer/x.py"'})):
            res = await gate(tool, inp)
            assert res.behavior == "deny" and "Chat Computer's own program files" in res.message, (tool, inp)
        # Reading them is fine.
        for tool, inp in (("Read", {"file_path": str(src / "chatcomputer/__init__.py")}),
                          ("Bash", {"command": f'cat "{src}/chatcomputer/__init__.py"'})):
            assert (await gate(tool, inp)).behavior == "allow", (tool, inp)
    asyncio.run(run())

    # Other folders next to it, even one whose name starts the same, are asked about as usual.
    asked = []

    async def run2():
        async def yes(question):
            asked.append(question)
            return "yes", "yes"
        gate = P.PermissionGate(paths, Journal(paths), yes)
        for cmd in (f'cp ~/x.pdf "{src.parent}/"', f'mkdir "{src}-old"', f'touch "{src.parent}/ChatComputer-backup/x"'):
            assert (await gate("Bash", {"command": cmd})).behavior == "allow", cmd
    asyncio.run(run2())
    assert len(asked) == 3


# ------------------------------------------------------------------ the logs on Windows' drive
def test_logs_are_copied_to_windows_without_secrets(tmp_path):
    logs = tmp_path / "logs"
    logs.mkdir()
    (logs / "chatcomputer.log").write_text("start\nremote control failed: token sk-ant-oat01-abcdef123\n"
                                       'Authorization: Bearer abcdefghijklmnopqrstuvwxyz\n{"refresh_token": "r123"}\n')
    (logs / "session.log").write_text("session start\n")
    home = tmp_path / "Users/Sam Lee"
    assert logcopy.copy_logs(logs, str(home), {}) is None        # no Chat Computer folder from Setup: nothing written
    (home / "AppData/Local/ChatComputer").mkdir(parents=True)
    dest = logcopy.copy_logs(logs, str(home), {"phone": {"link": "failed", "detail": "token sk-ant-oat01-zzz"}})
    assert dest == home / "AppData/Local/ChatComputer/from-chatcomputer"
    text = (dest / "chatcomputer.log.txt").read_bytes().decode()
    assert "remote control failed" in text and "\r\n" in text
    for secret in ("abcdef123", "abcdefghijklmnopqrstuvwxyz", "r123"):
        assert secret not in text
    info = json.loads((dest / "info.json").read_text())
    assert info["phone"]["link"] == "failed" and "zzz" not in json.dumps(info) and info["copied"]
    assert (dest / "session.log.txt").read_text().startswith("session start")
    assert logcopy.copy_logs(logs, None, {}) is None


def test_only_the_end_of_a_long_log_is_copied(tmp_path, monkeypatch):
    monkeypatch.setattr(logcopy, "TAIL_BYTES", 100)
    logs = tmp_path / "logs"
    logs.mkdir()
    (logs / "chatcomputer.log").write_text("".join(f"line {i:04d}\n" for i in range(100)))
    home = tmp_path / "home"
    (home / "AppData/Local/ChatComputer").mkdir(parents=True)
    dest = logcopy.copy_logs(logs, str(home))
    lines = (dest / "chatcomputer.log.txt").read_text().splitlines()
    assert lines[-1] == "line 0099" and len(lines) <= 10 and all(len(ln) == 9 for ln in lines)


# ------------------------------------------------------------------ what the log says about the screen
def test_graphics_and_screens_in_the_log(tmp_path):
    drm = tmp_path / "drm"
    dev = tmp_path / "devices/pci0000:00/0000:00:02.0"
    (dev / "driver").parent.mkdir(parents=True)
    os.symlink(tmp_path / "drivers/amdgpu", dev / "driver")
    (dev / "vendor").write_text("0x1002\n")
    (dev / "device").write_text("0x164e\n")
    (drm / "card1").mkdir(parents=True)
    os.symlink(dev, drm / "card1/device")
    for name, status, modes in (("card1-DP-1", "connected", "5120x2880\n2560x1440\n"),
                                ("card1-HDMI-A-1", "disconnected", "")):
        (drm / name).mkdir()
        (drm / name / "status").write_text(status + "\n")
        (drm / name / "modes").write_text(modes)
    assert graphics_report(str(drm)) == "card1: amdgpu (1002:164e)"
    assert displays_report(str(drm)) == "card1-DP-1 5120x2880"
    assert graphics_report(str(tmp_path / "none")) == "no graphics chips found"
    assert displays_report(str(tmp_path / "none")) == "none found"
