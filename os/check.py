#!/usr/bin/env python3
"""Starts Chat Computer in a virtual PC (QEMU with OVMF, the open-source UEFI firmware) without a window,
waits until the kernel says it is ready, takes two pictures of the screen a few seconds apart, and
checks them: the screen is in Chat Computer's colours (black text on the warm off-white, a white message
box), and the "Up for" line moved on (so the kernel's own timer interrupts work). Pictures and the kernel's
log go to out/.

    ./check.py                       build, start, check
    ./check.py --size 2560x1440 --memory 1024 --cpu qemu64
    ./check.py --keyboard            keyboards: PS/2; USB on a port and behind a hub, with MSI-X, MSI and
                                     no interrupts, and with Intel's and AMD's IOMMUs present; typing
                                     (Shift, Backspace, Caps Lock) reaches the screen; a keyboard plugged
                                     in later, then unplugged; a hub unplugged
    ./check.py --crash page-fault    a kernel that faults on purpose once ready: it must stop cleanly,
                                     with the fault in its log and a message on the screen (also: panic)
    ./check.py --network             the network, with each kind of card there's a driver for (virtio, Intel's
                                     e1000e, e1000 and igb, a USB network adapter by CDC ECM, plugged in once the
                                     kernel is running and then unplugged, and by RNDIS): an address from DHCP, a
                                     name looked up with DNS, and a TCP conversation with a server on this computer
                                     (--card igb: only that one)
    ./check.py --claude              a conversation with Claude, typed on the keyboard: with a stand-in for
                                     Claude's API on this computer (HTTPS, with a certificate authority made for
                                     the test, which the test build trusts instead of Mozilla's): the request
                                     (model, effort, the key from the settings file, the conversation so far),
                                     the answer streamed onto the screen, Claude's tools (the computer's status,
                                     a bigger text size, a restart, which ends the virtual PC), the answers'
                                     thinking sent back unchanged, a web search (run by Anthropic: its results and
                                     citations kept as they came, the answer paused and carried on), the web tools
                                     refused and left out, and a failure shown
    ./check.py --remember            the conversation kept across a restart: started from a disk laid out like a
                                     PC's (a FAT32 start-up partition with Windows' files and Chat Computer's,
                                     its memory file as Windows' setup makes it), on an NVMe SSD and then a SATA
                                     drive; a question answered, the PC restarted, the conversation back on the
                                     screen and sent with the next question; the memory file's two copies, and
                                     nothing else on the disk changed (fsck.vfat too, so also needs dosfstools,
                                     mtools and gdisk)
    ./check.py --secure-boot        Secure Boot, as on a PC that has approved Chat Computer's key: signs the
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
import json
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
CHATCOMPUTER_KEY_OWNER = "1dd7d236-3557-417b-b0ab-6cf839b24829"  # who added the key, as recorded in db


def build(test: str | None = None, extra: dict[str, str] | None = None) -> None:
    env = dict(os.environ)
    env.pop("CHATCOMPUTER_TEST", None)
    env.pop("CHATCOMPUTER_TEST_CA", None)
    if test:
        env["CHATCOMPUTER_TEST"] = test
    env.update(extra or {})
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
                 extra: tuple[str, ...] = (), settings: str | None = None, disk: tuple[Path, str] | None = None):
        """`disk`: start from this disk image instead (on an NVMe or SATA drive: "nvme" or "sata")."""
        self.tmp = tempfile.TemporaryDirectory()
        tmp = Path(self.tmp.name)
        esp = tmp / "esp"
        (esp / "EFI/BOOT").mkdir(parents=True)
        shutil.copy(efi, esp / "EFI/BOOT/BOOTX64.EFI")
        if settings is not None:
            # Where the Windows setup puts it (own-os.ps1).
            (esp / "EFI/chatcomputer-own").mkdir(parents=True)
            (esp / "EFI/chatcomputer-own/settings.txt").write_text(settings)
        if disk is None:
            drive = ("-drive", f"format=raw,file=fat:rw:{esp}")
        elif disk[1] == "nvme":
            drive = ("-drive", f"if=none,id=d0,format=raw,file={disk[0]}", "-device", "nvme,serial=ccmemory1,drive=d0")
        else:
            drive = ("-drive", f"if=none,id=d0,format=raw,file={disk[0]}", "-device", "ide-hd,drive=d0,bus=ide.0")
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
            *drive,
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

    def settled_picture(self, name: str, tries: int = 8):
        """A picture once the screen has stopped changing (two in a row the same), or the last one taken."""
        last = None
        for _ in range(tries):
            image = self.picture(name)
            if last is not None and image.tobytes() == last.tobytes():
                break
            last = image
        return image

    def stop(self) -> None:
        try:
            monitor(self.mon, "quit")
            self.qemu.wait(timeout=10)
        except Exception:
            self.qemu.kill()
        self.tmp.cleanup()


# Chat Computer's colours (theme.css): the background, black, your messages' bubbles, the message box.
GROUND, INK, BUBBLE, PAPER = (248, 245, 240), (0, 0, 0), (231, 228, 221), (255, 255, 255)


def colours(image) -> dict:
    return {c: n for n, c in (image.getcolors(maxcolors=1 << 20) or [])}


def check_colours(a, size: str) -> list[str]:
    """`a` is the size asked for, and in Chat Computer's colours: mostly its background, black text, a white message
    box, and nothing that isn't a mix of its warm greys (each of which has as much red as green, and as much green
    as blue, or more)."""
    problems = []
    xres, yres = (int(v) for v in size.split("x"))
    if a.size != (xres, yres):
        problems.append(f"the screen is {a.size[0]}x{a.size[1]}, not {size}")
    seen = colours(a)
    total = a.size[0] * a.size[1]
    foreign = [c for c in seen if not (c[0] >= c[1] >= c[2])]
    if foreign:
        problems.append(f"{len(foreign)} colours that aren't Chat Computer's, e.g. {foreign[:3]}")
    if seen.get(GROUND, 0) < total * 0.6:
        problems.append(f"only {seen.get(GROUND, 0) * 100 // total}% of the screen is the background")
    if seen.get(INK, 0) < 500:
        problems.append("almost no black: is there any text?")
    if seen.get(PAPER, 0) < total * 0.01:
        problems.append("no message box (hardly any white)")
    return problems


def check_screen(a, b, size: str) -> list[str]:
    from PIL import ImageChops

    problems = check_colours(a, size)
    if ImageChops.difference(a, b).getbbox() is None:
        problems.append("the screen didn't change in 3 seconds: are timer interrupts working?")
    return problems


def box_rows(log: str) -> tuple[int, int] | None:
    """Where the message box is, as the kernel says in its log."""
    import re

    m = re.search(r"message box at rows (\d+) to (\d+)", log)
    return (int(m.group(1)), int(m.group(2))) if m else None


def normal_start(efi: Path, args, name: str = "screen", secure_vars: Path | None = None, expect_secure: bool = False) -> list[str]:
    pc = VirtualPC(efi, size=args.size, memory=args.memory, cpu=args.cpu, secure_vars=secure_vars)
    try:
        got = wait_for(pc.log, ("Chat Computer: ready", "stopped", "couldn't start", "Access Denied"), args.timeout)
        if got != "Chat Computer: ready":
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
    if expect_secure and "Chat Computer kernel: Secure Boot on" not in text:
        problems.append("the kernel didn't see Secure Boot on")
    return problems


def crash(kind: str, args) -> list[str]:
    expected = {"page-fault": "Chat Computer kernel stopped: Page fault", "panic": "Chat Computer kernel stopped: panicked"}[kind]
    build(kind)
    try:
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu)
        try:
            got = wait_for(pc.log, ("Chat Computer kernel stopped",), args.timeout)
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
    its ports, a hub with a keyboard behind it, and a tablet (which Chat Computer doesn't use yet and must
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
        got = wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout)
        if got != "Chat Computer: ready":
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
        rows = box_rows(pc.text())
        if rows is None:
            problems.append(f"{label}: the kernel didn't say where the message box is")
        else:
            box = (0, rows[0], before.size[0], rows[1])
            if ImageChops.difference(before.crop(box), after.crop(box)).getbbox() is None:
                problems.append(f"{label}: the typed line didn't appear in the message box")
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
        if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
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
        if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
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


TEST_PORT = 8471


# The network cards `network` tries: a name, QEMU's devices for it, how the kernel's log names it, what the test
# build needs to know (QEMU's USB adapter offers both CDC ECM and RNDIS; a build told so prefers RNDIS), and a
# device plugged in once the kernel is running (then unplugged), if any.
USB_ADAPTER = "usb-net,netdev=n0,bus=xhci.0,id=usbnet"
CARDS = [
    ("virtio", ("-device", "virtio-net-pci,netdev=n0,romfile="), "virtio network card", None, None),
    ("e1000e", ("-device", "e1000e,netdev=n0,romfile="), "Intel 82574L network card", None, None),  # QEMU's e1000e
    ("e1000", ("-device", "e1000,netdev=n0,romfile="), "Intel 82540EM network card", None, None),  # VirtualBox's, VMware's
    ("igb", ("-device", "igb,netdev=n0,romfile="), "Intel 82576 network chip", None, None),  # the I210's and I211's family
    ("usb-ecm", ("-device", "qemu-xhci,id=xhci"), "USB network adapter \"RNDIS/QEMU USB Network Device\" (USB CDC ECM",
     None, USB_ADAPTER),
    ("usb-rndis", ("-device", "qemu-xhci,id=xhci", "-device", USB_ADAPTER),
     "USB network adapter \"RNDIS/QEMU USB Network Device\" (RNDIS", {"CHATCOMPUTER_TEST_USB_NET": "rndis"}, None),
]


def network(args) -> list[str]:
    """A test build with QEMU's virtual network (its DHCP and DNS, and 10.0.2.2 for this computer), once with each
    card in CARDS: it must get 10.0.2.15, look up api.anthropic.com, and talk to a server here (which answers its
    line)."""
    import socketserver
    import threading

    heard: list[str] = []

    class Answer(socketserver.StreamRequestHandler):
        def handle(self):
            line = self.rfile.readline(200).decode(errors="replace").strip()
            heard.append(line)
            self.wfile.write(f"hello, Chat Computer: you said {line!r}\n".encode())

    server = socketserver.ThreadingTCPServer(("127.0.0.1", TEST_PORT), Answer)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    problems = []
    built = None
    try:
        for device, devices, named, env, later in CARDS:
            if args.card and device not in args.card:
                continue
            print(f"--- {device}")
            if built != (env or {}):
                build("network", env)
                built = env or {}
            heard.clear()
            extra = ("-netdev", "user,id=n0", *devices)
            pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra)
            unplugged = None
            try:
                if later:
                    if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) == "Chat Computer: ready":
                        monitor(pc.mon, f"device_add {later}")
                got = wait_for(pc.log, ("test: the server said", "test: couldn't", "test: no answer", "never opened", "stopped"),
                               args.timeout)
                time.sleep(1)
                pc.picture(f"network-{device}")
                if later:
                    monitor(pc.mon, "device_del usbnet")
                    unplugged = wait_for(pc.log, ("network adapter was unplugged",), 30)
            finally:
                pc.stop()
            text = pc.text()
            print(text)
            if f"network: {named}" not in text:
                problems.append(f"{device}: the card wasn't started")
            if "network: address 10.0.2.15/24, gateway 10.0.2.2, DNS 10.0.2.3" not in text:
                problems.append(f"{device}: no address from DHCP")
            if "test: api.anthropic.com is at " not in text:
                problems.append(f"{device}: api.anthropic.com wasn't found with DNS")
            if heard != ["hello from Chat Computer"]:
                problems.append(f"{device}: the server heard {heard!r}")
            if got != "test: the server said" or "you said 'hello from Chat Computer'" not in text:
                problems.append(f"{device}: the server's answer didn't come back")
            if later and not unplugged:
                problems.append(f"{device}: unplugging the adapter wasn't noticed")
    finally:
        server.shutdown()
        build()  # never leave a test build in out/
    return problems


CLAUDE_PORT = 8472  # assistant.rs TEST_PORT
TEST_KEY = "test-key-for-the-stand-in"
FIRST_WORDS = "Let me look."


def stream(blocks: list[tuple], stop: str) -> list[tuple[str, dict]]:
    """An answer as Claude's API streams one: each block ("thinking", signature) (its words left out, as Opus 5.5 does
    unless asked), ("text", [words...]) or ("text", [words...], [citations...]), ("tool_use", id, name, [pieces of its
    input's JSON]), ("server_tool_use", id, name, [pieces]) for the tools Anthropic runs, or ("result", block) for
    their results (which come whole), then why it stopped. ("wait", seconds) is a pause in the stream, as while
    Anthropic searches the web."""
    events = [("message_start", {"type": "message_start", "message": {"id": "msg_test", "type": "message", "role": "assistant",
                                                                       "model": "claude-opus-5-5", "content": [], "stop_reason": None}})]
    i = -1
    for b in blocks:
        if b[0] == "wait":
            events.append(("wait", {"seconds": b[1]}))
            continue
        i += 1

        def delta(d: dict, i: int = i) -> tuple[str, dict]:
            return "content_block_delta", {"type": "content_block_delta", "index": i, "delta": d}
        if b[0] == "thinking":
            events += [("content_block_start", {"type": "content_block_start", "index": i,
                                                "content_block": {"type": "thinking", "thinking": "", "signature": ""}}),
                       delta({"type": "thinking_delta", "thinking": ""}), delta({"type": "signature_delta", "signature": b[1]})]
        elif b[0] == "text":
            events.append(("content_block_start", {"type": "content_block_start", "index": i, "content_block": {"type": "text", "text": ""}}))
            events += [delta({"type": "text_delta", "text": w}) for w in b[1]]
            events += [delta({"type": "citations_delta", "citation": c}) for c in (b[2] if len(b) > 2 else [])]
        elif b[0] == "result":
            events.append(("content_block_start", {"type": "content_block_start", "index": i, "content_block": b[1]}))
        else:
            events.append(("content_block_start", {"type": "content_block_start", "index": i,
                                                   "content_block": {"type": b[0], "id": b[1], "name": b[2], "input": {}}}))
            events += [delta({"type": "input_json_delta", "partial_json": piece}) for piece in b[3]]
        events.append(("content_block_stop", {"type": "content_block_stop", "index": i}))
        if i == 0:
            events.append(("ping", {"type": "ping"}))
    events.append(("message_delta", {"type": "message_delta", "delta": {"stop_reason": stop}, "usage": {"output_tokens": 20}}))
    events.append(("message_stop", {"type": "message_stop"}))
    return events


def test_authority(tmp: Path) -> None:
    """A certificate authority made for the test (tmp/ca.der, which a test build trusts instead of Mozilla's), and a
    certificate from it for "localhost" (tmp/leaf.pem, tmp/leaf.key)."""
    def openssl(*a: str) -> None:
        subprocess.run(["openssl", *a], cwd=tmp, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    p256 = ["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256"]
    openssl("req", "-x509", "-nodes", *p256, "-keyout", "ca.key", "-out", "ca.pem", "-days", "2",
            "-subj", "/CN=Chat Computer test authority (not for real use)",
            "-addext", "basicConstraints=critical,CA:true", "-addext", "keyUsage=critical,keyCertSign")
    (tmp / "leaf.ext").write_text("subjectAltName=DNS:localhost\nextendedKeyUsage=serverAuth\n"
                                  "keyUsage=critical,digitalSignature\nbasicConstraints=critical,CA:false\n")
    openssl("req", "-nodes", *p256, "-keyout", "leaf.key", "-out", "leaf.csr", "-subj", "/CN=localhost")
    openssl("x509", "-req", "-in", "leaf.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", "leaf.pem",
            "-days", "2", "-extfile", "leaf.ext")
    openssl("x509", "-in", "ca.pem", "-outform", "DER", "-out", "ca.der")


def serve_https(handler, tmp: Path):
    """The stand-in for Claude's API: `handler` over HTTPS (TLS 1.3, tmp's certificate) on CLAUDE_PORT."""
    import http.server
    import ssl
    import threading

    server = http.server.ThreadingHTTPServer(("127.0.0.1", CLAUDE_PORT), handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_3
    context.load_cert_chain(tmp / "leaf.pem", tmp / "leaf.key")
    server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def send_stream(handler, blocks: list[tuple], stop: str) -> None:
    """An answer, streamed as Claude's API streams one (see `stream`)."""
    handler.send_response(200)
    handler.send_header("content-type", "text/event-stream")
    handler.send_header("transfer-encoding", "chunked")
    handler.end_headers()
    for event, payload in stream(blocks, stop):
        if event == "wait":
            time.sleep(payload["seconds"])
            continue
        chunk = f"event: {event}\ndata: {json.dumps(payload)}\n\n".encode()
        handler.wfile.write(b"%x\r\n%s\r\n" % (len(chunk), chunk))
        handler.wfile.flush()
        time.sleep(0.05)
    handler.wfile.write(b"0\r\n\r\n")
    handler.wfile.flush()


def blocks_as_sent(blocks: list[tuple]) -> list[dict]:
    """The same blocks as the answer's content, which must come back unchanged in the next request."""
    out = []
    for b in blocks:
        if b[0] == "wait":
            continue
        if b[0] == "thinking":
            out.append({"type": "thinking", "thinking": "", "signature": b[1]})
        elif b[0] == "text":
            out.append({"type": "text", "text": "".join(b[1])} | ({"citations": b[2]} if len(b) > 2 else {}))
        elif b[0] == "result":
            out.append(b[1])
        else:
            out.append({"type": b[0], "id": b[1], "name": b[2], "input": json.loads("".join(b[3]) or "{}")})
    return out


def claude(args) -> list[str]:
    """A conversation typed on the keyboard, with a stand-in for Claude's API on this computer (HTTPS on
    CLAUDE_PORT, as "localhost", with a certificate from an authority made for the test, which the test build trusts
    instead of Mozilla's). The first question is answered with tools: the stand-in has Claude look at the computer's
    status, then make the text bigger, then answer with what the status said (streamed, a piece at a time); each
    answer, thinking included, must come back unchanged with the tool's result. The second question gets Claude's
    API's "overloaded" error, which the screen must show. The third is answered after a web search (which Anthropic
    runs): its results and the answer's citations must come back as they came, and the answer, paused by the API,
    must be sent back to go on, the rest joining it. The fourth asks to restart: the API first refuses the web tools
    (as it does when an organization has switched them off), so the question must be asked again without them;
    then Claude restarts the PC. The stand-in checks what it was sent."""
    import http.server

    tmp = Path(tempfile.mkdtemp(prefix="cc-claude-"))
    test_authority(tmp)

    requests: list[dict] = []
    r1 = [("thinking", "sig-1/+="), ("text", ["Let me ", "look."]), ("tool_use", "toolu_1", "computer_status", [])]
    r2 = [("thinking", "sig-2"), ("tool_use", "toolu_2", "set_text_size", ['{"percent"', ': 125}'])]
    final: list[str] = []
    search_result = {"type": "web_search_tool_result", "tool_use_id": "srvtoolu_1",
                     "content": [{"type": "web_search_result", "title": "Weather", "url": "https://example.com/w",
                                  "encrypted_content": "EqgfCi+/=", "page_age": None}]}
    citation = {"type": "web_search_result_location", "url": "https://example.com/w", "title": "Weather",
                "encrypted_index": "Eo8B=", "cited_text": "Sunny all day."}
    r_search = [("thinking", "sig-s"), ("server_tool_use", "srvtoolu_1", "web_search", ['{"query": ', '"weather"}']),
                ("wait", 4), ("result", search_result), ("text", ["It's ", "sunny."], [citation])]
    r_more = [("text", ["Enjoy the day."])]
    r5 = [("thinking", "sig-5"), ("tool_use", "toolu_5", "restart_computer", ["{}"])]
    r6 = [("text", ["Restarting now: see you in Windows."])]

    def r3() -> list[tuple]:
        # What the status said (the processor), in the answer: the tool's result came back.
        messages = (requests[-1]["body"] or {}).get("messages") or []
        result = str((((messages[2:3] or [{}])[0].get("content") or [{}])[0]).get("content", ""))
        processor = next((line[len("Processor: "):] for line in result.splitlines() if line.startswith("Processor: ")), "?")
        final.append(f"I see {processor}, and the text is bigger now.")
        return [("thinking", "sig-3"), ("text", [final[0][:10], final[0][10:]])]

    class Api(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *a) -> None:
            pass

        def do_POST(self) -> None:
            body = self.rfile.read(int(self.headers.get("content-length", "0")))
            try:
                data = json.loads(body)
            except ValueError:
                data = None
            requests.append({"path": self.path, "key": self.headers.get("x-api-key"),
                             "version": self.headers.get("anthropic-version"), "body": data})
            script = {1: (r1, "tool_use"), 2: (r2, "tool_use"), 5: (r_search, "pause_turn"), 6: (r_more, "end_turn"),
                      8: (r5, "tool_use"), 9: (r6, "end_turn")}.get(len(requests))
            if len(requests) == 3:
                script = (r3(), "end_turn")
            if len(requests) == 7:
                # As the API answers when an organization has switched web search off for its keys.
                error = json.dumps({"type": "error", "error": {"type": "invalid_request_error",
                                                               "message": "web search is not enabled for this organization"}}).encode()
                self.send_response(400)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(error)))
                self.end_headers()
                self.wfile.write(error)
            elif script:
                self.send_response(200)
                self.send_header("content-type", "text/event-stream")
                self.send_header("transfer-encoding", "chunked")
                self.end_headers()
                for event, payload in stream(*script):
                    if event == "wait":
                        time.sleep(payload["seconds"])
                        continue
                    chunk = f"event: {event}\ndata: {json.dumps(payload)}\n\n".encode()
                    self.wfile.write(b"%x\r\n%s\r\n" % (len(chunk), chunk))
                    self.wfile.flush()
                    time.sleep(0.1)
                self.wfile.write(b"0\r\n\r\n")
            else:
                error = json.dumps({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}).encode()
                self.send_response(529)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(error)))
                self.end_headers()
                self.wfile.write(error)
            self.wfile.flush()

    server = serve_https(Api, tmp)
    build("claude", {"CHATCOMPUTER_TEST_CA": str(tmp / "ca.der")})
    problems: list[str] = []
    try:
        # (Intel's card here; --network tries every kind.)
        extra = ("-netdev", "user,id=n0", "-device", "e1000e,netdev=n0,romfile=")
        # As Chat Computer Own System writes it on Windows (New-OwnSettingsText): notes, Windows' line breaks.
        settings = ("# Chat Computer's settings, read by its own system when it starts. Written by Chat Computer Own System.\r\n"
                    "# The Claude API key is kept here as it is: anyone who can read this partition can read it.\r\n"
                    f"claude-api-key = {TEST_KEY}\r\nclock-offset-minutes = 0\r\n")
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra, settings=settings)
        first = second = third = searched = searching = None
        restarted = ended = False
        try:
            if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
                print(pc.text())
                return ["the kernel didn't say it was ready"]
            if not wait_for(pc.log, ("network: address",), 60):
                problems.append("no network address")
            time.sleep(1)
            pc.picture("claude-0")
            monitor_many(pc.mon, ["sendkey h", "sendkey i", "sendkey ret"])
            first = wait_for(pc.log, ("test: Claude said", "Claude: failed", "stopped"), args.timeout)
            answered = pc.settled_picture("claude-1")
            monitor_many(pc.mon, [f"sendkey {k}" for k in "again"] + ["sendkey ret"])
            second = wait_for(pc.log, ("Claude: failed", "stopped"), 60)
            pc.settled_picture("claude-2")
            # A question answered after a web search.
            monitor_many(pc.mon, [f"sendkey {k}" for k in "weather"] + ["sendkey ret"])
            if wait_for(pc.log, ("Claude: searches the web",), 60):
                time.sleep(1)
                searching = pc.picture("claude-searching")
            searched = wait_for(pc.log, ("Enjoy the day",), 60)
            pc.settled_picture("claude-search")
            # Asked to restart: Claude's answer, then a restart (which ends a virtual PC started with -no-reboot).
            monitor_many(pc.mon, [f"sendkey {k}" for k in "restart"] + ["sendkey ret"])
            third = wait_for(pc.log, ("Restarting now", "Claude: failed", "stopped"), 60)
            pc.settled_picture("claude-3")
            restarted = wait_for(pc.log, ("kernel: restarting",), 30) is not None
            try:
                pc.qemu.wait(timeout=20)
                ended = True
            except subprocess.TimeoutExpired:
                ended = False
        finally:
            pc.stop()
        text = pc.text()
        print(text)
        if "clock: " not in text or "(local time, offset from the settings)" not in text:
            problems.append("the clock offset in the settings wasn't used")
        if "Claude: connected (TLS 1.3" not in text:
            problems.append("no secure connection to the stand-in")
        said = FIRST_WORDS + "\n\n" + (final[0] if final else "?")
        if first != "test: Claude said" or f"test: Claude said {json.dumps(said)}" not in text:
            problems.append(f"the answer didn't come back whole (the log says {first!r})")
        if "Claude: uses computer_status" not in text or "Claude: uses set_text_size" not in text:
            problems.append("the tools weren't used")
        if "text size 125%" not in text:
            problems.append("the text wasn't made bigger")
        if second != "Claude: failed" or "Claude is very busy right now (Overloaded)" not in text:
            problems.append("the second question's failure wasn't reported")
        if TEST_KEY in text:
            problems.append("the API key is in the log")
        if third is None or "Claude: uses restart_computer" not in text:
            problems.append("asking to restart didn't reach the restart tool")
        elif not restarted or not ended:
            problems.append(f"the PC didn't restart (the kernel said so: {restarted}; the virtual PC ended: {ended})")
        if searched != "Enjoy the day" or 'test: Claude said "It\'s sunny.\\n\\nEnjoy the day."' not in text:
            problems.append(f"the answer after a web search didn't come whole (the log says {searched!r})")
        if "Claude: searches the web" not in text:
            problems.append("the web search wasn't noticed")
        if "the web tools aren't enabled for this API key" not in text:
            problems.append("the web tools weren't left out once the API refused them")
        if len(requests) != 9:
            problems.append(f"the stand-in got {len(requests)} requests, not 9")
        else:
            r = requests[0]
            body = r["body"] or {}
            wanted = {"model": "claude-opus-5-5", "max_tokens": 64000, "stream": True, "output_config": {"effort": "max"}}
            for k, v in wanted.items():
                if body.get(k) != v:
                    problems.append(f"the request's {k} was {body.get(k)!r}, not {v!r}")
            if (r["path"], r["key"], r["version"]) != ("/v1/messages", TEST_KEY, "2023-06-01"):
                problems.append(f"the request went to {r['path']!r} with key {r['key'] == TEST_KEY} and version {r['version']!r}")
            if "plain text" not in str(body.get("system")):
                problems.append("the request didn't say the screen shows plain text")
            if body.get("messages") != [{"role": "user", "content": "hi"}]:
                problems.append(f"the first request's messages were {body.get('messages')!r}")
            tools = sorted((t.get("name"), t.get("type")) for t in body.get("tools") or [])
            if tools != [("computer_status", None), ("restart_computer", None), ("set_text_size", None),
                         ("web_fetch", "web_fetch_20250910"), ("web_search", "web_search_20250305")]:
                problems.append(f"the request offered the tools {body.get('tools')!r}")
            m2 = (requests[1]["body"] or {}).get("messages") or []
            if m2[:2] != [{"role": "user", "content": "hi"}, {"role": "assistant", "content": blocks_as_sent(r1)}]:
                problems.append(f"the first answer didn't come back unchanged: {m2[:2]!r}")
            result = ((m2[2:3] or [{}])[0].get("content") or [{}])[0] if len(m2) == 3 else {}
            if (result.get("type"), result.get("tool_use_id"), result.get("is_error")) != ("tool_result", "toolu_1", False) \
                    or "Processor: " not in str(result.get("content")) or "Memory: " not in str(result.get("content")):
                problems.append(f"the computer's status didn't come back as the tool's result: {result!r}")
            m3 = (requests[2]["body"] or {}).get("messages") or []
            if m3[:3] != m2 or m3[3:4] != [{"role": "assistant", "content": blocks_as_sent(r2)}] or len(m3) != 5:
                problems.append(f"the second round's request wasn't the first's plus the answer and result: {m3[3:]!r}")
            m4 = (requests[3]["body"] or {}).get("messages") or []
            if len(m4) != 7 or m4[:5] != m3 or m4[6] != {"role": "user", "content": "again"} \
                    or m4[5] != {"role": "assistant", "content": blocks_as_sent([("thinking", "sig-3"), ("text", final[:1])])}:
                problems.append(f"the second question's messages were {m4!r}")
            # The web search: the paused answer sent back as it came, to go on; then joined by the rest.
            m5 = (requests[4]["body"] or {}).get("messages") or []
            if m5 != m4[:6] + [{"role": "user", "content": "weather"}]:
                problems.append(f"the third question's messages were {m5[5:]!r}")
            m6 = (requests[5]["body"] or {}).get("messages") or []
            if m6 != m5 + [{"role": "assistant", "content": blocks_as_sent(r_search)}]:
                problems.append(f"the paused answer didn't come back as it came: {m6[6:]!r}")
            m7 = (requests[6]["body"] or {}).get("messages") or []
            joined = {"role": "assistant", "content": blocks_as_sent(r_search) + blocks_as_sent(r_more)}
            if m7 != m5 + [joined, {"role": "user", "content": "restart"}]:
                problems.append(f"the answer after the search wasn't kept whole: {m7[6:]!r}")
            b8 = requests[7]["body"] or {}
            if b8.get("messages") != m7:
                problems.append("the refused question wasn't asked again as it was")
            if any(t.get("type", "").startswith("web_") for t in b8.get("tools") or []) or "can't use the web" not in str(b8.get("system")):
                problems.append(f"the question asked again still offered the web tools: {b8.get('tools')!r}")
        if first == "test: Claude said":
            problems += check_colours(answered, args.size)
            if colours(answered).get(BUBBLE, 0) < 1000:
                problems.append("no bubble for what was typed")
    finally:
        server.shutdown()
        shutil.rmtree(tmp, ignore_errors=True)
        build()  # never leave a test build in out/
    return problems


