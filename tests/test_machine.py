"""The computer's hardware, as Windows and macOS let people use it: USB drives and memory cards (opened when plugged
in, ejected safely), printers found by themselves, the screen's brightness and night light, the keyboard's layout,
and timers, reminders and alarms that ring on time. The Linux parts are checked against what the real commands
print; the rest against the fake machine."""
import asyncio
import datetime as dt
import json
import time
from types import SimpleNamespace

import pytest

from chatcomputer import answers, schedule as sch
from chatcomputer import tools as T
from chatcomputer.config import Paths, Settings
from chatcomputer.controller import Controller
from chatcomputer.done import describe
from chatcomputer.journal import Journal
from chatcomputer.memory import Memory
from chatcomputer.platform import gamma, linux as L
from chatcomputer.platform.base import CommandResult
from chatcomputer.platform.fake import FakePlatform
from chatcomputer.screen import RecordingScreen

LSBLK = {"blockdevices": [
    {"name": "nvme0n1", "label": None, "size": 512110190592, "fstype": None, "mountpoints": [None], "rm": False,
     "hotplug": False, "tran": "nvme", "type": "disk", "model": "SAMSUNG", "vendor": None, "ro": False, "fsavail": None,
     "children": [
         {"name": "nvme0n1p1", "label": "SYSTEM", "size": 272629760, "fstype": "vfat", "mountpoints": ["/boot/efi"],
          "rm": False, "hotplug": False, "tran": None, "type": "part", "ro": False, "fsavail": 200000000},
         {"name": "nvme0n1p3", "label": "Windows", "size": 400000000000, "fstype": "ntfs", "mountpoints": ["/mnt/windows"],
          "rm": False, "hotplug": False, "tran": None, "type": "part", "ro": False, "fsavail": 100000000000},
         {"name": "nvme0n1p4", "label": None, "size": 64000000000, "fstype": "ext4", "mountpoints": ["/"],
          "rm": False, "hotplug": False, "tran": None, "type": "part", "ro": False, "fsavail": 40000000000}]},
    {"name": "sda", "label": None, "size": 32010928128, "fstype": None, "mountpoints": [None], "rm": True,
     "hotplug": True, "tran": "usb", "type": "disk", "model": "Ultra", "vendor": "SanDisk ", "ro": False, "fsavail": None,
     "children": [{"name": "sda1", "label": "SANDISK", "size": 32009879552, "fstype": "vfat",
                   "mountpoints": ["/media/sam/SANDISK"], "rm": True, "hotplug": True, "tran": None, "type": "part",
                   "ro": False, "fsavail": 12500000000}]},
    {"name": "sdb", "label": None, "size": 1000204886016, "fstype": None, "mountpoints": [None], "rm": False,
     "hotplug": True, "tran": "usb", "type": "disk", "model": "My Passport", "vendor": "WD", "ro": False, "fsavail": None,
     "children": [{"name": "sdb1", "label": None, "size": 1000202043392, "fstype": "exfat", "mountpoints": [None],
                   "rm": False, "hotplug": True, "tran": None, "type": "part", "ro": False, "fsavail": None}]},
    {"name": "mmcblk0", "label": "CANON", "size": 63864569856, "fstype": "exfat", "mountpoints": [None], "rm": "0",
     "hotplug": "0", "tran": None, "type": "disk", "model": None, "vendor": None, "ro": "0", "fsavail": None},
]}


# ---------------------------------------------------------------- Linux: what the commands print
def test_drives_that_can_be_unplugged_and_never_the_computer_s_own():
    ds = L.parse_drives(json.dumps(LSBLK))
    assert [(d["id"], d["name"], d["kind"], d["mounted"]) for d in ds] == [
        ("sda1", "SANDISK", "USB drive", True), ("sdb1", "WD My Passport", "USB drive", False),
        ("mmcblk0", "CANON", "memory card", False)]
    assert ds[0]["path"] == "/media/sam/SANDISK" and ds[0]["free_gb"] == 12.5 and ds[0]["size_gb"] == 32.0
    assert ds[1]["path"] is None and ds[1]["free_gb"] is None
    # A Chat Computer installed onto a USB disk: its own partitions are still the system's.
    sysdisk = {"blockdevices": [{"name": "sdc", "type": "disk", "rm": True, "hotplug": True, "tran": "usb", "children": [
        {"name": "sdc2", "fstype": "ext4", "mountpoints": ["/"], "type": "part"},
        {"name": "sdc1", "fstype": "vfat", "mountpoints": ["/boot/efi"], "type": "part"}]}]}
    assert L.parse_drives(json.dumps(sysdisk)) == []
    assert L.parse_drives("not json") == []


def test_where_udisks_opened_a_drive():
    assert L.parse_mounted_at("Mounted /dev/sda1 at /media/sam/SANDISK\n") == "/media/sam/SANDISK"
    assert L.parse_mounted_at("Mounted /dev/sda1 at /media/sam/My Stick.\n") == "/media/sam/My Stick"
    assert L.parse_mounted_at("Error mounting /dev/sda1: GDBus.Error:org.freedesktop.UDisks2.Error.AlreadyMounted: "
                              "Device /dev/sda1 is already mounted at `/media/sam/SANDISK'.") == "/media/sam/SANDISK"
    assert L.parse_mounted_at("Error mounting /dev/sda1: wrong fs type") is None


class Runs:
    """Stands in for run(): answers each command from a table, and keeps what was run."""

    def __init__(self, answers_by_cmd):
        self.answers = answers_by_cmd
        self.ran = []

    def __call__(self, cmd, timeout=20, input=None, env=None):
        self.ran.append(cmd)
        for prefix, res in self.answers.items():
            if " ".join(cmd).startswith(prefix):
                return CommandResult(*res) if isinstance(res, tuple) else res(cmd)
        return CommandResult(127, "", f"{cmd[0]} is not installed")


def test_opening_and_ejecting_a_drive_through_udisks(monkeypatch):
    unmounted = json.loads(json.dumps(LSBLK))
    unmounted["blockdevices"][1]["children"][0]["mountpoints"] = [None]
    state = {"lsblk": unmounted}
    runs = Runs({"lsblk": lambda cmd: CommandResult(0, json.dumps(state["lsblk"]), ""),
                 "udisksctl mount -b /dev/sda1": (0, "Mounted /dev/sda1 at /media/sam/SANDISK\n", ""),
                 "udisksctl unmount -b /dev/sda1": (0, "Unmounted /dev/sda1.\n", ""),
                 "udisksctl power-off -b /dev/sda": (0, "", "")})
    monkeypatch.setattr(L, "run", runs)
    p = L.LinuxPlatform()
    assert p.open_drive("sandisk") == "/media/sam/SANDISK"
    state["lsblk"] = LSBLK
    assert p.open_drive("SANDISK") == "/media/sam/SANDISK"       # already open: nothing run
    assert sum(c[:2] == ["udisksctl", "mount"] for c in runs.ran) == 1

    def after_unmount(cmd):
        state["lsblk"] = unmounted
        return CommandResult(0, "Unmounted /dev/sda1.\n", "")
    runs.answers["udisksctl unmount -b /dev/sda1"] = after_unmount
    assert p.eject_drive("SANDISK") == "SANDISK"
    assert ["udisksctl", "power-off", "-b", "/dev/sda", "--no-user-interaction"] in runs.ran
    with pytest.raises(RuntimeError, match="no drive called 'camera'"):
        p.open_drive("camera")
    # In use: it says so, and doesn't turn the stick off.
    state["lsblk"] = LSBLK
    runs.answers["udisksctl unmount -b /dev/sda1"] = (1, "", "Error unmounting /dev/sda1: GDBus.Error:org."
                                                             "freedesktop.UDisks2.Error.DeviceBusy: target is busy")
    with pytest.raises(RuntimeError, match="still using SANDISK"):
        p.eject_drive("SANDISK")


