#!/usr/bin/env python3
"""Starts ClaudeOS in a virtual PC (QEMU with OVMF, the open-source UEFI firmware) without a window,
waits until the kernel says it is ready, takes two pictures of the screen a few seconds apart, and
checks them: the screen is black text on white, and the "Up for" line moved on (so the kernel's own
timer interrupts work). Pictures and the kernel's log go to out/.

    ./check.py                       build, start, check
    ./check.py --size 2560x1440 --memory 1024 --cpu qemu64
    ./check.py --keyboard            keyboards: PS/2; USB on a port and behind a hub, with MSI-X, MSI and
                                     no interrupts, and with Intel's and AMD's IOMMUs present; typing
                                     (Shift, Backspace, Caps Lock) reaches the screen; a keyboard plugged
                                     in later, then unplugged; a hub unplugged
    ./check.py --crash page-fault    a kernel that faults on purpose once ready: it must stop cleanly,
                                     with the fault in its log and a message on the screen (also: panic)
    ./check.py --secure-boot         Secure Boot, as on a PC that has approved ClaudeOS's key: signs the
                                     build with a test key, adds that key to the firmware's allowed list
                                     (db, next to Microsoft's), starts with Secure Boot enforced; then
                                     checks the firmware refuses the same build unsigned
    ./check.py --signed FILE --cert CER   the same with a file someone else signed (e.g. Windows) and
                                     their certificate

Needs qemu-system-x86_64, OVMF (/usr/share/OVMF) and Pillow; for Secure Boot also openssl, sbsign and
virt-fw-vars (pip install virt-firmware).
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
BUILT = OUT / "esp/EFI/BOOT/BOOTX64.EFI"
OVMF = Path("/usr/share/OVMF")
CODE = OVMF / "OVMF_CODE_4M.fd"
VARS = OVMF / "OVMF_VARS_4M.fd"
CODE_SECURE = OVMF / "OVMF_CODE_4M.secboot.fd"
VARS_MICROSOFT = OVMF / "OVMF_VARS_4M.ms.fd"  # Microsoft's keys enrolled, Secure Boot on: like a PC from a shop
CLAUDEOS_KEY_OWNER = "1dd7d236-3557-417b-b0ab-6cf839b24829"  # who added the key, as recorded in db


def build(test: str | None = None) -> None:
    env = dict(os.environ)
    env.pop("CLAUDEOS_TEST", None)
    if test:
        env["CLAUDEOS_TEST"] = test
    subprocess.run([str(HERE / "build.sh")], check=True, env=env, stdout=subprocess.DEVNULL)


def monitor_many(sock_path: Path, commands: list[str], gap: float = 0.15) -> None:
    """Sends several monitor commands over one connection, `gap` seconds apart (for typing)."""
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.connect(str(sock_path))
        s.settimeout(0.05)
        for command in commands:
            s.sendall(command.encode() + b"\n")
            time.sleep(gap)
            try:
                while s.recv(65536):
                    pass
            except (TimeoutError, BlockingIOError):
                pass


def monitor(sock_path: Path, command: str) -> None:
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
            s.recv(65536)
        except TimeoutError:
            pass


def wait_for(log: Path, words: tuple[str, ...], timeout: float) -> str | None:
    end = time.time() + timeout
    while time.time() < end:
        text = log.read_text(errors="replace") if log.exists() else ""
        for w in words:
            if w in text:
                return w
        time.sleep(0.5)
    return None


class VirtualPC:
    """One run of the virtual PC, started from `efi` as \\EFI\\BOOT\\BOOTX64.EFI."""

    def __init__(self, efi: Path, *, size: str, memory: str, cpu: str, secure_vars: Path | None = None,
                 extra: tuple[str, ...] = ()):
        self.tmp = tempfile.TemporaryDirectory()
        tmp = Path(self.tmp.name)
        esp = tmp / "esp"
        (esp / "EFI/BOOT").mkdir(parents=True)
        shutil.copy(efi, esp / "EFI/BOOT/BOOTX64.EFI")
        vars_fd = tmp / "vars.fd"
        shutil.copy(secure_vars or VARS, vars_fd)
        self.mon = tmp / "monitor.sock"
        OUT.mkdir(exist_ok=True)
        self.log = OUT / "serial.log"
        self.log.unlink(missing_ok=True)
        xres, yres = size.split("x")
        machine = ["-machine", "q35,smm=on", "-global", "driver=cfi.pflash01,property=secure,value=on"] if secure_vars else ["-machine", "q35"]
        code = CODE_SECURE if secure_vars else CODE
        self.qemu = subprocess.Popen([
            "qemu-system-x86_64", *machine,
            "-cpu", cpu,
            "-m", memory,
            "-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={code}",
            "-drive", f"if=pflash,format=raw,unit=1,file={vars_fd}",
            "-drive", f"format=raw,file=fat:rw:{esp}",
            "-vga", "none",
            "-device", f"VGA,edid=on,xres={xres},yres={yres}",
            "-display", "none",
            "-serial", f"file:{self.log}",
            "-monitor", f"unix:{self.mon},server,nowait",
            "-net", "none",
            "-no-reboot",
            *extra,
        ])

    def text(self) -> str:
        return self.log.read_text(errors="replace") if self.log.exists() else ""

    def picture(self, name: str):
        from PIL import Image

        ppm = OUT / f"{name}.ppm"
        monitor(self.mon, f"screendump {ppm}")
        time.sleep(0.5)
        image = Image.open(ppm).convert("RGB")
        image.save(OUT / f"{name}.png")
        ppm.unlink()
        return image

    def stop(self) -> None:
        try:
            monitor(self.mon, "quit")
            self.qemu.wait(timeout=10)
        except Exception:
            self.qemu.kill()
        self.tmp.cleanup()


def check_screen(a, b, size: str) -> list[str]:
    from PIL import ImageChops

    problems = []
    xres, yres = (int(v) for v in size.split("x"))
    if a.size != (xres, yres):
        problems.append(f"the screen is {a.size[0]}x{a.size[1]}, not {size}")
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
    if ImageChops.difference(a, b).getbbox() is None:
        problems.append("the screen didn't change in 3 seconds: are timer interrupts working?")
    return problems


def normal_start(efi: Path, args, name: str = "screen", secure_vars: Path | None = None, expect_secure: bool = False) -> list[str]:
    pc = VirtualPC(efi, size=args.size, memory=args.memory, cpu=args.cpu, secure_vars=secure_vars)
    try:
        got = wait_for(pc.log, ("ClaudeOS: ready", "stopped", "couldn't start", "Access Denied"), args.timeout)
        if got != "ClaudeOS: ready":
            time.sleep(1)
            pc.picture(f"{name}-failed")
            print(pc.text())
            return ["the kernel didn't say it was ready" + (f" (the log says {got!r})" if got else " in time")]
        time.sleep(1.5)
        a = pc.picture(f"{name}-1")
        time.sleep(3.0)
        b = pc.picture(f"{name}-2")
    finally:
        pc.stop()
    text = pc.text()
    print(text)
    problems = check_screen(a, b, args.size)
    if expect_secure and "ClaudeOS kernel: Secure Boot on" not in text:
        problems.append("the kernel didn't see Secure Boot on")
    return problems


def crash(kind: str, args) -> list[str]:
    expected = {"page-fault": "ClaudeOS kernel stopped: Page fault", "panic": "ClaudeOS kernel stopped: panicked"}[kind]
    build(kind)
    try:
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu)
        try:
            got = wait_for(pc.log, ("ClaudeOS kernel stopped",), args.timeout)
            time.sleep(1.5)
            pc.picture(f"screen-{kind}")
        finally:
            pc.stop()
    finally:
        build()  # never leave a kernel that crashes on purpose in out/
    text = pc.text()
    print(text)
    return [] if got and expected in text else [f"expected {expected!r} in the log"]


# What `keyboard` types (QEMU key names), and the line that should come out: Shift, Backspace and Caps
# Lock included.
KEYS = ["shift-h", "i", "shift-1", "spc", "c", "x", "backspace", "l", "a", "u", "d", "e", "spc",
        "caps_lock", "o", "k", "caps_lock"]
TYPED = "Hi! claude OK"


def keyboard_run(label: str, controller: str, args, plug: bool) -> list[str]:
    """One start with a USB controller (`controller`: QEMU's device and options), a keyboard on one of
    its ports, a hub with a keyboard behind it, and a tablet (which ClaudeOS doesn't use yet and must
    leave alone). QEMU types on the keyboard made last: the one behind the hub."""
    print(f"--- USB keyboards, interrupts: {label}")
    extra = ("-device", f"{controller},id=xhci",
             "-device", "usb-kbd,bus=xhci.0,port=1,id=kbd1",
             "-device", "usb-tablet,bus=xhci.0,port=3",
             "-device", "usb-hub,bus=xhci.0,port=2,id=hub",
             "-device", "usb-kbd,bus=xhci.0,port=2.1,id=kbd2")
    pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra)
    problems = []
    name = "keyboard-" + label.lower().replace(" ", "-")
    try:
        got = wait_for(pc.log, ("ClaudeOS: ready", "stopped"), args.timeout)
        if got != "ClaudeOS: ready":
            print(pc.text())
            return [f"{label}: the kernel didn't say it was ready"]
        text = pc.text()
        if text.count(": a keyboard, ") != 2:
            problems.append(f"{label}: expected two keyboards in the log")
        if "port 6.1: a keyboard" not in text:
            problems.append(f"{label}: the keyboard behind the hub wasn't found")
        if "QEMU USB Tablet" not in text or "doesn't use yet" not in text:
            problems.append(f"{label}: the tablet wasn't found and left alone")
        time.sleep(1)
        before = pc.picture(f"{name}-1")
        monitor_many(pc.mon, [f"sendkey {k}" for k in KEYS])
        time.sleep(1)
        after = pc.picture(f"{name}-2")
        monitor_many(pc.mon, ["sendkey ret"])
        if not wait_for(pc.log, (f"typed {TYPED!r}".replace("'", '"'),), 10):
            problems.append(f"{label}: {TYPED!r} didn't come out")
        from PIL import ImageChops
        changed = ImageChops.difference(before, after).getbbox()
        if changed is None or changed[3] - changed[1] > before.size[1] // 4:
            problems.append(f"{label}: the typed line didn't appear on the screen (changed area {changed})")
        if plug:
            # Plug a third keyboard in (QEMU then types on it), type on it, and unplug it; then unplug
            # the hub, which takes the keyboard behind it along.
            monitor(pc.mon, "device_add usb-kbd,bus=xhci.0,port=4,id=kbd3")
            if not wait_for(pc.log, ("port 8: a keyboard",), 15):
                problems.append(f"{label}: the keyboard plugged in later wasn't found")
            monitor_many(pc.mon, ["sendkey o", "sendkey k", "sendkey ret"])
            if not wait_for(pc.log, ('typed "ok"',), 10):
                problems.append(f"{label}: typing on the keyboard plugged in later didn't work")
            monitor(pc.mon, "device_del kbd3")
            if not wait_for(pc.log, ("port 8: \"QEMU USB Keyboard\" was unplugged",), 15):
                problems.append(f"{label}: unplugging the keyboard wasn't noticed")
            monitor(pc.mon, "device_del hub")
            if not wait_for(pc.log, ("port 6: \"QEMU USB Hub\" was unplugged",), 15):
                problems.append(f"{label}: unplugging the hub wasn't noticed")
            elif "port 6.1: \"QEMU USB Keyboard\" was unplugged" not in pc.text():
                problems.append(f"{label}: the keyboard behind the unplugged hub wasn't let go")
            time.sleep(1)
            pc.picture(f"{name}-3")
    finally:
        pc.stop()
    print(pc.text())
    return problems


def keyboard_ps2(args) -> list[str]:
    """No USB keyboard: QEMU types on its PS/2 keyboard, the kind most laptops' built-in keyboards are."""
    print("--- PS/2 keyboard")
    pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu)
    problems = []
    try:
        if wait_for(pc.log, ("ClaudeOS: ready", "stopped"), args.timeout) != "ClaudeOS: ready":
            print(pc.text())
            return ["PS/2: the kernel didn't say it was ready"]
        if "PS/2 keyboard found" not in pc.text():
            problems.append("PS/2: the keyboard wasn't found")
        time.sleep(1)
        monitor_many(pc.mon, [f"sendkey {k}" for k in KEYS] + ["sendkey ret"])
        if not wait_for(pc.log, (f"typed {TYPED!r}".replace("'", '"'),), 10):
            problems.append(f"PS/2: {TYPED!r} didn't come out")
        pc.picture("keyboard-ps2")
    finally:
        pc.stop()
    print(pc.text())
    return problems


