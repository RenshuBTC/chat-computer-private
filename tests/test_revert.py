"""Revert, in Actions (chatcomputer/done.py, journal.py, Controller.revert): what one action changed is taken back
(files as they were, settings as they were), and that is listed as an action of its own, which can be reverted in
turn (the action is back). Nothing is lost on the way: a file overwritten is copied first, one removed goes to the
trash."""
import asyncio
import os
import shutil

from chatcomputer import journal as jn
from chatcomputer import tools as T
from chatcomputer.brain.permissions import PermissionGate
from chatcomputer.config import Paths, Settings
from chatcomputer.controller import Controller
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen

OURS = "mcp__chatcomputer__"


class Rig:
    """A Chat Computer with its tools (the platform pretend), its trash a folder the test can look in."""

    def __init__(self, tmp_path, monkeypatch):
        monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "ChatComputer"))
        self.bin = tmp_path / "trash-bin"
        self.bin.mkdir()

        def trash(p):
            dest = self.bin / f"{len(list(self.bin.iterdir()))}-{os.path.basename(p)}"
            shutil.move(p, dest)
            return str(dest)
        monkeypatch.setattr(jn, "move_to_trash", trash)
        monkeypatch.setattr(T, "move_to_trash", trash)
        self.screen, self.p = RecordingScreen(), FakePlatform()
        self.c = Controller(Paths().ensure(), Settings(user_name="Sam Lee"), self.p, self.screen, voice=False, index=False)
        c = self.c
        c.ctx = T.ToolContext(c.paths, c.settings, self.p, c.journal, self.screen, c.memory, history=c.history,
                              schedule=c.schedule)
        _, policies, _ = T.build(c.ctx)
        self.gate = PermissionGate(c.paths, c.journal, None, policies)
        self.n = 0

    async def tool(self, name, inp, builtin=False):
        """One of Claude's tool calls: the gate lets it through (marking it), it runs, and Actions lists it."""
        self.n += 1
        use_id = f"toolu_{self.n}"
        await self.gate(name if builtin else OURS + name, inp, SimpleCtx(use_id))
        if builtin:                      # (what Claude's Write does, after the gate)
            with open(inp["file_path"], "w") as f:
                f.write(inp["content"])
            text, ok = "ok", True
        else:
            res = await self.c.ctx.tool_fns[name](inp)
            text, ok = "\n".join(x["text"] for x in res["content"]), not res.get("is_error")
        self.c._on_tool("t1", name if builtin else OURS + name, inp, ok, text, use_id)
        return text

    def last_done(self):
        return [e[1] for e in self.screen.events if e[0] == "done"][-1]

    def states(self):
        return [e[1] for e in self.screen.events if e[0] == "done_states"][-1]

    def notices(self):
        return [e[1][0] for e in self.screen.events if e[0] == "notice"]


class SimpleCtx:
    def __init__(self, use_id):
        self.tool_use_id = use_id


def run(rig, coro_fn):
    async def go():
        rig.c.loop = asyncio.get_running_loop()
        await coro_fn()
        await asyncio.sleep(0.05)               # (what was spawned meanwhile: the list's marks)
    asyncio.run(go())


def test_a_saved_file_goes_back_as_it_was_and_the_revert_can_be_reverted(tmp_path, monkeypatch):
    r = Rig(tmp_path, monkeypatch)
    notes = tmp_path / "notes.txt"
    notes.write_text("my list")

    async def go():
        await r.tool("Write", {"file_path": str(notes), "content": "Claude's list"}, builtin=True)
        saved = r.last_done()
        assert saved["text"] == "Saved notes.txt" and saved["revert"] is True and "reverted" not in saved
        await r.c.revert(saved["id"])
        assert notes.read_text() == "my list"
        reverted = r.last_done()
        assert reverted["text"] == "Reverted “Saved notes.txt”" and reverted["revert"] is True
        assert r.states()[saved["id"]] == "reverted" and r.states()[reverted["id"]] == "revert"
        # Reverting the revert: Claude's version again, and the save is no longer reverted.
        await r.c.revert(reverted["id"])
        assert notes.read_text() == "Claude's list"
        assert r.last_done()["text"] == "Reverted “Reverted “Saved notes.txt””"
        assert r.states()[saved["id"]] == "revert" and r.states()[reverted["id"]] == "reverted"
        # And once more: the person's own words, and nothing was ever lost (each version is kept in undo/).
        await r.c.revert(saved["id"])
        assert notes.read_text() == "my list"
        kept = sorted(p.read_text() for p in r.c.paths.undo.rglob("*") if p.is_file())
        assert kept == ["Claude's list", "Claude's list", "my list", "my list"]
    run(r, go)


