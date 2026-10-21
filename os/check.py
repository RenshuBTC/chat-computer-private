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
    ./check.py --network             the network: a virtio card (QEMU's virtual network), an address from DHCP,
                                     a name looked up with DNS, and a TCP conversation with a server on this
                                     computer
    ./check.py --claude              a conversation with Claude, typed on the keyboard: with a stand-in for
                                     Claude's API on this computer (HTTPS, with a certificate authority made for
                                     the test, which the test build trusts instead of Mozilla's): the request
                                     (model, effort, the key from the settings file, the conversation so far),
                                     the answer streamed onto the screen, and a failure shown
    ./check.py --secure-boot         Secure Boot, as on a PC that has approved Chat Computer's key: signs the
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
                 extra: tuple[str, ...] = (), settings: str | None = None):
        self.tmp = tempfile.TemporaryDirectory()
        tmp = Path(self.tmp.name)
        esp = tmp / "esp"
        (esp / "EFI/BOOT").mkdir(parents=True)
        shutil.copy(efi, esp / "EFI/BOOT/BOOTX64.EFI")
        if settings is not None:
            # Where the Windows setup puts it (own-os.ps1).
            (esp / "EFI/chatcomputer-own").mkdir(parents=True)
            (esp / "EFI/chatcomputer-own/settings.txt").write_text(settings)
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


def network(args) -> list[str]:
    """A test build with QEMU's virtual network (its DHCP and DNS, and 10.0.2.2 for this computer) and a virtio card:
    it must get 10.0.2.15, look up api.anthropic.com, and talk to a server here (which answers its line)."""
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
    build("network")
    problems = []
    try:
        extra = ("-netdev", "user,id=n0", "-device", "virtio-net-pci,netdev=n0,romfile=")
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra)
        try:
            got = wait_for(pc.log, ("test: the server said", "test: couldn't", "test: no answer", "never opened", "stopped"), args.timeout)
            time.sleep(1)
            pc.picture("network")
        finally:
            pc.stop()
        text = pc.text()
        print(text)
        if "network: virtio network card" not in text:
            problems.append("the virtio card wasn't started")
        if "network: address 10.0.2.15/24, gateway 10.0.2.2, DNS 10.0.2.3" not in text:
            problems.append("no address from DHCP")
        if "test: api.anthropic.com is at " not in text:
            problems.append("api.anthropic.com wasn't found with DNS")
        if heard != ["hello from Chat Computer"]:
            problems.append(f"the server heard {heard!r}")
        if got != "test: the server said" or "you said 'hello from Chat Computer'" not in text:
            problems.append("the server's answer didn't come back")
    finally:
        server.shutdown()
        build()  # never leave a test build in out/
    return problems


CLAUDE_PORT = 8472  # assistant.rs TEST_PORT
TEST_KEY = "test-key-for-the-stand-in"
ANSWER_WORDS = ["Hello from the stand-in", " for Claude's API: ", "your own kernel reached me", " over its own TLS."]
ANSWER = "".join(ANSWER_WORDS)