def keyboard_iommu(model: str, args) -> list[str]:
    """With an IOMMU (QEMU's model of Intel's or AMD's): found, and USB still reaches memory."""
    print(f"--- USB keyboard with an IOMMU: {model}")
    extra = ("-device", model, "-device", "qemu-xhci,id=xhci", "-device", "usb-kbd,bus=xhci.0,port=1")
    pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra)
    problems = []
    try:
        if wait_for(pc.log, ("ClaudeOS: ready", "stopped"), args.timeout) != "ClaudeOS: ready":
            print(pc.text())
            return [f"{model}: the kernel didn't say it was ready"]
        if "1 IOMMU (not used yet)" not in pc.text():
            problems.append(f"{model}: the IOMMU wasn't found")
        time.sleep(0.5)
        monitor_many(pc.mon, ["sendkey o", "sendkey k", "sendkey ret"])
        if not wait_for(pc.log, ('typed "ok"',), 10):
            problems.append(f"{model}: typing didn't work")
    finally:
        pc.stop()
    print(pc.text())
    return problems


def keyboard(args) -> list[str]:
    build("keyboard")
    try:
        problems = keyboard_ps2(args)
        problems += keyboard_iommu("intel-iommu", args)
        problems += keyboard_iommu("amd-iommu", args)
        # QEMU's own controller (MSI-X), then its model of NEC's, which can be told to use MSI or no
        # interrupts at all.
        problems += keyboard_run("MSI-X", "qemu-xhci", args, plug=True)
        problems += keyboard_run("MSI", "nec-usb-xhci,msix=off", args, plug=False)
        problems += keyboard_run("none", "nec-usb-xhci,msix=off,msi=off", args, plug=False)
    finally:
        build()  # never leave a test build in out/
    return problems