def test_without_udisks_it_says_what_to_install(monkeypatch):
    unmounted = json.loads(json.dumps(LSBLK))
    unmounted["blockdevices"][1]["children"][0]["mountpoints"] = [None]
    monkeypatch.setattr(L, "run", Runs({"lsblk": (0, json.dumps(unmounted), "")}))
    with pytest.raises(RuntimeError, match="udisks2"):
        L.LinuxPlatform().open_drive("SANDISK")


def test_printers_from_cups():
    names = "HP_OfficeJet_Pro_9010_E1A2B3\nCanon_TS8350\n"
    ps = L.parse_printers(names, "system default destination: Canon_TS8350\n",
                          "printer HP_OfficeJet_Pro_9010_E1A2B3 is idle.  enabled since Tue\nprinter Canon_TS8350 now printing Canon_TS8350-3.  enabled since\n")
    assert [(p["name"], p["state"], p["default"]) for p in ps] == [
        ("HP OfficeJet Pro 9010", "idle", False), ("Canon TS8350", "printing", True)]
    assert L.choose_printer(ps, None)["id"] == "Canon_TS8350"
    assert L.choose_printer(ps, "hp officejet")["id"] == "HP_OfficeJet_Pro_9010_E1A2B3"
    with pytest.raises(RuntimeError, match="No printer is connected"):
        L.choose_printer([], None)
    with pytest.raises(RuntimeError, match="Which printer"):
        L.choose_printer([dict(p, default=False) for p in ps], None)
    with pytest.raises(RuntimeError, match="no printer called 'Epson'"):
        L.choose_printer(ps, "Epson")
    # One printer and no default: that one.
    one = L.parse_printers("Brother_HL\n", "no system default destination\n", "")
    assert one[0]["default"] and L.choose_printer(one, None)["name"] == "Brother HL"
    q = L.parse_print_queue("Canon_TS8350-3     sam          1024000   Tue 20 Oct 2026 12:00:00\n")
    assert q == [{"id": "Canon_TS8350-3", "printer": "Canon TS8350", "size_kb": 1000, "since": "Tue 20 Oct 2026 12:00:00"}]


def test_printing_a_file_sends_it_with_the_right_options(monkeypatch, tmp_path):
    pdf, photo, cv = tmp_path / "Boarding pass.pdf", tmp_path / "Beach.jpg", tmp_path / "CV.docx"
    for f in (pdf, photo, cv):
        f.write_bytes(b"x")
    runs = Runs({"lpstat -e": (0, "HP_OfficeJet_Pro_9010\n", ""), "lpstat -d": (0, "no system default destination\n", ""),
                 "lpstat -p": (0, "", ""), "lp ": (0, "request id is HP_OfficeJet_Pro_9010-7 (1 file(s))\n", "")})
    monkeypatch.setattr(L, "run", runs)
    p = L.LinuxPlatform()
    assert p.print_file(str(pdf), copies=2, pages="1-2", two_sided=True, colour=False) == "HP OfficeJet Pro 9010"
    lp = [c for c in runs.ran if c[0] == "lp"][-1]
    assert lp[:5] == ["lp", "-d", "HP_OfficeJet_Pro_9010", "-n", "2"] and lp[-2:] == ["--", str(pdf)]
    assert ["-P", "1-2"] == lp[lp.index("-P"):lp.index("-P") + 2]
    assert "sides=two-sided-long-edge" in lp and "print-color-mode=monochrome" in lp
    p.print_file(str(photo))
    assert "fit-to-page" in [c for c in runs.ran if c[0] == "lp"][-1]
    with pytest.raises(RuntimeError, match="can't be printed directly"):
        p.print_file(str(cv))
    with pytest.raises(RuntimeError, match="no file"):
        p.print_file(str(tmp_path / "gone.pdf"))
    monkeypatch.setattr(L, "run", Runs({}))
    with pytest.raises(RuntimeError, match="cups"):
        L.LinuxPlatform().print_file(str(pdf))


def test_the_backlight_and_its_brightness_through_logind(monkeypatch, tmp_path):
    for name, kind, cur, mx in (("acpi_video0", "firmware", 5, 10), ("intel_backlight", "raw", 9600, 19200)):
        d = tmp_path / name
        d.mkdir()
        (d / "type").write_text(kind + "\n")
        (d / "brightness").write_text(f"{cur}\n")
        (d / "max_brightness").write_text(f"{mx}\n")
    b = L.read_backlight(str(tmp_path))
    assert b == {"name": "acpi_video0", "value": 5, "max": 10, "percent": 50}       # the kind the kernel means
    assert L.read_backlight(str(tmp_path / "none")) is None
    monkeypatch.setattr(L, "BACKLIGHT", str(tmp_path))
    monkeypatch.setattr(L, "read_backlight", lambda root=str(tmp_path): {"name": "intel_backlight", "value": 9600,
                                                                          "max": 19200, "percent": 50})
    runs = Runs({"busctl call": (0, "", "")})
    monkeypatch.setattr(L, "run", runs)
    p = L.LinuxPlatform()
    assert p.set_brightness(30) == 30
    assert runs.ran[-1][-4:] == ["ssu", "backlight", "intel_backlight", "5760"]
    assert p.set_brightness(None, -60) == L.MIN_BRIGHTNESS        # never all the way down: the screen would go dark
    assert p.display_status()["brightness"] == 50


def test_a_desktop_monitor_that_can_t_be_dimmed_says_so(monkeypatch):
    monkeypatch.setattr(L, "read_backlight", lambda root=None: None)
    monkeypatch.setattr(L, "run", Runs({"ddcutil detect": (0, "No displays found.\n", "")}))
    monkeypatch.setattr(L.shutil, "which", lambda c: "/usr/bin/ddcutil")
    p = L.LinuxPlatform()
    st = p.display_status()
    assert st["brightness"] is None and "monitor's own buttons" in st["why_not"]
    with pytest.raises(RuntimeError, match="own buttons"):
        p.set_brightness(30)


def test_keyboard_layouts(monkeypatch, tmp_path):
    lst = tmp_path / "evdev.lst"
    lst.write_text("! model\n  pc105  Generic 105-key PC\n\n! layout\n  us   English (US)\n  fr   French\n"
                   "  de   German\n\n! variant\n  azerty   fr: French (AZERTY)\n  bepo  fr: French (BEPO)\n"
                   "  nodeadkeys  de: German (no dead keys)\n")
    known = L.xkb_layouts(str(lst))
    assert known["fr"] == {"azerty", "bepo"} and known["us"] == set()
    assert L.parse_setxkbmap("rules:      evdev\nmodel:      pc105\nlayout:     fr\nvariant:    bepo\n") == \
        {"layout": "fr", "variant": "bepo"}
    runs = Runs({"setxkbmap": (0, "", "")})
    monkeypatch.setattr(L, "run", runs)
    monkeypatch.setattr(L, "xkb_layouts", lambda path=None: known)
    p = L.LinuxPlatform()
    assert p.set_keyboard_layout("FR") == "fr" and runs.ran[-1] == ["setxkbmap", "-layout", "fr"]
    assert p.set_keyboard_layout("fr(bepo)") == "fr(bepo)" and runs.ran[-1][-2:] == ["-variant", "bepo"]
    for bad in ("klingon", "fr(dvorak)", "us; rm -rf ~", ""):
        with pytest.raises(RuntimeError):
            p.set_keyboard_layout(bad)