MEMORY_ID = 0x5EEDCAFE


def blank_memory(memory_id: int, size: int) -> bytes:
    """Chat Computer's memory file as Windows' setup makes it (the remember crate's format, windows/lib/OwnOs.ps1):
    512-byte chunks, each starting with "CCMEMORY", the file's id and the chunk's number (little-endian)."""
    out = bytearray(size // 512 * 512)
    for i in range(size // 512):
        out[i * 512:i * 512 + 16] = b"CCMEMORY" + memory_id.to_bytes(4, "little") + i.to_bytes(4, "little")
    return bytes(out)


def kept_in(memory: bytes, memory_id: int) -> tuple[int, bytes] | None:
    """What a memory file holds (its newest whole copy: sequence number and data), read as the remember crate does;
    None if nothing. Every chunk's tag must still be there."""
    import struct

    chunks = len(memory) // 512
    for i in range(chunks):
        if memory[i * 512:i * 512 + 16] != b"CCMEMORY" + memory_id.to_bytes(4, "little") + i.to_bytes(4, "little"):
            raise ValueError(f"chunk {i}'s tag was changed")
    best = None
    for h in (0, 1):
        def data(c: int) -> bytes:
            return memory[c * 512 + 16:(c + 1) * 512]
        first = data(h * (chunks // 2))
        if first[:8] != b"CCMEMDAT":
            continue
        _, sequence, length, expected = struct.unpack("<IQIQ", first[8:32])
        body = b"".join(data(h * (chunks // 2) + k) for k in range((32 + length + 495) // 496))[32:32 + length]
        h64 = 0xCBF29CE484222325
        for b in body:
            h64 = ((h64 ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
        if h64 == expected and (best is None or sequence > best[0]):
            best = (sequence, body)
    return best


ESP_START, ESP_MIB = 1 << 20, 80


def memory_disk(path: Path, efi: Path, settings: str, memory_id: int) -> dict[str, bytes]:
    """A disk like a PC's: a GPT, one EFI system partition from 1 MiB (FAT32, 1 KiB clusters, as Windows makes it),
    with Chat Computer's files, a stand-in for Windows' start-up file, and a 4 MiB memory file. Returns the files."""
    with open(path, "wb") as f:
        f.truncate(96 << 20)
    subprocess.run(["sgdisk", "-n", f"1:2048:+{ESP_MIB}M", "-t", "1:EF00", str(path)], check=True, stdout=subprocess.DEVNULL)
    esp = path.with_suffix(".esp")
    esp.unlink(missing_ok=True)
    subprocess.run(["mkfs.vfat", "-C", "-F", "32", "-s", "2", str(esp), str(ESP_MIB * 1024)], check=True,
                   stdout=subprocess.DEVNULL)
    files = {"EFI/Microsoft/Boot/bootmgfw.efi": os.urandom(300_000), "EFI/BOOT/BOOTX64.EFI": efi.read_bytes(),
             "EFI/chatcomputer-own/settings.txt": settings.encode(),
             "EFI/chatcomputer-own/memory.bin": blank_memory(memory_id, 4 << 20)}
    for folder in ("::/EFI", "::/EFI/Microsoft", "::/EFI/Microsoft/Boot", "::/EFI/BOOT", "::/EFI/chatcomputer-own"):
        subprocess.run(["mmd", "-i", str(esp), folder], check=True)
    for name, data in files.items():
        local = path.with_suffix(".file")
        local.write_bytes(data)
        subprocess.run(["mcopy", "-i", str(esp), str(local), f"::/{name}"], check=True)
        local.unlink()
    with open(path, "r+b") as f:
        f.seek(ESP_START)
        f.write(esp.read_bytes())
    esp.unlink()
    return files


def esp_now(path: Path, files: dict[str, bytes]) -> tuple[list[str], bytes]:
    """The disk's start-up partition checked: its file system clean (fsck.vfat, changing nothing), every file but the
    memory file as it was. Returns the problems, and the memory file."""
    problems = []
    esp = path.with_suffix(".esp")
    with open(path, "rb") as f:
        f.seek(ESP_START)
        esp.write_bytes(f.read(ESP_MIB << 20))
    fsck = subprocess.run(["fsck.vfat", "-n", str(esp)], capture_output=True, text=True)
    if fsck.returncode != 0:
        problems.append(f"the start-up partition's file system isn't clean: {fsck.stdout.strip()} {fsck.stderr.strip()}")
    memory = b""
    for name, data in files.items():
        local = path.with_suffix(".out")
        local.unlink(missing_ok=True)
        subprocess.run(["mcopy", "-i", str(esp), f"::/{name}", str(local)], check=True)
        now = local.read_bytes()
        local.unlink()
        if name.endswith("memory.bin"):
            memory = now
            if len(now) != len(data):
                problems.append("the memory file's size changed")
        elif now != data:
            problems.append(f"{name} changed")
    esp.unlink()
    return problems, memory


def remember(args) -> list[str]:
    """The conversation kept across a restart, with the start-up partition on an NVMe drive, then on a SATA one: a
    disk like a PC's (memory_disk), a question answered by a stand-in for Claude's API, then the virtual PC started
    again: the conversation must be on the screen, and in the next question's request, and in the memory file in the
    format Windows' setup made it for. The partition's file system must stay clean and its other files untouched."""
    import http.server

    tmp = Path(tempfile.mkdtemp(prefix="cc-memory-"))
    test_authority(tmp)
    requests: list[dict] = []
    first_answer = [("thinking", "sig-m1"), ("text", ["Hello! The word is ", "pumpkin."])]

    class Api(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *a) -> None:
            pass

        def do_POST(self) -> None:
            body = json.loads(self.rfile.read(int(self.headers.get("content-length", "0"))) or b"null") or {}
            requests.append(body)
            asked = ((body.get("messages") or [{}])[-1]).get("content")
            send_stream(self, first_answer if asked == "hi" else [("text", ["Pumpkin."])], "end_turn")

    server = serve_https(Api, tmp)
    build("claude", {"CHATCOMPUTER_TEST_CA": str(tmp / "ca.der")})
    problems: list[str] = []
    first = [{"role": "user", "content": "hi"}, {"role": "assistant", "content": blocks_as_sent(first_answer)}]
    try:
        for kind in ("nvme", "sata"):
            requests.clear()
            disk = tmp / f"{kind}.img"
            settings = (f"claude-api-key = {TEST_KEY}\r\nclock-offset-minutes = 0\r\n"
                        f"memory-id = {MEMORY_ID:08x}\r\n")
            files = memory_disk(disk, BUILT, settings, MEMORY_ID)
            extra = ("-netdev", "user,id=n0", "-device", "e1000e,netdev=n0,romfile=")
            texts = []
            for start in (1, 2):
                pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra, disk=(disk, kind))
                try:
                    if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
                        print(pc.text())
                        return problems + [f"{kind}, start {start}: the kernel didn't say it was ready"]
                    wait_for(pc.log, ("network: address",), 60)
                    time.sleep(1)
                    if start == 2:
                        pc.settled_picture(f"memory-{kind}-restored")
                    question = "hi" if start == 1 else "word"
                    monitor_many(pc.mon, [f"sendkey {k}" for k in question] + ["sendkey ret"])
                    wait_for(pc.log, ("memory: the conversation kept", "couldn't be kept", "Claude: failed"), 120)
                    pc.settled_picture(f"memory-{kind}-{start}")
                finally:
                    pc.stop()
                texts.append(pc.text())
                print(pc.text())
                fs_problems, memory_file = esp_now(disk, files)
                problems += [f"{kind}, start {start}: {p}" for p in fs_problems]
                try:
                    kept = kept_in(memory_file, MEMORY_ID)
                except ValueError as e:
                    problems.append(f"{kind}, start {start}: {e}")
                    continue
                wanted = first if start == 1 else first + [{"role": "user", "content": "word"},
                                                           {"role": "assistant", "content": [{"type": "text", "text": "Pumpkin."}]}]
                if kept is None or kept[0] != start or json.loads(kept[1]) != wanted:
                    problems.append(f"{kind}, start {start}: the memory file holds {kept!r}")
            drive = "NVMe SSD" if kind == "nvme" else "SATA drive"
            if f"memory: {drive}" not in texts[0]:
                problems.append(f"{kind}: the memory file wasn't found on the {drive}")
            if "the conversation from before the restart (2 turns)" not in texts[1]:
                problems.append(f"{kind}: the conversation didn't come back after the restart")
            if len(requests) != 2 or requests[1].get("messages") != first + [{"role": "user", "content": "word"}]:
                problems.append(f"{kind}: the question after the restart wasn't asked with the conversation before it: "
                                f"{[r.get('messages') for r in requests]!r}")
    finally:
        server.shutdown()
        shutil.rmtree(tmp, ignore_errors=True)
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
    out = where / "vars-chatcomputer.fd"
    subprocess.run(["virt-fw-vars", "-i", str(VARS_MICROSOFT), "-o", str(out), "--add-db", CHATCOMPUTER_KEY_OWNER, str(pem)],
                   check=True, stdout=subprocess.DEVNULL)
    return out


def refused(efi: Path, secure_vars: Path, args) -> list[str]:
    """The firmware must not start `efi` (unsigned, or signed with a key it doesn't trust)."""
    pc = VirtualPC(efi, size=args.size, memory=args.memory, cpu=args.cpu, secure_vars=secure_vars)
    try:
        got = wait_for(pc.log, ("Chat Computer kernel: started", "Access Denied", "Security Violation"), args.timeout)
    finally:
        pc.stop()
    if got == "Chat Computer kernel: started":
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
                        "-subj", "/CN=Chat Computer test key (not for real PCs)/", "-keyout", str(key), "-out", str(cert)],
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
    ap.add_argument("--network", action="store_true", help="each network card there's a driver for: DHCP, DNS and TCP")
    ap.add_argument("--card", action="append", choices=[c[0] for c in CARDS], help="with --network: only this card")
    ap.add_argument("--claude", action="store_true", help="a conversation with a stand-in for Claude's API, over HTTPS")
    ap.add_argument("--remember", action="store_true", help="the conversation kept across a restart, on NVMe and SATA")
    ap.add_argument("--secure-boot", action="store_true")
    ap.add_argument("--signed", help="a signed BOOTX64.EFI to start with Secure Boot on")
    ap.add_argument("--cert", help="the certificate (PEM or DER) that signed it")
    args = ap.parse_args()

    if args.crash:
        problems = crash(args.crash, args)
    elif args.keyboard:
        problems = keyboard(args)
    elif args.network:
        problems = network(args)
    elif args.claude:
        problems = claude(args)
    elif args.remember:
        problems = remember(args)
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
