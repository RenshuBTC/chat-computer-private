"""The Linux engine: NetworkManager (Wi-Fi), BlueZ (AirPods), PipeWire (sound), systemd
(power), X11 + xdotool (screen input), and the Chat Computer root helper for admin tasks.

`LinuxPlatform` is Chat Computer on a PC, next to Windows (Ubuntu underneath). `MacPlatform` is Chat Computer on an
Apple silicon Mac, next to macOS (Fedora Asahi Remix underneath): the same, except how it goes back to macOS
and that macOS's files come from the drive macOS setup sends them to.
"""
from __future__ import annotations

import contextlib
import glob
import io
import json
import os
import re
import secrets
import select
import shutil
import socket
import subprocess
import tempfile
import time
from typing import Any, Callable

from .base import CommandResult, Platform, run

ROOT_HELPER = "/usr/local/libexec/chatcomputer/chatcomputer-root"
# On a Mac: the drive both macOS and Chat Computer open (exFAT, "ChatComp" in macOS's Finder), with the folders
# Chat Computer Setup sent over from macOS. Mounted here (linux/system/etc/fstab line written at the first start).
MAC_FILES = "~/Mac"


_ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]|[\x01\x02\r]")
_AGENT = [  # what bluetoothctl's own agent says while pairing, and what it means
    ("confirm", re.compile(r"Confirm passkey (\d+)[^\n]*?\(yes/no\)", re.I)),
    ("ask", re.compile(r"(?:Authorize|Accept|Allow)[^\n]*?\(yes/no\)", re.I)),
    ("enter", re.compile(r"Enter (?:PIN code|passkey)[^\n:]*:", re.I)),
    ("show", re.compile(r"(?:Passkey|PIN code):\s*(\d{4,8})\b", re.I)),
    ("ok", re.compile(r"Pairing successful", re.I)),
    ("failed", re.compile(r"Failed to pair:?\s*([^\n]*)", re.I)),
    ("failed", re.compile(r"Device [0-9A-F:]{17} not available", re.I)),
]


def pair_with_agent(address: str, on_code: Callable[[str], None] | None, timeout: float = 60.0,
                    cmd: tuple[str, ...] = ("bluetoothctl",)) -> tuple[bool, str]:
    """Pair through bluetoothctl's own agent as a keyboard-and-screen, answering what it asks: a code to show
    (the person types it on the keyboard: on_code gets it), a code to confirm, a PIN to make up (shown, then
    typed on the keyboard). Keyboards that want a code typed on them pair this way; NoInputNoOutput can't.
    Returns (paired, why not)."""
    proc = subprocess.Popen(list(cmd), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)

    def send(line: str) -> None:
        try:
            proc.stdin.write((line + "\n").encode())
            proc.stdin.flush()
        except (BrokenPipeError, OSError):
            pass

    def show(code: str) -> None:
        if code not in shown and on_code:
            shown.add(code)
            try:
                on_code(code)
            except Exception:
                pass

    shown: set[str] = set()
    send("agent KeyboardDisplay")
    send("default-agent")
    send(f"pair {address}")
    buf, pos, result = "", 0, (False, "it didn't answer in time")
    deadline = time.monotonic() + timeout
    try:
        while time.monotonic() < deadline:
            ready, _, _ = select.select([proc.stdout], [], [], 0.2)
            if not ready:
                if proc.poll() is not None:
                    result = (False, "bluetoothctl stopped")
                    break
                continue
            chunk = os.read(proc.stdout.fileno(), 4096)
            if not chunk:
                result = (False, "bluetoothctl stopped")
                break
            buf += _ANSI.sub("", chunk.decode("utf-8", "replace"))
            done = False
            while True:
                found = [(m.start(), kind, m) for kind, rx in _AGENT for m in [rx.search(buf, pos)] if m]
                if not found:
                    break
                _start, kind, m = min(found, key=lambda f: (f[0], -len(f[2].group(0))))
                pos = m.end()
                if kind == "confirm":
                    show(m.group(1))
                    send("yes")
                elif kind == "ask":
                    send("yes")
                elif kind == "enter":
                    pin = f"{secrets.randbelow(1000000):06d}"
                    show(pin)
                    send(pin)
                elif kind == "show":
                    show(m.group(1))
                elif kind == "ok":
                    result, done = (True, ""), True
                    break
                else:
                    reason = (m.group(1) if m.groups() else m.group(0)).strip()
                    result, done = (False, reason or "it refused"), True
                    break
            if done:
                break
    finally:
        send("quit")
        try:
            proc.wait(3)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(3)
    return result


def is_apple_silicon(compatible: str = "/proc/device-tree/compatible") -> bool:
    """True on an Apple silicon Mac running Linux (its device tree says "apple,...")."""
    try:
        with open(compatible, "rb") as f:
            return any(c.startswith(b"apple,") for c in f.read().split(b"\0"))
    except OSError:
        return False
MAC_RE = re.compile(r"^[0-9A-Fa-f]{2}(:[0-9A-Fa-f]{2}){5}$")
# A package name, as the root helper takes it: never ending in "-" (apt-get reads "name-" as "remove name").
PKG_RE = re.compile(r"^[a-z0-9][a-z0-9+.\-]{0,61}[a-z0-9+]$")

KEY_ALIASES = {
    "enter": "Return", "return": "Return", "esc": "Escape", "escape": "Escape",
    "backspace": "BackSpace", "tab": "Tab", "space": "space", "delete": "Delete", "del": "Delete",
    "pageup": "Page_Up", "page_up": "Page_Up", "pagedown": "Page_Down", "page_down": "Page_Down",
    "home": "Home", "end": "End", "left": "Left", "right": "Right", "up": "Up", "down": "Down",
    "cmd": "super", "command": "super", "win": "super", "windows": "super", "meta": "super",
    "control": "ctrl", "option": "alt", "plus": "plus", "minus": "minus",
    "volumeup": "XF86AudioRaiseVolume", "volumedown": "XF86AudioLowerVolume", "mute": "XF86AudioMute",
    "playpause": "XF86AudioPlay",
}


def xdotool_combo(combo: str) -> str:
    parts = [p.strip() for p in combo.replace(" ", "").split("+") if p.strip()]
    out = []
    for p in parts:
        low = p.lower()
        if low in KEY_ALIASES:
            out.append(KEY_ALIASES[low])
        elif re.fullmatch(r"f([1-9]|1[0-9]|2[0-4])", low):
            out.append(low.upper())
        else:
            out.append(p)
    return "+".join(out)


def split_nmcli(line: str) -> list[str]:
    """Split one line of `nmcli -t` output (':' separated, '\\:' and '\\\\' escaped)."""
    fields, cur, i = [], [], 0
    while i < len(line):
        ch = line[i]
        if ch == "\\" and i + 1 < len(line):
            cur.append(line[i + 1])
            i += 2
            continue
        if ch == ":":
            fields.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
        i += 1
    fields.append("".join(cur))
    return fields


WPCTL_LINE = re.compile(r"^[\s│├└─]*(\*)?\s*(\d+)\.\s+(.+?)\s+\[vol:\s*([\d.]+)(\s+MUTED)?\]")


def parse_wpctl_status(text: str) -> dict[str, Any]:
    """Pull the sinks (outputs) and sources (inputs) out of `wpctl status`."""
    section = None
    in_audio = False
    res: dict[str, Any] = {"outputs": [], "inputs": []}
    for raw in text.splitlines():
        line = raw.rstrip()
        stripped = line.strip(" │├└─")
        if line.startswith("Audio"):
            in_audio = True
            continue
        if line and not line[0].isspace() and not line.startswith(("│", "├", "└")) and not line.startswith("Audio"):
            in_audio = False if line.startswith(("Video", "Settings")) else in_audio
            continue
        if not in_audio:
            continue
        if stripped.endswith(":"):
            section = stripped[:-1].strip().lower()
            continue
        m = WPCTL_LINE.match(line)
        if m and section in ("sinks", "sources"):
            item = {"id": int(m.group(2)), "name": m.group(3).strip(), "volume": round(float(m.group(4)) * 100),
                    "muted": bool(m.group(5)), "default": bool(m.group(1))}
            res["outputs" if section == "sinks" else "inputs"].append(item)
    return res


LSBLK_COLS = "NAME,LABEL,SIZE,FSTYPE,MOUNTPOINTS,RM,HOTPLUG,TRAN,TYPE,MODEL,VENDOR,RO,FSAVAIL"
SYSTEM_MOUNTS = re.compile(r"^/(boot(/.*)?|home|usr|var|opt|srv)?$|^\[SWAP\]$")
NEEDS_UDISKS = ("Opening drives needs the package udisks2, which isn't on this computer yet: install it with the "
                "software tool (they're asked first), then try again.")
NEEDS_CUPS = ("Printing needs the packages cups, cups-filters, avahi-daemon and ipp-usb, which aren't on this "
              "computer yet: install them with the software tool (they're asked first), then try again.")
CANT_PRINT_KIND = ("{ext} files can't be printed directly: PDFs, photos and text files can. Open it with show_file "
                   "and tell them it can be printed from the program it came from, or saved as a PDF there.")