def test_night_light_colours():
    assert gamma.kelvin(0) == 6500 and gamma.kelvin(50) == 4600 and gamma.kelvin(100) == 2700
    assert gamma.whitepoint(6500) == (1.0, 1.0, 1.0)
    r, g, b = gamma.whitepoint(4600)
    assert r == 1.0 and 0.8 < g < 0.9 and 0.7 < b < 0.8 and b < g          # warm: less blue, then less green
    warmer = gamma.whitepoint(2700)
    assert warmer[2] < b and warmer[1] < g


def test_night_light_ramps_are_scaled_not_curved(monkeypatch):
    """White becomes warm white (each colour scaled), so a white page turns warm too (xrandr --gamma would curve
    the middle and leave white white)."""
    set_ramps = {}

    class Ramp:
        def __init__(self, n):
            self.contents = SimpleNamespace(size=n, red=[0] * n, green=[0] * n, blue=[0] * n)

    xrr = SimpleNamespace(XRRAllocGamma=lambda n: Ramp(n), XRRFreeGamma=lambda r: None,
                          XRRSetCrtcGamma=lambda dpy, crtc, ramp: set_ramps.__setitem__(crtc, ramp.contents))
    monkeypatch.setattr(gamma, "_each_crtc", lambda fn: (fn(xrr, None, 71, 256), fn(xrr, None, 72, 256)) and 2)
    assert gamma.set_temperature(gamma.kelvin(50)) == 2
    r = set_ramps[71]
    w = gamma.whitepoint(gamma.kelvin(50))
    assert r.red[255] == 65535 and r.green[255] == int(65535 * w[1]) and r.blue[255] == int(65535 * w[2])
    assert r.green[128] == int(128 * 65535 / 255 * w[1])           # a straight line from black: scaled
    assert r.red[0] == r.green[0] == r.blue[0] == 0 and 72 in set_ramps


# ---------------------------------------------------------------- the schedule
def test_when_things_ring():
    now = dt.datetime(2026, 10, 20, 12, 5).timestamp()
    six = dt.datetime(2026, 10, 20, 18, 0).timestamp()
    assert sch.when("6pm", now=now) == six == sch.when("18:00", now=now) == sch.when("18.00", "today", now=now)
    assert sch.when("7", "tomorrow", now=now) == dt.datetime(2026, 10, 21, 7, 0).timestamp()
    assert sch.when("07:00", now=now) == dt.datetime(2026, 10, 21, 7, 0).timestamp()     # passed today: tomorrow
    assert sch.when("2026-10-25 09:30", now=now) == dt.datetime(2026, 10, 25, 9, 30).timestamp()
    assert sch.when(in_minutes=10, now=now) == now + 600
    assert sch.when("12:00 am", "2026-10-22", now=now) == dt.datetime(2026, 10, 22, 0, 0).timestamp()
    for bad in (dict(at="25:00"), dict(at="soon"), dict(at="10:00", date="today"), dict(), dict(in_minutes=0),
                dict(at="9am", date="next week")):
        with pytest.raises(ValueError):
            sch.when(**bad, now=now)
    assert sch.say_duration(600) == "10-minute" and sch.say_duration(90) == "90-second"
    assert sch.say_duration(7200) == "2-hour" and sch.clock(six) == "6 pm"
    assert sch.clock(dt.datetime(2026, 10, 21, 7, 30).timestamp()) == "7:30 am"


def test_the_schedule_keeps_through_a_restart_and_repeats(tmp_path):
    path = tmp_path / "schedule.json"
    s = sch.Schedule(path)
    fri = dt.datetime(2026, 10, 23, 12, 0).timestamp()          # a Friday
    t = s.add("timer", fri + 600, "pasta", duration=600, now=fri)
    a = s.add("alarm", dt.datetime(2026, 10, 24, 7, 0).timestamp(), "", "weekdays", now=fri)
    assert dt.datetime.fromtimestamp(a["at"]) == dt.datetime(2026, 10, 26, 7, 0)    # Saturday: Monday instead
    assert sch.describe(t, fri) == "a 10-minute timer for pasta (rings at 12:10 pm)"
    assert sch.describe(a, fri) == "an alarm at 7 am every weekday"
    s2 = sch.Schedule(path)
    assert [i["id"] for i in s2.items()] == [t["id"], a["id"]] and s2.next_at() == fri + 600
    assert [i["id"] for i in s2.due(fri + 601)] == [t["id"]]
    s2.rang(t["id"], fri + 601)
    s2.rang(a["id"], dt.datetime(2026, 10, 26, 7, 0, 5).timestamp())
    left = s2.items()
    assert len(left) == 1 and dt.datetime.fromtimestamp(left[0]["at"]) == dt.datetime(2026, 10, 27, 7, 0)
    assert [i["kind"] for i in s2.cancel("alarms")] == ["alarm"] and s2.items() == []
    (tmp_path / "bad.json").write_text("{not json")
    assert sch.Schedule(tmp_path / "bad.json").items() == []


def test_what_stops_a_ringing_alarm_and_what_is_a_request():
    for said in ("stop", "Okay, thanks.", "I'm up", "turn it off", "Claude stop"):
        assert answers.stops_ringing_only(said), said
    for said in ("what's the weather today", "play some jazz", "stop the music and open youtube"):
        assert not answers.stops_ringing_only(said), said
    assert answers.is_snooze("Snooze!") and not answers.is_snooze("stop")


# ---------------------------------------------------------------- the tools, on the fake machine
def _tools(tmp_path, platform=None):
    home = tmp_path / "home"
    home.mkdir(exist_ok=True)
    paths = Paths(home / "ChatComputer").ensure()
    ctx = T.ToolContext(paths, Settings(), platform or FakePlatform(), Journal(paths), RecordingScreen(), Memory(paths),
                        schedule=sch.Schedule(paths.root / "schedule.json"))
    _, _, names = T.build(ctx)
    return ctx, ctx, names


def call(ctx, name, args):
    out = asyncio.run(ctx.tool_fns[name](args))
    texts = [c["text"] for c in out["content"] if c.get("type") == "text"]
    return "\n".join(texts), bool(out.get("is_error"))


def test_drive_tool(tmp_path):
    p = FakePlatform()
    ctx, server, names = _tools(tmp_path, p)
    assert {"drives", "printer", "display", "reminders"} <= set(names)
    text, err = call(server, "drives", {"action": "list"})
    assert not err and text == "No USB drive or memory card is plugged in."
    p.drive_list = [{"id": "sdb1", "name": "SANDISK", "kind": "USB drive", "size_gb": 32.0, "mounted": False,
                     "path": None, "free_gb": None}]
    text, err = call(server, "drives", {"action": "open", "drive": "sandisk"})
    assert not err and text.endswith("/media/sam/SANDISK")
    assert json.loads(call(server, "drives", {"action": "list"})[0])[0]["path"] == "/media/sam/SANDISK"
    text, err = call(server, "drives", {"action": "eject", "drive": "SANDISK"})
    assert not err and "safe to pull it out" in text and p.drive_list == []
    text, err = call(server, "drives", {"action": "eject", "drive": "SANDISK"})
    assert err and "no drive" in text