def secure_vars_with(cert: Path, where: Path) -> Path:
    """Firmware settings like a shop PC's (Microsoft's keys, Secure Boot on), plus `cert` in db."""
    pem = where / "cert.pem"
    data = cert.read_bytes()
    if b"-----BEGIN" in data:
        pem.write_bytes(data)
    else:
        subprocess.run(["openssl", "x509", "-inform", "DER", "-in", str(cert), "-out", str(pem)], check=True)
    out = where / "vars-claudeos.fd"
    subprocess.run(["virt-fw-vars", "-i", str(VARS_MICROSOFT), "-o", str(out), "--add-db", CLAUDEOS_KEY_OWNER, str(pem)],
                   check=True, stdout=subprocess.DEVNULL)
    return out


def refused(efi: Path, secure_vars: Path, args) -> list[str]:
    """The firmware must not start `efi` (unsigned, or signed with a key it doesn't trust)."""
    pc = VirtualPC(efi, size=args.size, memory=args.memory, cpu=args.cpu, secure_vars=secure_vars)
    try:
        got = wait_for(pc.log, ("ClaudeOS kernel: started", "Access Denied", "Security Violation"), args.timeout)
    finally:
        pc.stop()
    if got == "ClaudeOS kernel: started":
        return ["the firmware started a build it shouldn't trust"]
    if not got:
        return ["the firmware neither refused nor started it in time"]
    print(f"Refused, as it should be ({got!r}).")
    return []