def test_a_new_file_goes_to_the_trash_and_comes_back(tmp_path, monkeypatch):
    r = Rig(tmp_path, monkeypatch)
    new = tmp_path / "plan.md"

    async def go():
        await r.tool("Write", {"file_path": str(new), "content": "# Plan"}, builtin=True)
        made = r.last_done()
        await r.c.revert(made["id"])
        assert not new.exists() and [p.read_text() for p in r.bin.iterdir()] == ["# Plan"]
        await r.c.revert(r.last_done()["id"])                  # out of the trash again
        assert new.read_text() == "# Plan"
    run(r, go)


def test_moved_copied_trashed_and_made_folders_go_back(tmp_path, monkeypatch):
    r = Rig(tmp_path, monkeypatch)
    docs = tmp_path / "docs"
    docs.mkdir()
    (docs / "a.txt").write_text("A")
    (docs / "b.txt").write_text("B")

    async def go():
        await r.tool("move_files", {"items": [{"from": str(docs / "a.txt"), "to": str(docs / "c.txt")},
                                              {"from": str(docs / "b.txt"), "to": str(docs / "d.txt")}]})
        moved = r.last_done()
        assert moved["text"] == "Moved a.txt, b.txt" and moved["revert"]
        await r.tool("copy_files", {"items": [{"from": str(docs / "c.txt"), "to": str(docs / "e.txt")}]})
        copied = r.last_done()
        await r.tool("trash_files", {"paths": [str(docs / "d.txt")]})
        trashed = r.last_done()
        await r.tool("make_folder", {"path": str(docs / "Old")})
        folder = r.last_done()
        # Each taken back by itself, in any order.
        await r.c.revert(trashed["id"])
        assert (docs / "d.txt").read_text() == "B"
        await r.c.revert(moved["id"])                          # both files, last one first
        assert (docs / "a.txt").read_text() == "A" and (docs / "b.txt").read_text() == "B"
        assert not (docs / "c.txt").exists() and not (docs / "d.txt").exists()
        await r.c.revert(copied["id"])
        assert not (docs / "e.txt").exists()
        await r.c.revert(folder["id"])
        assert not (docs / "Old").exists()
        assert all(r.states()[e["id"]] == "reverted" for e in (moved, copied, trashed, folder))
        assert r.notices() == []
    run(r, go)


def test_what_cant_be_taken_back_is_said_and_nothing_is_marked(tmp_path, monkeypatch):
    r = Rig(tmp_path, monkeypatch)
    docs = tmp_path / "docs"
    docs.mkdir()
    (docs / "a.txt").write_text("A")

    async def go():
        await r.tool("move_files", {"items": [{"from": str(docs / "a.txt"), "to": str(docs / "b.txt")}]})
        moved = r.last_done()
        (docs / "a.txt").write_text("a new a.txt")             # something is where it would go back to now
        await r.c.revert(moved["id"])
        assert (docs / "b.txt").read_text() == "A" and (docs / "a.txt").read_text() == "a new a.txt"
        assert r.notices()[-1].startswith("Couldn't revert “Moved a.txt”: something is already at")
        assert r.states()[moved["id"]] == "revert"             # still can be, once that's out of the way
        # An action that changed nothing that can be taken back, or one already reverted.
        await r.tool("show_website", {"url": "youtube.com"})
        opened = r.last_done()
        assert "revert" not in opened and "reverted" not in opened
        await r.c.revert(opened["id"])
        assert r.notices()[-1] == "Nothing reverted: That can't be reverted."
        (docs / "a.txt").unlink()
        await r.c.revert(moved["id"])
        await r.c.revert(moved["id"])
        assert r.notices()[-1] == "Nothing reverted: It was already reverted."
    run(r, go)