def answer_events() -> list[tuple[str, dict]]:
    """A streamed answer as Claude's API sends one (thinking first, which the screen doesn't show)."""
    text = [("content_block_delta", {"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": w}})
            for w in ANSWER_WORDS]
    return [
        ("message_start", {"type": "message_start", "message": {"id": "msg_test", "type": "message", "role": "assistant",
                                                                "model": "claude-opus-5-5", "content": [], "stop_reason": None}}),
        ("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
        ("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "Hm."}}),
        ("content_block_stop", {"type": "content_block_stop", "index": 0}),
        ("ping", {"type": "ping"}),
        ("content_block_start", {"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}),
        *text,
        ("content_block_stop", {"type": "content_block_stop", "index": 1}),
        ("message_delta", {"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 20}}),
        ("message_stop", {"type": "message_stop"}),
    ]


def claude(args) -> list[str]:
    """A conversation typed on the keyboard, with a stand-in for Claude's API on this computer (HTTPS on
    CLAUDE_PORT, as "localhost", with a certificate from an authority made for the test, which the test build trusts
    instead of Mozilla's). The first question is answered (streamed, a piece at a time); the second gets Claude's
    API's "overloaded" error, which the screen must show. The stand-in checks what it was sent."""
    import http.server
    import json
    import ssl
    import threading

    tmp = Path(tempfile.mkdtemp(prefix="cc-claude-"))

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

    requests: list[dict] = []

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
            if len(requests) == 1:
                self.send_response(200)
                self.send_header("content-type", "text/event-stream")
                self.send_header("transfer-encoding", "chunked")
                self.end_headers()
                for event, payload in answer_events():
                    chunk = f"event: {event}\ndata: {json.dumps(payload)}\n\n".encode()
                    self.wfile.write(b"%x\r\n%s\r\n" % (len(chunk), chunk))
                    self.wfile.flush()
                    time.sleep(0.25)
                self.wfile.write(b"0\r\n\r\n")
            else:
                error = json.dumps({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}).encode()
                self.send_response(529)
                self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(error)))
                self.end_headers()
                self.wfile.write(error)
            self.wfile.flush()

    server = http.server.ThreadingHTTPServer(("127.0.0.1", CLAUDE_PORT), Api)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_3
    context.load_cert_chain(tmp / "leaf.pem", tmp / "leaf.key")
    server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    build("claude", {"CHATCOMPUTER_TEST_CA": str(tmp / "ca.der")})
    problems: list[str] = []
    try:
        extra = ("-netdev", "user,id=n0", "-device", "virtio-net-pci,netdev=n0,romfile=")
        # As Chat Computer Own System writes it on Windows (New-OwnSettingsText): notes, Windows' line breaks.
        settings = ("# Chat Computer's settings, read by its own system when it starts. Written by Chat Computer Own System.\r\n"
                    "# The Claude API key is kept here as it is: anyone who can read this partition can read it.\r\n"
                    f"claude-api-key = {TEST_KEY}\r\nclock-offset-minutes = 0\r\n")
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra, settings=settings)
        first = second = None
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
            time.sleep(1.5)
            answered = pc.picture("claude-1")
            monitor_many(pc.mon, [f"sendkey {k}" for k in "again"] + ["sendkey ret"])
            second = wait_for(pc.log, ("Claude: failed", "stopped"), 60)
            time.sleep(1.5)
            pc.picture("claude-2")
        finally:
            pc.stop()
        text = pc.text()
        print(text)
        if "clock: " not in text or "(local time, offset from the settings)" not in text:
            problems.append("the clock offset in the settings wasn't used")
        if "Claude: connected (TLS 1.3" not in text:
            problems.append("no secure connection to the stand-in")
        if first != "test: Claude said" or f"test: Claude said {json.dumps(ANSWER)}" not in text:
            problems.append(f"the answer didn't come back whole (the log says {first!r})")
        if second != "Claude: failed" or "Claude is very busy right now (Overloaded)" not in text:
            problems.append("the second question's failure wasn't reported")
        if TEST_KEY in text:
            problems.append("the API key is in the log")
        if len(requests) != 2:
            problems.append(f"the stand-in got {len(requests)} requests, not 2")
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
            later = (requests[1]["body"] or {}).get("messages")
            if later != [{"role": "user", "content": "hi"}, {"role": "assistant", "content": ANSWER},
                         {"role": "user", "content": "again"}]:
                problems.append(f"the second request's messages were {later!r}")
        if first == "test: Claude said":
            problems += check_colours(answered, args.size)
            if colours(answered).get(BUBBLE, 0) < 1000:
                problems.append("no bubble for what was typed")
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
    ap.add_argument("--network", action="store_true", help="a virtio network card: DHCP, DNS and a TCP conversation")
    ap.add_argument("--claude", action="store_true", help="a conversation with a stand-in for Claude's API, over HTTPS")
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