def secure_boot(args) -> list[str]:
    keys = OUT / "test-key"
    keys.mkdir(parents=True, exist_ok=True)
    key, cert = keys / "test.key", keys / "test.crt"
    if not key.exists():
        subprocess.run(["openssl", "req", "-new", "-x509", "-newkey", "rsa:2048", "-nodes", "-sha256", "-days", "3650",
                        "-subj", "/CN=ClaudeOS test key (not for real PCs)/", "-keyout", str(key), "-out", str(cert)],
                       check=True, stderr=subprocess.DEVNULL)
    signed = OUT / "signed-test.efi"
    subprocess.run(["sbsign", "--key", str(key), "--cert", str(cert), "--output", str(signed), str(BUILT)],
                   check=True, stderr=subprocess.DEVNULL)
    with tempfile.TemporaryDirectory() as tmp:
        vars_fd = secure_vars_with(cert, Path(tmp))
        problems = normal_start(signed, args, "screen-secure-boot", vars_fd, expect_secure=True)
        problems += refused(BUILT, vars_fd, args)
    return problems


def signed_by_someone(args) -> list[str]:
    with tempfile.TemporaryDirectory() as tmp:
        vars_fd = secure_vars_with(Path(args.cert), Path(tmp))
        problems = normal_start(Path(args.signed), args, "screen-signed", vars_fd, expect_secure=True)
        problems += refused(BUILT, vars_fd, args)
    return problems


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--size", default="1920x1080", help="screen resolution the virtual monitor reports")
    ap.add_argument("--memory", default="512", help="memory in MiB")
    ap.add_argument("--cpu", default="max", help="QEMU CPU model")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--timeout", type=float, default=240)
    ap.add_argument("--crash", choices=["page-fault", "panic"])
    ap.add_argument("--keyboard", action="store_true", help="USB keyboards: on a port, behind a hub, plugged in and out")
    ap.add_argument("--secure-boot", action="store_true")
    ap.add_argument("--signed", help="a signed BOOTX64.EFI to start with Secure Boot on")
    ap.add_argument("--cert", help="the certificate (PEM or DER) that signed it")
    args = ap.parse_args()

    if args.crash:
        problems = crash(args.crash, args)
    elif args.keyboard:
        problems = keyboard(args)
    else:
        if not args.no_build:
            build()
        if not BUILT.exists():
            print("Nothing to start: run ./build.sh first.")
            return 2
        if args.signed:
            problems = signed_by_someone(args)
        elif args.secure_boot:
            problems = secure_boot(args)
        else:
            problems = normal_start(BUILT, args)
    for p in problems:
        print("FAILED:", p)
    if not problems:
        print("OK" + (f": {args.crash} stopped cleanly" if args.crash else "") + f". Pictures and the log are in {OUT}.")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
