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
        self.bt = {"powered": True, "connected": [{"address": "AA:BB:CC:DD:EE:01", "name": "AirPods Pro"}],
                   "paired": [{"address": "AA:BB:CC:DD:EE:01", "name": "AirPods Pro"}]}
        self.headset = True
        self.size = (1920, 1080)
        self.update_result: dict[str, Any] = {"ok": True, "changed": 3}

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
        return "ok"

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
        return f"Volume is {self.volume}%."

    def set_mute(self, mute: bool | None) -> str:
        self._c("set_mute", mute)
        self.muted = (not self.muted) if mute is None else mute
        return "Muted." if self.muted else "Unmuted."

    def set_output(self, name_or_id: str) -> str:
        self._c("set_output", name_or_id)
        return f"Sound now plays on {name_or_id}."

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
        return "Installed " + ", ".join(packages) + "."

    def remove(self, packages: list[str]) -> str:
        self._c("remove", tuple(packages))
        return "Removed " + ", ".join(packages) + "."

    def update_system(self) -> str:
        self._c("update_system")
        return "The system is up to date."

    def set_timezone(self, zone: str) -> str:
        self._c("set_timezone", zone)
        return f"Time zone set to {zone}."

    def update_app(self) -> dict[str, Any]:
        self._c("update_app")
        return dict(self.update_result)

    def update_done(self) -> None:
        self._c("update_done")

    def system_info(self) -> dict[str, Any]:
        return {"time": "Friday 16 October 2026, 09:00", "timezone": "Asia/Singapore", "uptime_hours": 1.0,
                "cpu_percent": 3.0, "memory_used_percent": 21.0, "disks": {"home": {"free_gb": 40.0, "total_gb": 64.0}}}

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
