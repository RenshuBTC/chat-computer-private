"""The computer the desktop evals run on: chatcomputer's FakePlatform, with each case's machine (world.py: a laptop
or a desktop, its battery, what's plugged in, the printers on the network, the backlight, the keyboard layout), the
time where the person is, and the laptop's speakers as a second place for sound. Everything Chat Computer does to the
machine is recorded in .calls (FakePlatform's); what the machine is like afterwards is world.snapshot().

The hardware methods (drives, printing, the screen, the keyboard) are the ones a Chat Computer with those abilities
calls. A Chat Computer from before them doesn't, and meets the same machine through its shell's commands."""
from __future__ import annotations

import os
import time
from pathlib import Path
from typing import Any

from chatcomputer.platform.fake import FakePlatform

from . import world
from .machine import PACKAGE_COMMANDS, hardware_command


class EvalPlatform(FakePlatform):
    def __init__(self, state: dict[str, Any], home: Path, installed: str | None = None) -> None:
        super().__init__()
        self.state = state
        self.home = home
        self.installed = installed          # where an "installed" program's stand-in goes (on the PATH)
        self.outputs = list(state.get("outputs") or ["AirPods Pro"])
        self.output = self.outputs[0]
        self.installed_packages: set[str] | None = None      # what's been installed during the case

    # --- software: only what Ubuntu's library has (this machine's apt lists say), as apt-get on a real one; what's
    # installed is then there to run (a stand-in that says it ran, or answers from the machine)
    def install(self, packages: list[str]) -> str:
        import subprocess
        for p in packages:
            try:
                r = subprocess.run(["apt-cache", "show", "--no-all-versions", p], capture_output=True, text=True, timeout=30)
                known = r.returncode == 0 and "Package:" in r.stdout
            except (OSError, subprocess.TimeoutExpired):
                known = True                     # no lists here to tell: as before
            if not known:
                self._c("install", tuple(packages))
                return f"Install failed: E: Unable to locate package {p}"
        self.installed_packages = (self.installed_packages or set()) | set(packages)
        out = super().install(packages)
        if self.installed:
            for p in packages:
                for cmd in PACKAGE_COMMANDS.get(p, []):
                    exe = Path(self.installed) / cmd
                    exe.write_text(hardware_command(cmd))
                    exe.chmod(0o755)
                exe = Path(self.installed) / p
                if not exe.exists():
                    exe.write_text(f"#!/bin/sh\necho '({p}: started)'\n")
                    exe.chmod(0o755)
        return out

    # --- Wi-Fi: the networks in range (the case's, or home's, a neighbour's and the café's downstairs), and joining
    # one that isn't in range fails as nmcli does (before, any name joined: a network called "CoffeeBean wifi" too)
    def wifi_scan(self) -> list[dict[str, Any]]:
        self._c("wifi_scan")
        names = self.state.get("wifi_networks") or ["HomeWiFi", "Neighbour", "CoffeeBean"]
        on = self.wifi.get("connected_to")
        return [{"name": n, "signal": 78 - 12 * i, "secured": True, "connected": n == on} for i, n in enumerate(names)]

    def wifi_connect(self, ssid: str, password: str | None) -> str:
        names = [n["name"] for n in self.wifi_scan()]
        if ssid not in names:
            self._c("wifi_connect", ssid, bool(password))
            return f"Couldn't connect: no network called '{ssid}' is in range."
        return super().wifi_connect(ssid, password)

    # --- what the machine says about itself
    def system_info(self) -> dict[str, Any]:
        info = super().system_info()
        m = world.load()
        info["time"] = time.strftime("%A %d %B %Y, %H:%M")
        info["timezone"] = m.get("timezone", "Asia/Singapore")
        b = self.state.get("battery")
        info["battery"] = {"percent": b["percent"], "charging": b["charging"]} if b else None
        info["kind"] = m.get("device", "laptop")
        info["cpu_temp_c"] = m.get("cpu", {}).get("temp_c")
        return info

    # --- what's busy: the sandbox's own programs, measured as on a real computer; the camera; updates; speed
    def busiest(self, n: int = 5) -> dict[str, Any]:
        """The sandbox's own programs, measured as on a real computer; what a sandbox can't see of the processor
        (its speed, its power mode) as the case's machine has it."""
        from chatcomputer.platform.linux import LinuxPlatform
        try:
            out = LinuxPlatform.busiest(self, n)        # type: ignore[arg-type]
        except Exception:
            out = super().busiest(n)
        cpu = world.load().get("cpu", {})
        out.setdefault("cpu_ghz", cpu.get("ghz"))
        out.setdefault("cpu_max_ghz", cpu.get("max_ghz"))
        if not out.get("power_mode"):
            out["power_mode"] = cpu.get("power_mode")
        return out

    def camera_status(self) -> dict[str, Any]:
        return {"cameras": list(world.load().get("cameras", [])), "in_use_by": []}

    def check_updates(self) -> dict[str, Any]:
        self._c("check_updates")
        ups = world.load().get("updates", [])
        return {"count": len(ups), "updates": [dict(u) for u in ups], "checked": time.strftime("%A %d %B, 06:12"),
                "automatic": "Security updates install by themselves every day (unattended-upgrades)."}

    def update_system(self) -> str:
        self._c("update_system")
        with world.changing() as m:
            m["updates"] = []
        return "The system is up to date."

    def speed_test(self) -> dict[str, Any]:
        self._c("speed_test")
        return {**world.load().get("speed", {}), "server": "Cloudflare's speed test (speed.cloudflare.com)"}

    # --- VPNs: the case's (none, unless it gives some)
    def vpns(self) -> list[dict[str, Any]]:
        return [dict(v) for v in world.load().get("vpns", [])]

    def _set_vpn(self, name: str, on: bool) -> str:
        with world.changing() as m:
            vs = m.get("vpns", [])
            if not vs:
                raise RuntimeError("No VPN is set up on this computer.")
            v = next((v for v in vs if not name or name.lower() in v["name"].lower()), None)
            if v is None:
                raise RuntimeError(f"There's no VPN called {name!r}: there's " + ", ".join(x["name"] for x in vs) + ".")
            v["on"] = on
            return v["name"]

    def vpn_connect(self, name: str) -> str:
        self._c("vpn_connect", name)
        return self._set_vpn(name, True)

    def vpn_disconnect(self, name: str) -> str:
        self._c("vpn_disconnect", name)
        return self._set_vpn(name, False)

    def vpn_import(self, path: str) -> str:
        self._c("vpn_import", path)
        ext = os.path.splitext(path)[1].lower()
        if ext not in (".conf", ".ovpn"):
            raise RuntimeError("That isn't a VPN settings file: a VPN provider gives a .conf file (WireGuard) or an .ovpn "
                               "file (OpenVPN).")
        if not os.path.isfile(path):
            raise RuntimeError(f"There's no file at {path}.")
        name = os.path.splitext(os.path.basename(path))[0]
        with world.changing() as m:
            m.setdefault("vpns", []).append({"name": name, "kind": "WireGuard" if ext == ".conf" else "VPN", "on": False})
        return name

    # --- office documents: LibreOffice isn't on a Chat Computer until it's installed (software install); then a PDF
    # with the document's words stands in for what it makes.
    def office_to_pdf(self, path: str, out_dir: str) -> str:
        from chatcomputer.platform.linux import NEEDS_OFFICE
        from .home import _pdf
        self._c("office_to_pdf", path, out_dir)
        if "libreoffice-nogui" not in (self.installed_packages or set()):
            raise RuntimeError(NEEDS_OFFICE.format(ext=os.path.splitext(path)[1].lower(), package="libreoffice-nogui"))
        if not os.path.isfile(path):
            raise RuntimeError(f"There's no file at {path}.")
        try:
            import docx
            lines = [p.text for p in docx.Document(path).paragraphs if p.text.strip()]
        except Exception:
            lines = [os.path.basename(path)]
        os.makedirs(out_dir, exist_ok=True)
        pdf = os.path.join(out_dir, os.path.splitext(os.path.basename(path))[0] + ".pdf")
        Path(pdf).write_bytes(_pdf(lines or [os.path.basename(path)]))
        return pdf

    def set_timezone(self, zone: str) -> str:
        """The zone every program in the sandbox reads (TZ=:file, as /etc/localtime on a real computer)."""
        self._c("set_timezone", zone)
        try:
            world.set_zone_file(zone)
        except RuntimeError:
            return f"Couldn't set the time zone: {zone} isn't one."
        with world.changing() as m:
            m["timezone"] = zone
        time.tzset()
        return f"Time zone set to {zone}."

    # --- what a real Chat Computer says when these are done (FakePlatform's "(pretend)" gave the game away)
    def set_default_os(self, system: str) -> str:
        self._c("set_default_os", system)
        self.default_os = system
        if system == "other":
            return ("Done: Windows now starts when the PC turns on. Chat Computer is still installed. To use it, pick "
                    "Chat Computer in the PC's boot menu, or run Chat Computer Setup in Windows and choose to start "
                    "Chat Computer.")
        return "Done: Chat Computer starts when the PC turns on."

    def power(self, action: str) -> str:
        self._c("power", action)
        return "OK"

    # --- the screen: the virtual display, with what's shown drawn on it (screen.py)
    def screen_size(self) -> tuple[int, int]:
        from chatcomputer.platform.linux import LinuxPlatform
        try:
            return LinuxPlatform.screen_size(self)        # type: ignore[arg-type]
        except Exception:
            return super().screen_size()

    def screenshot_png(self) -> bytes:
        from chatcomputer.platform.linux import LinuxPlatform
        try:
            return LinuxPlatform.screenshot_png(self)     # type: ignore[arg-type]
        except Exception:
            return super().screenshot_png()

    # --- sound: AirPods and the laptop's own speakers
    def audio_status(self) -> dict[str, Any]:
        outs = [{"id": 81 + i, "name": n, "volume": self.volume, "muted": self.muted, "default": n == self.output}
                for i, n in enumerate(self.outputs)]
        return {"outputs": outs,
                "inputs": [{"id": 90, "name": "AirPods Pro", "volume": 100, "muted": False, "default": True}],
                "output_is_headset": self.output.startswith("AirPods")}

    def set_output(self, name_or_id: str) -> str:
        self._c("set_output", name_or_id)
        want = str(name_or_id).lower()
        for i, n in enumerate(self.outputs):
            if want in (n.lower(), str(81 + i)) or want in n.lower():
                self.output = n
                return f"Sound now plays on {n}."
        return f"There's no sound output called {name_or_id}."

    def output_is_headset(self) -> bool:
        return self.output.startswith("AirPods")

    # --- drives: a USB stick, if the case has one plugged in
    def drives(self) -> list[dict[str, Any]]:
        m = world.load()
        out = []
        for d in m.get("drives", []):
            if d.get("ejected"):
                continue
            out.append({"id": d["id"], "name": d["label"], "kind": "USB drive", "size_gb": d["size_gb"],
                        "mounted": bool(d.get("mounted")),
                        "path": str(world.MEDIA / d["label"]) if d.get("mounted") else None,
                        "free_gb": round(d["size_gb"] - world.used_gb(d), 1) if d.get("mounted") else None})
        return out

    def open_drive(self, drive: str) -> str:
        self._c("drive_open", drive)
        with world.changing() as m:
            d = world.find_drive(m, drive)
            if d is None or d.get("ejected"):
                raise RuntimeError(f"There's no drive called {drive!r} plugged in.")
            path = world.mount(d)
        return str(path)

    def eject_drive(self, drive: str) -> str:
        self._c("drive_eject", drive)
        with world.changing() as m:
            d = world.find_drive(m, drive)
            if d is None or d.get("ejected"):
                raise RuntimeError(f"There's no drive called {drive!r} plugged in.")
            world.unmount(d)
            d["ejected"] = True
            return d["label"]

    # --- printing: the case's printers, on the network
    def printers(self) -> list[dict[str, Any]]:
        ps = world.load().get("printers", [])
        return [{"name": p["name"], "id": p["queue"], "state": p["state"], "default": len(ps) == 1} for p in ps]

    def print_file(self, path: str, printer: str | None = None, copies: int = 1, pages: str = "",
                   two_sided: bool = False, colour: bool = True) -> str:
        from chatcomputer.platform.linux import CANT_PRINT_KIND, printable_kind
        self._c("print_file", path, printer, copies, pages, two_sided, colour)
        if not os.path.isfile(path):
            raise RuntimeError(f"There's no file at {path}.")
        if printable_kind(path) is None:          # as CUPS: a Word file needs making into a PDF first
            raise RuntimeError(CANT_PRINT_KIND.format(ext=os.path.splitext(path)[1] or "this kind of file"))
        with world.changing() as m:
            ps = m.get("printers", [])
            if not ps:
                raise RuntimeError("No printer is connected.")
            if printer:
                p = next((p for p in ps if printer.lower() in (p["name"].lower(), p["queue"].lower())
                          or printer.lower() in p["name"].lower()), None)
                if p is None:
                    raise RuntimeError(f"There's no printer called {printer!r}.")
            elif len(ps) == 1:
                p = ps[0]
            else:
                raise RuntimeError("Which printer? " + ", ".join(x["name"] for x in ps))
            jobs = m.setdefault("print_jobs", [])
            jobs.append({"id": len(jobs) + 1, "queue": p["queue"], "printer": p["name"],
                         "file": os.path.basename(path), "path": path, "copies": int(copies), "pages": pages,
                         "two_sided": two_sided, "colour": colour, "via": "chatcomputer"})
            return p["name"]

    def print_queue(self) -> list[dict[str, Any]]:
        return [{"id": j["id"], "file": j["file"], "printer": j["printer"], "copies": j["copies"]}
                for j in world.load().get("print_jobs", []) if not j.get("done")]

    def cancel_printing(self, job: str | None = None) -> str:
        self._c("cancel_printing", job)
        with world.changing() as m:
            n = 0
            for j in m.get("print_jobs", []):
                if not j.get("done") and (job in (None, "", "all") or str(j["id"]) == str(job)):
                    j["done"] = True
                    n += 1
        return f"Cancelled {n} print job{'s' if n != 1 else ''}."

    # --- the screen: a laptop's backlight; a desktop's monitor can't be dimmed from the computer
    def display_status(self) -> dict[str, Any]:
        m = world.load()
        nl = m.get("night_light") or {}
        if m.get("backlight") is None:
            return {"brightness": None, "brightness_control": None,
                    "why_not": "The monitor doesn't let the computer change its brightness (it has no DDC/CI)",
                    "night_light": nl}
        return {"brightness": m["backlight"], "brightness_control": "the laptop's backlight", "night_light": nl}

    def set_brightness(self, percent: int | None = None, delta: int | None = None) -> int:
        self._c("set_brightness", percent, delta)
        with world.changing() as m:
            if m.get("backlight") is None:
                raise RuntimeError("This monitor doesn't let the computer change its brightness: use the buttons "
                                   "on the monitor itself.")
            b = m["backlight"] if percent is None else percent
            if delta is not None:
                b += delta
            m["backlight"] = max(5, min(100, int(round(b))))
            return m["backlight"]

    def set_night_light(self, on: bool, warmth: int | None = None) -> str:
        self._c("set_night_light", on, warmth)
        with world.changing() as m:
            m["night_light"] = {"on": bool(on), "warmth": int(warmth or 50) if on else 0}
        return "on" if on else "off"

    # --- the keyboard
    def keyboard_layout(self) -> dict[str, str]:
        m = world.load()
        return {"layout": m.get("keyboard_layout", "us"), "variant": m.get("keyboard_variant", "")}

    def set_keyboard_layout(self, layout: str, variant: str = "") -> str:
        self._c("set_keyboard_layout", layout, variant)
        base = (layout or "").split("(")[0].strip().lower()
        if base not in world.KEYBOARD_LAYOUTS:
            raise RuntimeError(f"{layout!r} isn't a keyboard layout this computer knows.")
        with world.changing() as m:
            m["keyboard_layout"], m["keyboard_variant"] = base, variant or ""
        return base