def test_settings_go_back_as_they_were(tmp_path, monkeypatch):
    r = Rig(tmp_path, monkeypatch)
    p, s = r.p, r.c.settings
    p.muted = True
    p.vpn_list = [{"name": "Work", "kind": "WireGuard", "on": False}]

    async def go():
        # The volume (it was muted: muted again), the brightness, the night light.
        await r.tool("sound", {"action": "set_volume", "value": 70})
        vol = r.last_done()
        await r.tool("display", {"action": "brightness", "value": 30})
        bright = r.last_done()
        await r.tool("display", {"action": "night_light", "mode": "evenings", "from": "20:00"})
        night = r.last_done()
        assert (p.volume, p.muted, p.backlight, s.night_light) == (70, False, 30, "evenings")
        for e in (night, bright, vol):
            await r.c.revert(e["id"])
        assert (p.volume, p.muted, p.backlight, s.night_light, s.night_light_from) == (40, True, 80, "off", "21:00")
        # Wi-Fi off, then a network joined; the time zone; a setting of Chat Computer's own; a VPN.
        await r.tool("wifi", {"action": "off"})
        off = r.last_done()
        await r.tool("wifi", {"action": "connect", "network": "Cafe"})
        joined = r.last_done()
        await r.tool("set_timezone", {"zone": "Europe/London"})
        zone = r.last_done()
        await r.tool("chatcomputer_settings", {"action": "set", "key": "text_scale", "value": 1.3})
        scale = r.last_done()
        await r.tool("vpn", {"action": "on", "name": "Work"})
        vpn = r.last_done()
        assert all(e.get("revert") for e in (off, joined, zone, scale, vpn))
        for e in (vpn, scale, zone, joined, off):
            await r.c.revert(e["id"])
        assert p.wifi["connected_to"] == "HomeWiFi" and p.wifi["radio"] == "enabled"
        assert p.zone == "Asia/Singapore" and s.text_scale == 1.0 and p.vpn_list[0]["on"] is False
        assert r.notices() == []
        # Reverting a revert puts the setting back the way the action left it.
        last = r.last_done()
        assert last["text"] == "Reverted “Turned the Wi-Fi off”"
        await r.c.revert(last["id"])
        assert p.wifi["radio"] == "disabled" and r.states()[off["id"]] == "revert"
    run(r, go)


def test_bluetooth_memory_timers_programs_and_the_start_up_system(tmp_path, monkeypatch):
    r = Rig(tmp_path, monkeypatch)
    p = r.p

    async def go():
        await r.tool("bluetooth", {"action": "disconnect", "device": "AA:BB:CC:DD:EE:01"})
        bt = r.last_done()
        await r.tool("memory", {"action": "remember", "text": "Likes laksa"})
        remembered = r.last_done()
        await r.tool("memory", {"action": "remember", "text": "Has a cat called Mochi"})
        await r.tool("memory", {"action": "forget", "text": "cat Mochi"})
        forgot = r.last_done()
        await r.tool("reminders", {"action": "timer", "minutes": 10, "label": "tea"})
        timer = r.last_done()
        await r.tool("software", {"action": "install", "packages": ["vlc", "firefox"]})
        installed = r.last_done()
        await r.tool("start_by_default", {"system": "other"})
        other = r.last_done()
        assert all(e.get("revert") for e in (bt, remembered, forgot, timer, installed, other))
        await r.c.revert(bt["id"])
        assert p.calls[-1] == ("bt_connect", ("AA:BB:CC:DD:EE:01",))
        await r.c.revert(forgot["id"])
        assert "Has a cat called Mochi" in r.c.memory.for_prompt()
        await r.c.revert(remembered["id"])
        assert "laksa" not in r.c.memory.for_prompt() and "Mochi" in r.c.memory.for_prompt()
        await r.c.revert(timer["id"])
        assert r.c.schedule.items() == []
        await r.c.revert(installed["id"])                      # only what wasn't there before goes
        assert p.calls[-1] == ("remove", (("vlc",),)) and "firefox" in p.packages
        await r.c.revert(other["id"])
        assert p.default_os == "chatcomputer"
        assert r.notices() == []
        # A timer that has rung can't be cancelled any more: said so, and not marked.
        await r.tool("reminders", {"action": "timer", "minutes": 5})
        five = r.last_done()
        r.c.schedule.cancel("all")
        await r.c.revert(five["id"])
        assert r.notices()[-1].startswith("Couldn't revert") and r.states()[five["id"]] == "revert"
    run(r, go)