def test_printer_tool(tmp_path):
    p = FakePlatform()
    ctx, server, _ = _tools(tmp_path, p)
    pdf = tmp_path / "home" / "Tax letter.pdf"
    pdf.write_bytes(b"%PDF")
    text, err = call(server, "printer", {"action": "print", "path": str(pdf), "copies": 2})
    assert not err and text == "Sent Tax letter.pdf to HP OfficeJet Pro 9010, 2 copies. It's printing."
    assert p.jobs[-1]["copies"] == 2
    text, err = call(server, "printer", {"action": "print", "path": str(ctx.paths.secrets / "claude_token")})
    assert err and "private" in text
    p.printer_list = []
    text, err = call(server, "printer", {"action": "print", "path": str(pdf)})
    assert err and text == "No printer is connected."
    assert "found by itself" in call(server, "printer", {"action": "list"})[0]


def test_display_tool(tmp_path):
    p = FakePlatform()
    ctx, server, _ = _tools(tmp_path, p)
    assert call(server, "display", {"action": "brightness", "value": "30%"}) == ("The brightness is 30% now.", False)
    assert call(server, "display", {"action": "dimmer"})[0] == "The brightness is 15% now."
    text, err = call(server, "display", {"action": "night_light", "mode": "evenings", "from": "8pm", "warmth": 70})
    assert not err and "every evening at 20:00" in text and "07:00" in text
    s = Settings.load(ctx.paths)
    assert (s.night_light, s.night_light_from, s.night_light_warmth) == ("evenings", "20:00", 70)
    assert call(server, "display", {"action": "night_light", "mode": "on"})[0].startswith("The night light is on")
    assert p.night == {"on": True, "warmth": 70}
    assert json.loads(call(server, "display", {"action": "status"})[0])["night_light_setting"] == "on"
    p.backlight = None                          # a desktop's monitor
    text, err = call(server, "display", {"action": "brightness", "value": 30})
    assert err and "monitor itself" in text
    text, err = call(server, "display", {"action": "night_light", "mode": "evenings", "from": "whenever"})
    assert err


def test_reminders_tool(tmp_path):
    ctx, server, _ = _tools(tmp_path)
    text, err = call(server, "reminders", {"action": "timer", "minutes": 10})
    assert not err and text.startswith("Set a 10-minute timer (rings at ")
    item = ctx.schedule.items()[0]
    assert item["kind"] == "timer" and abs(item["at"] - time.time() - 600) < 5
    text, err = call(server, "reminders", {"action": "reminder", "text": "call Mum", "in_minutes": 30})
    assert not err and "call Mum" in text
    text, err = call(server, "reminders", {"action": "alarm", "at": "7am", "date": "tomorrow"})
    assert not err and "an alarm at 7 am tomorrow" in text
    assert call(server, "reminders", {"action": "reminder", "at": "6pm"})[1]            # no text: refused
    assert call(server, "reminders", {"action": "timer"})[1]
    listed = call(server, "reminders", {"action": "list"})[0].splitlines()
    assert len(listed) == 3
    text, _ = call(server, "reminders", {"action": "cancel", "id": "all"})
    assert text.startswith("Cancelled a 10-minute timer") and ctx.schedule.items() == []
    assert call(server, "reminders", {"action": "stop"})[0] == "Nothing is ringing."


def test_keyboard_layout_setting(tmp_path):
    p = FakePlatform()
    ctx, server, _ = _tools(tmp_path, p)
    text, err = call(server, "chatcomputer_settings", {"action": "set", "key": "keyboard_layout", "value": "fr"})
    assert not err and p.layout["layout"] == "fr" and Settings.load(ctx.paths).keyboard_layout == "fr"

    def refuse(layout, variant=""):
        raise RuntimeError("'qq' isn't a keyboard layout this computer knows.")
    p.set_keyboard_layout = refuse
    text, err = call(server, "chatcomputer_settings", {"action": "set", "key": "keyboard_layout", "value": "qq"})
    assert err and Settings.load(ctx.paths).keyboard_layout == "fr"


def test_whats_done_says_what_these_did():
    assert describe("mcp__chatcomputer__drives", {"action": "eject", "drive": "SANDISK"}, "SANDISK is ejected") == \
        ("Ejected SANDISK", False)
    assert describe("mcp__chatcomputer__drives", {"action": "list"}, "[]") is None
    assert describe("mcp__chatcomputer__printer", {"action": "print", "path": "/h/Tax.pdf", "copies": 2}, "Sent") == \
        ("Printed Tax.pdf (2 copies)", False)
    assert describe("mcp__chatcomputer__display", {"action": "brightness", "value": 30}, "ok") == \
        ("Set the brightness to 30%", False)
    assert describe("mcp__chatcomputer__display", {"action": "night_light", "mode": "evenings"}, "ok")[0] == \
        "Set the night light to come on in the evenings"
    assert describe("mcp__chatcomputer__reminders", {"action": "timer", "minutes": 10},
                    "Set a 10-minute timer (rings at 3:10 pm). [t1a2b3]") == ("Set a 10-minute timer (rings at 3:10 pm)", False)
    assert describe("mcp__chatcomputer__reminders", {"action": "list"}, "Nothing is set.") is None


# ---------------------------------------------------------------- the controller: ringing, drives, night light
def _controller(tmp_path, monkeypatch, platform=None):
    monkeypatch.setenv("CHATCOMPUTER_HOME", str(tmp_path / "ChatComputer"))
    screen = RecordingScreen()
    c = Controller(Paths().ensure(), Settings(user_name="Sam Lee"), platform or FakePlatform(), screen, voice=False,
                   index=False)
    return c, screen


def _said(screen):
    return [v[1] for k, v in screen.events if k == "ai"]


def test_a_timer_rings_on_time_and_stop_silences_it(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)
    asked = []

    async def ask(text, typed=False):
        asked.append(text)
        return "t1"
    c.brain = SimpleNamespace(busy=False, last_activity=0.0, ask=ask, current_origin="here")
    c.model = SimpleNamespace(ready=True)

    async def run():
        c.loop = asyncio.get_running_loop()
        c._time_wake = asyncio.Event()
        keeper = asyncio.create_task(c._keep_time())
        now = time.time()
        c.schedule.add("timer", now + 0.3, "", duration=600, now=now - 600)
        c._schedule_changed()
        await asyncio.sleep(0.8)
        assert c.ringing is not None and c.ringing["kind"] == "timer"
        await c.handle("stop")                      # about the timer: silences it, goes no further
        assert c.ringing is None and asked == []
        # An alarm, snoozed.
        c.schedule.add("alarm", time.time() + 0.2, "", now=time.time())
        c._schedule_changed()
        await asyncio.sleep(0.7)
        assert c.ringing is not None
        await c.handle("snooze")
        assert c.ringing is None and c.schedule.items()[0]["kind"] == "alarm"
        assert abs(c.schedule.items()[0]["at"] - time.time() - 540) < 5
        # Ringing, and asked for something else: it stops, and that goes to Claude.
        c.schedule.add("reminder", time.time() + 0.2, "call Mum", now=time.time())
        c._schedule_changed()
        await asyncio.sleep(0.6)
        await c.handle("what's the weather tomorrow", typed=True)
        keeper.cancel()
    asyncio.run(run())
    said = _said(screen)
    assert "Your 10-minute timer is done." in said and "Reminder: call Mum." in said
    assert any(s.startswith("Snoozed. I'll ring again at ") for s in said)
    assert any(s.startswith("It's ") for s in said)
    assert len(asked) == 1 and asked[0].endswith("what's the weather tomorrow")


def test_one_that_came_due_while_the_computer_was_off_is_shown_as_missed(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)
    an_hour_ago = time.time() - 3600
    c.schedule.add("alarm", an_hour_ago, "", now=an_hour_ago - 60)

    async def run():
        c.loop = asyncio.get_running_loop()
        c._time_wake = asyncio.Event()
        keeper = asyncio.create_task(c._keep_time())
        await asyncio.sleep(0.3)
        keeper.cancel()
    asyncio.run(run())
    assert c.ringing is None and c.schedule.items() == []
    assert any("couldn't ring" in s for s in _said(screen))


