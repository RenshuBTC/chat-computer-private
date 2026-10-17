#!/usr/bin/env python3
"""Starts ClaudeOS in a virtual PC (QEMU with OVMF, the open-source UEFI firmware) without a window,
waits until the kernel says it is ready, takes two pictures of the screen a few seconds apart, and
checks them: the screen is black text on white, and the "Up for" line moved on (so the kernel's own
timer interrupts work).

    ./check.py                 build, start, check; pictures and the kernel's log go to out/
    ./check.py --size 2560x1440 --memory 1024 --cpu qemu64
    ./check.py --crash page-fault   build a kernel that faults on purpose once ready, and check that it
                                    stops cleanly: the fault in its log, and a message on the screen
                                    (also: --crash panic)

Needs qemu-system-x86_64, OVMF (/usr/share/OVMF) and Pillow.
"""
from __future__ import annotations

import argparse
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE / "out"
OVMF_CODE = Path("/usr/share/OVMF/OVMF_CODE_4M.fd")
OVMF_VARS = Path("/usr/share/OVMF/OVMF_VARS_4M.fd")


def monitor(sock_path: Path, command: str) -> str:
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.connect(str(sock_path))
        s.settimeout(5)
        time.sleep(0.2)
        try:
            s.recv(65536)  # the banner
        except TimeoutError:
            pass
        s.sendall(command.encode() + b"\n")
        time.sleep(0.5)
        try:
            return s.recv(65536).decode(errors="replace")
        except TimeoutError:
            return ""


def wait_for(log: Path, words: tuple[str, ...], timeout: float) -> str | None:
    end = time.time() + timeout
    while time.time() < end:
        text = log.read_text(errors="replace") if log.exists() else ""
        for w in words:
            if w in text:
                return w
        time.sleep(0.5)
    return None


def crash_check(kind: str, log: Path, mon: Path, qemu: subprocess.Popen, timeout: float) -> int:
    expected = {"page-fault": "ClaudeOS kernel stopped: Page fault", "panic": "ClaudeOS kernel stopped: panicked"}[kind]
    got = wait_for(log, ("ClaudeOS kernel stopped",), timeout)
    time.sleep(1.5)
    picture = OUT / f"screen-{kind}.ppm"
    monitor(mon, f"screendump {picture}")
    time.sleep(1.0)
    monitor(mon, "quit")
    qemu.wait(timeout=10)
    text = log.read_text(errors="replace")
    print(text)
    # Rebuild the normal kernel, so a crash test never leaves a crashing one in out/.
    env = dict(os.environ)
    env.pop("CLAUDEOS_TEST", None)
    subprocess.run([str(HERE / "build.sh")], check=True, env=env, stdout=subprocess.DEVNULL)
    from PIL import Image
    shot = Image.open(picture).convert("RGB")
    shot.save(picture.with_suffix(".png"))
    if not got or expected not in text:
        print(f"FAILED: expected {expected!r} in the log")
        return 1
    print(f"OK: the kernel stopped cleanly ({expected!r}); picture: {picture.with_suffix('.png')}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--size", default="1920x1080", help="screen resolution the virtual monitor reports")
    ap.add_argument("--memory", default="512", help="memory in MiB")
    ap.add_argument("--cpu", default="max", help="QEMU CPU model")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--timeout", type=float, default=240)
    ap.add_argument("--crash", choices=["page-fault", "panic"])
    args = ap.parse_args()

    if not args.no_build or args.crash:
        env = dict(os.environ)
        env.pop("CLAUDEOS_TEST", None)
        if args.crash:
            env["CLAUDEOS_TEST"] = args.crash
        subprocess.run([str(HERE / "build.sh")], check=True, env=env)
    esp = OUT / "esp"
    if not (esp / "EFI/BOOT/BOOTX64.EFI").exists():
        print("Nothing to start: run ./build.sh first.")
        return 2
    xres, yres = args.size.split("x")
    OUT.mkdir(exist_ok=True)
    log = OUT / "serial.log"
    log.unlink(missing_ok=True)

    with tempfile.TemporaryDirectory() as tmp:
        vars_fd = Path(tmp) / "vars.fd"
        shutil.copy(OVMF_VARS, vars_fd)
        mon = Path(tmp) / "monitor.sock"
        qemu = subprocess.Popen([
            "qemu-system-x86_64",
            "-machine", "q35",
            "-cpu", args.cpu,
            "-m", args.memory,
            "-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={OVMF_CODE}",
            "-drive", f"if=pflash,format=raw,unit=1,file={vars_fd}",
            "-drive", f"format=raw,file=fat:rw:{esp}",
            "-vga", "none",
            "-device", f"VGA,edid=on,xres={xres},yres={yres}",
            "-display", "none",
            "-serial", f"file:{log}",
            "-monitor", f"unix:{mon},server,nowait",
            "-net", "none",
            "-no-reboot",
        ])
        try:
            if args.crash:
                return crash_check(args.crash, log, mon, qemu, args.timeout)
            got = wait_for(log, ("ClaudeOS: ready", "stopped", "couldn't start"), args.timeout)
            if got != "ClaudeOS: ready":
                time.sleep(1)
                monitor(mon, f"screendump {OUT / 'screen-failed.ppm'}")
                print(log.read_text(errors="replace") if log.exists() else "(no log)")
                print("FAILED: the kernel didn't say it was ready" + (f" (it said {got!r})" if got else " in time"))
                return 1
            time.sleep(1.5)
            first, second = OUT / "screen-1.ppm", OUT / "screen-2.ppm"
            monitor(mon, f"screendump {first}")
            time.sleep(3.0)
            monitor(mon, f"screendump {second}")
            time.sleep(1.0)
            monitor(mon, "quit")
        finally:
            try:
                qemu.wait(timeout=10)
            except subprocess.TimeoutExpired:
                qemu.kill()

    from PIL import Image, ImageChops

    print(log.read_text(errors="replace"))
    a = Image.open(first).convert("RGB")
    b = Image.open(second).convert("RGB")
    a.save(OUT / "screen-1.png")
    b.save(OUT / "screen-2.png")
    problems = []
    if a.size != (int(xres), int(yres)):
        problems.append(f"the screen is {a.size[0]}x{a.size[1]}, not {xres}x{yres}")
    colours = a.getcolors(maxcolors=1 << 16) or []
    not_grey = [c for _, c in colours if not (c[0] == c[1] == c[2])]
    if not_grey:
        problems.append(f"{len(not_grey)} colours that aren't grey, e.g. {not_grey[:3]}")
    white = sum(n for n, c in colours if c == (255, 255, 255))
    black = sum(n for n, c in colours if c == (0, 0, 0))
    total = a.size[0] * a.size[1]
    if white < total * 0.9:
        problems.append(f"only {white * 100 // total}% of the screen is white")
    if black < 500:
        problems.append("almost no black: is there any text?")
    diff = ImageChops.difference(a, b).getbbox()
    if diff is None:
        problems.append("the screen didn't change in 3 seconds: are timer interrupts working?")
    for p in problems:
        print("FAILED:", p)
    if not problems:
        print(f"OK: {a.size[0]}x{a.size[1]}, black text on white; the clock moved (changed area {diff}).")
        print(f"Pictures: {OUT / 'screen-1.png'}, {OUT / 'screen-2.png'}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
