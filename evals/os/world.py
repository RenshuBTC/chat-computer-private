"""The simulated computer's hardware, as one small JSON file in the case's sandbox: what's plugged in (a USB stick
and the files on it), the printers and what was sent to them, the screen's backlight and night light, the
keyboard layout, the camera, the updates waiting, the internet's speed and the VPNs set up. Chat Computer's platform (platform.py) and the commands of the person's shell (standin.py: lsblk,
udisksctl, lp, lpstat, setxkbmap...) both read and change it, so whichever way Chat Computer goes about something,
its tools or a command, it meets the same machine.

A drive's files are where a real computer has them: under /media/sam/<label> while it's mounted (opened), and out
of reach otherwise (kept in UNPLUGGED, which stands for the stick itself)."""
from __future__ import annotations

import contextlib
import fcntl
import json
import os
import shutil
from pathlib import Path
from typing import Any, Iterator

FILE = Path(os.environ.get("EVAL_WORLD", "/tmp/.ev/machine.json"))
MEDIA = Path(os.environ.get("EVAL_MEDIA", "/media/sam"))
UNPLUGGED = Path(os.environ.get("EVAL_UNPLUGGED", "/tmp/.ev/drive-contents"))
# The sandbox's time zone: every program in it has TZ=:LOCALTIME (run.py), so changing this file changes the time
# zone for all of them, as /etc/localtime does on a real computer.
LOCALTIME = Path(os.environ.get("EVAL_LOCALTIME", "/tmp/.ev/io/localtime"))
ZONEINFO = Path("/usr/share/zoneinfo")
# The updates waiting on an Ubuntu computer a few weeks after its last ones.
UPDATES = [{"name": "firefox", "version": "131.0+build1-0ubuntu0.24.04.1", "from": "130.0+build2-0ubuntu0.24.04.1"},
           {"name": "libssl3t64", "version": "3.0.13-0ubuntu3.5", "from": "3.0.13-0ubuntu3.4"},
           {"name": "tzdata", "version": "2026b-0ubuntu0.24.04", "from": "2026a-0ubuntu0.24.04"}]
KEYBOARD_LAYOUTS = {"us", "gb", "fr", "de", "es", "it", "pt", "br", "be", "ch", "nl", "se", "no", "dk", "fi", "pl",
                    "cz", "hu", "ru", "ua", "gr", "tr", "jp", "kr", "cn", "latam", "ca", "ie", "in", "ara", "il"}


def init(state: dict[str, Any], usb_files: dict[str, bytes]) -> dict[str, Any]:
    """The machine at the start of a case, from the case's state; the USB stick's files put on the stick."""
    laptop = state.get("device") == "laptop"
    m: dict[str, Any] = {
        "device": state.get("device", "laptop"),
        "backlight": state.get("backlight") if laptop else None,
        "night_light": {"on": False, "warmth": 0},
        "keyboard_layout": "us", "keyboard_variant": "",
        "printers": [{"name": p["name"], "queue": p["name"].replace(" ", "_"), "state": p.get("state", "idle")}
                     for p in state.get("printers") or []],
        "print_jobs": [],
        "drives": [],
        "timezone": state.get("timezone", "Asia/Singapore"),
        # A laptop's own camera; a desktop has none unless the case gives one.
        "cameras": state.get("cameras", ["Integrated Camera"] if state.get("device", "laptop") == "laptop" else []),
        "updates": state.get("updates", UPDATES),
        "speed": state.get("speed", {"download_mbps": 9.8, "upload_mbps": 2.4, "latency_ms": 58, "router_ms": 3.4,
                                     "in_use_before_mbps": {"down": 0.05, "up": 0.01}}),
        "vpns": state.get("vpns", []),
        "cpu": state.get("cpu", {"ghz": 1.6, "max_ghz": 4.4, "power_mode": "balanced", "temp_c": 46}),
    }
    set_zone_file(m["timezone"])
    usb = state.get("usb")
    if usb:
        label = usb.get("label", "USB")
        m["drives"].append({"id": "sdb1", "disk": "sdb", "label": label, "size_gb": float(usb.get("size_gb", 32)),
                            "fstype": "vfat", "model": usb.get("model", "Ultra"), "vendor": "SanDisk",
                            "mounted": False, "ejected": False})
        root = UNPLUGGED / label
        shutil.rmtree(root, ignore_errors=True)
        for rel, data in usb_files.items():
            p = root / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(data)
    save(m)
    return m