PRINTABLE = {"pdf": "pdf", "jpg": "image", "jpeg": "image", "png": "image", "gif": "image", "bmp": "image",
             "tif": "image", "tiff": "image", "webp": "image", "txt": "text", "md": "text", "csv": "text",
             "log": "text", "ps": "pdf"}
BACKLIGHT = "/sys/class/backlight"
MIN_BRIGHTNESS = 5        # never all the way to 0: on many laptops that turns the screen off, and then nobody can see


def printable_kind(path: str) -> str | None:
    """What CUPS can print as it is: "pdf", "image" or "text"; None for anything else (Word, Excel...)."""
    return PRINTABLE.get(os.path.splitext(path)[1].lower().lstrip("."))


def parse_drives(lsblk_json: str) -> list[dict[str, Any]]:
    """The drives that can be unplugged (USB sticks and disks, memory cards), one per partition with a filesystem,
    from `lsblk -J -b -o LSBLK_COLS`. The computer's own disks (Windows and Chat Computer) are never among them."""
    try:
        devices = json.loads(lsblk_json or "{}").get("blockdevices") or []
    except ValueError:
        return []
    out: list[dict[str, Any]] = []
    for disk in devices:
        if disk.get("type") != "disk":
            continue
        tran = (disk.get("tran") or "").lower()
        name = disk.get("name") or ""
        removable = _true(disk.get("rm")) or _true(disk.get("hotplug")) or tran in ("usb", "mmc") \
            or name.startswith("mmcblk")
        if not removable:
            continue
        parts = [c for c in disk.get("children") or [] if c.get("fstype")] or ([disk] if disk.get("fstype") else [])
        for p in parts:
            if p.get("fstype") in ("swap", "crypto_LUKS", "BitLocker", "LVM2_member", "linux_raid_member"):
                continue
            mounts = p.get("mountpoints") if "mountpoints" in p else [p.get("mountpoint")]
            mount = next((m for m in mounts or [] if m), None)
            if mount and SYSTEM_MOUNTS.match(mount):
                continue        # part of the running system after all (installed onto a USB disk)
            kind = "memory card" if tran == "mmc" or name.startswith("mmcblk") else "USB drive" if tran == "usb" else "drive"
            label = (p.get("label") or " ".join(x for x in ((disk.get("vendor") or "").strip(),
                                                              (disk.get("model") or "").strip()) if x) or kind).strip()
            free = p.get("fsavail")
            out.append({"id": p.get("name"), "name": label, "kind": kind, "disk": name,
                        "size_gb": round(int(p.get("size") or 0) / 1e9, 1), "mounted": bool(mount), "path": mount,
                        "free_gb": round(int(free) / 1e9, 1) if mount and free not in (None, "") else None,
                        "read_only": _true(p.get("ro"))})
    return out


def _true(v: Any) -> bool:
    return v is True or str(v).strip() in ("1", "true")


def parse_mounted_at(text: str) -> str | None:
    """Where udisksctl mounted a drive ("Mounted /dev/sdb1 at /media/sam/SANDISK", with or without a full stop),
    or where it already was ("...AlreadyMounted: Device /dev/sdb1 is already mounted at `/media/sam/X'.")."""
    m = re.search(r"^Mounted \S+ at (.+?)\.?\s*$", text, re.M)
    if m:
        return m.group(1)
    m = re.search(r"already mounted at [`'](.+?)'", text)
    return m.group(1) if m else None


def _udisks_error(r: CommandResult) -> str:
    err = (r.err or r.out).strip()
    if "NotAuthorized" in err or "not authorized" in err.lower():
        return "the system wants an administrator's permission for it."
    if "wrong fs type" in err or "unknown filesystem" in err.lower():
        return "this computer can't read how it's formatted."
    m = re.search(r"GDBus\.Error:[\w.]+:\s*(.+)", err)
    return (m.group(1) if m else err)[:200] or f"exit {r.code}"


def parse_printers(names: str, default: str, states: str) -> list[dict[str, Any]]:
    """Printers from `lpstat -e` (names), `lpstat -d` (the default) and `lpstat -p` (which are idle or printing)."""
    dm = re.search(r"system default destination:\s*(\S+)", default)
    dflt = dm.group(1) if dm else None
    state: dict[str, str] = {}
    for line in states.splitlines():
        m = re.match(r"printer (\S+) (?:is )?(idle|now printing|disabled)", line)
        if m:
            state[m.group(1)] = {"now printing": "printing", "disabled": "stopped"}.get(m.group(2), m.group(2))
    out = []
    ids = [n.strip() for n in names.splitlines() if n.strip() and " " not in n.strip()]
    for n in ids:
        out.append({"name": printer_name(n), "id": n, "state": state.get(n, "ready"),
                    "default": n == dflt or (dflt is None and len(ids) == 1)})
    return out


def printer_name(queue: str) -> str:
    """A queue's name as people know the printer: HP_OfficeJet_Pro_9010_E1A2B3 -> HP OfficeJet Pro 9010."""
    name = re.sub(r"_[0-9A-F]{6}$", "", queue)          # the end of its network address, added to tell two apart
    return name.replace("_", " ").strip() or queue


def choose_printer(printers: list[dict[str, Any]], wanted: str | None) -> dict[str, Any]:
    if not printers:
        raise RuntimeError("No printer is connected. A printer on the same Wi-Fi as this computer, or plugged into "
                           "it with a USB cable, is found by itself once it's turned on.")
    if wanted:
        w = wanted.strip().lower()
        for p in printers:
            if w in (p["id"].lower(), p["name"].lower()):
                return p
        for p in printers:
            if all(word in p["name"].lower() for word in w.split()):
                return p
        raise RuntimeError(f"There's no printer called {wanted!r}. Printers: " + ", ".join(p["name"] for p in printers))
    for p in printers:
        if p.get("default"):
            return p
    if len(printers) == 1:
        return printers[0]
    raise RuntimeError("Which printer? " + ", ".join(p["name"] for p in printers))


def parse_print_queue(text: str) -> list[dict[str, Any]]:
    out = []
    for line in text.splitlines():
        m = re.match(r"^(\S+)-(\d+)\s+(\S+)\s+(\d+)\s+(.*)$", line)
        if m:
            out.append({"id": f"{m.group(1)}-{m.group(2)}", "printer": printer_name(m.group(1)),
                        "size_kb": round(int(m.group(4)) / 1024), "since": m.group(5).strip()})
    return out


def read_backlight(root: str = BACKLIGHT) -> dict[str, Any] | None:
    """The screen's backlight, if it has one: {"name", "value", "max", "percent"}. With several, the kind the
    kernel means to be used (firmware, then platform, then raw)."""
    try:
        names = sorted(os.listdir(root))
    except OSError:
        return None
    best = None
    for n in names:
        try:
            with open(os.path.join(root, n, "max_brightness"), encoding="ascii") as f:
                mx = int(f.read().strip())
            with open(os.path.join(root, n, "brightness"), encoding="ascii") as f:
                cur = int(f.read().strip())
        except (OSError, ValueError):
            continue
        try:
            with open(os.path.join(root, n, "type"), encoding="ascii") as f:
                kind = f.read().strip()
        except OSError:
            kind = "raw"
        rank = {"firmware": 0, "platform": 1, "raw": 2}.get(kind, 3)
        if mx > 0 and (best is None or rank < best[0]):
            best = (rank, {"name": n, "value": cur, "max": mx, "percent": round(100 * cur / mx)})
    return best[1] if best else None


def parse_setxkbmap(text: str) -> dict[str, str]:
    out = {"layout": "us", "variant": ""}
    for line in text.splitlines():
        k, _, v = line.partition(":")
        if k.strip() in ("layout", "variant"):
            out[k.strip()] = v.strip()
    return out


