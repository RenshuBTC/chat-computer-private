"""The Mac engine: ClaudeOS as an app that fills the screen on macOS.

macOS stays underneath and keeps doing what it does well: Wi-Fi, AirPods (they connect by themselves to
the Mac they're paired with), sound routing and updates. ClaudeOS uses its command-line tools and, through
PyObjC, a few of its frameworks (Core WLAN for Wi-Fi names, IOBluetooth for connecting AirPods, Quartz for
clicking and typing on websites).

Coordinates at this boundary are in screen pixels, like on Linux; Quartz works in points, so clicks and
moves are divided by the screen's backing scale factor.

macOS asks the person once for some of this: the microphone (ClaudeOS's first start), Bluetooth, and
Accessibility (clicking and typing on websites). Without Accessibility the click and typing methods raise,
and the tools fall back to clicking and typing inside the page.
"""
from __future__ import annotations

import json
import os
import plistlib
import re
import shutil
import socket
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Callable

from .base import CommandResult, Platform, run

MAC_RE = re.compile(r"^[0-9A-Fa-f]{2}([:-][0-9A-Fa-f]{2}){5}$")
PKG_RE = re.compile(r"^[a-z0-9][a-z0-9@+._\-]{0,79}$")
AGENT_LABEL = "com.claudeos.start"
APP_PATH = Path.home() / "Applications" / "ClaudeOS.app"

# System Settings panes (macOS 13+ names; older names are tried second).
PANES = {
    "bluetooth": ["x-apple.systempreferences:com.apple.BluetoothSettings",
                  "x-apple.systempreferences:com.apple.preferences.Bluetooth"],
    "sound": ["x-apple.systempreferences:com.apple.Sound-Settings.extension",
              "x-apple.systempreferences:com.apple.preference.sound"],
    "wifi": ["x-apple.systempreferences:com.apple.wifi-settings-extension",
             "x-apple.systempreferences:com.apple.preference.network"],
    "accessibility": ["x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"],
    "location": ["x-apple.systempreferences:com.apple.preference.security?Privacy_LocationServices"],
    "date": ["x-apple.systempreferences:com.apple.Date-Time-Settings.extension",
             "x-apple.systempreferences:com.apple.preference.datetime"],
}

# Virtual key codes (Carbon kVK_*) for the ANSI layout.
KEYCODES = {
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9, "b": 11, "q": 12, "w": 13,
    "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19, "3": 20, "4": 21, "6": 22, "5": 23, "=": 24, "9": 25,
    "7": 26, "-": 27, "8": 28, "0": 29, "]": 30, "o": 31, "u": 32, "[": 33, "i": 34, "p": 35, "l": 37, "j": 38,
    "'": 39, "k": 40, ";": 41, "\\": 42, ",": 43, "/": 44, "n": 45, "m": 46, ".": 47, "`": 50,
    "return": 36, "enter": 36, "tab": 48, "space": 49, "backspace": 51, "escape": 53, "esc": 53,
    "delete": 117, "del": 117, "home": 115, "end": 119, "pageup": 116, "page_up": 116, "pagedown": 121,
    "page_down": 121, "left": 123, "right": 124, "down": 125, "up": 126,
    "f1": 122, "f2": 120, "f3": 99, "f4": 118, "f5": 96, "f6": 97, "f7": 98, "f8": 100, "f9": 101, "f10": 109,
    "f11": 103, "f12": 111,
}
MODIFIERS = {"cmd": 1 << 20, "command": 1 << 20, "super": 1 << 20, "meta": 1 << 20, "win": 1 << 20,
             "shift": 1 << 17, "ctrl": 1 << 18, "control": 1 << 18, "alt": 1 << 19, "option": 1 << 19}


def parse_key_combo(combo: str) -> tuple[int, int]:
    """'cmd+shift+t' -> (keycode, modifier flags). Raises ValueError for an unknown key."""
    parts = [p.strip().lower() for p in combo.replace(" ", "").split("+") if p.strip()]
    if not parts:
        raise ValueError("No key given.")
    flags = 0
    for p in parts[:-1]:
        if p not in MODIFIERS:
            raise ValueError(f"Unknown modifier {p!r}.")
        flags |= MODIFIERS[p]
    key = parts[-1]
    if key not in KEYCODES:
        raise ValueError(f"Unknown key {key!r}.")
    return KEYCODES[key], flags