def test_a_drive_plugged_in_opens_by_itself_and_an_ejected_one_stays_closed(tmp_path, monkeypatch):
    p = FakePlatform()
    c, screen = _controller(tmp_path, monkeypatch, p)
    monkeypatch.setattr("chatcomputer.controller.DRIVE_CHECK_S", 0.05)

    async def run():
        c.loop = asyncio.get_running_loop()
        watcher = asyncio.create_task(c._watch_hardware())
        await asyncio.sleep(0.15)
        p.drive_list.append({"id": "sdb1", "name": "SANDISK", "kind": "USB drive", "size_gb": 32.0, "mounted": False,
                             "path": None})
        await asyncio.sleep(0.2)
        assert p.drive_list[0]["mounted"]
        p.drive_list[0]["mounted"] = False          # ejected here, but still plugged in
        await asyncio.sleep(0.2)
        assert not p.drive_list[0]["mounted"]
        watcher.cancel()
    asyncio.run(run())
    assert ("open_drive", ("sdb1",)) in p.calls and len([x for x in p.calls if x[0] == "open_drive"]) == 1
    assert "SANDISK is plugged in: its files are ready." in [v[0] for k, v in screen.events if k == "notice"]


def test_the_night_light_by_the_clock(tmp_path, monkeypatch):
    p = FakePlatform()
    c, _ = _controller(tmp_path, monkeypatch, p)
    at = lambda h, m=0: dt.datetime(2026, 10, 20, h, m).timestamp()      # noqa: E731
    c.settings.night_light = "evenings"
    assert c._night_light_wanted(at(22)) and c._night_light_wanted(at(6, 59)) and not c._night_light_wanted(at(7))
    assert not c._night_light_wanted(at(12))
    c.settings.night_light_from, c.settings.night_light_until = "13:00", "15:00"
    assert c._night_light_wanted(at(14)) and not c._night_light_wanted(at(16))
    c.settings.night_light = "on"
    c.apply_night_light()
    assert p.night == {"on": True, "warmth": 50}
    n = len(p.calls)
    c.apply_night_light()                       # already so: nothing done
    assert len(p.calls) == n
    c.settings.night_light = "off"
    c.apply_night_light()
    assert p.night["on"] is False


# ---------------------------------------------------------------- the lock screen
def test_the_pin_is_kept_only_as_a_hash_and_guessing_is_slowed(tmp_path):
    from chatcomputer.lock import PinLock
    lock = PinLock(tmp_path / "secrets" / "lock_pin")
    assert not lock.has_pin() and lock.check("1234") == (True, "")
    with pytest.raises(ValueError):
        lock.set_pin("12a4")
    lock.set_pin("482613")
    stored = lock.path.read_text()
    assert "482613" not in stored and (lock.path.stat().st_mode & 0o777) == 0o600
    assert lock.check("482613") == (True, "")
    now = 1000.0
    for _ in range(4):
        assert lock.check("000000", now) == (False, "That's not the PIN.")
    ok, why = lock.check("000000", now)
    assert not ok and "30 seconds" in why
    assert lock.check("482613", now + 5)[0] is False             # even the right one waits
    assert lock.check("482613", now + 31) == (True, "")
    lock.remove_pin()
    assert not lock.has_pin()


def test_locking_choosing_a_pin_and_unlocking(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)
    asked = []

    async def ask(text, typed=False):
        asked.append(text)
        return "t1"
    c.brain = SimpleNamespace(busy=False, last_activity=0.0, ask=ask, current_origin="here")
    c.model = SimpleNamespace(ready=True)

    async def run():
        c.loop = asyncio.get_running_loop()
        said = await c.lock_action("lock")                   # no PIN yet: one is chosen first, on the screen
        assert "no PIN yet" in said and not c.locked and screen.form["id"] == "lock_pin"
        c.page_event({"type": "account", "form": "lock_pin", "values": {"pin": "2468", "again": "2486"}})
        assert screen.form["error"].startswith("The two PINs aren't the same") and not c.locked
        c.page_event({"type": "account", "form": "lock_pin", "values": {"pin": "2468", "again": "2468"}})
        assert c.locked and screen.form is None and c.has_lock_pin()
        await c.handle("open my email")                        # someone else, talking: nothing happens
        await c.handle("show me the photos", typed=True)
        assert asked == []
        c.page_event({"type": "open_message", "id": "m1"})    # nothing else from the page while locked
        c.page_event({"type": "unlock", "pin": "1111"})
        assert c.locked and screen.events[-1] == ("lock", {"error": "That's not the PIN."})
        c.page_event({"type": "unlock", "pin": "2468"})
        assert not c.locked and screen.events[-1] == ("lock", None)
        await c.handle("open my email")
        assert len(asked) == 1
        # Going to sleep locks it again.
        await c._sleep_when_quiet()
        assert c.locked
        c.page_event({"type": "unlock", "pin": "2468"})
        # Turning the PIN off needs the PIN.
        assert "type their PIN" in await c.lock_action("remove_pin")
        c.page_event({"type": "account", "form": "lock_pin", "values": {"current": "9999"}})
        assert c.has_lock_pin() and screen.form["error"] == "That's not the PIN."
        c.page_event({"type": "account", "form": "lock_pin", "values": {"current": "2468"}})
        assert not c.has_lock_pin() and await c.lock_action("lock") != "Locked."
    asyncio.run(run())
    assert "2468" not in (c.paths.logs / "chatcomputer.log").read_text() if (c.paths.logs / "chatcomputer.log").exists() else True


def test_a_chat_computer_with_a_pin_starts_locked(tmp_path, monkeypatch):
    c, screen = _controller(tmp_path, monkeypatch)
    c.pinlock.set_pin("1357")
    c2, screen2 = _controller(tmp_path, monkeypatch)

    async def run():
        await c2.start()
        await c2.stop()
    asyncio.run(run())
    assert c2.locked and ("lock", {"note": "Type your PIN to unlock."}) in screen2.events


def test_lock_screen_tool(tmp_path):
    calls = []

    async def lock(action):
        calls.append(action)
        return "Locked. Nothing can be used until the PIN is typed on the screen."
    ctx, server, names = _tools(tmp_path)
    assert "lock_screen" in names
    ctx.lock = lock
    text, err = call(server, "lock_screen", {"action": "lock"})
    assert not err and calls == ["lock"] and text.startswith("Locked.")
    assert describe("mcp__chatcomputer__lock_screen", {"action": "lock"}, text) == ("Locked the screen", False)


# ---------------------------------------------------------------- what a computer is expected to answer and do besides:
# what's keeping it busy, its camera, the kind of computer, updates, the internet's speed, VPNs, PDFs, screenshots,
# and backups to a USB drive
def test_the_kind_of_computer_from_its_firmware(tmp_path):
    for n, kind in ((3, "desktop"), (10, "laptop"), (13, "all-in-one"), (35, "mini PC"), (31, "tablet"), (1, None)):
        (tmp_path / "chassis").write_text(f"{n}\n")
        assert L.computer_kind(str(tmp_path / "chassis"), str(tmp_path / "none")) == kind
    for model, kind in ((b"Apple MacBook Air (M1, 2020)\0", "laptop"), (b"Apple Mac mini (M2, 2023)\0", "desktop"),
                        (b"Apple iMac (24-inch, M3, 2023)\0", "all-in-one")):
        (tmp_path / "model").write_bytes(model)
        assert L.computer_kind(str(tmp_path / "none"), str(tmp_path / "model")) == kind