def xkb_layouts(path: str = "/usr/share/X11/xkb/rules/evdev.lst") -> dict[str, set[str] | None]:
    """The keyboard layouts X knows (us, fr, de...), each with its variants (fr: azerty, bepo, mac...)."""
    try:
        with open(path, encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return {}
    out: dict[str, set[str] | None] = {}
    section = ""
    for line in text.splitlines():
        if line.startswith("!"):
            section = line[1:].strip()
            continue
        parts = line.split()
        if not parts:
            continue
        if section == "layout":
            out.setdefault(parts[0], set())
        elif section == "variant" and len(parts) > 1 and parts[1].endswith(":"):
            v = out.setdefault(parts[1][:-1], set())
            if v is not None:
                v.add(parts[0])
    return out


# The kind of computer, from the SMBIOS chassis type the firmware gives (/sys/class/dmi/id/chassis_type).
CHASSIS = {**dict.fromkeys((3, 4, 5, 6, 7, 15, 16, 24), "desktop"), **dict.fromkeys((8, 9, 10, 14), "laptop"),
           13: "all-in-one", **dict.fromkeys((30, 31, 32), "tablet"), **dict.fromkeys((35, 36), "mini PC")}


def computer_kind(chassis: str = "/sys/class/dmi/id/chassis_type", model: str = "/proc/device-tree/model") -> str | None:
    """desktop, laptop, all-in-one, tablet or mini PC; None when the computer doesn't say."""
    try:
        with open(chassis, encoding="ascii") as f:
            return CHASSIS.get(int(f.read().strip()))
    except (OSError, ValueError):
        pass
    try:                                     # an Apple silicon Mac: "Apple MacBook Air (M1, 2020)"
        with open(model, "rb") as f:
            name = f.read().rstrip(b"\0").decode("utf-8", "replace")
    except OSError:
        return None
    if "MacBook" in name:
        return "laptop"
    if "iMac" in name:
        return "all-in-one"
    return "desktop" if re.search(r"Mac (mini|Studio|Pro)", name) else None


def power_mode(root: str = "/sys") -> str | None:
    """The computer's power mode: the firmware's (low-power, balanced, performance), else the processor's own
    setting (its frequency governor, and its energy preference: balance_power, performance...)."""
    def read(rel: str) -> str:
        with open(os.path.join(root, rel), encoding="ascii") as f:
            return f.read().strip()
    with contextlib.suppress(OSError):
        return read("firmware/acpi/platform_profile")
    try:
        gov = read("devices/system/cpu/cpu0/cpufreq/scaling_governor")
    except OSError:
        return None
    with contextlib.suppress(OSError):
        return f"{gov} ({read('devices/system/cpu/cpu0/cpufreq/energy_performance_preference')})"
    return gov


# Programs as people know them: the processes Chat Computer and the system run under their own names.
FRIENDLY = {"WebKitWebProcess": "web pages open in Chat Computer", "WebKitNetworkProcess": "Chat Computer's web connections",
            "WebKitGPUProcess": "Chat Computer's drawing", "chat-computer-model": "Chat Computer's model",
            "Xorg": "the display (Xorg)", "pipewire": "sound (PipeWire)", "pipewire-pulse": "sound (PipeWire)",
            "wireplumber": "sound (WirePlumber)", "bluetoothd": "Bluetooth", "NetworkManager": "the network",
            "unattended-upgr": "automatic updates", "apt": "updates", "apt-get": "updates", "dpkg": "updates",
            "packagekitd": "updates", "dnf": "updates", "tracker-miner-f": "file indexing"}


def friendly_name(name: str, cmdline: list[str], own_pid: bool = False) -> str:
    if own_pid or (name.startswith("python") and any("chatcomputer" in c for c in cmdline[:3])):
        return "Chat Computer"
    return FRIENDLY.get(name, name)


def parse_apt_upgradable(text: str) -> list[dict[str, str]]:
    """`apt list --upgradable`: "firefox/noble-updates 131.0+b1 amd64 [upgradable from: 130.0+b2]"."""
    out = []
    for line in text.splitlines():
        m = re.match(r"^([^/\s]+)/\S+\s+(\S+)\s+\S+\s+\[upgradable from: ([^\]]+)\]", line.strip())
        if m:
            out.append({"name": m.group(1), "version": m.group(2), "from": m.group(3)})
    return out


def parse_dnf_check_update(text: str) -> list[dict[str, str]]:
    """`dnf check-update`: "firefox.aarch64   131.0-1.fc42   updates" (a section of obsoleted packages after)."""
    out = []
    for line in text.splitlines():
        if line.strip().lower().startswith("obsoleting"):
            break
        m = re.match(r"^([A-Za-z0-9_.+\-]+)\.(\w+)\s+(\S+)\s+\S+\s*$", line.strip())
        if m and m.group(2) in ("noarch", "aarch64", "x86_64", "i686", "src"):
            out.append({"name": m.group(1), "version": m.group(3), "from": ""})
    return out


def parse_bt_battery(info: str) -> int | None:
    """`bluetoothctl info`: "Battery Percentage: 0x5a (90)" -> 90; None when the device doesn't say."""
    m = re.search(r"Battery Percentage:\s*(?:0x[0-9a-f]+\s*)?\(?(\d{1,3})\)?", info, re.I)
    return int(m.group(1)) if m else None


def parse_nmcli_vpns(text: str) -> list[dict[str, Any]]:
    """`nmcli -t -f NAME,TYPE,ACTIVE connection show`: the VPNs among the connections."""
    out = []
    for line in text.splitlines():
        f = split_nmcli(line)
        if len(f) >= 3 and f[1] in ("vpn", "wireguard"):
            out.append({"name": f[0], "kind": "WireGuard" if f[1] == "wireguard" else "VPN", "on": f[2] == "yes"})
    return out


SPEED_HOST = "speed.cloudflare.com"       # Cloudflare's speed test: __down?bytes=N sends N bytes, __up takes them
OFFICE = {".doc", ".docx", ".odt", ".rtf", ".xls", ".xlsx", ".ods", ".ppt", ".pptx", ".odp", ".pages"}
NEEDS_OFFICE = ("Making a PDF of a {ext} file as it looks needs LibreOffice, which isn't on this computer yet: install it "
                "with the software tool ({package}, free, a few hundred MB), then try again.")


class LinuxPlatform(Platform):
    name = "linux"
    office_package = "libreoffice-nogui"       # LibreOffice without its windows: enough to make PDFs

    # ------------------------------------------------------------------ helpers
    def _root(self, verb: str, *args: str, timeout: float = 600) -> dict[str, Any]:
        r = run(["sudo", "-n", ROOT_HELPER, verb, *args], timeout=timeout)
        try:
            data = json.loads(r.out.strip().splitlines()[-1]) if r.out.strip() else {}
        except (ValueError, IndexError):
            data = {}
        data.setdefault("ok", r.ok)
        if not r.ok and "error" not in data:
            data["error"] = (r.err or r.out).strip()[-400:] or f"exit {r.code}"
        return data

    @staticmethod
    def _say(r: CommandResult, ok: str) -> str:
        if r.ok:
            return ok
        return "Failed: " + ((r.err or r.out).strip().splitlines() or [f"exit {r.code}"])[-1][:300]

    def just_started(self, within_s: float = 120.0) -> bool:
        try:
            with open("/proc/uptime", encoding="ascii") as f:
                return float(f.read().split()[0]) < within_s
        except (OSError, ValueError, IndexError):
            return False

    # ------------------------------------------------------------------ network
    def internet_ok(self) -> bool:
        # (Hugging Face: where the model's files come from, and what Chat Computer reaches most.)
        try:
            with socket.create_connection(("huggingface.co", 443), timeout=3):
                return True
        except OSError:
            return False

    def _wifi_device(self) -> str | None:
        r = run(["nmcli", "-t", "-f", "DEVICE,TYPE,STATE", "device", "status"], timeout=8)
        for line in r.out.splitlines():
            f = split_nmcli(line)
            if len(f) >= 2 and f[1] == "wifi":
                return f[0]
        return None

    def wifi_status(self) -> dict[str, Any]:
        st: dict[str, Any] = {"wifi_device": None, "connected_to": None, "signal": None, "ethernet": False,
                              "connectivity": None, "radio": None}
        r = run(["nmcli", "-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"], timeout=8)
        for line in r.out.splitlines():
            f = split_nmcli(line)
            if len(f) < 4:
                continue
            dev, typ, state, conn = f[:4]
            if typ == "wifi":
                st["wifi_device"] = dev
                if state == "connected":
                    st["connected_to"] = conn
            elif typ == "ethernet" and state == "connected":
                st["ethernet"] = True
        r = run(["nmcli", "-t", "-f", "IN-USE,SSID,SIGNAL", "device", "wifi", "list", "--rescan", "no"], timeout=8)
        for line in r.out.splitlines():
            f = split_nmcli(line)
            if len(f) >= 3 and f[0] == "*":
                st["connected_to"] = f[1] or st["connected_to"]
                st["signal"] = int(f[2]) if f[2].isdigit() else None
        st["connectivity"] = run(["nmcli", "networking", "connectivity", "check"], timeout=8).out.strip() or None
        st["radio"] = run(["nmcli", "radio", "wifi"], timeout=5).out.strip() or None
        return st

    def wifi_scan(self) -> list[dict[str, Any]]:
        r = run(["nmcli", "-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", "yes"], timeout=25)
        nets: dict[str, dict[str, Any]] = {}
        for line in r.out.splitlines():
            f = split_nmcli(line)
            if len(f) < 4 or not f[1]:
                continue
            sig = int(f[2]) if f[2].isdigit() else 0
            if f[1] not in nets or sig > nets[f[1]]["signal"]:
                nets[f[1]] = {"name": f[1], "signal": sig, "secured": bool(f[3] and f[3] != "--"), "connected": f[0] == "*"}
        return sorted(nets.values(), key=lambda n: -n["signal"])

    def wifi_connect(self, ssid: str, password: str | None) -> str:
        cmd = ["nmcli", "device", "wifi", "connect", ssid]
        if password:
            cmd += ["password", password]
        r = run(cmd, timeout=45)
        return self._say(r, f"Connected to {ssid}.")

    def wifi_disconnect(self) -> str:
        dev = self._wifi_device()
        if not dev:
            return "There's no Wi-Fi adapter."
        return self._say(run(["nmcli", "device", "disconnect", dev], timeout=15), "Wi-Fi disconnected.")

    def wifi_forget(self, ssid: str) -> str:
        return self._say(run(["nmcli", "connection", "delete", "id", ssid], timeout=15), f"Forgot {ssid}.")

    def wifi_radio(self, on: bool) -> str:
        return self._say(run(["nmcli", "radio", "wifi", "on" if on else "off"], timeout=10),
                         "Wi-Fi turned " + ("on." if on else "off."))

    # ------------------------------------------------------------------ bluetooth
    def has_bluetooth(self, sysfs: str = "/sys/class/bluetooth") -> bool:
        """Whether the computer has a Bluetooth adapter. Without one bluetoothd may not run at all (its service
        starts only when the kernel's Bluetooth folder is there), and then every bluetoothctl command waits for
        it until it's stopped: 8 s each here."""
        try:
            with os.scandir(sysfs) as it:
                return any(True for _ in it)
        except OSError:
            return False

    def _need_bluetooth(self) -> None:
        if not self.has_bluetooth():
            raise RuntimeError("This computer has no Bluetooth.")

    def _bt_devices(self, which: str | None) -> list[dict[str, str]]:
        args = ["bluetoothctl", "devices"] + ([which] if which else [])
        out = []
        for line in run(args, timeout=8).out.splitlines():
            m = re.match(r"^Device ([0-9A-F:]{17}) (.*)$", line.strip(), re.I)
            if m:
                out.append({"address": m.group(1).upper(), "name": m.group(2)})
        return out

    def bt_status(self) -> dict[str, Any]:
        if not self.has_bluetooth():
            return {"adapter": False, "powered": False, "connected": [], "paired": []}
        show = run(["bluetoothctl", "show"], timeout=8).out
        connected = self._bt_devices("Connected")
        for d in connected:               # the battery, when the device tells it (BlueZ's Battery1: AirPods, keyboards)
            d["battery"] = parse_bt_battery(run(["bluetoothctl", "info", d["address"]], timeout=8).out)
        return {
            "adapter": True,
            "powered": "Powered: yes" in show,
            "connected": connected,
            "paired": self._bt_devices("Paired"),
        }

    def bt_scan(self, seconds: int = 10) -> list[dict[str, Any]]:
        self._need_bluetooth()
        run(["bluetoothctl", "power", "on"], timeout=8)
        run(["bluetoothctl", "--timeout", str(max(3, min(seconds, 30))), "scan", "on"], timeout=seconds + 10)
        paired = {d["address"] for d in self._bt_devices("Paired")}
        return [{**d, "paired": d["address"] in paired} for d in self._bt_devices(None)]

    def _check_mac(self, address: str) -> str:
        if not MAC_RE.match(address or ""):
            raise ValueError("That isn't a Bluetooth address like AA:BB:CC:DD:EE:FF.")
        return address.upper()

    def bt_is_keyboard(self, address: str) -> bool:
        """Whether a (found) Bluetooth device says it's a keyboard."""
        info = run(["bluetoothctl", "info", self._check_mac(address)], timeout=8).out
        return bool(re.search(r"^\s*Icon:\s*input-keyboard", info, re.M))

    def bt_pair(self, address: str, on_code: Callable[[str], None] | None = None) -> str:
        """Pair, trust and connect. Headphones and mice pair without a code. A keyboard may want one typed on
        it: on_code is given the code to show the person (a keyboard pairs with bluetoothctl's own agent)."""
        a = self._check_mac(address)
        self._need_bluetooth()
        run(["bluetoothctl", "power", "on"], timeout=8)
        if self.bt_is_keyboard(a):
            ok, why = pair_with_agent(a, on_code)
            if not ok and "AlreadyExists" not in why:
                return "Failed: " + (why or "it didn't pair")[:300]
        else:
            r = run(["bluetoothctl", "--agent", "NoInputNoOutput", "--timeout", "25", "pair", a], timeout=40)
            if not r.ok and "AlreadyExists" not in (r.out + r.err):
                return self._say(r, "")
        run(["bluetoothctl", "trust", a], timeout=10)
        return self._say(run(["bluetoothctl", "--timeout", "15", "connect", a], timeout=25), "Paired and connected.")

    def bt_connect(self, address: str) -> str:
        a = self._check_mac(address)
        self._need_bluetooth()
        run(["bluetoothctl", "power", "on"], timeout=8)
        return self._say(run(["bluetoothctl", "--timeout", "15", "connect", a], timeout=25), "Connected.")

    def bt_disconnect(self, address: str) -> str:
        a = self._check_mac(address)
        self._need_bluetooth()
        return self._say(run(["bluetoothctl", "disconnect", a], timeout=15), "Disconnected.")

    def bt_forget(self, address: str) -> str:
        a = self._check_mac(address)
        self._need_bluetooth()
        return self._say(run(["bluetoothctl", "remove", a], timeout=15), "Removed.")

    def bt_import_windows_keys(self) -> str:
        d = self._root("bt-import-windows-keys", timeout=60)
        if not d.get("ok"):
            return "Couldn't copy the pairings from Windows: " + str(d.get("error", "unknown error"))
        # What was copied is in the importer's own report, under "result" (root-helper/bt-import-windows-keys.py).
        res = d.get("result") if isinstance(d.get("result"), dict) else {}
        imported = res.get("imported") or []
        n = res.get("changed", len(imported))
        if n:
            names = [i.get("name") for i in imported if i.get("name") and i.get("action") in ("created", "updated")]
            which = f" ({', '.join(names[:4])})" if names else ""
            return f"Copied {n} Bluetooth pairing{'' if n == 1 else 's'} from Windows{which}."
        if imported:
            return "Windows' Bluetooth pairings are already here."
        if res.get("errors"):
            return "Couldn't copy the pairings from Windows: " + str(res["errors"][0])
        if res.get("skipped_no_adapter_dir"):
            return ("Windows' Bluetooth pairings are for a different Bluetooth adapter than the one Chat Computer uses, so "
                    "none could be copied. Pair the device again here.")
        return "Windows has no Bluetooth pairings to copy. Pair the device here instead."

    # ------------------------------------------------------------------ sound
    def audio_status(self) -> dict[str, Any]:
        st = parse_wpctl_status(run(["wpctl", "status"], timeout=8).out)
        st["output_is_headset"] = self.output_is_headset()
        return st

    def set_volume(self, percent: int | None = None, delta: int | None = None) -> str:
        if percent is not None:
            val = f"{max(0, min(int(percent), 100)) / 100:.2f}"
        elif delta is not None:
            val = f"{abs(int(delta))}%{'+' if delta >= 0 else '-'}"
        else:
            return "No volume given."
        r = run(["wpctl", "set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", val], timeout=8)
        if r.ok:
            run(["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "0"], timeout=8)
        got = run(["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"], timeout=8).out
        m = re.search(r"([\d.]+)", got)
        return self._say(r, f"Volume is {round(float(m.group(1)) * 100)}%." if m else "Volume changed.")

    def set_mute(self, mute: bool | None) -> str:
        val = "toggle" if mute is None else ("1" if mute else "0")
        return self._say(run(["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", val], timeout=8),
                         "Muted." if mute else ("Unmuted." if mute is False else "Toggled mute."))

    def set_output(self, name_or_id: str) -> str:
        st = parse_wpctl_status(run(["wpctl", "status"], timeout=8).out)
        if not (name_or_id or "").strip():      # "" is part of every name: it would pick the first output
            return "Which output? Available: " + ", ".join(o["name"] for o in st["outputs"])
        target = None
        for o in st["outputs"]:
            if str(o["id"]) == str(name_or_id) or name_or_id.lower() in o["name"].lower():
                target = o
                break
        if not target:
            return "I couldn't find that sound output. Available: " + ", ".join(o["name"] for o in st["outputs"])
        return self._say(run(["wpctl", "set-default", str(target["id"])], timeout=8), f"Sound now plays on {target['name']}.")

    def set_input(self, name_or_id: str) -> str:
        """The microphone Chat Computer (and everything else) listens with: the sound system's default input."""
        st = parse_wpctl_status(run(["wpctl", "status"], timeout=8).out)
        target = next((i for i in st["inputs"] if str(i["id"]) == str(name_or_id)), None)
        if target is None and (name_or_id or "").strip():
            target = next((i for i in st["inputs"] if name_or_id.lower() in i["name"].lower()), None)
        if not target:
            return "I couldn't find that microphone. Available: " + (", ".join(i["name"] for i in st["inputs"]) or "none")
        return self._say(run(["wpctl", "set-default", str(target["id"])], timeout=8), f"Listening with {target['name']} now.")

    def _inspect_default(self, which: str) -> str:
        return run(["wpctl", "inspect", which], timeout=8).out

    def output_is_headset(self) -> bool:
        info = self._inspect_default("@DEFAULT_AUDIO_SINK@")
        return 'device.api = "bluez5"' in info or "headphone" in info.lower() or "headset" in info.lower()

    def input_is_bluetooth(self) -> bool:
        return 'device.api = "bluez5"' in self._inspect_default("@DEFAULT_AUDIO_SOURCE@")

    # ------------------------------------------------------------------ power
    def power(self, action: str) -> str:
        if action == "shutdown":
            return self._say(run(["systemctl", "poweroff"], timeout=20), "Shutting down.")
        if action == "restart":
            return self._say(run(["systemctl", "reboot"], timeout=20), "Restarting.")
        if action == "sleep":
            return self._say(run(["systemctl", "suspend"], timeout=20), "Going to sleep.")
        if action == "switch_to_windows":
            d = self._root("boot-windows-next", timeout=30)
            if not d.get("ok"):
                return "I couldn't set Windows for the next start: " + str(d.get("error", "unknown error"))
            return self._say(run(["systemctl", "reboot"], timeout=20), "Restarting into Windows.")
        return f"Unknown power action {action!r}."

    def prepare_power(self, action: str) -> str | None:
        if action == "switch_to_windows":
            d = self._root("boot-windows-next", timeout=30)
            if not d.get("ok"):
                return "I couldn't set Windows for the next start: " + str(d.get("error", "unknown error"))
        return None

    def set_default_os(self, system: str) -> str:
        which = "chatcomputer" if system == "chatcomputer" else "windows"
        d = self._root("boot-default", which, timeout=30)
        if not d.get("ok"):
            return "I couldn't change what starts first: " + str(d.get("error", "unknown error"))
        if which == "windows":
            return ("Done: Windows now starts when the PC turns on. Chat Computer is still installed. To use it, pick "
                    "Chat Computer in the PC's boot menu, or run Chat Computer Setup in Windows and choose to start Chat Computer.")
        return "Done: Chat Computer starts when the PC turns on."

    # ------------------------------------------------------------------ software
    def install(self, packages: list[str]) -> str:
        bad = [p for p in packages if not PKG_RE.match(p)]
        if bad or not packages:
            return "Those don't look like package names: " + ", ".join(bad or ["(none)"])
        d = self._root("apt-install", *packages, timeout=1800)
        return "Installed " + ", ".join(packages) + "." if d.get("ok") else "Install failed: " + str(d.get("error"))

    def remove(self, packages: list[str]) -> str:
        bad = [p for p in packages if not PKG_RE.match(p)]
        if bad or not packages:
            return "Those don't look like package names: " + ", ".join(bad or ["(none)"])
        d = self._root("apt-remove", *packages, timeout=900)
        return "Removed " + ", ".join(packages) + "." if d.get("ok") else "Remove failed: " + str(d.get("error"))

    def installed(self, packages: list[str]) -> list[str]:
        names = [p for p in packages if PKG_RE.match(p)]
        if not names:
            return []
        # (dpkg-query fails when one of them is unknown to it, but lists the others all the same.)
        r = run(["dpkg-query", "-W", "-f", "${Package} ${db:Status-Status}\n", *names], timeout=20)
        return [ln.split()[0] for ln in r.out.splitlines() if ln.endswith(" installed") and ln.split()[0] in names]

    def update_system(self) -> str:
        d = self._root("system-update", timeout=3600)
        return "The system is up to date." if d.get("ok") else "Update failed: " + str(d.get("error"))

    def set_timezone(self, zone: str) -> str:
        d = self._root("set-timezone", zone, timeout=30)
        return f"Time zone set to {zone}." if d.get("ok") else "Couldn't set the time zone: " + str(d.get("error"))

    # ------------------------------------------------------------------ Chat Computer itself
    def update_app(self) -> dict[str, Any]:
        return self._root("update-app", timeout=600)

    def update_done(self) -> None:
        self._root("update-done", timeout=30)

    # ------------------------------------------------------------------ drives (udisks2)
    # Opening (mounting) and ejecting go through udisks2, which lets the person at the computer (the tty1 session,
    # active on its seat) do both for drives that can be unplugged, without an administrator: no root helper.
    def drives(self) -> list[dict[str, Any]]:
        r = run(["lsblk", "-J", "-b", "-o", LSBLK_COLS], timeout=10)
        if not r.ok and "MOUNTPOINTS" in r.err:      # util-linux before 2.37
            r = run(["lsblk", "-J", "-b", "-o", LSBLK_COLS.replace("MOUNTPOINTS", "MOUNTPOINT")], timeout=10)
        if not r.ok:
            raise RuntimeError("The drives couldn't be listed: " + (r.err.strip() or f"exit {r.code}")[:200])
        return parse_drives(r.out)

    def _find_drive(self, drive: str) -> dict[str, Any]:
        ds = self.drives()
        key = (drive or "").strip().lower()
        if not key and len(ds) == 1:
            return ds[0]
        for d in ds:
            if key in (d["id"].lower(), "/dev/" + d["id"].lower(), d["name"].lower(), (d.get("path") or "").lower()):
                return d
        for d in ds:
            if key and key in d["name"].lower():
                return d
        if not ds:
            raise RuntimeError("No USB drive or memory card is plugged in.")
        raise RuntimeError(f"There's no drive called {drive!r}. Plugged in: " + ", ".join(d["name"] for d in ds))

    def open_drive(self, drive: str) -> str:
        d = self._find_drive(drive)
        if d["mounted"]:
            return d["path"]
        r = run(["udisksctl", "mount", "-b", "/dev/" + d["id"], "--no-user-interaction"], timeout=60)
        if r.code == 127:
            raise RuntimeError(NEEDS_UDISKS)
        path = parse_mounted_at(r.out + "\n" + r.err)
        if path:
            return path
        raise RuntimeError(f"{d['name']} couldn't be opened: " + _udisks_error(r))

    def eject_drive(self, drive: str) -> str:
        d = self._find_drive(drive)
        if d["mounted"]:
            r = run(["udisksctl", "unmount", "-b", "/dev/" + d["id"], "--no-user-interaction"], timeout=120)
            if r.code == 127:
                raise RuntimeError(NEEDS_UDISKS)
            if not r.ok and "NotMounted" not in r.err:
                if "DeviceBusy" in r.err or "target is busy" in r.err:
                    raise RuntimeError(f"Something is still using {d['name']} (a file from it is open, or being "
                                       "copied): close it first, then eject it again.")
                raise RuntimeError(f"{d['name']} couldn't be ejected: " + _udisks_error(r))
        # Then the stick itself is turned off (its light goes out), as Windows and macOS do. Its other partitions
        # may still be open, or a card reader may not turn off: closing this one is what makes pulling it out safe.
        if d.get("disk") and not any(o["mounted"] for o in self.drives() if o.get("disk") == d["disk"]):
            run(["udisksctl", "power-off", "-b", "/dev/" + d["disk"], "--no-user-interaction"], timeout=60)
        return d["name"]

    # ------------------------------------------------------------------ printing (CUPS)
    # Printers on the same network (IPP Everywhere, AirPrint: nearly every printer since about 2012) and USB ones
    # (ipp-usb) are found by CUPS and avahi without any setup or drivers; lpstat -e lists them, and lp prints on
    # one by name. Printing needs no administrator.
    def printers(self) -> list[dict[str, Any]]:
        names = run(["lpstat", "-e"], timeout=20)
        if names.code == 127:
            raise RuntimeError(NEEDS_CUPS)
        default = run(["lpstat", "-d"], timeout=10).out
        states = run(["lpstat", "-p"], timeout=10).out
        return parse_printers(names.out, default, states)

    def print_file(self, path: str, printer: str | None = None, copies: int = 1, pages: str = "",
                   two_sided: bool = False, colour: bool = True) -> str:
        if not os.path.isfile(path):
            raise RuntimeError(f"There's no file at {path}.")
        kind = printable_kind(path)
        if kind is None:
            raise RuntimeError(CANT_PRINT_KIND.format(ext=os.path.splitext(path)[1] or "this kind of file"))
        ps = self.printers()
        p = choose_printer(ps, printer)
        cmd = ["lp", "-d", p["id"], "-n", str(max(1, min(int(copies or 1), 99))), "-t", os.path.basename(path)]
        if pages and re.fullmatch(r"[0-9,\- ]+", str(pages)):
            cmd += ["-P", str(pages).replace(" ", "")]
        if two_sided:
            cmd += ["-o", "sides=two-sided-long-edge"]
        if not colour:
            cmd += ["-o", "print-color-mode=monochrome"]
        if kind == "image":
            cmd += ["-o", "fit-to-page"]
        r = run(cmd + ["--", path], timeout=60)
        if not r.ok:
            raise RuntimeError(f"{p['name']} didn't take it: " + ((r.err or r.out).strip()[:200] or f"exit {r.code}"))
        return p["name"]

    def print_queue(self) -> list[dict[str, Any]]:
        r = run(["lpstat", "-o"], timeout=10)
        if r.code == 127:
            raise RuntimeError(NEEDS_CUPS)
        return parse_print_queue(r.out)

    def cancel_printing(self, job: str | None = None) -> str:
        jobs = self.print_queue()
        todo = [j for j in jobs if job in (None, "", "all") or j["id"] == job or j["id"].endswith("-" + str(job))]
        if not todo:
            return "Nothing is waiting to print." if not jobs else f"There's no print job {job}."
        for j in todo:
            run(["cancel", j["id"]], timeout=20)
        return f"Cancelled {len(todo)} print job{'s' if len(todo) != 1 else ''}."

    # ------------------------------------------------------------------ the screen itself
    # A laptop's screen: its backlight (/sys/class/backlight), set through logind, which lets the person at the
    # computer do that without an administrator. A desktop's monitor: DDC/CI (ddcutil), when the monitor has it.
    _night: dict[str, Any] = {"on": False, "warmth": 0}
    _ddc: tuple[float, list[str]] | None = None       # (when looked, the monitors that answer)

    def _ddc_displays(self) -> list[str]:
        if self._ddc and time.time() - self._ddc[0] < 300:
            return self._ddc[1]
        r = run(["ddcutil", "detect", "--brief"], timeout=30)
        found = re.findall(r"^Display (\d+)", r.out, re.M) if r.ok else []
        self._ddc = (time.time(), found)
        return found

    def display_status(self) -> dict[str, Any]:
        out: dict[str, Any] = {"night_light": dict(self._night)}
        b = read_backlight()
        if b:
            out.update(brightness=b["percent"], brightness_control="the screen's backlight")
            return out
        for n in self._ddc_displays():
            r = run(["ddcutil", "getvcp", "10", "--brief", "--display", n], timeout=15)
            m = re.search(r"VCP 10 C (\d+) (\d+)", r.out)
            if m and int(m.group(2)):
                out.update(brightness=round(100 * int(m.group(1)) / int(m.group(2))),
                           brightness_control="the monitor (DDC/CI)")
                return out
        why = ("The monitor doesn't let the computer change its brightness: it's set with the monitor's own buttons"
               if shutil.which("ddcutil") else
               "This screen's brightness is set with the monitor's own buttons")
        out.update(brightness=None, brightness_control=None, why_not=why)
        return out

    def set_brightness(self, percent: int | None = None, delta: int | None = None) -> int:
        b = read_backlight()
        if b:
            want = b["percent"] if percent is None else int(percent)
            want = max(MIN_BRIGHTNESS, min(100, want + int(delta or 0)))
            value = max(1, round(b["max"] * want / 100))
            for session in ("auto", "self"):
                r = run(["busctl", "call", "org.freedesktop.login1", f"/org/freedesktop/login1/session/{session}",
                         "org.freedesktop.login1.Session", "SetBrightness", "ssu", "backlight", b["name"], str(value)],
                        timeout=10)
                if r.ok:
                    return want
            try:
                with open(os.path.join(BACKLIGHT, b["name"], "brightness"), "w", encoding="ascii") as f:
                    f.write(str(value))
                return want
            except OSError:
                raise RuntimeError("The screen's brightness couldn't be changed: " +
                                   (r.err.strip()[:200] or "no permission")) from None
        displays = self._ddc_displays()
        if not displays:
            raise RuntimeError(self.display_status().get("why_not") or "The brightness can't be changed here.")
        now = self.display_status().get("brightness")
        want = int(now if percent is None and now is not None else (percent or 50))
        want = max(0, min(100, want + int(delta or 0)))
        for n in displays:
            run(["ddcutil", "setvcp", "10", str(want), "--display", n], timeout=20)
        return want

    def set_night_light(self, on: bool, warmth: int | None = None) -> str:
        from . import gamma
        w = max(1, min(100, int(warmth or 50))) if on else 0
        gamma.set_temperature(gamma.kelvin(w) if on else gamma.NORMAL_K)
        LinuxPlatform._night = {"on": bool(on), "warmth": w}
        return "on" if on else "off"

    def night_light_lost(self) -> bool:
        """The night light is on but the screen's colours are back to normal (after a sleep, or a new monitor)."""
        if not self._night.get("on"):
            return False
        from . import gamma
        try:
            return any(c == (65535, 65535, 65535) for c in gamma.current())
        except Exception:
            return False

    # ------------------------------------------------------------------ the keyboard (XKB)
    def keyboard_layout(self) -> dict[str, str]:
        return parse_setxkbmap(run(["setxkbmap", "-query"], timeout=10).out)

    def set_keyboard_layout(self, layout: str, variant: str = "") -> str:
        layout, variant = (layout or "").strip().lower(), (variant or "").strip().lower()
        m = re.fullmatch(r"([a-z]{2,8})(?:\(([a-z0-9_]+)\))?", layout)
        if not m:
            raise RuntimeError(f"{layout!r} isn't a keyboard layout name (like us, gb, fr, de).")
        layout, variant = m.group(1), variant or (m.group(2) or "")
        known = xkb_layouts()
        if known and layout not in known:
            raise RuntimeError(f"{layout!r} isn't a keyboard layout this computer knows.")
        if variant and known.get(layout) is not None and variant not in known[layout]:
            raise RuntimeError(f"{variant!r} isn't a kind of the {layout} keyboard this computer knows.")
        r = run(["setxkbmap", "-layout", layout] + (["-variant", variant] if variant else []), timeout=10)
        if not r.ok:
            raise RuntimeError("The keyboard layout couldn't be changed: " + (r.err.strip()[:200] or f"exit {r.code}"))
        return layout + (f"({variant})" if variant else "")

    def input_devices(self) -> list[str]:
        r = run(["xinput", "list", "--name-only"], timeout=5)
        return sorted(line.strip() for line in r.out.splitlines() if line.strip()) if r.ok else []

    # ------------------------------------------------------------------ machine info
    def system_info(self) -> dict[str, Any]:
        import psutil
        info: dict[str, Any] = {"time": time.strftime("%A %d %B %Y, %H:%M"),
                                "timezone": run(["timedatectl", "show", "-p", "Timezone", "--value"], timeout=5).out.strip() or None,
                                "uptime_hours": round((time.time() - psutil.boot_time()) / 3600, 1),
                                "cpu_percent": psutil.cpu_percent(interval=0.2),
                                "memory_used_percent": psutil.virtual_memory().percent}
        disks = {}
        for label, path in (("home", os.path.expanduser("~")), ("windows", "/mnt/windows")):
            try:
                if os.path.ismount(path) or label == "home":
                    du = shutil.disk_usage(path)
                    disks[label] = {"free_gb": round(du.free / 1e9, 1), "total_gb": round(du.total / 1e9, 1)}
            except OSError:
                pass
        info["disks"] = disks
        try:
            temps = psutil.sensors_temperatures()
            for key in ("k10temp", "coretemp", "zenpower"):
                if key in temps and temps[key]:
                    info["cpu_temp_c"] = round(temps[key][0].current)
                    break
        except Exception:
            pass
        info["battery"] = None          # none (it runs on mains power), unless one answers
        try:
            b = psutil.sensors_battery()
            if b:
                info["battery"] = {"percent": round(b.percent), "charging": b.power_plugged}
        except Exception:
            pass
        info["kind"] = computer_kind()
        return info

    def windows_status(self) -> dict[str, Any]:
        d = self._root("status", timeout=20)
        d.pop("ok", None)
        return d

    def busiest(self, n: int = 5) -> dict[str, Any]:
        import psutil
        me, procs = os.getpid(), []
        for p in psutil.process_iter(["name", "cmdline"]):
            try:
                p.cpu_percent(None)                  # each one's count starts now
                procs.append(p)
            except (psutil.Error, OSError):
                pass
        io0 = psutil.disk_io_counters()
        psutil.cpu_percent(None)
        t0 = time.monotonic()
        time.sleep(0.5)
        dt = max(0.05, time.monotonic() - t0)
        total, io1, cores = psutil.cpu_percent(None), psutil.disk_io_counters(), psutil.cpu_count() or 1
        progs: dict[str, dict[str, Any]] = {}
        for p in procs:
            try:
                cpu, mem = p.cpu_percent(None) / cores, p.memory_info().rss / 1e6
                name = friendly_name(p.info.get("name") or "?", p.info.get("cmdline") or [], p.pid == me)
            except (psutil.Error, OSError):
                continue
            g = progs.setdefault(name, {"name": name, "cpu_percent": 0.0, "memory_mb": 0.0})
            g["cpu_percent"] += cpu
            g["memory_mb"] += mem
        top = sorted(progs.values(), key=lambda g: (-g["cpu_percent"], -g["memory_mb"]))[:n]
        top += [g for g in sorted(progs.values(), key=lambda g: -g["memory_mb"])[:n] if g not in top]
        vm = psutil.virtual_memory()
        out: dict[str, Any] = {
            "cpu_percent": total, "cores": cores,
            "memory": {"used_gb": round((vm.total - vm.available) / 1e9, 1), "total_gb": round(vm.total / 1e9, 1),
                       "percent": vm.percent},
            "programs": [{"name": g["name"], "cpu_percent": round(g["cpu_percent"], 1), "memory_mb": round(g["memory_mb"])}
                         for g in top]}
        try:
            f = psutil.cpu_freq()
            if f and f.current:
                out["cpu_ghz"] = round(f.current / 1000, 2)
                if f.max:
                    out["cpu_max_ghz"] = round(f.max / 1000, 2)
            sw = psutil.swap_memory()
            if sw.total:
                out["swap_used_gb"] = round(sw.used / 1e9, 1)
        except Exception:
            pass
        if io0 is not None and io1 is not None:
            out["disk_mb_s"] = {"read": round((io1.read_bytes - io0.read_bytes) / dt / 1e6, 1),
                                "write": round((io1.write_bytes - io0.write_bytes) / dt / 1e6, 1)}
        out["measured_over_s"] = round(dt, 1)
        out["power_mode"] = power_mode()
        with contextlib.suppress(OSError):
            # How busy it's been over the last 1, 5 and 15 minutes: programs waiting for a processor, where
            # "cores" means all of them busy.
            out["load_1_5_15_min"] = [round(x, 2) for x in os.getloadavg()]
        return out

    def camera_status(self) -> dict[str, Any]:
        cams: list[str] = []
        for dev in sorted(glob.glob("/sys/class/video4linux/video*")):
            try:
                with open(os.path.join(dev, "name"), encoding="utf-8", errors="replace") as f:
                    name = f.read().strip()
            except OSError:
                continue
            # A camera is two or more video devices (its pictures, and data about them): one name for them all.
            name = name.split(":")[0].replace("_", " ").strip()
            if name and name not in cams:
                cams.append(name)
        users: list[str] = []
        me = os.getpid()
        for pid in filter(str.isdigit, os.listdir("/proc")):
            try:
                fds = os.listdir(f"/proc/{pid}/fd")
            except OSError:
                continue                         # another user's program: not this one's to look into
            for fd in fds:
                try:
                    if not os.readlink(f"/proc/{pid}/fd/{fd}").startswith("/dev/video"):
                        continue
                    if int(pid) == me:
                        name = "Chat Computer"
                    else:
                        with open(f"/proc/{pid}/comm", encoding="utf-8", errors="replace") as f:
                            name = friendly_name(f.read().strip(), [])
                except OSError:
                    continue
                if name not in users:
                    users.append(name)
                break
        return {"cameras": cams, "in_use_by": users}

    # ------------------------------------------------------------------ updates
    def check_updates(self) -> dict[str, Any]:
        r = run(["apt", "list", "--upgradable"], timeout=90)
        if r.code == 127:
            raise RuntimeError("Updates can't be checked here: apt isn't on this computer.")
        ups = parse_apt_upgradable(r.out)
        return {"count": len(ups), "updates": ups[:40],
                "checked": _when_changed("/var/lib/apt/periodic/update-success-stamp", "/var/lib/apt/lists"),
                "automatic": ("Security updates install by themselves every day (unattended-upgrades)."
                              if os.path.exists("/usr/bin/unattended-upgrade") else None)}

    # ------------------------------------------------------------------ the internet's speed
    def speed_test(self) -> dict[str, Any]:
        import http.client
        import ssl

        def connect() -> http.client.HTTPSConnection:
            return http.client.HTTPSConnection(SPEED_HOST, timeout=20, context=ssl.create_default_context())

        def get(conn: http.client.HTTPSConnection, nbytes: int) -> http.client.HTTPResponse:
            conn.request("GET", f"/__down?bytes={nbytes}", headers={"User-Agent": "ChatComputer"})
            res = conn.getresponse()
            if res.status != 200:
                raise http.client.HTTPException(f"the speed test answered {res.status}")
            return res

        import psutil
        n0 = psutil.net_io_counters()
        t0 = time.monotonic()
        router = self._router_ms()              # takes about a second: what else uses the network is counted meanwhile
        time.sleep(max(0.0, 1.0 - (time.monotonic() - t0)))
        n1, span = psutil.net_io_counters(), max(0.5, time.monotonic() - t0)
        busy = {"down": round((n1.bytes_recv - n0.bytes_recv) * 8 / span / 1e6, 2),
                "up": round((n1.bytes_sent - n0.bytes_sent) * 8 / span / 1e6, 2)}
        try:
            conn = connect()
            get(conn, 0).read()                  # connected: what's timed next is only the trip there and back
            trips = []
            for _ in range(4):
                t0 = time.monotonic()
                get(conn, 0).read()
                trips.append((time.monotonic() - t0) * 1000)
            t0, got = time.monotonic(), 0
            res = get(conn, 25_000_000)          # up to 25 MB, for at most 8 seconds
            while time.monotonic() - t0 < 8:
                chunk = res.read(256 * 1024)
                if not chunk:
                    break
                got += len(chunk)
            down = got * 8 / max(1e-3, time.monotonic() - t0) / 1e6
            conn.close()
            conn = connect()
            body = b"0" * 4_000_000
            t0 = time.monotonic()
            conn.request("POST", "/__up", body=body, headers={"Content-Type": "application/octet-stream",
                                                                "User-Agent": "ChatComputer"})
            conn.getresponse().read()
            up = len(body) * 8 / max(1e-3, time.monotonic() - t0) / 1e6
            conn.close()
        except (OSError, http.client.HTTPException) as exc:
            raise RuntimeError(f"The speed couldn't be measured: the speed test can't be reached ({exc}).") from None
        return {"download_mbps": round(down, 1), "upload_mbps": round(up, 1), "latency_ms": round(min(trips)),
                "router_ms": router, "in_use_before_mbps": busy, "server": "Cloudflare's speed test (speed.cloudflare.com)"}

    def _router_ms(self) -> float | None:
        """The delay to the home router (the default gateway), from three pings; None when it doesn't answer."""
        r = run(["ip", "route", "show", "default"], timeout=5)
        m = re.search(r"\bvia (\S+)", r.out)
        if not m:
            return None
        p = run(["ping", "-c", "3", "-i", "0.2", "-W", "1", "-q", m.group(1)], timeout=8)
        avg = re.search(r"= [\d.]+/([\d.]+)/", p.out)
        return round(float(avg.group(1)), 1) if avg else None

    # ------------------------------------------------------------------ VPNs (NetworkManager's)
    def vpns(self) -> list[dict[str, Any]]:
        r = run(["nmcli", "-t", "-f", "NAME,TYPE,ACTIVE", "connection", "show"], timeout=10)
        return parse_nmcli_vpns(r.out) if r.ok else []

    def _vpn(self, name: str) -> dict[str, Any]:
        vs = self.vpns()
        if not vs:
            raise RuntimeError("No VPN is set up on this computer.")
        want = (name or "").strip().lower()
        if not want and len(vs) == 1:
            return vs[0]
        found = [v for v in vs if want and want in v["name"].lower()]
        exact = [v for v in found if v["name"].lower() == want]
        if len(found) == 1 or exact:
            return (exact or found)[0]
        if not want or found:
            raise RuntimeError("Which VPN? " + ", ".join(v["name"] for v in (found or vs)))
        raise RuntimeError(f"There's no VPN called {name!r}: there's " + ", ".join(v["name"] for v in vs) + ".")

    def vpn_connect(self, name: str) -> str:
        v = self._vpn(name)
        r = run(["nmcli", "connection", "up", "id", v["name"]], timeout=60)
        if not r.ok:
            err = (r.err or r.out).strip()
            if re.search(r"secrets were required|no secrets|password", err, re.I):
                raise RuntimeError(f"{v['name']} needs a password to connect, and none is saved for it. A VPN added from "
                                   "its provider's WireGuard file connects without one.")
            raise RuntimeError(f"{v['name']} didn't connect: " + (err[:200] or f"exit {r.code}"))
        return v["name"]

    def vpn_disconnect(self, name: str) -> str:
        v = self._vpn(name)
        r = run(["nmcli", "connection", "down", "id", v["name"]], timeout=30)
        if not r.ok and v.get("on"):
            raise RuntimeError(f"{v['name']} didn't turn off: " + ((r.err or r.out).strip()[:200] or f"exit {r.code}"))
        return v["name"]

    def vpn_import(self, path: str) -> str:
        kind = {".conf": "wireguard", ".ovpn": "openvpn"}.get(os.path.splitext(path)[1].lower())
        if kind is None:
            raise RuntimeError("That isn't a VPN settings file: a VPN provider gives a .conf file (WireGuard) or an .ovpn "
                               "file (OpenVPN).")
        if not os.path.isfile(path):
            raise RuntimeError(f"There's no file at {path}.")
        src, tmp = path, None
        base = os.path.splitext(os.path.basename(path))[0]
        if kind == "wireguard" and not re.fullmatch(r"[A-Za-z0-9_=+.-]{1,15}", base):
            # NetworkManager names a WireGuard VPN after its file, which must then be a network interface's name.
            tmp = tempfile.mkdtemp(prefix="chatcomputer-vpn-")
            base = (re.sub(r"[^A-Za-z0-9_-]+", "-", base).strip("-")[:15] or "wg0")
            src = os.path.join(tmp, base + ".conf")
            shutil.copyfile(path, src)
        try:
            r = run(["nmcli", "connection", "import", "type", kind, "file", src], timeout=30)
        finally:
            if tmp:
                shutil.rmtree(tmp, ignore_errors=True)
        if not r.ok:
            err = (r.err or r.out).strip()
            if kind == "openvpn" and re.search(r"plugin|unknown|not (found|installed)|failed to load", err, re.I):
                raise RuntimeError("An OpenVPN file needs NetworkManager's OpenVPN part, which isn't on this computer yet: "
                                   f"install it with the software tool ({self.openvpn_package}), then add it again.")
            raise RuntimeError("The VPN couldn't be added: " + (err[:200] or f"exit {r.code}"))
        m = re.search(r"Connection '([^']+)'", r.out)
        return m.group(1) if m else base

    openvpn_package = "network-manager-openvpn"

    # ------------------------------------------------------------------ office documents as PDFs
    def office_to_pdf(self, path: str, out_dir: str) -> str:
        ext = os.path.splitext(path)[1].lower() or "this kind of"
        soffice = shutil.which("soffice") or shutil.which("libreoffice")
        if not soffice:
            raise RuntimeError(NEEDS_OFFICE.format(ext=ext, package=self.office_package))
        if not os.path.isfile(path):
            raise RuntimeError(f"There's no file at {path}.")
        os.makedirs(out_dir, exist_ok=True)
        # A profile of its own, so a first start's setup, or LibreOffice already open, doesn't get in the way.
        profile = tempfile.mkdtemp(prefix="chatcomputer-office-")
        try:
            r = run([soffice, f"-env:UserInstallation=file://{profile}", "--headless", "--convert-to", "pdf",
                     "--outdir", out_dir, path], timeout=240)
        finally:
            shutil.rmtree(profile, ignore_errors=True)
        pdf = os.path.join(out_dir, os.path.splitext(os.path.basename(path))[0] + ".pdf")
        if not r.ok or not os.path.isfile(pdf):
            raise RuntimeError("LibreOffice couldn't make a PDF of it: " + ((r.err or r.out).strip()[:200] or f"exit {r.code}"))
        return pdf

    # ------------------------------------------------------------------ screen input
    # The whole screen: every monitor together (mss's monitor 0), in the same pixels xdotool clicks in. (Monitor 1
    # alone would show only one of two side by side, and clicks there would land on the other.)
    def screen_size(self) -> tuple[int, int]:
        with _grabber() as s:
            m = s.monitors[0]
            return m["width"], m["height"]

    def screenshot_png(self) -> bytes:
        import mss.tools
        with _grabber() as s:
            shot = s.grab(s.monitors[0])
            return mss.tools.to_png(shot.rgb, shot.size)

    def screenshot_jpeg(self, max_w: int, max_h: int) -> tuple[bytes, float]:
        # Straight from the grab: making a PNG of it first, to decode it again, took three quarters of the time
        # (a 4K screen of text and photos: 0.65 s that way, 0.17 s this way).
        from PIL import Image
        with _grabber() as s:
            shot = s.grab(s.monitors[0])
        return _jpeg_that_fits(Image.frombytes("RGB", shot.size, shot.bgra, "raw", "BGRX"), max_w, max_h)

    def _xdo(self, *args: str) -> None:
        r = run(["xdotool", *args], timeout=15)
        if not r.ok:
            raise RuntimeError(r.err.strip() or "xdotool failed")

    def click(self, x: int, y: int, button: str = "left", clicks: int = 1) -> None:
        b = {"left": "1", "middle": "2", "right": "3"}.get(button, "1")
        self._xdo("mousemove", "--sync", str(int(x)), str(int(y)), "click", "--repeat", str(max(1, min(clicks, 3))),
                  "--delay", "80", b)

    def move(self, x: int, y: int) -> None:
        self._xdo("mousemove", "--sync", str(int(x)), str(int(y)))

    def drag(self, x1: int, y1: int, x2: int, y2: int) -> None:
        self._xdo("mousemove", "--sync", str(x1), str(y1), "mousedown", "1", "mousemove", "--sync", str(x2), str(y2),
                  "mouseup", "1")

    def type_text(self, text: str) -> None:
        self._xdo("type", "--clearmodifiers", "--delay", "8", "--", text)

    def key(self, combo: str, repeat: int = 1) -> None:
        self._xdo("key", "--clearmodifiers", "--repeat", str(max(1, min(repeat, 100))), "--", xdotool_combo(combo))

    def scroll(self, x: int | None, y: int | None, direction: str, amount: int) -> None:
        btn = {"up": "4", "down": "5", "left": "6", "right": "7"}[direction]
        args = []
        if x is not None and y is not None:
            args += ["mousemove", "--sync", str(int(x)), str(int(y))]
        self._xdo(*args, "click", "--repeat", str(max(1, min(amount, 30))), "--delay", "30", btn)


class MacPlatform(LinuxPlatform):
    """Chat Computer on an Apple silicon Mac (M1 to M3), started instead of macOS. macOS stays installed: going back
    is a restart (the Mac's startup disk, set with asahi-bless through the root helper)."""
    name = "mac"
    other_os = "macOS"
    setup_place = "Chat Computer Setup in macOS"

    def power(self, action: str) -> str:
        if action == "switch_to_macos":
            d = self._root("boot-macos-next", timeout=30)
            if not d.get("ok"):
                return "I couldn't set macOS for the next start: " + str(d.get("error", "unknown error"))
            return self._say(run(["systemctl", "reboot"], timeout=20), "Restarting into macOS.")
        if action == "switch_to_windows":
            return "There's no Windows on this Mac."
        return super().power(action)

    def prepare_power(self, action: str) -> str | None:
        if action == "switch_to_macos":
            d = self._root("boot-macos-next", timeout=30)
            if not d.get("ok"):
                return "I couldn't set macOS for the next start: " + str(d.get("error", "unknown error"))
            return None
        if action == "switch_to_windows":
            return "There's no Windows on this Mac."
        return None

    def set_default_os(self, system: str) -> str:
        which = "chatcomputer" if system == "chatcomputer" else "macos"
        d = self._root("boot-default", which, timeout=30)
        if not d.get("ok"):
            return "I couldn't change what starts first: " + str(d.get("error", "unknown error"))
        if which == "macos":
            return ("Done: macOS now starts when the Mac turns on. Chat Computer is still installed. To use it, hold the "
                    "power button while the Mac starts and choose Chat Computer, or choose it in macOS's System Settings, "
                    "General, Startup Disk.")
        return "Done: Chat Computer starts when the Mac turns on."

    def bt_import_windows_keys(self) -> str:
        return "There's no Windows on this Mac to copy pairings from."

    def installed(self, packages: list[str]) -> list[str]:
        names = [p for p in packages if PKG_RE.match(p)]
        if not names:
            return []
        # (For one that isn't installed, rpm says so in a line of its own, not its name.)
        r = run(["rpm", "-q", "--qf", "%{NAME}\n", *names], timeout=20)
        return [ln.strip() for ln in r.out.splitlines() if ln.strip() in names]

    office_package = "libreoffice-writer libreoffice-calc libreoffice-impress"
    openvpn_package = "NetworkManager-openvpn"

    def check_updates(self) -> dict[str, Any]:
        # From what dnf fetched last (its own timer does, daily): no download, no administrator needed.
        r = run(["dnf", "-q", "--cacheonly", "check-update"], timeout=120)
        if r.code == 127:
            raise RuntimeError("Updates can't be checked here: dnf isn't on this computer.")
        if r.code not in (0, 100):              # 100: there are updates
            raise RuntimeError("Updates couldn't be checked: " + ((r.err or r.out).strip()[:200] or f"exit {r.code}"))
        ups = parse_dnf_check_update(r.out)
        return {"count": len(ups), "updates": ups[:40], "checked": _when_changed("/var/cache/libdnf5", "/var/cache/dnf"),
                "automatic": None}

    def system_info(self) -> dict[str, Any]:
        info = super().system_info()
        disks = {k: v for k, v in info.get("disks", {}).items() if k != "windows"}
        path = os.path.expanduser(MAC_FILES)
        try:
            if os.path.ismount(path):
                du = shutil.disk_usage(path)
                disks["mac"] = {"free_gb": round(du.free / 1e9, 1), "total_gb": round(du.total / 1e9, 1)}
        except OSError:
            pass
        info["disks"] = disks
        return info

    def windows_status(self) -> dict[str, Any]:
        """For a Mac: the drive with the files from macOS (no Windows here)."""
        path = os.path.expanduser(MAC_FILES)
        return {"macos_files": path if os.path.ismount(path) else None}


def _when_changed(*paths: str) -> str | None:
    """When the first of these that exists last changed, as people say it ("Tuesday 20 October, 06:12")."""
    for path in paths:
        try:
            return time.strftime("%A %d %B, %H:%M", time.localtime(os.path.getmtime(path)))
        except OSError:
            continue
    return None


def _grabber() -> Any:
    """mss's screen grabber: mss.MSS from mss 10.2 (which deprecates mss.mss), mss.mss before."""
    import mss
    return (getattr(mss, "MSS", None) or mss.mss)()


def _jpeg_that_fits(im: Any, max_w: int, max_h: int, quality: int = 80) -> tuple[bytes, float]:
    from PIL import Image
    w, h = im.size
    f = min(1.0, max_w / w, max_h / h)
    if f < 1.0:
        im = im.resize((max(1, int(w * f)), max(1, int(h * f))), Image.LANCZOS)
    buf = io.BytesIO()
    im.save(buf, "JPEG", quality=quality, optimize=True)
    return buf.getvalue(), (1.0 / f)


def png_to_jpeg_scaled(png: bytes, max_w: int = 1366, max_h: int = 1366, quality: int = 80) -> tuple[bytes, float]:
    """Shrink a screenshot to a size a model reads well. Returns (jpeg, scale) where
    original_coordinate = sent_coordinate * scale."""
    from PIL import Image
    return _jpeg_that_fits(Image.open(io.BytesIO(png)).convert("RGB"), max_w, max_h, quality)