def parse_bluetooth_profile(data: dict[str, Any]) -> dict[str, Any]:
    """`system_profiler SPBluetoothDataType -json` -> powered, connected and paired devices."""
    st: dict[str, Any] = {"powered": None, "connected": [], "paired": []}
    for item in data.get("SPBluetoothDataType") or []:
        ctl = item.get("controller_properties") or {}
        state = str(ctl.get("controller_state", ""))
        if state:
            st["powered"] = state.endswith("_on")
        for key, connected in (("device_connected", True), ("device_not_connected", False)):
            for entry in item.get(key) or []:
                for name, props in (entry or {}).items():
                    d = {"name": name, "address": str((props or {}).get("device_address", "")).upper().replace("-", ":")}
                    st["paired"].append(d)
                    if connected:
                        st["connected"].append(d)
    return st


def parse_audio_profile(data: dict[str, Any]) -> dict[str, Any]:
    """`system_profiler SPAudioDataType -json` -> outputs and inputs, which is the default, which is Bluetooth."""
    res: dict[str, Any] = {"outputs": [], "inputs": []}
    for item in data.get("SPAudioDataType") or []:
        for dev in item.get("_items") or []:
            name = dev.get("_name", "")
            bt = "bluetooth" in str(dev.get("coreaudio_device_transport", "")).lower()
            if dev.get("coreaudio_device_output"):
                res["outputs"].append({"name": name, "bluetooth": bt,
                                       "default": dev.get("coreaudio_default_audio_output_device") == "spaudio_yes"})
            if dev.get("coreaudio_device_input"):
                res["inputs"].append({"name": name, "bluetooth": bt,
                                      "default": dev.get("coreaudio_default_audio_input_device") == "spaudio_yes"})
    return res


def launch_agent_plist(app: Path) -> bytes:
    """Starts ClaudeOS when the person logs in to the Mac (once; quitting it is respected)."""
    return plistlib.dumps({"Label": AGENT_LABEL, "ProgramArguments": ["/usr/bin/open", "-a", str(app)],
                           "RunAtLoad": True, "ProcessType": "Interactive"})