def set_zone_file(zone: str) -> None:
    src = ZONEINFO / zone
    if not src.is_file() or ".." in zone:
        raise RuntimeError(f"{zone} isn't a time zone")
    LOCALTIME.parent.mkdir(parents=True, exist_ok=True)
    LOCALTIME.write_bytes(src.read_bytes())


@contextlib.contextmanager
def changing() -> Iterator[dict[str, Any]]:
    """The machine, to read and change, with no one else changing it meanwhile; written back afterwards."""
    FILE.parent.mkdir(parents=True, exist_ok=True)
    with open(str(FILE) + ".lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        m = load()
        yield m
        save(m)


def load() -> dict[str, Any]:
    try:
        return json.loads(FILE.read_text())
    except (OSError, ValueError):
        return {"drives": [], "printers": [], "print_jobs": []}


def save(m: dict[str, Any]) -> None:
    tmp = FILE.with_suffix(".tmp")
    tmp.write_text(json.dumps(m, indent=1))
    os.replace(tmp, FILE)


def find_drive(m: dict[str, Any], key: str) -> dict[str, Any] | None:
    """A drive by its id (sdb1), its device (/dev/sdb1, /dev/sdb), its label or where it's mounted."""
    k = (key or "").strip().rstrip("/").lower()
    for d in m.get("drives", []):
        names = {d["id"], "/dev/" + d["id"], d["disk"], "/dev/" + d["disk"], d["label"].lower(),
                 str(MEDIA / d["label"]).lower()}
        if k in names or (k and k in d["label"].lower()):
            return d
    return None


def drive_root(d: dict[str, Any]) -> Path:
    return MEDIA / d["label"] if d.get("mounted") else UNPLUGGED / d["label"]


def mount(d: dict[str, Any]) -> Path:
    """Open the drive: its files appear at /media/sam/<label>."""
    if d.get("mounted"):
        return MEDIA / d["label"]
    if d.get("ejected"):
        raise RuntimeError(f"{d['label']} was ejected: unplug it and plug it in again to use it.")
    src, dst = UNPLUGGED / d["label"], MEDIA / d["label"]
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.move(str(src), str(dst))
    d["mounted"] = d["was_mounted"] = True
    return dst


def unmount(d: dict[str, Any]) -> None:
    if not d.get("mounted"):
        return
    src, dst = MEDIA / d["label"], UNPLUGGED / d["label"]
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.rmtree(dst, ignore_errors=True)
    shutil.move(str(src), str(dst))
    d["mounted"] = False


def files_on(d: dict[str, Any]) -> list[str]:
    root = drive_root(d)
    out = []
    for dirpath, _, names in os.walk(root):
        for n in names:
            out.append(os.path.relpath(os.path.join(dirpath, n), root))
    return sorted(out)


def used_gb(d: dict[str, Any]) -> float:
    root = drive_root(d)
    total = sum(os.path.getsize(os.path.join(p, n)) for p, _, ns in os.walk(root) for n in ns)
    return round(total / 1e9, 3)


def snapshot() -> dict[str, Any]:
    """The machine as it is now, for the trace (evaluate.py reads it)."""
    m = load()
    out = {k: m.get(k) for k in ("device", "backlight", "night_light", "keyboard_layout", "keyboard_variant",
                                  "print_jobs", "timezone", "vpns")}
    out["drives"] = [{"label": d["label"], "mounted": d.get("mounted", False), "ejected": d.get("ejected", False),
                      "was_mounted": d.get("was_mounted", False), "files": files_on(d)} for d in m.get("drives", [])]
    return out