def test_programs_by_the_names_people_know():
    assert L.friendly_name("WebKitWebProcess", []) == "web pages open in Chat Computer"
    assert L.friendly_name("python3", ["/opt/chatcomputer/venv/bin/python", "-m", "chatcomputer"]) == "Chat Computer"
    assert L.friendly_name("python3", ["python3", "myscript.py"]) == "python3"
    assert L.friendly_name("anything", [], own_pid=True) == "Chat Computer"


def test_what_is_keeping_it_busy_and_the_camera_on_this_machine():
    b = L.LinuxPlatform().busiest(3)
    assert b["cores"] >= 1 and 0 <= b["cpu_percent"] <= 100 and b["memory"]["total_gb"] > 0
    assert 0.4 <= b["measured_over_s"] <= 2 and len(b["load_1_5_15_min"]) == 3
    assert 1 <= len(b["programs"]) <= 6 and {"name", "cpu_percent", "memory_mb"} <= set(b["programs"][0])
    cam = L.LinuxPlatform().camera_status()
    assert isinstance(cam["cameras"], list) and isinstance(cam["in_use_by"], list)


def test_updates_waiting_from_apt_and_dnf(monkeypatch):
    out = ("Listing...\nfirefox/noble-updates 131.0+build1-0ubuntu0.24.04.1 amd64 [upgradable from: 130.0+build2]\n"
           "tzdata/noble-updates,noble-security 2026b-0ubuntu0.24.04 all [upgradable from: 2026a-0ubuntu0.24.04]\n")
    assert L.parse_apt_upgradable(out) == [
        {"name": "firefox", "version": "131.0+build1-0ubuntu0.24.04.1", "from": "130.0+build2"},
        {"name": "tzdata", "version": "2026b-0ubuntu0.24.04", "from": "2026a-0ubuntu0.24.04"}]
    runs = Runs({"apt list --upgradable": (0, out, "WARNING: apt does not have a stable CLI interface.")})
    monkeypatch.setattr(L, "run", runs)
    r = L.LinuxPlatform().check_updates()
    assert r["count"] == 2 and r["updates"][0]["name"] == "firefox"
    dnf = ("\nfirefox.aarch64          131.0-1.fc42          updates\nkernel-16k.aarch64   6.17.4-400.asahi.fc42   "
           "updates\nObsoleting Packages\nfoo.noarch   1-1   updates\n")
    assert [u["name"] for u in L.parse_dnf_check_update(dnf)] == ["firefox", "kernel-16k"]
    monkeypatch.setattr(L, "run", Runs({"dnf -q --cacheonly check-update": (100, dnf, "")}))
    assert L.MacPlatform().check_updates()["count"] == 2
    monkeypatch.setattr(L, "run", Runs({"dnf -q --cacheonly check-update": (0, "", "")}))
    assert L.MacPlatform().check_updates()["count"] == 0


def test_the_internet_s_speed_against_a_stand_in_for_cloudflare(monkeypatch):
    import http.client
    import http.server
    import threading

    class Speed(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def do_GET(self):
            n = int(self.path.split("bytes=")[1])
            self.send_response(200)
            self.send_header("Content-Length", str(n))
            self.end_headers()
            self.wfile.write(b"0" * n)

        def do_POST(self):
            self.rfile.read(int(self.headers["Content-Length"]))
            self.send_response(200)
            self.send_header("Content-Length", "0")
            self.end_headers()

        def log_message(self, *a):
            pass
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Speed)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    monkeypatch.setattr(L, "SPEED_HOST", f"127.0.0.1:{srv.server_address[1]}")
    monkeypatch.setattr(L, "run", Runs({"ip route show default": (0, "default via 192.168.1.1 dev wlan0 proto dhcp\n", ""),
                                        "ping -c 3": (0, "rtt min/avg/max/mdev = 1.012/2.533/4.104/1.2 ms\n", "")}))
    monkeypatch.setattr(http.client, "HTTPSConnection", lambda host, timeout=None, context=None: http.client.HTTPConnection(host, timeout=timeout))
    try:
        r = L.LinuxPlatform().speed_test()
    finally:
        srv.shutdown()
    assert r["download_mbps"] > 1 and r["upload_mbps"] > 1 and r["latency_ms"] >= 0 and "Cloudflare" in r["server"]
    assert r["router_ms"] == 2.5 and set(r["in_use_before_mbps"]) == {"down", "up"}
    monkeypatch.setattr(L, "SPEED_HOST", "127.0.0.1:9")            # nothing there: the internet can't be reached
    with pytest.raises(RuntimeError, match="can't be reached"):
        L.LinuxPlatform().speed_test()


def test_vpns_through_networkmanager(monkeypatch, tmp_path):
    shown = "Home Wi-Fi:802-11-wireless:yes\nWork:wireguard:no\nProton NL\\:1:vpn:no\nWired:802-3-ethernet:no\n"
    assert L.parse_nmcli_vpns(shown) == [{"name": "Work", "kind": "WireGuard", "on": False},
                                         {"name": "Proton NL:1", "kind": "VPN", "on": False}]
    runs = Runs({"nmcli -t -f NAME,TYPE,ACTIVE connection show": (0, shown, ""),
                 "nmcli connection up id Work": (0, "Connection successfully activated", ""),
                 "nmcli connection up id Proton NL:1": (4, "", "Error: Connection activation failed: Secrets were required, but not provided."),
                 "nmcli connection import type wireguard": lambda cmd: CommandResult(
                     0, f"Connection '{cmd[-1].rsplit('/', 1)[-1][:-5]}' (5a1c) successfully added.\n", "")})
    monkeypatch.setattr(L, "run", runs)
    p = L.LinuxPlatform()
    assert p.vpn_connect("work") == "Work"
    with pytest.raises(RuntimeError, match="needs a password"):
        p.vpn_connect("proton")
    with pytest.raises(RuntimeError, match="Which VPN"):
        p.vpn_connect("")
    with pytest.raises(RuntimeError, match="no VPN called 'Mullvad'"):
        p.vpn_connect("Mullvad")
    conf = tmp_path / "My Office VPN.conf"                  # not a network interface's name: it's renamed for it
    conf.write_text("[Interface]\n")
    assert p.vpn_import(str(conf)) == "My-Office-VPN"
    assert runs.ran[-1][:5] == ["nmcli", "connection", "import", "type", "wireguard"] and runs.ran[-1][-1].endswith("/My-Office-VPN.conf")
    with pytest.raises(RuntimeError, match="isn't a VPN settings file"):
        p.vpn_import(str(tmp_path / "notes.txt"))
    monkeypatch.setattr(L, "run", Runs({"nmcli -t -f NAME,TYPE,ACTIVE connection show": (0, "Home Wi-Fi:802-11-wireless:yes\n", "")}))
    with pytest.raises(RuntimeError, match="No VPN is set up"):
        p.vpn_connect("")


