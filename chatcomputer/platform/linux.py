"""The Linux engine: NetworkManager (Wi-Fi), BlueZ (AirPods), PipeWire (sound), systemd
(power), X11 + xdotool (screen input), and the Chat Computer root helper for admin tasks.

`LinuxPlatform` is Chat Computer on a PC, next to Windows (Ubuntu underneath). `MacPlatform` is Chat Computer on an
Apple silicon Mac, next to macOS (Fedora Asahi Remix underneath): the same, except how it goes back to macOS
and that macOS's files come from the drive macOS setup sends them to.
"""
from __future__ import annotations

import io
import json
import os
import re
import secrets
import select
import shutil
import socket
import subprocess
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


class LinuxPlatform(Platform):
    name = "linux"

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
        try:
            with socket.create_connection(("api.anthropic.com", 443), timeout=3):
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
        return {
            "adapter": True,
            "powered": "Powered: yes" in show,
            "connected": self._bt_devices("Connected"),
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
        try:
            b = psutil.sensors_battery()
            if b:
                info["battery"] = {"percent": round(b.percent), "charging": b.power_plugged}
        except Exception:
            pass
        return info

    def windows_status(self) -> dict[str, Any]:
        d = self._root("status", timeout=20)
        d.pop("ok", None)
        return d

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
    """Shrink a screenshot to fit Claude's image limits. Returns (jpeg, scale) where
    original_coordinate = sent_coordinate * scale."""
    from PIL import Image
    return _jpeg_that_fits(Image.open(io.BytesIO(png)).convert("RGB"), max_w, max_h, quality)
