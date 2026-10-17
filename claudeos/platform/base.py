"""Interface every engine implements. All methods are synchronous and quick (they shell out
or read /proc); callers run them in a worker thread."""
from __future__ import annotations

import subprocess
from dataclasses import dataclass
from typing import Any


@dataclass
class CommandResult:
    code: int
    out: str
    err: str

    @property
    def ok(self) -> bool:
        return self.code == 0


def run(cmd: list[str], timeout: float = 20, input: str | None = None, env: dict[str, str] | None = None) -> CommandResult:
    """Run a command without a shell. Never raises for a missing program or a timeout."""
    try:
        # errors="replace": output that isn't valid UTF-8 (a Wi-Fi network's name, say) is still read.
        p = subprocess.run(cmd, capture_output=True, text=True, errors="replace", timeout=timeout, input=input, env=env)
        return CommandResult(p.returncode, p.stdout, p.stderr)
    except FileNotFoundError:
        return CommandResult(127, "", f"{cmd[0]} is not installed")
    except subprocess.TimeoutExpired:
        return CommandResult(124, "", f"{cmd[0]} took too long")
    except OSError as exc:
        return CommandResult(126, "", f"{cmd[0]} couldn't run: {exc}")


class Platform:
    name = "base"
    other_os = "Windows"     # the system ClaudeOS sits next to, and that people can go back to
    setup_place = "ClaudeOS Setup on Windows"   # where people sign in again, change what starts, or remove ClaudeOS

    # --- network
    def internet_ok(self) -> bool: raise NotImplementedError
    def wifi_status(self) -> dict[str, Any]: raise NotImplementedError
    def wifi_scan(self) -> list[dict[str, Any]]: raise NotImplementedError
    def wifi_connect(self, ssid: str, password: str | None) -> str: raise NotImplementedError
    def wifi_disconnect(self) -> str: raise NotImplementedError
    def wifi_forget(self, ssid: str) -> str: raise NotImplementedError
    def wifi_radio(self, on: bool) -> str: raise NotImplementedError
    # --- bluetooth
    def bt_status(self) -> dict[str, Any]: raise NotImplementedError
    def bt_scan(self, seconds: int = 10) -> list[dict[str, Any]]: raise NotImplementedError
    def bt_pair(self, address: str) -> str: raise NotImplementedError
    def bt_connect(self, address: str) -> str: raise NotImplementedError
    def bt_disconnect(self, address: str) -> str: raise NotImplementedError
    def bt_forget(self, address: str) -> str: raise NotImplementedError
    def bt_import_windows_keys(self) -> str: raise NotImplementedError
    # --- sound
    def audio_status(self) -> dict[str, Any]: raise NotImplementedError
    def set_volume(self, percent: int | None = None, delta: int | None = None) -> str: raise NotImplementedError
    def set_mute(self, mute: bool | None) -> str: raise NotImplementedError
    def set_output(self, name_or_id: str) -> str: raise NotImplementedError
    def output_is_headset(self) -> bool: raise NotImplementedError
    def input_is_bluetooth(self) -> bool: raise NotImplementedError
    # --- power
    def power(self, action: str) -> str: raise NotImplementedError
    def prepare_power(self, action: str) -> str | None:
        """What a power action needs before the goodbye (a switch: the other system for the next start). None when
        it's ready, else what went wrong."""
        return None
    def set_default_os(self, system: str) -> str:
        """What starts when the computer turns on: "claudeos" or "other" (other_os). Removes nothing."""
        raise NotImplementedError
    # --- software
    def install(self, packages: list[str]) -> str: raise NotImplementedError
    def remove(self, packages: list[str]) -> str: raise NotImplementedError
    def update_system(self) -> str: raise NotImplementedError
    def set_timezone(self, zone: str) -> str: raise NotImplementedError
    # --- machine info
    def just_started(self) -> bool:
        """Whether the computer started moments ago (the other system's drive may still be mounting)."""
        return False
    def system_info(self) -> dict[str, Any]: raise NotImplementedError
    def windows_status(self) -> dict[str, Any]: raise NotImplementedError
    # --- screen input (whole display)
    def screen_size(self) -> tuple[int, int]: raise NotImplementedError
    def screenshot_png(self) -> bytes: raise NotImplementedError
    def screenshot_jpeg(self, max_w: int, max_h: int) -> tuple[bytes, float]:
        """The whole screen as a JPEG no bigger than max_w x max_h, and the scale from its pixels to the
        screen's (screen = image * scale)."""
        from .linux import png_to_jpeg_scaled
        return png_to_jpeg_scaled(self.screenshot_png(), max_w, max_h)
    def click(self, x: int, y: int, button: str = "left", clicks: int = 1) -> None: raise NotImplementedError
    def move(self, x: int, y: int) -> None: raise NotImplementedError
    def drag(self, x1: int, y1: int, x2: int, y2: int) -> None: raise NotImplementedError
    def type_text(self, text: str) -> None: raise NotImplementedError
    def key(self, combo: str, repeat: int = 1) -> None: raise NotImplementedError
    def scroll(self, x: int | None, y: int | None, direction: str, amount: int) -> None: raise NotImplementedError