class MacPlatform(Platform):
    name = "mac"
    other_os = "macOS"
    setup_place = "ClaudeOS Setup on this Mac"

    def __init__(self, runner: Callable[..., CommandResult] = run, home: Path | None = None) -> None:
        self.run = runner
        self.home = home or Path.home()
        self.agent_path = self.home / "Library" / "LaunchAgents" / f"{AGENT_LABEL}.plist"
        self.app_path = self.home / "Applications" / "ClaudeOS.app"
        # Set by the window: step aside to show macOS, and a picture of what the window shows.
        self.on_step_aside: Callable[[], None] | None = None
        self.snapshot_provider: Callable[[], bytes | None] | None = None
        self._cache: dict[str, tuple[float, Any]] = {}
        self._ax_prompted = False
        self._location_manager: Any = None
        self._location_asked = False

    # ------------------------------------------------------------------ helpers
    @staticmethod
    def _say(r: CommandResult, ok: str) -> str:
        if r.ok:
            return ok
        return "Failed: " + ((r.err or r.out).strip().splitlines() or [f"exit {r.code}"])[-1][:300]

    def _profile(self, kind: str, max_age: float = 5.0) -> dict[str, Any]:
        hit = self._cache.get(kind)
        if hit and time.time() - hit[0] < max_age:
            return hit[1]
        r = self.run(["system_profiler", kind, "-json"], timeout=20)
        try:
            data = json.loads(r.out) if r.ok and r.out.strip() else {}
        except ValueError:
            data = {}
        self._cache[kind] = (time.time(), data)
        return data

    def _open_pane(self, which: str) -> None:
        for url in PANES.get(which, []):
            if self.run(["open", url], timeout=10).ok:
                return

    def _osa(self, script: str, timeout: float = 15) -> CommandResult:
        return self.run(["osascript", "-e", script], timeout=timeout)

    # ------------------------------------------------------------------ network
    def internet_ok(self) -> bool:
        try:
            with socket.create_connection(("api.anthropic.com", 443), timeout=3):
                return True
        except OSError:
            return False

    def _wifi_device(self) -> str | None:
        r = self.run(["networksetup", "-listallhardwareports"], timeout=10)
        lines = r.out.splitlines()
        for i, line in enumerate(lines):
            if re.match(r"^Hardware Port: (Wi-Fi|AirPort)$", line.strip()) and i + 1 < len(lines):
                m = re.match(r"^Device: (\S+)$", lines[i + 1].strip())
                if m:
                    return m.group(1)
        return None

    # Since macOS 14, Wi-Fi names count as location: apps see them only once allowed to use Location.
    def _location_status(self) -> int | None:
        """CLAuthorizationStatus: 0 not asked yet, 1 restricted, 2 denied, 3 or 4 allowed; None if unknown."""
        try:
            import CoreLocation  # type: ignore
            if self._location_manager is not None:
                return int(self._location_manager.authorizationStatus())
            return int(CoreLocation.CLLocationManager.authorizationStatus())
        except Exception:
            return None

    def _ask_location(self) -> None:
        """Asks macOS once, on the main thread, to let ClaudeOS use Location (so it can see Wi-Fi names)."""
        if self._location_asked:
            return
        self._location_asked = True
        try:
            import CoreLocation  # type: ignore
            from PyObjCTools import AppHelper  # type: ignore
        except Exception:
            return

        def ask() -> None:
            self._location_manager = CoreLocation.CLLocationManager.alloc().init()   # kept: macOS needs it alive
            self._location_manager.requestWhenInUseAuthorization()
        AppHelper.callAfter(ask)

    def _hidden_names_note(self) -> str:
        status = self._location_status()
        if status in (1, 2):
            self._open_pane("location")
            return ("macOS hides Wi-Fi names from ClaudeOS because it isn't allowed to use Location. I opened "
                    "Location Services in System Settings: turn on ClaudeOS there, then ask again.")
        if status in (None, 0):
            self._ask_location()
            return ("macOS hides Wi-Fi names from apps that can't use Location. It's asking now whether ClaudeOS "
                    "may: choose Allow, then ask again.")
        return "macOS didn't show the networks' names."

    def _corewlan_interface(self) -> Any:
        try:
            import CoreWLAN  # type: ignore
            return CoreWLAN.CWWiFiClient.sharedWiFiClient().interface()
        except Exception:
            return None

    def wifi_status(self) -> dict[str, Any]:
        dev = self._wifi_device()
        st: dict[str, Any] = {"wifi_device": dev, "connected_to": None, "signal": None, "radio": None,
                              "connectivity": "full" if self.internet_ok() else "none"}
        if not dev:
            return st
        power = self.run(["networksetup", "-getairportpower", dev], timeout=8).out
        st["radio"] = "enabled" if power.strip().endswith(": On") else ("disabled" if power.strip() else None)
        iface = self._corewlan_interface()
        if iface is not None:
            try:
                st["connected_to"] = iface.ssid()
                st["signal"] = int(iface.rssiValue()) if iface.ssid() else None
            except Exception:
                pass
        if not st["connected_to"]:
            m = re.search(r"Current Wi-Fi Network: (.+)$", self.run(["networksetup", "-getairportnetwork", dev], timeout=8).out.strip())
            if m:
                st["connected_to"] = m.group(1).strip()
        if not st["connected_to"] and self.run(["ipconfig", "getifaddr", dev], timeout=5).out.strip():
            st["connected_to"] = "(connected; macOS hides the network's name from apps without Location access)"
            st["note"] = self._hidden_names_note()
        return st

    def wifi_scan(self) -> list[dict[str, Any]]:
        iface = self._corewlan_interface()
        if iface is None:
            return []
        try:
            found, _err = iface.scanForNetworksWithName_error_(None, None)
        except Exception:
            return []
        current = None
        try:
            current = iface.ssid()
        except Exception:
            pass
        nets: dict[str, dict[str, Any]] = {}
        hidden = 0
        for n in list(found or []):
            name = n.ssid()
            if not name:
                hidden += 1       # macOS hides names from apps without Location access
                continue
            sig = max(0, min(100, 2 * (int(n.rssiValue()) + 100)))
            if name not in nets or sig > nets[name]["signal"]:
                nets[name] = {"name": name, "signal": sig, "secured": True, "connected": name == current}
        if hidden and not nets:
            return [{"note": self._hidden_names_note()}]
        return sorted(nets.values(), key=lambda x: -x["signal"])

    def wifi_connect(self, ssid: str, password: str | None) -> str:
        dev = self._wifi_device()
        if not dev:
            return "This Mac has no Wi-Fi."
        cmd = ["networksetup", "-setairportnetwork", dev, ssid] + ([password] if password else [])
        r = self.run(cmd, timeout=45)
        # networksetup prints its errors on stdout with exit status 0.
        if r.ok and ("Error" in r.out or "Could not" in r.out or "failed" in r.out.lower()):
            return "Failed: " + r.out.strip()[:300]
        return self._say(r, f"Connected to {ssid}.")

    def wifi_disconnect(self) -> str:
        iface = self._corewlan_interface()
        if iface is None:
            return "I can't reach Wi-Fi from here."
        try:
            iface.disassociate()
            return "Wi-Fi disconnected."
        except Exception as exc:
            return f"Failed: {exc}"

    def wifi_forget(self, ssid: str) -> str:
        dev = self._wifi_device()
        if not dev:
            return "This Mac has no Wi-Fi."
        r = self.run(["networksetup", "-removepreferredwirelessnetwork", dev, ssid], timeout=15)
        if not r.ok or "rror" in r.out:
            self._open_pane("wifi")
            return ("macOS wants a password for that, so I opened Wi-Fi settings: choose the network's "
                    "options button and Forget This Network.")
        return f"Forgot {ssid}."

    def wifi_radio(self, on: bool) -> str:
        dev = self._wifi_device()
        if not dev:
            return "This Mac has no Wi-Fi."
        return self._say(self.run(["networksetup", "-setairportpower", dev, "on" if on else "off"], timeout=10),
                         "Wi-Fi turned " + ("on." if on else "off."))

    # ------------------------------------------------------------------ bluetooth
    def bt_status(self) -> dict[str, Any]:
        return parse_bluetooth_profile(self._profile("SPBluetoothDataType", max_age=3))

    def bt_scan(self, seconds: int = 10) -> list[dict[str, Any]]:
        # Pairing new devices happens in System Settings on a Mac; AirPods signed in to the same Apple
        # Account are already paired.
        self._open_pane("bluetooth")
        return [{**d, "paired": True} for d in self.bt_status()["paired"]]

    def _bt_device(self, address: str) -> Any:
        if not MAC_RE.match(address or ""):
            raise ValueError("That isn't a Bluetooth address like AA:BB:CC:DD:EE:FF.")
        import IOBluetooth  # type: ignore
        dev = IOBluetooth.IOBluetoothDevice.deviceWithAddressString_(address.replace(":", "-"))
        if dev is None:
            raise ValueError("No device with that address.")
        return dev

    def bt_pair(self, address: str) -> str:
        self._open_pane("bluetooth")
        return ("On a Mac, new Bluetooth devices are paired in System Settings, which is open now. "
                "Put the device in pairing mode (AirPods: open the case and hold its button), then choose Connect.")

    def bt_connect(self, address: str) -> str:
        try:
            dev = self._bt_device(address)
            rc = dev.openConnection()
        except Exception as exc:
            return f"Failed: {exc}"
        self._cache.pop("SPBluetoothDataType", None)
        return "Connected." if rc == 0 else f"Failed: the device didn't answer (error {rc}). Is it nearby and out of its case?"

    def bt_disconnect(self, address: str) -> str:
        try:
            rc = self._bt_device(address).closeConnection()
        except Exception as exc:
            return f"Failed: {exc}"
        self._cache.pop("SPBluetoothDataType", None)
        return "Disconnected." if rc == 0 else f"Failed (error {rc})."

    def bt_forget(self, address: str) -> str:
        self._open_pane("bluetooth")
        return "I opened Bluetooth settings: choose the device's info button, then Forget This Device."

    def bt_import_windows_keys(self) -> str:
        return "That's only for PCs that also run Windows. On a Mac, AirPods pair in System Settings."

    # ------------------------------------------------------------------ sound
    def _volume(self) -> tuple[int | None, bool | None]:
        out = self._osa("get volume settings").out
        v = re.search(r"output volume:(\d+)", out)
        m = re.search(r"output muted:(true|false)", out)
        return (int(v.group(1)) if v else None), (m.group(1) == "true" if m else None)

    def audio_status(self) -> dict[str, Any]:
        st = parse_audio_profile(self._profile("SPAudioDataType"))
        st["volume"], st["muted"] = self._volume()
        st["output_is_headset"] = self.output_is_headset()
        return st

    def set_volume(self, percent: int | None = None, delta: int | None = None) -> str:
        if percent is None and delta is None:
            return "No volume given."
        if percent is None:
            cur, _ = self._volume()
            percent = (cur if cur is not None else 50) + int(delta or 0)
        val = max(0, min(int(percent), 100))
        r = self._osa(f"set volume output volume {val} without output muted")
        return self._say(r, f"Volume is {val}%.")

    def set_mute(self, mute: bool | None) -> str:
        if mute is None:
            _, cur = self._volume()
            mute = not cur
        r = self._osa("set volume output muted " + ("true" if mute else "false"))
        return self._say(r, "Muted." if mute else "Unmuted.")

    def set_output(self, name_or_id: str) -> str:
        self._open_pane("sound")
        outs = [o["name"] for o in parse_audio_profile(self._profile("SPAudioDataType"))["outputs"]]
        return ("I opened Sound settings: choose the output there (or click the sound icon in the menu bar). "
                "Outputs: " + (", ".join(outs) or "none found") + ".")

    def output_is_headset(self) -> bool:
        st = parse_audio_profile(self._profile("SPAudioDataType"))
        for o in st["outputs"]:
            if o["default"]:
                return o["bluetooth"] or bool(re.search(r"airpods|headphone|headset|beats", o["name"], re.I))
        return False

    def input_is_bluetooth(self) -> bool:
        st = parse_audio_profile(self._profile("SPAudioDataType"))
        return any(i["default"] and i["bluetooth"] for i in st["inputs"])

    # ------------------------------------------------------------------ power
    def power(self, action: str) -> str:
        if action == "switch_to_macos":
            if self.on_step_aside is None:
                return "I can't step aside right now."
            self.on_step_aside()
            return "Stepped aside: macOS is showing. Say “Claude” to bring me back."
        if action == "sleep":
            return self._say(self.run(["pmset", "sleepnow"], timeout=20), "Going to sleep.")
        if action in ("shutdown", "restart"):
            verb = "shut down" if action == "shutdown" else "restart"
            return self._say(self._osa(f'tell application "System Events" to {verb}', timeout=30),
                             "Shutting down." if action == "shutdown" else "Restarting.")
        return f"Unknown power action {action!r}."

    def set_default_os(self, system: str) -> str:
        """On a Mac, "what starts" is whether ClaudeOS opens when the person logs in."""
        if system == "claudeos":
            try:
                self.agent_path.parent.mkdir(parents=True, exist_ok=True)
                self.agent_path.write_bytes(launch_agent_plist(self.app_path))
            except OSError as exc:
                return f"Failed: {exc}"
            return "Done: ClaudeOS opens when you log in to this Mac."
        if self.agent_path.exists():
            # The job only opens the app at login, so stopping it doesn't close ClaudeOS now.
            self.run(["launchctl", "bootout", f"gui/{os.getuid()}/{AGENT_LABEL}"], timeout=10)
            try:
                self.agent_path.unlink()
            except OSError as exc:
                return f"Failed: {exc}"
        return ("Done: the Mac now starts in macOS as usual. ClaudeOS stays installed: open it from Applications "
                "whenever you like, and to remove it, run ClaudeOS Setup and choose Remove.")

    # ------------------------------------------------------------------ software
    def _brew(self) -> str | None:
        for b in ("/opt/homebrew/bin/brew", "/usr/local/bin/brew"):
            if os.path.exists(b):
                return b
        return shutil.which("brew")

    def install(self, packages: list[str]) -> str:
        brew = self._brew()
        if not brew:
            return ("On a Mac, apps come from the App Store or the maker's website, and I can open either. "
                    "(Homebrew isn't installed, so I can't install command-line programs.)")
        bad = [p for p in packages if not PKG_RE.match(p)]
        if bad or not packages:
            return "Those don't look like package names: " + ", ".join(bad or ["(none)"])
        return self._say(self.run([brew, "install", *packages], timeout=1800), "Installed " + ", ".join(packages) + ".")

    def remove(self, packages: list[str]) -> str:
        brew = self._brew()
        if not brew:
            return "To remove an app on a Mac, drag it from Applications to the Trash; I can open Applications for you."
        bad = [p for p in packages if not PKG_RE.match(p)]
        if bad or not packages:
            return "Those don't look like package names: " + ", ".join(bad or ["(none)"])
        return self._say(self.run([brew, "uninstall", *packages], timeout=900), "Removed " + ", ".join(packages) + ".")

    def update_system(self) -> str:
        self.run(["open", "x-apple.systempreferences:com.apple.Software-Update-Settings.extension"], timeout=10)
        return "I opened Software Update: macOS updates itself from there."

    def set_timezone(self, zone: str) -> str:
        self._open_pane("date")
        return "macOS sets the time zone in Date & Time settings, which I opened. Turn on 'Set time zone automatically'."

    # ------------------------------------------------------------------ machine info
    def system_info(self) -> dict[str, Any]:
        info: dict[str, Any] = {"time": time.strftime("%A %d %B %Y, %H:%M"), "timezone": time.strftime("%Z")}
        ver = self.run(["sw_vers", "-productVersion"], timeout=5).out.strip()
        info["macos"] = ver or None
        info["model"] = self.run(["sysctl", "-n", "hw.model"], timeout=5).out.strip() or None
        info["chip"] = self.run(["sysctl", "-n", "machdep.cpu.brand_string"], timeout=5).out.strip() or None
        try:
            import psutil
            info["uptime_hours"] = round((time.time() - psutil.boot_time()) / 3600, 1)
            info["cpu_percent"] = psutil.cpu_percent(interval=0.2)
            info["memory_used_percent"] = psutil.virtual_memory().percent
            b = psutil.sensors_battery()
            if b:
                info["battery"] = {"percent": round(b.percent), "charging": b.power_plugged}
        except Exception:
            pass
        try:
            du = shutil.disk_usage(str(self.home))
            info["disks"] = {"home": {"free_gb": round(du.free / 1e9, 1), "total_gb": round(du.total / 1e9, 1)}}
        except OSError:
            pass
        return info

    def windows_status(self) -> dict[str, Any]:
        return {"applies": False, "note": "This is a Mac; there's no Windows here."}

    # ------------------------------------------------------------------ screen input
    @staticmethod
    def _scale() -> float:
        try:
            from AppKit import NSScreen  # type: ignore
            return float(NSScreen.mainScreen().backingScaleFactor() or 1.0)
        except Exception:
            return 1.0

    def screen_size(self) -> tuple[int, int]:
        from AppKit import NSScreen  # type: ignore
        f = NSScreen.mainScreen().frame()
        s = self._scale()
        return int(f.size.width * s), int(f.size.height * s)

    def screenshot_png(self) -> bytes:
        if self.snapshot_provider is not None:
            png = self.snapshot_provider()
            if png:
                return png
        fd, path = tempfile.mkstemp(suffix=".png")
        os.close(fd)
        try:
            r = self.run(["screencapture", "-x", "-t", "png", path], timeout=20)
            data = Path(path).read_bytes() if r.ok else b""
        finally:
            try:
                os.unlink(path)
            except OSError:
                pass
        if not data:
            raise RuntimeError("macOS didn't allow a screenshot. Allow ClaudeOS in System Settings → Privacy & "
                               "Security → Screen Recording.")
        return data

    def _need_accessibility(self) -> Any:
        try:
            import Quartz  # type: ignore
            from ApplicationServices import (AXIsProcessTrusted, AXIsProcessTrustedWithOptions,  # type: ignore
                                             kAXTrustedCheckOptionPrompt)
        except Exception as exc:
            raise RuntimeError(f"Screen control isn't available ({exc}).")
        if not AXIsProcessTrusted():
            if not self._ax_prompted:
                # Once per run: macOS shows its own window that leads to the setting, and lists ClaudeOS there.
                self._ax_prompted = True
                try:
                    AXIsProcessTrustedWithOptions({kAXTrustedCheckOptionPrompt: True})
                except Exception:
                    self._open_pane("accessibility")
            raise RuntimeError("macOS hasn't allowed ClaudeOS to click and type yet. Turn on ClaudeOS in System "
                               "Settings → Privacy & Security → Accessibility, then ask again.")
        # Clicks and key presses go to whatever is in front: if that isn't ClaudeOS (it's hidden, or macOS
        # didn't let it come to the front), they would land in someone else's app. The page is used instead.
        if not self._in_front():
            raise RuntimeError("ClaudeOS isn't in front, so clicking and typing go through the page instead.")
        return Quartz

    def _in_front(self) -> bool:
        try:
            from AppKit import NSRunningApplication  # type: ignore
            return bool(NSRunningApplication.currentApplication().isActive())
        except Exception:
            return True

    def _post_mouse(self, kind: str, x: float, y: float, button: str = "left", clicks: int = 1) -> None:
        Q = self._need_accessibility()
        s = self._scale()
        pt = (x / s, y / s)
        btn = {"left": Q.kCGMouseButtonLeft, "right": Q.kCGMouseButtonRight, "middle": Q.kCGMouseButtonCenter}.get(
            button, Q.kCGMouseButtonLeft)
        down, up = {"left": (Q.kCGEventLeftMouseDown, Q.kCGEventLeftMouseUp),
                    "right": (Q.kCGEventRightMouseDown, Q.kCGEventRightMouseUp),
                    "middle": (Q.kCGEventOtherMouseDown, Q.kCGEventOtherMouseUp)}.get(button, (Q.kCGEventLeftMouseDown, Q.kCGEventLeftMouseUp))
        if kind == "move":
            Q.CGEventPost(Q.kCGHIDEventTap, Q.CGEventCreateMouseEvent(None, Q.kCGEventMouseMoved, pt, btn))
            return
        for n in range(1, max(1, min(clicks, 3)) + 1):
            for t in (down, up):
                ev = Q.CGEventCreateMouseEvent(None, t, pt, btn)
                Q.CGEventSetIntegerValueField(ev, Q.kCGMouseEventClickState, n)
                Q.CGEventPost(Q.kCGHIDEventTap, ev)
            time.sleep(0.08)

    def click(self, x: int, y: int, button: str = "left", clicks: int = 1) -> None:
        self._post_mouse("move", x, y)
        time.sleep(0.05)
        self._post_mouse("click", x, y, button, clicks)

    def move(self, x: int, y: int) -> None:
        self._post_mouse("move", x, y)

    def drag(self, x1: int, y1: int, x2: int, y2: int) -> None:
        Q = self._need_accessibility()
        s = self._scale()
        a, b = (x1 / s, y1 / s), (x2 / s, y2 / s)
        for t, pt in ((Q.kCGEventLeftMouseDown, a), (Q.kCGEventLeftMouseDragged, b), (Q.kCGEventLeftMouseUp, b)):
            Q.CGEventPost(Q.kCGHIDEventTap, Q.CGEventCreateMouseEvent(None, t, pt, Q.kCGMouseButtonLeft))
            time.sleep(0.08)

    def type_text(self, text: str) -> None:
        Q = self._need_accessibility()
        for i in range(0, len(text), 16):
            chunk = text[i:i + 16]
            for down in (True, False):
                ev = Q.CGEventCreateKeyboardEvent(None, 0, down)
                Q.CGEventKeyboardSetUnicodeString(ev, len(chunk.encode("utf-16-le")) // 2, chunk)
                Q.CGEventPost(Q.kCGHIDEventTap, ev)
            time.sleep(0.01)

    def key(self, combo: str, repeat: int = 1) -> None:
        Q = self._need_accessibility()
        code, flags = parse_key_combo(combo)
        for _ in range(max(1, min(repeat, 100))):
            for down in (True, False):
                ev = Q.CGEventCreateKeyboardEvent(None, code, down)
                Q.CGEventSetFlags(ev, flags)
                Q.CGEventPost(Q.kCGHIDEventTap, ev)
            time.sleep(0.02)

    def scroll(self, x: int | None, y: int | None, direction: str, amount: int) -> None:
        Q = self._need_accessibility()
        if x is not None and y is not None:
            self._post_mouse("move", x, y)
        lines = max(1, min(amount, 30)) * 3
        dy = {"up": lines, "down": -lines}.get(direction, 0)
        dx = {"left": lines, "right": -lines}.get(direction, 0)
        Q.CGEventPost(Q.kCGHIDEventTap, Q.CGEventCreateScrollWheelEvent(None, Q.kCGScrollEventUnitLine, 2, dy, dx))


def on_mac() -> bool:
    return sys.platform == "darwin"


__all__ = ["MacPlatform", "parse_bluetooth_profile", "parse_audio_profile", "parse_key_combo", "launch_agent_plist", "on_mac"]
