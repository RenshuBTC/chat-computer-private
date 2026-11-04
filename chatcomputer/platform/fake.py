"""An in-memory machine for tests and the on-screen preview. Records every call."""
from __future__ import annotations

import io
from typing import Any, Callable

from .base import Platform


class FakePlatform(Platform):
    name = "fake"
    KEYBOARD = "AA:BB:CC:DD:EE:03"

    def __init__(self) -> None:
        self.calls: list[tuple[str, tuple[Any, ...]]] = []
        self.online = True
        self.volume = 40
        self.muted = False
        self.wifi = {"wifi_device": "wlan0", "connected_to": "HomeWiFi", "signal": 78, "ethernet": False,
                     "connectivity": "full", "radio": "enabled"}
        self.bt = {"powered": True, "connected": [{"address": "AA:BB:CC:DD:EE:01", "name": "AirPods Pro", "battery": 85}],
                   "paired": [{"address": "AA:BB:CC:DD:EE:01", "name": "AirPods Pro"}]}
        self.headset = True
        self.size = (1920, 1080)
        self.update_result: dict[str, Any] = {"ok": True, "changed": 3}
        # Hardware: a USB stick (its files in drive_dir, when a test gives one), a printer, the laptop's backlight.
        self.drive_list: list[dict[str, Any]] = []
        self.printer_list: list[dict[str, Any]] = [{"name": "HP OfficeJet Pro 9010", "id": "HP_OfficeJet_Pro_9010",
                                                     "state": "idle", "default": True}]
        self.jobs: list[dict[str, Any]] = []
        self.backlight: int | None = 80
        self.night = {"on": False, "warmth": 0}
        self.layout = {"layout": "us", "variant": ""}
        # A laptop with its camera, three updates waiting, a decent connection, and no VPN set up.
        self.kind = "laptop"
        self.battery: dict[str, Any] | None = {"percent": 76, "charging": False}
        self.cameras = {"cameras": ["Integrated Camera"], "in_use_by": []}
        self.updates = [{"name": "firefox", "version": "131.0", "from": "130.0"},
                        {"name": "libssl3", "version": "3.0.13-0ubuntu3.5", "from": "3.0.13-0ubuntu3.4"},
                        {"name": "tzdata", "version": "2026b", "from": "2026a"}]
        self.speed = {"download_mbps": 48.2, "upload_mbps": 11.6, "latency_ms": 21, "router_ms": 2.1,
                      "in_use_before_mbps": {"down": 0.02, "up": 0.01}, "server": "a stand-in"}
        self.vpn_list: list[dict[str, Any]] = []
        self.zone = "Asia/Singapore"
        self.packages = {"firefox", "libreoffice"}

    def _c(self, name: str, *a: Any) -> None:
        self.calls.append((name, a))

    def internet_ok(self) -> bool:
        return self.online

    def wifi_status(self) -> dict[str, Any]:
        return dict(self.wifi)

    def wifi_scan(self) -> list[dict[str, Any]]:
        self._c("wifi_scan")
        return [{"name": "HomeWiFi", "signal": 78, "secured": True, "connected": True},
                {"name": "Neighbour", "signal": 31, "secured": True, "connected": False}]

    def wifi_connect(self, ssid: str, password: str | None) -> str:
        self._c("wifi_connect", ssid, bool(password))
        self.wifi["connected_to"] = ssid
        return f"Connected to {ssid}."

    def wifi_disconnect(self) -> str:
        self._c("wifi_disconnect")
        self.wifi["connected_to"] = None
        return "Wi-Fi disconnected."

    def wifi_forget(self, ssid: str) -> str:
        self._c("wifi_forget", ssid)
        return f"Forgot {ssid}."

    def wifi_radio(self, on: bool) -> str:
        self._c("wifi_radio", on)
        self.wifi["radio"] = "enabled" if on else "disabled"
        return "Wi-Fi turned " + ("on." if on else "off.")

    def bt_status(self) -> dict[str, Any]:
        return dict(self.bt)

    def bt_scan(self, seconds: int = 10) -> list[dict[str, Any]]:
        self._c("bt_scan", seconds)
        return [{"address": "AA:BB:CC:DD:EE:01", "name": "AirPods Pro", "paired": True},
                {"address": "AA:BB:CC:DD:EE:02", "name": "Speaker", "paired": False},
                {"address": self.KEYBOARD, "name": "Magic Keyboard", "paired": False}]

    def bt_pair(self, address: str, on_code: Callable[[str], None] | None = None) -> str:
        self._c("bt_pair", address)
        if on_code and address == self.KEYBOARD:
            on_code("482916")                 # a keyboard that wants a code typed on it
        return "Paired and connected."

    def bt_connect(self, address: str) -> str:
        self._c("bt_connect", address)
        return "Connected."

    def bt_disconnect(self, address: str) -> str:
        self._c("bt_disconnect", address)
        return "Disconnected."

    def bt_forget(self, address: str) -> str:
        self._c("bt_forget", address)
        return "Removed."

    def bt_import_windows_keys(self) -> str:
        self._c("bt_import_windows_keys")
        return "Copied 1 Bluetooth pairing(s) from Windows."

    def audio_status(self) -> dict[str, Any]:
        return {"outputs": [{"id": 81, "name": "AirPods Pro", "volume": self.volume, "muted": self.muted, "default": True}],
                "inputs": [{"id": 82, "name": "AirPods Pro", "volume": 100, "muted": False, "default": True}],
                "output_is_headset": self.headset}

    def set_volume(self, percent: int | None = None, delta: int | None = None) -> str:
        self._c("set_volume", percent, delta)
        if percent is not None:
            self.volume = max(0, min(100, percent))
        elif delta is not None:
            self.volume = max(0, min(100, self.volume + delta))
        self.muted = False                    # (as the real one: setting the volume unmutes)
        return f"Volume is {self.volume}%."

    def set_mute(self, mute: bool | None) -> str:
        self._c("set_mute", mute)
        self.muted = (not self.muted) if mute is None else mute
        return "Muted." if self.muted else "Unmuted."

    def set_output(self, name_or_id: str) -> str:
        self._c("set_output", name_or_id)
        return f"Sound now plays on {name_or_id}."

    def set_input(self, name_or_id: str) -> str:
        self._c("set_input", name_or_id)
        return f"Listening with {name_or_id} now."

    def output_is_headset(self) -> bool:
        return self.headset

    def input_is_bluetooth(self) -> bool:
        return self.headset

    def power(self, action: str) -> str:
        self._c("power", action)
        return f"(pretend) {action}"

    def set_default_os(self, system: str) -> str:
        self._c("set_default_os", system)
        self.default_os = system
        return f"(pretend) {system} starts first"

    def install(self, packages: list[str]) -> str:
        self._c("install", tuple(packages))
        self.packages |= set(packages)
        return "Installed " + ", ".join(packages) + "."

    def remove(self, packages: list[str]) -> str:
        self._c("remove", tuple(packages))
        self.packages -= set(packages)
        return "Removed " + ", ".join(packages) + "."

    def installed(self, packages: list[str]) -> list[str]:
        return [p for p in packages if p in self.packages]

    def update_system(self) -> str:
        self._c("update_system")
        return "The system is up to date."

    def set_timezone(self, zone: str) -> str:
        self._c("set_timezone", zone)
        self.zone = zone
        return f"Time zone set to {zone}."

    def timezone(self) -> str | None:
        return self.zone

    def update_app(self) -> dict[str, Any]:
        self._c("update_app")
        return dict(self.update_result)

    def update_done(self) -> None:
        self._c("update_done")

    def drives(self) -> list[dict[str, Any]]:
        return [dict(d) for d in self.drive_list]

    def _drive(self, drive: str) -> dict[str, Any]:
        for d in self.drive_list:
            if (drive or "").lower() in (d["id"].lower(), d["name"].lower()):
                return d
        raise RuntimeError(f"There's no drive called {drive!r} plugged in.")

    def open_drive(self, drive: str) -> str:
        self._c("open_drive", drive)
        d = self._drive(drive)
        d["mounted"], d["path"] = True, d.get("folder") or f"/media/sam/{d['name']}"
        return d["path"]

    def eject_drive(self, drive: str) -> str:
        self._c("eject_drive", drive)
        d = self._drive(drive)
        self.drive_list.remove(d)
        return d["name"]

    def printers(self) -> list[dict[str, Any]]:
        return [dict(p) for p in self.printer_list]

    def print_file(self, path: str, printer: str | None = None, copies: int = 1, pages: str = "",
                   two_sided: bool = False, colour: bool = True) -> str:
        self._c("print_file", path, printer, copies)
        if not self.printer_list:
            raise RuntimeError("No printer is connected.")
        p = next((p for p in self.printer_list if printer and printer.lower() in p["name"].lower()), None) \
            if printer else next((p for p in self.printer_list if p.get("default")), self.printer_list[0])
        if p is None:
            raise RuntimeError(f"There's no printer called {printer!r}.")
        self.jobs.append({"id": str(len(self.jobs) + 1), "file": path, "printer": p["name"], "copies": copies})
        return p["name"]

    def print_queue(self) -> list[dict[str, Any]]:
        return [dict(j) for j in self.jobs]

    def cancel_printing(self, job: str | None = None) -> str:
        self._c("cancel_printing", job)
        n = len(self.jobs)
        self.jobs = [j for j in self.jobs if job and job != "all" and j["id"] != str(job)]
        return f"Cancelled {n - len(self.jobs)}."

    def display_status(self) -> dict[str, Any]:
        if self.backlight is None:
            return {"brightness": None, "brightness_control": None,
                    "why_not": "The monitor doesn't let the computer change its brightness", "night_light": dict(self.night)}
        return {"brightness": self.backlight, "brightness_control": "the screen's backlight", "night_light": dict(self.night)}

    def set_brightness(self, percent: int | None = None, delta: int | None = None) -> int:
        self._c("set_brightness", percent, delta)
        if self.backlight is None:
            raise RuntimeError("This monitor doesn't let the computer change its brightness: use the buttons on the "
                               "monitor itself.")
        b = self.backlight if percent is None else percent
        self.backlight = max(5, min(100, b + (delta or 0)))
        return self.backlight

    def set_night_light(self, on: bool, warmth: int | None = None) -> str:
        self._c("set_night_light", on, warmth)
        self.night = {"on": bool(on), "warmth": (warmth or 50) if on else 0}
        return "on" if on else "off"

    def keyboard_layout(self) -> dict[str, str]:
        return dict(self.layout)

    def set_keyboard_layout(self, layout: str, variant: str = "") -> str:
        self._c("set_keyboard_layout", layout, variant)
        self.layout = {"layout": layout, "variant": variant}
        return layout

    def system_info(self) -> dict[str, Any]:
        return {"time": "Friday 16 October 2026, 09:00", "timezone": "Asia/Singapore", "uptime_hours": 1.0,
                "cpu_percent": 3.0, "memory_used_percent": 21.0, "disks": {"home": {"free_gb": 40.0, "total_gb": 64.0}},
                "cpu_temp_c": 47, "kind": self.kind, "battery": dict(self.battery) if self.battery else None}

    def busiest(self, n: int = 5) -> dict[str, Any]:
        return {"cpu_percent": 4.0, "cores": 8, "cpu_ghz": 1.4, "memory": {"used_gb": 3.1, "total_gb": 16.0, "percent": 19.4},
                "disk_mb_s": {"read": 0.0, "write": 0.2}, "measured_over_s": 0.5, "load_1_5_15_min": [0.21, 0.3, 0.28],
                "programs": [{"name": "Chat Computer", "cpu_percent": 1.2, "memory_mb": 420},
                             {"name": "web pages open in Chat Computer", "cpu_percent": 0.8, "memory_mb": 310}][:n]}

    def camera_status(self) -> dict[str, Any]:
        return {"cameras": list(self.cameras["cameras"]), "in_use_by": list(self.cameras["in_use_by"])}

    def check_updates(self) -> dict[str, Any]:
        self._c("check_updates")
        return {"count": len(self.updates), "updates": [dict(u) for u in self.updates], "checked": "Friday 16 October, 06:12",
                "automatic": "Security updates install by themselves every day (unattended-upgrades)."}

    def speed_test(self) -> dict[str, Any]:
        self._c("speed_test")
        if not self.online:
            raise RuntimeError("The speed couldn't be measured: the speed test can't be reached.")
        return dict(self.speed)

    def vpns(self) -> list[dict[str, Any]]:
        return [dict(v) for v in self.vpn_list]

    def _vpn(self, name: str) -> dict[str, Any]:
        if not self.vpn_list:
            raise RuntimeError("No VPN is set up on this computer.")
        for v in self.vpn_list:
            if not name or name.lower() in v["name"].lower():
                return v
        raise RuntimeError(f"There's no VPN called {name!r}.")

    def vpn_connect(self, name: str) -> str:
        self._c("vpn_connect", name)
        v = self._vpn(name)
        v["on"] = True
        return v["name"]

    def vpn_disconnect(self, name: str) -> str:
        self._c("vpn_disconnect", name)
        v = self._vpn(name)
        v["on"] = False
        return v["name"]

    def vpn_import(self, path: str) -> str:
        self._c("vpn_import", path)
        import os
        name = os.path.splitext(os.path.basename(path))[0]
        self.vpn_list.append({"name": name, "kind": "WireGuard" if path.endswith(".conf") else "VPN", "on": False})
        return name

    def office_to_pdf(self, path: str, out_dir: str) -> str:
        import os
        self._c("office_to_pdf", path, out_dir)
        pdf = os.path.join(out_dir, os.path.splitext(os.path.basename(path))[0] + ".pdf")
        os.makedirs(out_dir, exist_ok=True)
        with open(pdf, "wb") as f:
            f.write(b"%PDF-1.4\n% a stand-in\n%%EOF\n")
        return pdf

    def windows_status(self) -> dict[str, Any]:
        return {"windows_mounted": True, "windows_rw": True, "windows_bitlocker": False,
                "windows_boot_entry": True, "efi": True}

    def screen_size(self) -> tuple[int, int]:
        return self.size

    def screenshot_png(self) -> bytes:
        from PIL import Image
        buf = io.BytesIO()
        Image.new("RGB", self.size, (32, 30, 29)).save(buf, "PNG")
        return buf.getvalue()

    def click(self, x: int, y: int, button: str = "left", clicks: int = 1) -> None:
        self._c("click", x, y, button, clicks)

    def move(self, x: int, y: int) -> None:
        self._c("move", x, y)

    def drag(self, x1: int, y1: int, x2: int, y2: int) -> None:
        self._c("drag", x1, y1, x2, y2)

    def type_text(self, text: str) -> None:
        self._c("type_text", text)

    def key(self, combo: str, repeat: int = 1) -> None:
        self._c("key", combo, repeat)

    def scroll(self, x: int | None, y: int | None, direction: str, amount: int) -> None:
        self._c("scroll", x, y, direction, amount)