def test_office_documents_become_pdfs_with_libreoffice(monkeypatch, tmp_path):
    doc = tmp_path / "CV.docx"
    doc.write_bytes(b"PK")
    monkeypatch.setattr(L.shutil, "which", lambda name: None)
    with pytest.raises(RuntimeError, match=r"needs LibreOffice.*libreoffice-nogui"):
        L.LinuxPlatform().office_to_pdf(str(doc), str(tmp_path / "out"))
    with pytest.raises(RuntimeError, match=r"libreoffice-writer"):
        L.MacPlatform().office_to_pdf(str(doc), str(tmp_path / "out"))

    def soffice(cmd):
        out = cmd[cmd.index("--outdir") + 1]
        (tmp_path / "out" / "CV.pdf").write_bytes(b"%PDF-1.7")
        assert cmd[1].startswith("-env:UserInstallation=file://") and out == str(tmp_path / "out")
        return CommandResult(0, "convert CV.docx -> CV.pdf", "")
    monkeypatch.setattr(L.shutil, "which", lambda name: "/usr/bin/soffice" if name == "soffice" else None)
    monkeypatch.setattr(L, "run", Runs({"/usr/bin/soffice": soffice}))
    assert L.LinuxPlatform().office_to_pdf(str(doc), str(tmp_path / "out")) == str(tmp_path / "out" / "CV.pdf")


def _home_tools(tmp_path, monkeypatch, platform=None):
    monkeypatch.setenv("HOME", str(tmp_path / "home"))
    return _tools(tmp_path, platform)


def test_system_status_says_what_s_busy_the_camera_and_the_battery(tmp_path):
    p = FakePlatform()
    ctx, server, _ = _tools(tmp_path, p)
    st = json.loads(call(server, "system_status", {})[0])
    assert st["machine"]["kind"] == "laptop" and st["machine"]["battery"] == {"percent": 76, "charging": False}
    assert st["busy"]["programs"][0]["name"] == "Chat Computer" and st["camera"]["in_use_by"] == []
    assert st["windows_drive"]["folder"] == "/mnt/windows"
    p.kind, p.battery = "desktop", None
    st = json.loads(call(server, "system_status", {})[0])
    assert st["machine"]["kind"] == "desktop" and st["machine"]["battery"] is None


def test_display_status_says_the_screen_s_size_and_the_text_size(tmp_path):
    ctx, server, _ = _tools(tmp_path)
    ctx.screen.layout_info = lambda: {"summary": "24-inch screen, viewed from about 70 cm away (estimated): drawn at 125%."}
    st = json.loads(call(server, "display", {"action": "status"})[0])
    assert st["pixels"] == "1920x1080" and st["screen"].startswith("24-inch") and st["text_scale"] == 1.0


def test_software_check_says_what_s_waiting_without_asking(tmp_path):
    ctx, server, _ = _tools(tmp_path)
    _, policies, _ = T.build(ctx)
    assert policies["software"]({"action": "check"}) is None
    assert policies["software"]({"action": "update"}).startswith("Update all the software")
    text, err = call(server, "software", {"action": "check"})
    assert not err and text.startswith("3 updates waiting (as of Friday 16 October, 06:12). Security updates install")
    assert "- firefox 131.0 (now 130.0)" in text
    ctx.platform.updates = []
    assert call(server, "software", {"action": "check"})[0].startswith("Everything is up to date")


def test_wifi_speed(tmp_path):
    p = FakePlatform()
    ctx, server, _ = _tools(tmp_path, p)
    r = json.loads(call(server, "wifi", {"action": "speed"})[0])
    assert r["speed"]["download_mbps"] == 48.2 and r["connection"] == {"connected_to": "HomeWiFi", "signal": 78, "ethernet": False}
    p.online = False
    text, err = call(server, "wifi", {"action": "speed"})
    assert err and "can't be reached" in text


def test_vpn_tool(tmp_path):
    p = FakePlatform()
    ctx, server, names = _tools(tmp_path, p)
    _, policies, _ = T.build(ctx)
    assert "vpn" in names
    text, err = call(server, "vpn", {"action": "list"})
    assert not err and text.startswith("No VPN is set up on this computer.")
    text, err = call(server, "vpn", {"action": "on"})
    assert err and text == "No VPN is set up on this computer."
    conf = tmp_path / "home" / "mullvad-se.conf"
    conf.write_text("[Interface]\n")
    assert policies["vpn"]({"action": "add", "file": str(conf)}).startswith("Add the VPN from mullvad-se.conf?")
    assert call(server, "vpn", {"action": "add", "file": str(conf)})[0].startswith("Added the VPN mullvad-se.")
    assert call(server, "vpn", {"action": "on"})[0] == "mullvad-se is on: the internet goes through it now."
    assert p.vpn_list[0]["on"] and call(server, "vpn", {"action": "off", "name": "mullvad"})[0] == "mullvad-se is off."
    assert describe("mcp__chatcomputer__vpn", {"action": "on", "name": "mullvad-se"}, "") == ("Turned on mullvad-se", False)


def test_printing_a_word_file_makes_a_pdf_of_it_first(tmp_path, monkeypatch):
    p = FakePlatform()
    ctx, server, _ = _home_tools(tmp_path, monkeypatch, p)
    docs = tmp_path / "home" / "Documents"
    docs.mkdir()
    (docs / "CV.docx").write_bytes(b"PK")
    text, err = call(server, "printer", {"action": "print", "path": str(docs / "CV.docx")})
    assert not err and text == "Sent CV.docx to HP OfficeJet Pro 9010. It's printing."
    assert p.jobs[-1]["file"].endswith("/CV.pdf") and ("office_to_pdf", ) == p.calls[-2][:1]
    assert sorted(f.name for f in docs.iterdir()) == ["CV.docx"]            # the PDF was only for printing
    p.printer_list = []
    text, err = call(server, "printer", {"action": "print", "path": str(docs / "CV.docx")})
    assert err and text.startswith("No printer is connected.")
    # A PDF to keep: next to the file, or in a folder of theirs; a Markdown file as Chat Computer shows it; a photo.
    text, err = call(server, "printer", {"action": "save_pdf", "path": str(docs / "CV.docx")})
    assert not err and text == f"Saved {docs / 'CV.pdf'}" and (docs / "CV.pdf").exists()
    assert call(server, "printer", {"action": "save_pdf", "path": str(docs / "CV.docx")})[0] == f"Saved {docs / 'CV 2.pdf'}"
    (docs / "Notes.md").write_text("# Notes\n\nBuy milk.")
    text, err = call(server, "printer", {"action": "save_pdf", "path": str(docs / "Notes.md"), "folder": "Desktop"})
    assert not err and text == f"Saved {tmp_path / 'home' / 'Desktop' / 'Notes.pdf'}"
    assert ctx.screen.events[-1] == ("pdf", (ctx.screen.events[-1][1][0], "Notes.md"))
    from PIL import Image
    Image.new("RGB", (40, 30), (200, 10, 10)).save(docs / "Beach.jpg")
    assert call(server, "printer", {"action": "save_pdf", "path": str(docs / "Beach.jpg")})[0] == f"Saved {docs / 'Beach.pdf'}"
    assert (docs / "Beach.pdf").read_bytes().startswith(b"%PDF")
    text, err = call(server, "printer", {"action": "save_pdf"})                # nothing open
    assert err and "Nothing is open" in text
    asyncio.run(ctx.screen.show_url("https://www.bbc.com/news"))         # what's open, named after its title
    assert call(server, "printer", {"action": "save_pdf"})[0] == f"Saved {docs / 'https www.bbc.com news.pdf'}"
    text, err = call(server, "printer", {"action": "save_pdf", "path": str(docs / "CV.docx"), "folder": str(ctx.paths.secrets)})
    assert err and "private" in text
    assert describe("mcp__chatcomputer__printer", {"action": "save_pdf"}, f"Saved {docs / 'CV.pdf'}") == ("Saved CV.pdf", False)