def test_undo_that_walks_back_past_reverts_and_shows_what_it_undid(tmp_path, monkeypatch):
    r = Rig(tmp_path, monkeypatch)
    one, two = tmp_path / "one.txt", tmp_path / "two.txt"
    one.write_text("1")
    two.write_text("2")

    async def go():
        await r.tool("Write", {"file_path": str(one), "content": "one changed"}, builtin=True)
        first = r.last_done()
        await r.tool("Write", {"file_path": str(two), "content": "two changed"}, builtin=True)
        second = r.last_done()
        await r.c.revert(second["id"])
        assert two.read_text() == "2"
        # "Undo that" now: the change before (not the revert, and not the change it reverted).
        said = await r.tool("undo_last_change", {})
        assert "one.txt" in said and one.read_text() == "1" and two.read_text() == "2"
        await asyncio.sleep(0.05)
        assert r.states()[first["id"]] == "reverted"
        assert r.last_done()["text"] == "Undid the last change" and r.last_done()["revert"] is True
    run(r, go)


def test_the_screen_gets_the_marks_with_the_list(tmp_path, monkeypatch):
    r = Rig(tmp_path, monkeypatch)
    f = tmp_path / "x.txt"
    f.write_text("x")

    async def go():
        await r.tool("Write", {"file_path": str(f), "content": "y"}, builtin=True)
        saved = r.last_done()
        await r.c.revert(saved["id"])
        await r.c._show_done()
    run(r, go)
    listed = [e for e in r.screen.events if e[0] == "done_list"][-1][1]
    assert listed == ["Saved x.txt", "Reverted “Saved x.txt”"]
    entries = r.c.history.recent(10, {}, r.c._change_state())
    assert [(e["text"], e.get("revert"), e.get("reverted")) for e in entries] == [
        ("Saved x.txt", None, True), ("Reverted “Saved x.txt”", True, None)]
    # The page asks with a page event.
    seen = []
    r.c._spawn = lambda coro: (seen.append(coro), coro.close())
    r.c.page_event({"type": "revert", "id": entries[0]["id"]})
    assert len(seen) == 1


def test_only_what_a_call_changed_is_its_own(tmp_path, monkeypatch):
    """Once a call's result is back, what's recorded is no longer put down to it: Revert on it takes back only what
    it changed. (Claude's next call is marked by the gate as it comes.)"""
    r = Rig(tmp_path, monkeypatch)

    async def go():
        await r.tool("sound", {"action": "set_volume", "value": 30})
        louder = r.last_done()
        assert r.c.journal.call is None
        r.c.journal.record_change("something else", {"type": "tool", "tool": "sound", "input": {"action": "mute"}})
        assert len(r.c.history.get(louder["id"])["changes"]) == len(r.c.journal.changes_of("toolu_1")) == 1
        # A result that comes back late (the next call already through the gate) leaves that one's mark alone.
        await r.gate(OURS + "sound", {"action": "set_volume", "value": 20}, SimpleCtx("toolu_9"))
        r.c._on_tool("t1", OURS + "sound", {"action": "set_volume", "value": 30}, True, "Volume 30%.", "toolu_1")
        assert r.c.journal.call == "toolu_9"
    run(r, go)
