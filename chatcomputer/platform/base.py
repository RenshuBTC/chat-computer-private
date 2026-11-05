"""Interface every engine implements. All methods are synchronous and quick (they shell out
or read /proc); callers run them in a worker thread."""
from __future__ import annotations

import os
import re
import subprocess
from dataclasses import dataclass
from typing import Any, Callable


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
    other_os = "Windows"     # the system Chat Computer sits next to, and that people can go back to
    setup_place = "Chat Computer Setup on Windows"  # where people sign in again, change what starts, or remove Chat Computer

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
    def bt_pair(self, address: str, on_code: Callable[[str], None] | None = None) -> str: raise NotImplementedError
    def bt_connect(self, address: str) -> str: raise NotImplementedError
    def bt_disconnect(self, address: str) -> str: raise NotImplementedError
    def bt_forget(self, address: str) -> str: raise NotImplementedError
    def bt_import_windows_keys(self) -> str: raise NotImplementedError
    # --- sound
    def audio_status(self) -> dict[str, Any]: raise NotImplementedError
    def set_volume(self, percent: int | None = None, delta: int | None = None) -> str: raise NotImplementedError
    def set_mute(self, mute: bool | None) -> str: raise NotImplementedError
    def set_output(self, name_or_id: str) -> str: raise NotImplementedError
    def set_input(self, name_or_id: str) -> str: raise NotImplementedError
    def output_is_headset(self) -> bool: raise NotImplementedError
    def input_is_bluetooth(self) -> bool: raise NotImplementedError
    # --- power
    def power(self, action: str) -> str: raise NotImplementedError
    def prepare_power(self, action: str) -> str | None:
        """What a power action needs before the goodbye (a switch: the other system for the next start). None when
        it's ready, else what went wrong."""
        return None
    def set_default_os(self, system: str) -> str:
        """What starts when the computer turns on: "chatcomputer" or "other" (other_os). Removes nothing."""
        raise NotImplementedError
    # --- software
    def install(self, packages: list[str]) -> str: raise NotImplementedError
    def remove(self, packages: list[str]) -> str: raise NotImplementedError
    def update_system(self) -> str: raise NotImplementedError
    def set_timezone(self, zone: str) -> str: raise NotImplementedError
    def timezone(self) -> str | None:
        """The computer's time zone, as an IANA name like Asia/Singapore (None: it can't be told)."""
        try:
            target = os.readlink("/etc/localtime")
        except OSError:
            return None
        m = re.search(r"zoneinfo/(.+)$", target)
        return m.group(1) if m else None
    def installed(self, packages: list[str]) -> list[str]:
        """Which of these programs (package names) are installed."""
        raise NotImplementedError
    # --- Chat Computer itself (chatcomputer/update.py)
    def update_app(self) -> dict[str, Any]:
        """Puts the other version of Chat Computer, in the folder Chat Computer Setup ran from, in place of this one; Chat Computer
        then starts again. {"ok": ..., "changed": how many files changed, "error": why not}."""
        return {"ok": False, "error": "Chat Computer can't update itself on this computer."}
    def update_done(self) -> None:
        """The updated version started well: it's kept (the session no longer goes back to the one before)."""
    # --- drives: USB sticks, memory cards, external disks. A drive is opened (mounted) to use its files, and ejected
    # before it's pulled out. Failures raise RuntimeError with what went wrong, in plain words.
    def drives(self) -> list[dict[str, Any]]:
        """Drives plugged in: [{"id": "sdb1", "name": "SANDISK", "kind": "USB drive", "size_gb": 32.0,
        "mounted": True, "path": "/media/sam/SANDISK", "free_gb": 12.5}] (path and free_gb when mounted)."""
        return []
    def open_drive(self, drive: str) -> str:
        """Mount a drive (by id or name) so its files can be used; returns the folder they're in."""
        raise RuntimeError("Drives can't be opened on this computer.")
    def eject_drive(self, drive: str) -> str:
        """Close a drive (unmount it and turn it off) so it can be pulled out safely; returns its name."""
        raise RuntimeError("Drives can't be ejected on this computer.")
    # --- printing
    def printers(self) -> list[dict[str, Any]]:
        """Printers this computer can print on: [{"name": "HP OfficeJet Pro 9010", "id": "HP_OfficeJet_Pro_9010",
        "state": "idle", "default": True}]."""
        return []
    def print_file(self, path: str, printer: str | None = None, copies: int = 1, pages: str = "",
                   two_sided: bool = False, colour: bool = True) -> str:
        """Send a file to a printer (the only one, or the default, when none is named); returns the printer's name."""
        raise RuntimeError("This computer can't print.")
    def print_queue(self) -> list[dict[str, Any]]:
        return []
    def cancel_printing(self, job: str | None = None) -> str:
        raise RuntimeError("This computer can't print.")
    # --- the screen itself
    def display_status(self) -> dict[str, Any]:
        """{"brightness": 0-100 or None, "brightness_control": what changes it, or None with "why_not",
        "night_light": {"on": bool, "warmth": 0-100}}."""
        return {"brightness": None, "brightness_control": None, "why_not": "not on this computer",
                "night_light": {"on": False, "warmth": 0}}
    def set_brightness(self, percent: int | None = None, delta: int | None = None) -> int:
        """Set the screen's brightness, or change it by delta; returns the new brightness (0-100)."""
        raise RuntimeError("The screen's brightness can't be changed from this computer.")
    def set_night_light(self, on: bool, warmth: int | None = None) -> str:
        """Warmer colours (less blue light), warmth 1-100, or the normal colours again."""
        raise RuntimeError("The night light isn't available on this computer.")
    # --- the keyboard
    def keyboard_layout(self) -> dict[str, str]:
        return {"layout": "us", "variant": ""}
    def set_keyboard_layout(self, layout: str, variant: str = "") -> str:
        """Use another keyboard layout (an XKB name: us, gb, fr, de...); returns the layout now in use."""
        raise RuntimeError("The keyboard layout can't be changed on this computer.")
    def input_devices(self) -> list[str]:
        """The keyboards, mice and touchscreens connected now (to notice a new keyboard)."""
        return []
    # --- machine info
    def just_started(self) -> bool:
        """Whether the computer started moments ago (the other system's drive may still be mounting)."""
        return False
    def system_info(self) -> dict[str, Any]:
        """The time, uptime, processor and memory use, disks, and the kind of computer ("kind": desktop, laptop...)
        with its battery ({"percent", "charging"}, or None when it has none: it runs on mains power)."""
        raise NotImplementedError
    def windows_status(self) -> dict[str, Any]: raise NotImplementedError
    def busiest(self, n: int = 5) -> dict[str, Any]:
        """What's keeping the computer busy, measured over half a second: {"cpu_percent", "cores", "cpu_ghz",
        "memory": {"used_gb", "total_gb", "percent"}, "disk_mb_s": {"read", "write"}, "programs": [{"name",
        "cpu_percent" (of the whole computer), "memory_mb"}] (the n busiest, a program's processes together)}."""
        return {}
    def camera_status(self) -> dict[str, Any]:
        """{"cameras": ["Integrated Camera"], "in_use_by": ["Zoom"]}: the programs that have a
        camera open now."""
        return {"cameras": [], "in_use_by": []}
    # --- updates, without installing them
    def check_updates(self) -> dict[str, Any]:
        """{"count": 3, "updates": [{"name", "version", "from"}], "checked": when the list was last fetched,
        "automatic": what updates by itself, or None}."""
        raise RuntimeError("Updates can't be checked on this computer.")
    # --- the internet's speed
    def speed_test(self) -> dict[str, Any]:
        """{"download_mbps", "upload_mbps", "latency_ms", "server"}. RuntimeError when the internet can't be reached."""
        raise RuntimeError("The internet's speed can't be measured on this computer.")
    # --- VPNs
    def vpns(self) -> list[dict[str, Any]]:
        """VPNs set up on this computer: [{"name": "Work", "kind": "WireGuard", "on": False}]."""
        return []
    def vpn_connect(self, name: str) -> str:
        """Turn a VPN on (by name); returns its name."""
        raise RuntimeError("VPNs can't be used on this computer.")
    def vpn_disconnect(self, name: str) -> str:
        raise RuntimeError("VPNs can't be used on this computer.")
    def vpn_import(self, path: str) -> str:
        """Add a VPN from the settings file its provider gives (WireGuard .conf, OpenVPN .ovpn); returns its name."""
        raise RuntimeError("VPNs can't be added on this computer.")
    # --- documents as PDFs (Word, Excel, PowerPoint and OpenDocument files: their look needs an office program)
    def office_to_pdf(self, path: str, out_dir: str) -> str:
        """Make a PDF of an office document in out_dir, as it looks; returns the PDF's path."""
        raise RuntimeError("Office documents can't be made into PDFs on this computer.")
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