def test_screenshots_go_where_they_said(tmp_path, monkeypatch):
    p = FakePlatform()
    ctx, server, names = _home_tools(tmp_path, monkeypatch, p)
    assert "screenshot" in names
    text, err = call(server, "screenshot", {})
    shots = list((tmp_path / "home" / "Pictures" / "Screenshots").iterdir())
    assert not err and len(shots) == 1 and text == f"Saved the screenshot: {shots[0]}"
    assert shots[0].name.startswith("Screenshot ") and shots[0].read_bytes().startswith(b"\x89PNG")
    text, err = call(server, "screenshot", {"part": "open", "folder": "Desktop"})
    assert err and "Nothing is open" in text
    asyncio.run(ctx.screen.show_url("https://www.bbc.com/news"))
    text, err = call(server, "screenshot", {"part": "open", "folder": "Desktop"})
    (shot,) = list((tmp_path / "home" / "Desktop").iterdir())
    from PIL import Image
    assert not err and Image.open(shot).size == (800, 600)          # just what's open (RecordingScreen's 800x600)
    text, err = call(server, "screenshot", {"folder": str(ctx.paths.secrets)})
    assert err and "private" in text
    assert call(server, "undo_last_change", {})[0]                    # a screenshot can be undone
    assert not shot.exists()
    assert describe("mcp__chatcomputer__screenshot", {"part": "open"}, "") == ("Took a screenshot of the page", False)


def test_backing_up_to_a_usb_drive_checks_every_file(tmp_path, monkeypatch):
    p = FakePlatform()
    ctx, server, names = _home_tools(tmp_path, monkeypatch, p)
    assert "backup" in names
    home = tmp_path / "home"
    for rel, data in (("Documents/Tax letter.pdf", b"%PDF tax"), ("Documents/Work/Plan.md", b"# Plan"),
                      ("Pictures/Beach.jpg", b"\xff\xd8 beach")):
        (home / rel).parent.mkdir(parents=True, exist_ok=True)
        (home / rel).write_bytes(data)
    text, err = call(server, "backup", {"action": "status"})
    assert not err and text.startswith("No backup has been made on this computer yet.") and "Nothing backs up by itself" in text
    assert "Windows still has its own copies" in text
    text, err = call(server, "backup", {"action": "now"})
    assert err and text.startswith("No USB drive")
    stick = tmp_path / "stick"
    stick.mkdir()
    p.drive_list = [{"id": "sdb1", "name": "SANDISK", "kind": "USB drive", "size_gb": 32.0, "mounted": False,
                     "path": None, "free_gb": None, "folder": str(stick)}]
    text, err = call(server, "backup", {"action": "now", "folders": ["Documents"]})
    assert not err and text.startswith("Backed up Documents to SANDISK: 2 files,") and "Every file was read back and matches." in text
    (dated,) = list((stick / "Chat Computer backup").iterdir())
    assert (dated / "Documents" / "Work" / "Plan.md").read_bytes() == b"# Plan"
    assert ("open_drive", ("sdb1",)) in p.calls
    text, err = call(server, "backup", {"action": "status"})
    assert text.startswith("Last backup: ") and "to SANDISK (Documents: 2 files" in text and "SANDISK (plugged in) has backups from" in text
    text, err = call(server, "backup", {"action": "now", "folders": ["~"]})       # the home folder holds the private files
    assert err and "private sign-in files" in text
    assert describe("mcp__chatcomputer__backup", {"action": "now"}, "Backed up Documents to SANDISK: 2 files") == \
        ("Backed up Documents to SANDISK", False)


def test_copying_a_folder_says_how_much_and_that_it_all_arrived(tmp_path, monkeypatch):
    ctx, server, _ = _home_tools(tmp_path, monkeypatch)
    src = tmp_path / "home" / "Documents"
    (src / "Work").mkdir(parents=True)
    (src / "a.txt").write_text("a" * 1500)
    (src / "Work" / "b.txt").write_text("b")
    text, err = call(server, "copy_files", {"items": [{"from": str(src), "to": str(tmp_path / "copy")}]})
    assert not err and text.endswith("(2 files, 1.5 KB, every one checked)")
    text, err = call(server, "move_files", {"items": [{"from": str(tmp_path / "copy"), "to": str(tmp_path / "moved")}]})
    assert not err and text.endswith("(2 files, 1.5 KB)")


def test_a_bluetooth_device_s_battery_when_it_tells_it(monkeypatch, tmp_path):
    assert L.parse_bt_battery("Device AA:BB\n\tName: AirPods Pro\n\tBattery Percentage: 0x5a (90)\n") == 90
    assert L.parse_bt_battery("\tName: Speaker\n") is None
    (tmp_path / "hci0").mkdir()
    monkeypatch.setattr(L.LinuxPlatform, "has_bluetooth", lambda self, sysfs="": True)
    monkeypatch.setattr(L, "run", Runs({"bluetoothctl show": (0, "Powered: yes\n", ""),
                                        "bluetoothctl devices Connected": (0, "Device AA:BB:CC:DD:EE:01 AirPods Pro\n", ""),
                                        "bluetoothctl devices Paired": (0, "Device AA:BB:CC:DD:EE:01 AirPods Pro\n", ""),
                                        "bluetoothctl info AA:BB:CC:DD:EE:01": (0, "\tBattery Percentage: 0x55 (85)\n", "")}))
    st = L.LinuxPlatform().bt_status()
    assert st["connected"] == [{"address": "AA:BB:CC:DD:EE:01", "name": "AirPods Pro", "battery": 85}]


def test_the_power_mode_from_the_firmware_or_the_processor(tmp_path):
    assert L.power_mode(str(tmp_path)) is None
    cpu = tmp_path / "devices/system/cpu/cpu0/cpufreq"
    cpu.mkdir(parents=True)
    (cpu / "scaling_governor").write_text("powersave\n")
    assert L.power_mode(str(tmp_path)) == "powersave"
    (cpu / "energy_performance_preference").write_text("balance_power\n")
    assert L.power_mode(str(tmp_path)) == "powersave (balance_power)"
    (tmp_path / "firmware/acpi").mkdir(parents=True)
    (tmp_path / "firmware/acpi/platform_profile").write_text("low-power\n")
    assert L.power_mode(str(tmp_path)) == "low-power"


def test_under_a_message_read_aloud_and_good_or_bad(tmp_path, monkeypatch):
    # Read aloud, under a message: said now, in place of what was being said, even with speech off; without the
    # voice yet, the screen says so. Good or bad, under an answer: kept with the question (feedback.jsonl).
    c, screen = _controller(tmp_path, monkeypatch)
    said, notices = [], []

    class FakeSpeaker:
        tts = object()
        muted = True

        def stop(self):
            said.append("(stopped)")

        def say(self, text):
            said.append((text, self.muted))
    c.speaker = FakeSpeaker()
    monkeypatch.setattr(screen, "notice", lambda text, kind="info": notices.append(text), raising=False)
    c.page_event({"type": "speak", "text": "  The air in Singapore is good.  "})
    assert said == ["(stopped)", ("The air in Singapore is good.", False)] and c.speaker.muted is True
    c.speaker = None
    c.page_event({"type": "speak", "text": "Hello"})
    assert notices and "voice" in notices[-1]
    c.page_event({"type": "feedback", "id": "a3", "good": False, "on": True, "prompt": "who won?", "text": "Spain."})
    c.page_event({"type": "feedback", "id": "a3", "good": False, "on": False, "prompt": "who won?", "text": "Spain."})
    lines = [json.loads(x) for x in (c.paths.root / "feedback.jsonl").read_text().splitlines()]
    assert [(x["good"], x["on"], x["prompt"], x["answer"], x["id"]) for x in lines] == [
        (False, True, "who won?", "Spain.", "a3"), (False, False, "who won?", "Spain.", "a3")]
