#!/usr/bin/env python3
"""Starts Chat Computer in a virtual PC (QEMU with OVMF, the open-source UEFI firmware) without a window,
waits until the kernel says it is ready, takes two pictures of the screen a few seconds apart, and
checks them: the screen is in Chat Computer's colours (light words on its dark, its message box), and the
"Up for" line moved on (so the kernel's own timer interrupts work). Pictures and the kernel's
log go to out/.

    ./check.py                       build, start, check
    ./check.py --size 2560x1440 --memory 1024 --cpu qemu64
    ./check.py --keyboard            keyboards: PS/2; USB on a port and behind a hub, with MSI-X, MSI and
                                     no interrupts, and with Intel's and AMD's IOMMUs present; typing
                                     (Shift, Backspace, Caps Lock) reaches the screen; a keyboard plugged
                                     in later, then unplugged; a hub unplugged; USB mice (QEMU's mouse
                                     and tablet), their wheels' turns reaching the screen, read from their
                                     report descriptors (and by the boot protocol in a build told to); a
                                     smart card reader left alone
    ./check.py --crash page-fault    a kernel that faults on purpose once ready: it must stop cleanly,
                                     with the fault in its log (the serial port's, and the one kept on its start-up
                                     partition for Windows to read back) and a message on the screen (also: panic; stack,
                                     one that calls itself until its stack runs out, which must be said; and core,
                                     a processor core other than the first panicking in the middle of the model's
                                     arithmetic, which the first must then show)
    ./check.py --cores 4             any check with a virtual PC of that many processor cores (2 if not said), every
                                     one of which the kernel must start and use
    ./check.py --network             the network, with each kind of card there's a driver for (virtio, Intel's
                                     e1000e, e1000 and igb, a USB network adapter by CDC ECM, plugged in once the
                                     kernel is running and then unplugged, and by RNDIS): an address from DHCP, a
                                     name looked up with DNS, and a TCP conversation with a server on this computer
                                     (--card igb: only that one)
    ./check.py --model               Chat Computer's own model: a tiny one (random weights) found where Chat
                                     Computer's app keeps its models, read into memory, its instructions read
                                     ahead, a question typed and answered, the answer the same as the engine's on
                                     Linux; then one with a vision part shown a picture and a scanned PDF's page by
                                     read_file, the pictures and its answers the same as the engine's on Linux; then
                                     named by the settings; then a file that isn't a model
    ./check.py --chat                a conversation typed on the keyboard, answered by a test's script in place of
                                     the model: what the model's given each round (the conversation so far, the
                                     tools' results, when each message was sent), its tools (the computer's
                                     status, a bigger text size, a timer, a restart, which ends the virtual PC),
                                     the answer on the screen (Markdown laid out), a failure shown, and the
                                     conversation scrolled back and forth by a USB mouse's wheel; then an answer's
                                     limits: the tools the model's given (tools.json's), its tools used up (what
                                     the eighth round's said tells it so; the ninth round's call not run, its words
                                     the answer, the question kept), an answer cut short at the most a round says,
                                     and a tool call cut off as it was written (not run, the model told why)
    ./check.py --web                the model's web tools (search_web, read_web_page) over the kernel's own TCP and TLS
                                     1.3, against a stand-in here for DuckDuckGo and a site (a test build trusting the
                                     test's own authority): results read, a redirect followed, a page compressed and in
                                     chunks read in two parts, a PDF read as its words, a missing page and a wrong
                                     certificate said
    ./check.py --remember           the conversation kept across a restart: started from a disk laid out like a
                                     PC's (a FAT32 start-up partition with Windows' files and Chat Computer's,
                                     its memory file as Windows' setup makes it), on an NVMe SSD and then a SATA
                                     drive; a question answered, the PC restarted, the conversation back on the
                                     screen and given with the next question, then a new conversation started (the
                                     next question given alone; a note the model made before the restart still in
                                     its instructions), then the PC turned off (ACPI's S5); the memory
                                     file's two copies, and nothing else on the disk changed (fsck.vfat too, so also
                                     needs dosfstools, mtools and gdisk)
    ./check.py --files               Windows' drive read: a disk like a PC's, with Windows' NTFS partition after the
                                     start-up one (the ntfs crate's test volume), and a test's script in place of
                                     the model that lists a folder, reads files (UTF-8 and UTF-16, Office's files and a
                                     PDF as their words, a damaged Word file it can't, a long one in two parts) and
                                     finds one by name (needs xz and sgdisk)
    ./check.py --bitlocker           Windows' drive encrypted with BitLocker (the same volume, encrypted as Windows
                                     10 and 11 do): locked, then unlocked by its recovery key typed in the message
                                     box (a wrong one refused), the key reaching neither the model nor the disk; and
                                     read at once when BitLocker keeps its key in the clear (also needs
                                     python3-cryptography)
    ./check.py --drives              the PC's other drives: a second NVMe SSD (GPT), a SATA drive partitioned the old
                                     way (MBR) and another (GPT), each with an NTFS volume, read by the letters
                                     Windows' registry gives them (one Windows hasn't seen given the next free one;
                                     also needs ntfs-3g's mkntfs)
    ./check.py --usb                 USB drives, as they come and go: a FAT32 stick Windows' registry names, a card
                                     reader with two slots (a FAT16 card with no partition table; an exFAT card put
                                     in later), a stick of 4 KiB blocks with NTFS plugged in later; a stick unplugged
                                     and a card taken out; Chat Computer's own folder on a stick never read (also
                                     needs dosfstools and mtools)
    ./check.py --power               the firmware's own programs (ACPI's AML) run by the kernel's interpreter: a battery,
                                     an AC adapter and a lid added to the virtual PC's firmware tables (aml/tests/
                                     power.asl), read at start-up and followed as they change (unplugged, the battery
                                     low, the lid closed: the notes said); a general-purpose event (a processor
                                     plugged in) handled by the firmware's own method; the power button pressed: the
                                     PC turns off, with the firmware's _TTS and _PTS run first (also needs iasl, from
                                     acpica-tools)
    ./check.py --sound               sound: a build that rings the timer's chime once it's ready, on each kind of
                                     HD Audio QEMU has (the ICH6's and ICH9's controllers; codecs with a line out,
                                     a line out and in, and a speaker and microphone), and on QEMU's USB sound card
                                     (alone, and beside HD Audio, which it then plays instead of), recorded: the
                                     chime's two notes, in order, in both channels, as loud as the main Chat
                                     Computer's (on the USB card, as its own volume makes it)
    ./check.py --microphone          microphones: a build that listens once it's ready, on each kind of HD Audio
                                     QEMU has with an input (the ICH6's and ICH9's controllers; codecs with a line
                                     in, and with a microphone), each fed a tone of its own through QEMU's audio
                                     (SDL's, reading it from a file): heard at its frequency and its loudness; and
                                     a codec without one, which says so
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
# The virtual PCs' processor cores (--cores), unless a check says otherwise.
CORES = 2


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


def monitor_ask(sock_path: Path, command: str) -> str:
    """Sends a monitor command and returns what the monitor answered."""
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.connect(str(sock_path))
        s.settimeout(0.5)
        time.sleep(0.2)
        try:
            s.recv(65536)  # the banner
        except TimeoutError:
            pass
        s.sendall(command.encode() + b"\n")
        time.sleep(0.5)
        answer = b""
        try:
            while chunk := s.recv(65536):
                answer += chunk
        except TimeoutError:
            pass
    return answer.decode(errors="replace")


def use_mouse(sock_path: Path, name: str) -> bool:
    """Makes the mouse QEMU's "info mice" lists as `name` the one the monitor's mouse commands move. Returns whether
    there is one."""
    import re

    m = re.search(rf"Mouse #(\d+): {re.escape(name)}", monitor_ask(sock_path, "info mice"))
    if m:
        monitor(sock_path, f"mouse_set {m.group(1)}")
    return m is not None


def turn_wheel(sock_path: Path, notches: list[int]) -> None:
    """Turns the mouse's wheel a notch at a time: away from you for 1, towards you for -1."""
    for n in notches:
        monitor(sock_path, f"mouse_move 0 0 {n}")


def wait_for(log: Path, words: tuple[str, ...], timeout: float, times: int = 1) -> str | None:
    """Waits until one of `words` is in the log (`times` times), and says which."""
    end = time.time() + timeout
    while time.time() < end:
        text = log.read_text(errors="replace") if log.exists() else ""
        for w in words:
            if text.count(w) >= times:
                return w
        time.sleep(0.5)
    return None


class VirtualPC:
    """One run of the virtual PC, started from `efi` as \\EFI\\BOOT\\BOOTX64.EFI."""

    def __init__(self, efi: Path, *, size: str, memory: str, cpu: str, secure_vars: Path | None = None,
                 extra: tuple[str, ...] = (), settings: str | None = None, disk: tuple[Path, str] | None = None,
                 env: dict[str, str] | None = None):
        """`disk`: start from this disk image instead (on an NVMe or SATA drive: "nvme" or "sata"). `env`: more of
        QEMU's environment (for its audio's SDL, say)."""
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
            drive = ("-drive", f"if=none,id=d0,format=raw,file={disk[0]}", "-device", "nvme,serial=ccmemory1,drive=d0,bootindex=0")
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
            *(() if "-smp" in extra else ("-smp", str(CORES))),
            *extra,
        ], env={**os.environ, **(env or {})})

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


# Chat Computer's colours (theme.css): the background, the words, your messages' bubbles, the message box.
GROUND, INK, BUBBLE, PAPER = (25, 25, 25), (240, 240, 240), (37, 37, 37), (48, 48, 48)


def colours(image) -> dict:
    return {c: n for n, c in (image.getcolors(maxcolors=1 << 20) or [])}


def check_colours(a, size: str) -> list[str]:
    """`a` is the size asked for, and in Chat Computer's colours: mostly its background, light text, its message
    box, and nothing that isn't a mix of its greys (each of which has as much red as green, and as much green as
    blue, or more)."""
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
    # (Light pixels, not only the words' white itself: Lora's strokes are thin, so two short lines have few pixels
    # wholly inked, and the rest of each stroke is lighter grey than anything but words on this screen.)
    if sum(n for c, n in seen.items() if min(c) >= 200) < 500:
        problems.append("almost no words' white: is there any text?")
    if seen.get(PAPER, 0) < total * 0.01:
        problems.append("no message box (hardly any of its grey)")
    return problems


def check_screen(a, b, size: str) -> list[str]:
    from PIL import ImageChops

    problems = check_colours(a, size)
    if ImageChops.difference(a, b).getbbox() is None:
        problems.append("the screen didn't change in 3 seconds: are timer interrupts working?")
    return problems


def stack_problems(log: str) -> list[str]:
    """The kernel's stack (main.rs says in its log how deep it has gone, measured): at most half of it used, so that
    something going much deeper is seen long before the stack runs out."""
    import re

    used = re.findall(r"its stack: (\d+) KiB of (\d+) KiB used at most", log)
    if not used:
        return ["the kernel didn't say how deep its stack has gone"]
    deepest, size = (int(n) for n in used[-1])
    return [] if deepest * 2 <= size else [f"the kernel's stack went {deepest} KiB deep: more than half of its {size} KiB"]


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
    problems = check_screen(a, b, args.size) + stack_problems(text) + cores_problems(text)
    if expect_secure and "Chat Computer kernel: Secure Boot on" not in text:
        problems.append("the kernel didn't see Secure Boot on")
    return problems


def cores_problems(log: str) -> list[str]:
    """Every processor core of the virtual PC started and used."""
    want = f"processor cores: {CORES} in use"
    return [] if want in log else [f"the kernel didn't say {want!r}"]


def crash(kind: str, args) -> list[str]:
    """A kernel that stops on purpose once ready, started from a disk like a PC's (memory_disk, with a log file): it
    must stop cleanly, saying why in its log and on the screen, and the log kept on the start-up partition (the one
    Windows' setup reads back) must say why too."""
    global CORES
    expected = {"page-fault": "Chat Computer kernel stopped: Page fault", "panic": "Chat Computer kernel stopped: panicked",
                "stack": "Chat Computer kernel stopped: The kernel's stack ran out",
                "core": "processor core 1 stopped: panicked at"}[kind]
    if kind == "core":
        CORES = max(CORES, 2)
    tmp = Path(tempfile.mkdtemp(prefix="cc-crash-"))
    build(kind)
    try:
        disk = tmp / "crash.img"
        settings = SETTINGS_HEAD + f"clock-offset-minutes = 0\r\nmemory-id = {MEMORY_ID:08x}\r\nlog-id = {LOG_ID:08x}\r\n"
        files = memory_disk(disk, BUILT, settings, MEMORY_ID)
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, disk=(disk, "nvme"))
        try:
            got = wait_for(pc.log, ("Chat Computer kernel stopped",), args.timeout)
            time.sleep(3)
            pc.picture(f"screen-{kind}")
        finally:
            pc.stop()
        fs_problems, _ = esp_now(disk, files)
    finally:
        build()  # never leave a kernel that crashes on purpose in out/
        shutil.rmtree(tmp, ignore_errors=True)
    text = pc.text()
    print(text)
    problems = [] if got and expected in text else [f"expected {expected!r} in the log"]
    if kind == "core" and "Chat Computer kernel stopped: panicked" not in text:
        problems.append("the first core didn't stop after the other did")
    problems += fs_problems
    try:
        kept = kept_in(LAST_LOG[0], LOG_ID)
    except ValueError as e:
        kept = None
        problems.append(f"the log file: {e}")
    kept_text = kept[1].decode(errors="replace") if kept else ""
    if expected not in kept_text or "Chat Computer: ready" not in kept_text:
        problems.append(f"the log kept on the start-up partition doesn't say why it stopped ({len(kept_text)} bytes kept)")
    return problems


# What `keyboard` types (QEMU key names), and the line that should come out: Shift, Backspace and Caps
# Lock included.
KEYS = ["shift-h", "i", "shift-1", "spc", "c", "x", "backspace", "l", "a", "u", "d", "e", "spc",
        "caps_lock", "o", "k", "caps_lock"]
TYPED = "Hi! claude OK"


def keyboard_run(label: str, controller: str, args, plug: bool, boot_mice: bool = False) -> list[str]:
    """One start with a USB controller (`controller`: QEMU's device and options), a keyboard on one of its ports, a
    tablet on another, a hub with a keyboard, a mouse and a smart card reader (which Chat Computer doesn't use yet and
    must leave alone) behind it. QEMU types on the keyboard made last: the one behind the hub. `boot_mice`: the build
    reads mice by the boot protocol, which QEMU's tablet doesn't offer."""
    print(f"--- USB keyboards and mice, interrupts: {label}")
    extra = ("-device", f"{controller},id=xhci",
             "-device", "usb-kbd,bus=xhci.0,port=1,id=kbd1",
             "-device", "usb-tablet,bus=xhci.0,port=3",
             "-device", "usb-hub,bus=xhci.0,port=2,id=hub",
             "-device", "usb-mouse,bus=xhci.0,port=2.2",
             "-device", "usb-ccid,bus=xhci.0,port=2.3",
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
        # QEMU's mouse: five buttons by its report descriptor, the boot protocol's three by that.
        mouse = f"port 6.2: a mouse ({3 if boot_mice else 5} buttons, a wheel), \"QEMU USB Mouse\""
        if mouse not in text:
            problems.append(f"{label}: the mouse behind the hub wasn't found as {mouse!r}")
        tablet = "port 7: " + ("a device Chat Computer doesn't use yet" if boot_mice else "a mouse (3 buttons, a wheel)")
        if f"{tablet}, \"QEMU USB Tablet\"" not in text:
            problems.append(f"{label}: the tablet wasn't found as {tablet!r}")
        if "port 6.3: a device Chat Computer doesn't use yet" not in text:
            problems.append(f"{label}: the smart card reader wasn't found and left alone")
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
        # The mouse's wheel turned away from you twice and back once, then the tablet's back once.
        import re
        if not use_mouse(pc.mon, "QEMU HID Mouse"):
            problems.append(f"{label}: QEMU has no USB mouse to turn the wheel of")
        turn_wheel(pc.mon, [1, 1, -1])
        wanted = [1, 1, -1]
        if not boot_mice and use_mouse(pc.mon, "QEMU HID Tablet"):
            turn_wheel(pc.mon, [-1])
            wanted.append(-1)
        time.sleep(1)
        turns = [int(t) for t in re.findall(r"test: wheel (-?\d+)", pc.text())]
        if turns != wanted:
            problems.append(f"{label}: the wheels' turns came out as {turns}, not {wanted}")
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
            elif "port 6.1: \"QEMU USB Keyboard\" was unplugged" not in pc.text() \
                    or "port 6.2: \"QEMU USB Mouse\" was unplugged" not in pc.text():
                problems.append(f"{label}: the keyboard and mouse behind the unplugged hub weren't let go")
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
        # A build that reads mice by the boot protocol, as it reads a mouse whose report descriptor can't be used.
        build("keyboard", {"CHATCOMPUTER_TEST_USB_MOUSE": "boot"})
        problems += keyboard_run("none", "nec-usb-xhci,msix=off,msi=off", args, plug=False, boot_mice=True)
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


# A Markdown answer, as models write them: a heading, bold and italic, a list, a table, a quote and code, which the
# screen lays out (its picture: chat-markdown.png); with characters the text and code fonts lack, from the fallback
# fonts: an arrow, a check mark, Greek and maths, emoji (one with the "as an emoji" selector, which shows nothing
# itself), box drawing, and Chinese, Japanese and Korean; and accents typed after their letters.
MARKDOWN = ("## Today\n\n**Sunny** all day, with *light* wind and ~~no~~ little cloud:\n\n- Morning: `12 °C`\n"
            "- Afternoon: 18 °C → 20 °C ✓\n\n| Time | Sky | Wind |\n|---|---|---|\n| 9:00 | Clear ☀\ufe0f | 5 km/h |\n"
            "| 15:00 | **Sunny** | 10 km/h (Δ ≈ 5) |\n\n> Enjoy the day. 🙂\n\n"
            # (Chinese, Japanese and Korean, wrapped between their characters, bold made up.)
            "今天**天气晴朗**，最高气温二十度，适合出门散步。午後は少し風が強くなりますが、雨は降りません。"
            "오후에는 바람이 조금 불지만 비는 오지 않습니다.\n\n"
            # (Accents typed after their letters, as Macs name files: composed, or placed by the font's anchors.)
            "Cafe\u0301 au lait, Tie\u0302\u0301ng Vie\u0323\u0302t, n\u0303\u0323, y\u030a\n\n"
            "```\nforecast --today\n└─ sunny, 20 °C\n```")
# The settings' first line, as Chat Computer Own System writes them on Windows (New-OwnSettingsText): Windows' line
# breaks.
SETTINGS_HEAD = "# Chat Computer's settings, read by its own system when it starts. Written by Chat Computer Own System.\r\n"
# The note each message the model's given starts with: when it was sent ("[Tue 3 Nov 2026 19:02] ").
NOTE_TIME = r"\[(Mon|Tue|Wed|Thu|Fri|Sat|Sun) \d{1,2} (Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) \d{4} \d\d:\d\d\] "


def script_build(script: dict, tmp: Path) -> None:
    """A test build that answers from `script` in place of a model (assistant.rs): for each question (or "else"), the
    rounds of its answer, each {"text": words, "calls": [[tool, {arguments}], ...]} or {"fail": why}."""
    path = tmp / "script.json"
    path.write_text(json.dumps(script, ensure_ascii=False))
    build("chat", {"CHATCOMPUTER_TEST_SCRIPT": str(path)})


def tool_script(calls: dict) -> dict:
    """A script that answers each question by calling its one tool ((name, arguments)), then saying "Done."."""
    return {q: [{"calls": [[name, inp]]}] for q, (name, inp) in calls.items()}


def logged(text: str, what: str) -> list:
    """What a test build logged as `what` ("asked", "tool", "round" or "said"), in order, each read as JSON."""
    mark = f"kernel: test: {what} "
    return [json.loads(line.split(mark, 1)[1]) for line in text.splitlines() if mark in line]


def tool_results(text: str, questions: list[str]) -> dict[str, dict]:
    """What each question's tool said ({"name", "error", "said"}), when each question calls one tool, asked one after
    another."""
    return dict(zip(questions, logged(text, "tool")))


def wait_answered(pc: "VirtualPC", times: int, timeout: float = 60) -> None:
    """Until a test's script has answered `times` questions with "Done."."""
    end = time.time() + timeout
    while time.time() < end and pc.text().count("model: answered (5 characters") < times:
        time.sleep(0.5)


def plain(messages: list[dict]) -> list[dict]:
    """The messages the model was given, the note of when each of the person's was sent taken off."""
    import re

    return [dict(m, content=re.sub("^" + NOTE_TIME, "", m["content"])) if m.get("role") == "user" else m for m in messages]


def chat(args) -> list[str]:
    """A conversation typed on the keyboard, answered by a test build's script in place of the model (assistant.rs), so
    all that's around the model is checked: what it's given each round, the tools it calls, the answer on the screen.
    The first question is answered with tools: the script has the model look at the computer's status, then make the
    text bigger and set a timer (two tools at once), then answer (a few characters at a time); each round must be
    given the conversation so far, the answers and what the tools said, the same instructions every time, and each of
    the person's messages with when it was sent. The second question fails, which the screen must show (and which
    must be taken back out of the conversation). The third is answered in Markdown, laid out on the screen; then a USB
    mouse's wheel scrolls the conversation back a notch and forward again. The fourth asks to restart: the model's
    answer, then a restart."""
    import re

    from PIL import ImageChops

    tmp = Path(tempfile.mkdtemp(prefix="cc-chat-"))
    final = "I see this computer, and the text is bigger now."
    script = {
        "hi": [{"text": "Let me look.", "calls": [["computer_status", {}]]},
               # (Two tools at once: the text bigger, and a timer, which rings long after the test.)
               {"calls": [["set_text_size", {"percent": 125}], ["reminders", {"action": "timer", "minutes": 30, "label": "the oven"}]]},
               {"text": final}],
        "again": [{"fail": "Chat Computer's model stopped (a test of a failure)."}],
        "weather": [{"text": "It's sunny.\n\n" + MARKDOWN}],
        "restart": [{"calls": [["restart_computer", {}]]}, {"text": "Restarting now: see you in Windows."}],
    }
    script_build(script, tmp)
    problems: list[str] = []
    try:
        # A USB mouse, to scroll with.
        extra = ("-device", "qemu-xhci,id=xhci", "-device", "usb-mouse,bus=xhci.0")
        settings = SETTINGS_HEAD + "clock-offset-minutes = 0\r\n"
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra, settings=settings)
        first = second = third = searched = None
        answered = None
        restarted = ended = False
        scrolled = []
        try:
            if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
                print(pc.text())
                return ["the kernel didn't say it was ready"]
            time.sleep(1)
            pc.picture("chat-0")
            monitor_many(pc.mon, ["sendkey h", "sendkey i", "sendkey ret"])
            first = wait_for(pc.log, ("test: said", "model: failed", "stopped"), args.timeout)
            answered = pc.settled_picture("chat-1")
            monitor_many(pc.mon, keys_for("again") + ["sendkey ret"])
            second = wait_for(pc.log, ("model: failed", "stopped"), 60)
            pc.settled_picture("chat-2")
            monitor_many(pc.mon, keys_for("weather") + ["sendkey ret"])
            searched = wait_for(pc.log, ("Enjoy the day",), 60)
            at_end = pc.settled_picture("chat-markdown")
            # The wheel a notch away from you: the conversation scrolls back; a notch towards you: forward again.
            if use_mouse(pc.mon, "QEMU HID Mouse"):
                turn_wheel(pc.mon, [1])
                back = pc.settled_picture("chat-scrolled")
                turn_wheel(pc.mon, [-1])
                scrolled = [at_end, back, pc.settled_picture("chat-scrolled-forward")]
            # Asked to restart: the model's answer, then a restart (which ends a virtual PC started with -no-reboot).
            monitor_many(pc.mon, keys_for("restart") + ["sendkey ret"])
            third = wait_for(pc.log, ("Restarting now", "model: failed", "stopped"), 60)
            pc.settled_picture("chat-3")
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
        problems += stack_problems(text)
        said = logged(text, "said")
        if "clock: " not in text or "(local time, offset from the settings)" not in text:
            problems.append("the clock offset in the settings wasn't used")
        if first != "test: said" or said[:1] != ["Let me look.\n\n" + final]:
            problems.append(f"the first answer didn't come whole (the log says {first!r}: {said[:1]!r})")
        if "model: uses computer_status" not in text or "model: uses set_text_size" not in text:
            problems.append("the tools weren't used")
        if "text size 125%" not in text:
            problems.append("the text wasn't made bigger")
        if second != "model: failed" or "model: failed: Chat Computer's model stopped (a test of a failure)." not in text:
            problems.append("the second question's failure wasn't reported")
        if third is None or "model: uses restart_computer" not in text:
            problems.append("asking to restart didn't reach the restart tool")
        elif not restarted or not ended:
            problems.append(f"the PC didn't restart (the kernel said so: {restarted}; the virtual PC ended: {ended})")
        if searched != "Enjoy the day" or said[1:2] != ["It's sunny.\n\n" + MARKDOWN]:
            problems.append(f"the Markdown answer didn't come whole (the log says {searched!r})")
        if not scrolled:
            problems.append("QEMU has no USB mouse to scroll with")
        elif [ImageChops.difference(scrolled[0], b).getbbox() is None for b in scrolled[1:]] != [False, True]:
            problems.append("the mouse's wheel didn't scroll the conversation back, then forward to where it was")
        # What the model was given, round by round: the same instructions, then the conversation so far.
        rounds = logged(text, "asked")
        if len(rounds) != 7:
            problems.append(f"the model was given {len(rounds)} rounds, not 7: {rounds!r}"[:2000])
            return problems
        system = rounds[0][0]
        if any(r[0] != system for r in rounds) or system.get("role") != "system":
            problems.append("the instructions changed from one round to the next")
        for words in ("You are Chat Computer", "nothing they say leaves the computer", "The screen shows Markdown",
                      "like [Tue 3 Nov 2026 19:02]"):
            if words not in str(system.get("content")):
                problems.append(f"the instructions don't say {words!r}")
        convo = [r[1:] for r in rounds]
        for r, question in ((0, "hi"), (3, "again"), (4, "weather"), (5, "restart")):
            if not re.fullmatch(NOTE_TIME + question, convo[r][-1].get("content", "")):
                problems.append(f"{question!r} wasn't given with when it was sent: {convo[r][-1]!r}")
        convo = [plain(c) for c in convo]
        user = lambda t: {"role": "user", "content": t}  # noqa: E731
        answer = lambda t, *calls: {"role": "assistant", "content": t, "calls": list(calls)}  # noqa: E731
        status = convo[1][2]["content"] if len(convo[1]) == 3 else ""
        if convo[0] != [user("hi")]:
            problems.append(f"the first question was given as {convo[0]!r}")
        if convo[1][:2] != [user("hi"), answer("Let me look.", {"name": "computer_status", "args": {}})] \
                or convo[1][2:3] != [{"role": "tool", "content": status}] \
                or any(w not in status for w in ("Processor: ", "Memory: ", "Sound: none: no HD Audio controller",
                                                 "Timers and reminders: none set", "Model: a test's script")):
            problems.append(f"the computer's status didn't come back to the model: {convo[1]!r}"[:2000])
        both = answer("", {"name": "set_text_size", "args": {"percent": "125"}},
                      {"name": "reminders", "args": {"action": "timer", "minutes": "30", "label": "the oven"}})
        results = [m.get("content", "") for m in convo[2][4:]]
        if convo[2][:4] != convo[1] + [both] or len(results) != 2 or [m.get("role") for m in convo[2][4:]] != ["tool", "tool"] \
                or not results[0].startswith("The text is now 125%") \
                or not results[1].startswith("Set a 30-minute timer for the oven (rings in 30 minutes)."):
            problems.append(f"the two tools' results didn't come back as they should: {convo[2][3:]!r}")
        if convo[3] != convo[2] + [answer(final), user("again")]:
            problems.append(f"the second question was given as {convo[3][5:]!r}")
        # (The question that failed is taken back out.)
        if convo[4] != convo[3][:-1] + [user("weather")]:
            problems.append(f"the third question was given as {convo[4][5:]!r}")
        if convo[5] != convo[4] + [answer("It's sunny.\n\n" + MARKDOWN), user("restart")]:
            problems.append(f"the fourth question was given as {convo[5][7:]!r}")
        if convo[6][:-1] != convo[5] + [answer("", {"name": "restart_computer", "args": {}})] or convo[6][-1].get("role") != "tool":
            problems.append(f"the restart's round was given as {convo[6][9:]!r}")
        if first == "test: said":
            problems += check_colours(answered, args.size)
            if colours(answered).get(BUBBLE, 0) < 1000:
                problems.append("no bubble for what was typed")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
        build()  # never leave a test build in out/
    return problems


def chat_limits(args) -> list[str]:
    """An answer's limits, with a test build's script in place of the model (assistant.rs). The tools the model's
    given, named at start-up, must be tools.json's (a test's script calls the tools itself, so it wouldn't notice the
    model having none). A question answered with tools in every round: what the eighth round's tools said must end by
    telling the model that was its last, and the ninth round's call isn't run (answered as not run), its words standing
    as the answer, with a line under it, the question kept rather than failed. An answer that reached the most a round
    says, with a line under it. A tool call cut off as it was written (its round reached the most a round says): not
    run, and the model told why. Then a question given the conversation with all of that in it."""
    import re

    tmp = Path(tempfile.mkdtemp(prefix="cc-limits-"))
    looking = {"text": "Looking.", "calls": [["computer_status", {}]]}
    script = {
        "busy": [looking] * 8 + [{"text": "This is what I found.", "calls": [["computer_status", {}]]}],
        "long": [{"text": "The first part of a long answer", "cut": True}],
        "note": [{"calls": [["remember", {"note": "The person likes"}]], "cut": True}, {"text": "Noted."}],
        "after": [{"text": "Done."}],
    }
    script_build(script, tmp)
    problems: list[str] = []
    try:
        settings = SETTINGS_HEAD + "clock-offset-minutes = 0\r\n"
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, settings=settings)
        try:
            if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
                print(pc.text())
                return ["the kernel didn't say it was ready"]
            for n, question in enumerate(script, 1):
                monitor_many(pc.mon, keys_for(question) + ["sendkey ret"])
                end = time.time() + 120
                while time.time() < end and pc.text().count("model: answered (") < n \
                        and not any(w in pc.text() for w in ("model: failed", "kernel stopped")):
                    time.sleep(0.5)
                pc.settled_picture(f"chat-limits-{question}")
        finally:
            pc.stop()
        text = pc.text()
        print(text)
        named = re.search(r"model: its tools: (.*)", text)
        tools = [t["function"]["name"] for t in json.loads((HERE / "kernel/src/tools.json").read_text())]
        if not named or named[1].split(", ") != tools:
            problems.append(f"the model wasn't given tools.json's tools: {named[1] if named else 'none named'}")
        if "model: failed" in text or text.count("model: answered (") != len(script):
            problems.append(f"{text.count('model: answered (')} of {len(script)} questions answered")
        if text.count("model: uses computer_status") != 8 or "its rounds of tools were used up: 1 more not run" not in text:
            problems.append(f"computer_status ran {text.count('model: uses computer_status')} times, not 8, the ninth not run")
        if "model: its answer reached the most a round says" not in text:
            problems.append("an answer cut short wasn't said to be")
        if "model: uses remember" in text or "its call of remember was cut off as it wrote it" not in text:
            problems.append("a call cut off as it was written was run, or not said to be")
        said = logged(text, "said")
        if said != ["\n\n".join(["Looking."] * 8 + ["This is what I found."]), "The first part of a long answer", "Noted.",
                    "Done."]:
            problems.append(f"the answers came as {said!r}"[:2000])
        # What the model was given: the last round's note only after the eighth round's tools; then, for the last
        # question, all of the conversation (the answer that used its tools up, the one cut short, the call cut off).
        rounds = [plain(r)[1:] for r in logged(text, "asked")]
        if len(rounds) != 13:
            problems.append(f"the model was given {len(rounds)} rounds, not 13")
            return problems
        last = "That was the last of the 8 rounds of tools you can use for one message"
        told = [i for i, r in enumerate(rounds) for m in r if m["role"] == "tool" and last in m["content"]]
        if told[:1] != [8] or rounds[8][-1]["role"] != "tool" or last not in rounds[8][-1]["content"]:
            problems.append(f"the model was told its tools' last round in rounds {sorted(set(told))}, not first in the ninth")
        user = lambda t: {"role": "user", "content": t}  # noqa: E731
        answer = lambda t, *calls: {"role": "assistant", "content": t, "calls": list(calls)}  # noqa: E731
        status = {"name": "computer_status", "args": {}}
        convo = rounds[12]
        roles = ["user"] + ["assistant", "tool"] * 9 + ["user", "assistant"] + ["user", "assistant", "tool", "assistant", "user"]
        if [m["role"] for m in convo] != roles:
            problems.append(f"the last question was given {[m['role'] for m in convo]}")
            return problems
        not_run = lambda m, words: m["role"] == "tool" and m["content"].startswith("Error: Not run: " + words)  # noqa: E731
        if convo[0] != user("busy") or any(convo[1 + 2 * k] != answer("Looking.", status) for k in range(8)) \
                or convo[17] != answer("This is what I found.", status) or not not_run(convo[18], "you had used all"):
            problems.append(f"the answer that used its tools up wasn't kept as it was: {convo[15:19]!r}"[:2000])
        if convo[19:21] != [user("long"), answer("The first part of a long answer")]:
            problems.append(f"the answer cut short wasn't kept: {convo[19:21]!r}")
        if convo[21:23] != [user("note"), answer("", {"name": "remember", "args": {"note": "The person likes"}})] \
                or not not_run(convo[23], "you reached the most you can write at once") \
                or convo[24:] != [answer("Noted."), user("after")]:
            problems.append(f"the call cut off wasn't answered as not run: {convo[21:]!r}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
        build()  # never leave a test build in out/
    return problems


# The kernel's names for the engine's arithmetic (assistant.rs), as the engine's kernel_check example takes them.
LEVELS = {"plain": "plain", "AVX2": "avx2", "AVX-VNNI": "avx-vnni", "AVX-512 VNNI": "avx-512-vnni"}
TINY = HERE.parent / "tests/data/tiny-model.ccm"


def model_check(args) -> list[str]:
    """Chat Computer's own model, run by the kernel: a tiny model of random weights (tests/data/tiny-model.ccm) where
    Chat Computer's app keeps its models on Windows (Users\\NAME\\AppData\\Local\\ai.chatcomputer.desktop\\models), on a
    second drive's FAT32 volume, found there with no setting naming it, read into memory and its instructions read
    ahead; then a question typed and answered (by a test build, with the model's likeliest tokens), the answer's first
    round the same as Chat Computer's engine says to what the kernel gave it, on Linux (os/model's kernel_check
    example, with the arithmetic the kernel used). Then started again with the settings naming the model's file (as
    Chat Computer Own System writes them), which must be read from there; and once more with a file that isn't a
    model, which the screen must say."""
    import re

    tmp = Path(tempfile.mkdtemp(prefix="cc-model-"))
    build("model")
    problems: list[str] = []
    try:
        models = "Users/Test/AppData/Local/ai.chatcomputer.desktop/models"
        # (FAT32 as a PC writes it: enough clusters, of a sector each, to be FAT32 by the count of them.)
        volume = fat_volume(tmp, "DATA", 64, 32, 1, {f"{models}/qwen3.5-tiny.ccm": TINY.read_bytes(),
                                                     "Documents/not a model.ccm": b"CCMODEL1 but not one"})
        gpt_disk(tmp / "data.img", volume)
        extra = ("-drive", f"if=none,id=d1,format=raw,file={tmp / 'data.img'}", "-device", "nvme,serial=ccdata1,drive=d1")
        settings = SETTINGS_HEAD + "clock-offset-minutes = 0\r\n"
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra, settings=settings)
        try:
            if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
                print(pc.text())
                return ["the kernel didn't say it was ready"]
            if not wait_for(pc.log, ("model: read its instructions ahead", "model: failed", "stopped"), args.timeout):
                problems.append("the model's instructions weren't read ahead")
            pc.settled_picture("model-ready")
            monitor_many(pc.mon, keys_for("hello") + ["sendkey ret"])
            if wait_for(pc.log, ("model: answered", "model: failed", "stopped"), args.timeout) != "model: answered":
                problems.append("the model didn't answer")
            pc.settled_picture("model-answer")
        finally:
            pc.stop()
        text = pc.text()
        print(text)
        problems += stack_problems(text) + cores_problems(text)
        if CORES > 1:
            # The other cores took part in the model's arithmetic (the answer, below, the same as on one).
            share = re.search(r"model: answered .*on (\d+) processor cores, the others doing (\d+)% of the jobs shared", text)
            if not share or int(share[1]) != CORES or int(share[2]) == 0:
                problems.append("the other processor cores didn't share the model's arithmetic")
        path = "D:\\Users\\Test\\AppData\\Local\\ai.chatcomputer.desktop\\models\\qwen3.5-tiny.ccm"
        if f"model: reading Qwen3.5 TINY (1 MB) from {path}" not in text:
            problems.append("the model's file wasn't found where Chat Computer's app keeps it")
        ready = re.search(r"model: Qwen3\.5 TINY ready \(1 MB read in [\d.]+ s\), conversations up to (\d+) tokens, its "
                          r"arithmetic (.+)", text)
        if not ready:
            problems.append("the model wasn't made ready")
        asked, rounds = logged(text, "asked"), logged(text, "round")
        if not asked or not rounds:
            problems.append("the model wasn't asked, or didn't answer")
        elif ready:
            given = plain(asked[0])
            if [m["role"] for m in given] != ["system", "user"] or given[1]["content"] != "hello" \
                    or "You are Chat Computer" not in given[0]["content"] or "(Qwen3.5 TINY, an open model" not in given[0]["content"]:
                problems.append(f"the model was given {given!r}"[:1000])
            # (A model test build gives the model no tools, and lets it say 24 tokens: assistant.rs.)
            (tmp / "asked.json").write_text(json.dumps(asked[0], ensure_ascii=False))
            (tmp / "tools.json").write_text("[]")
            engine = subprocess.run(["cargo", "run", "-q", "--release", "-p", "model", "--features", "std", "--example",
                                     "kernel_check", "--", str(TINY), str(tmp / "asked.json"), "--tools", str(tmp / "tools.json"),
                                     "--max", "24", "--level", LEVELS.get(ready[2], "plain")],
                                    cwd=HERE, capture_output=True, text=True)
            try:
                want = json.loads(engine.stdout)
            except ValueError:
                want = engine.stderr[-2000:]
            if rounds[0] != want:
                problems.append(f"the kernel's model said {rounds[0]!r}, the engine on Linux {want!r}"[:3000])
            else:
                print(f"The kernel's model said what the engine says ({len(rounds[0]['text'])} characters, "
                      f"{len(rounds[0]['calls'])} tool calls), with {ready[2]} arithmetic.")
        problems += seeing_check(args, tmp, settings, models)
        # Named by the settings: read from there; and a file that isn't a model.
        for named, want_words in ((path, "model: Qwen3.5 TINY ready"),
                                  ("D:\\Documents\\not a model.ccm", "isn't a model Chat Computer's engine can run")):
            pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra,
                           settings=settings + f"model = {named}\r\n")
            try:
                if wait_for(pc.log, (want_words, "stopped"), args.timeout) != want_words:
                    problems.append(f"with the settings naming {named}, the log didn't say {want_words!r}")
                if "not a model" in named:
                    pc.settled_picture("model-not-one")
            finally:
                pc.stop()
            if "settings: the model's file named" not in pc.text():
                problems.append("the settings' model wasn't read")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
        build()  # never leave a test build in out/
    return problems


SEEING = HERE.parent / "tests/data/tiny-seeing-model.ccm"


def seeing_check(args, tmp: Path, settings: str, models: str) -> list[str]:
    """The model sees: a tiny model with a vision part (tests/data/tiny-seeing-model.ccm), given by read_file a
    picture (a PNG drawn here) and a scanned PDF's page (fax's Group 3, os/pdf's test file), each asked for by a model
    test build's "look at" (in place of a model of random weights asking; each in a conversation of its own): each
    picture made as the engine on Linux makes it from the same file (its size and mark the same), looked at by the
    vision part a step at a time, and the model's answer the same as the engine's on Linux to the same conversation
    with the same picture."""
    import re
    from PIL import Image, ImageDraw

    problems: list[str] = []
    sun = Image.new("RGB", (120, 90), (40, 110, 200))
    d = ImageDraw.Draw(sun)
    d.ellipse((20, 15, 60, 55), fill=(250, 220, 90))
    d.rectangle((70, 50, 115, 85), fill=(30, 140, 60))
    sun.save(tmp / "sun.png")
    scan = HERE / "pdf/tests/scan-fax.pdf"
    volume = fat_volume(tmp, "DATA", 64, 32, 1, {f"{models}/qwen3.5-tiny.ccm": SEEING.read_bytes(),
                                                 "Pictures/Sun.png": (tmp / "sun.png").read_bytes(),
                                                 "Documents/Scan.pdf": scan.read_bytes()})
    gpt_disk(tmp / "seeing.img", volume)
    extra = ("-drive", f"if=none,id=d1,format=raw,file={tmp / 'seeing.img'}", "-device", "nvme,serial=ccdata1,drive=d1")
    questions = ["look at D:\\Pictures\\Sun.png", "look at D:\\Documents\\Scan.pdf page 2"]
    pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra, settings=settings)
    try:
        if not wait_for(pc.log, ("model: read its instructions ahead", "model: failed", "stopped"), args.timeout):
            return ["the seeing model's instructions weren't read ahead"]
        for n, q in enumerate(questions, 1):
            monitor_many(pc.mon, keys_for(q) + ["sendkey ret"])
            if wait_for(pc.log, ("model: answered", "model: failed", "stopped"), args.timeout, times=n) != "model: answered":
                problems.append(f"the model didn't answer {q!r}")
                break
        pc.settled_picture("model-seeing")
    finally:
        pc.stop()
    text = pc.text()
    problems += stack_problems(text)
    if problems:
        print(text)
        return problems
    if "seeing pictures" not in text and "you see it" not in text:
        problems.append("the seeing model's file wasn't read as one that sees")
    tools, rounds, asked = logged(text, "tool"), logged(text, "round"), logged(text, "asked")
    pictures = [t for t in tools if t.get("picture")]
    want = [(r"D:\\Pictures\\Sun\.png \(\d+ (bytes|KB), 120×90 pixels, changed .*\): the picture is here \(as you see it, "
             r"72×48 pixels\)\.", "72x48"),
            (r"D:\\Documents\\Scan\.pdf \(a PDF, 3 pages, \d+ KB, changed .*\) has no words that can be read: its pages are pictures "
             r"\(scanned, perhaps\)\. Page 2's pictures are here, where they are on the page \(as you see it, 56×72 pixels\)\. "
             r"read_file with page 3 shows the next\.", "56x72")]
    if len(pictures) != 2:
        return problems + [f"read_file gave the model {len(pictures)} pictures, not 2: {tools!r}"[:2000]]
    for t, (pattern, size) in zip(pictures, want):
        if not re.fullmatch(pattern, t["said"]) or not t["picture"].startswith(size + " "):
            problems.append(f"read_file said {t['said']!r} with the picture {t['picture']!r}")
    if not re.search(r"model: answered .*; a picture seen in [\d.]+ s", text):
        problems.append("the log didn't say a picture was seen")
    # The model's rounds (each after the round that's "look at"'s call), and what it was asked for each (the last
    # "asked" before it).
    ours, given, last, after_call = [], [], None, False
    for line in text.splitlines():
        if "kernel: test: asked " in line:
            last = json.loads(line.split("kernel: test: asked ", 1)[1])
        elif "kernel: test: round " in line:
            r = json.loads(line.split("kernel: test: round ", 1)[1])
            if after_call:
                ours.append(r)
                given.append(last)
            after_call = not r["text"] and [c["name"] for c in r["calls"]] == ["read_file"]
    if len(ours) != 2 or None in given:
        return problems + [f"the model's rounds weren't found: {rounds!r}"[:2000]]
    asked = given
    level = re.search(r"its arithmetic (.+)", text)
    files = [str(tmp / "sun.png"), f"{scan}#2"]
    for k in range(2):
        (tmp / "asked.json").write_text(json.dumps(asked[k], ensure_ascii=False))
        (tmp / "tools.json").write_text("[]")
        engine = subprocess.run(["cargo", "run", "-q", "--release", "-p", "model", "--features", "std", "--example",
                                 "kernel_check", "--", str(SEEING), str(tmp / "asked.json"), "--tools", str(tmp / "tools.json"),
                                 "--max", "24", "--level", LEVELS.get(level[1] if level else "", "plain")]
                                + ["--picture", files[k]],
                                cwd=HERE, capture_output=True, text=True)
        try:
            theirs = json.loads(engine.stdout)
        except ValueError:
            theirs = engine.stderr[-2000:]
        if ours[k] != theirs:
            problems.append(f"seeing {questions[k]!r}, the kernel's model said {ours[k]!r}, the engine on Linux {theirs!r}"[:3000])
        else:
            print(f"Seeing {questions[k]!r}, the kernel's model said what the engine says ({len(ours[k]['text'])} characters).")
    return problems


WEB_PORT = 8473
# The stand-in's pages: DuckDuckGo's plain results page (its form: see os/web/src/search.rs), a page that's sent on
# (302) to the page, the page (long enough to be read in two parts), and a page that isn't there.
WEB_RESULTS = """<html><body><div class="result results_links results_links_deep web-result">
<h2 class="result__title"><a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.example.com%2Fstart&amp;rut=1">Chat Computer &amp; its <b>own</b> system</a></h2>
<a class="result__snippet" href="//duckduckgo.com/l/?uddg=x">A computer you talk to, with an operating system of its own.</a></div>
<div class="result results_links web-result"><h2 class="result__title"><a class="result__a" href="https://duckduckgo.com/y.js?ad=1">An advertisement</a></h2></div>
</body></html>"""
WEB_PAGE_WORDS = "The own system reads the web over its own TLS. " * 300


def test_authority(tmp: Path, names: list[str]) -> None:
    """A certificate authority made for the test (tmp/ca.der, which a test build trusts instead of Mozilla's), and a
    certificate from it for `names` (tmp/leaf.pem, tmp/leaf.key)."""
    def openssl(*a: str) -> None:
        subprocess.run(["openssl", *a], cwd=tmp, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    p256 = ["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256"]
    openssl("req", "-x509", "-nodes", *p256, "-keyout", "ca.key", "-out", "ca.pem", "-days", "2",
            "-subj", "/CN=Chat Computer test authority (not for real use)",
            "-addext", "basicConstraints=critical,CA:true", "-addext", "keyUsage=critical,keyCertSign")
    sans = ",".join(f"DNS:{n}" for n in names)
    (tmp / "leaf.ext").write_text(f"subjectAltName={sans}\nextendedKeyUsage=serverAuth\n"
                                  "keyUsage=critical,digitalSignature\nbasicConstraints=critical,CA:false\n")
    openssl("req", "-nodes", *p256, "-keyout", "leaf.key", "-out", "leaf.csr", "-subj", f"/CN={names[0]}")
    openssl("x509", "-req", "-in", "leaf.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", "leaf.pem",
            "-days", "2", "-extfile", "leaf.ext")
    openssl("x509", "-in", "ca.pem", "-outform", "DER", "-out", "ca.der")


def web_check(args) -> list[str]:
    """The web, as the own system's model reads it: a test build trusting a test's own certificate authority, whose
    sites are all a stand-in here (10.0.2.2: TLS 1.3, a certificate for html.duckduckgo.com and www.example.com),
    reached over QEMU's virtual network by a virtio card, and a test's script in place of the model: a search (the
    results page read, its advertisement left out), a page sent on (302) to the page, which comes compressed (gzip) in
    chunks and is read in two parts (the second from what's kept: the stand-in asked once), a PDF (LibreOffice's, read
    as its words), a page that isn't there, and a site whose certificate isn't for its name (refused). The results
    reach the model, and the screen says each search and site."""
    import gzip
    import gzip as _gz  # noqa: F401
    import http.server
    import ssl
    import threading
    from urllib.parse import parse_qs, urlparse

    tmp = Path(tempfile.mkdtemp(prefix="cc-web-"))
    test_authority(tmp, ["html.duckduckgo.com", "www.example.com"])
    asked: list[tuple[str, str, str]] = []

    class Stand(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *a):
            pass

        def do_GET(self):
            host = self.headers.get("Host", "")
            asked.append((host, self.path, self.headers.get("User-Agent", "")))
            url = urlparse(self.path)
            if host == "html.duckduckgo.com" and url.path == "/html/":
                q = parse_qs(url.query).get("q", [""])[0]
                body = WEB_RESULTS.encode() if q == "chat computer own system" else b"<html>nothing</html>"
                self.send_response(200)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            elif host == "www.example.com" and url.path == "/start":
                self.send_response(302)
                self.send_header("Location", "/page")
                self.send_header("Content-Length", "0")
                self.end_headers()
            elif host == "www.example.com" and url.path == "/paper.pdf":
                body = (HERE / "pdf/tests/letter-libreoffice.pdf").read_bytes()
                self.send_response(200)
                self.send_header("Content-Type", "application/pdf")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            elif host == "www.example.com" and url.path == "/page":
                page = (f"<html><head><title>Example &amp; Page</title><script>no()</script></head><body>"
                        f"<h1>Hello</h1><p>{WEB_PAGE_WORDS}</p><p>The end.</p></body></html>")
                body = gzip.compress(page.encode())
                self.send_response(200)
                self.send_header("Content-Type", "text/html")
                self.send_header("Content-Encoding", "gzip")
                self.send_header("Transfer-Encoding", "chunked")
                self.end_headers()
                for i in range(0, len(body), 700):
                    piece = body[i:i + 700]
                    self.wfile.write(b"%x\r\n%s\r\n" % (len(piece), piece))
                self.wfile.write(b"0\r\n\r\n")
            else:
                body = b"<html>no such page</html>"
                self.send_response(404)
                self.send_header("Content-Type", "text/html")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            self.close_connection = True

    server = http.server.ThreadingHTTPServer(("127.0.0.1", WEB_PORT), Stand)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_3
    context.load_cert_chain(tmp / "leaf.pem", tmp / "leaf.key")
    server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    script = {"web": [
        {"calls": [["search_web", {"query": "chat computer own system"}]]},
        {"calls": [["read_web_page", {"url": "https://www.example.com/start"}]]},
        {"calls": [["read_web_page", {"url": "https://www.example.com/page", "offset": 12000}]]},
        {"calls": [["read_web_page", {"url": "https://www.example.com/paper.pdf"}]]},
        {"calls": [["read_web_page", {"url": "www.example.com/missing"}], ["read_web_page", {"url": "https://wrong.example.org/"}]]},
        {"text": "Done."},
    ]}
    problems: list[str] = []
    try:
        (tmp / "script.json").write_text(json.dumps(script))
        build("chat", {"CHATCOMPUTER_TEST_SCRIPT": str(tmp / "script.json"), "CHATCOMPUTER_TEST_CA": str(tmp / "ca.der"),
                       "CHATCOMPUTER_TEST_PORT": str(WEB_PORT)})
        extra = ("-netdev", "user,id=n0", "-device", "virtio-net-pci,netdev=n0")
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra)
        try:
            if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
                print(pc.text())
                return ["the kernel didn't say it was ready"]
            wait_for(pc.log, ("network: address",), 60)
            monitor_many(pc.mon, keys_for("web") + ["sendkey ret"])
            got = wait_for(pc.log, ("model: answered", "test: failed", "stopped"), 180)
            time.sleep(2)
            pc.settled_picture("web")
        finally:
            pc.stop()
        text = pc.text()
        print(text)
        if got != "model: answered":
            problems.append(f"the answer didn't end (the log says {got!r})")
        said = [t["said"] for t in logged(text, "tool")]
        errors = [t["error"] for t in logged(text, "tool")]
        if len(said) != 6:
            return problems + [f"{len(said)} tools' results, not 6: {said!r}"[:3000]]
        if errors != [False, False, False, False, True, True]:
            problems.append(f"the tools' errors were {errors!r}")
        want = ("Results for “chat computer own system” (DuckDuckGo):\n1. Chat Computer & its own system\n"
                "   https://www.example.com/start\n   A computer you talk to, with an operating system of its own.")
        if said[0] != want:
            problems.append(f"the search said {said[0]!r}")
        words = "Hello\n\n" + WEB_PAGE_WORDS.strip() + "\n\nThe end."
        first = f"Example & Page\nhttps://www.example.com/page\n\n# {words[:12000 - 2]}"
        if not said[1].startswith(first[:400]) or "read_web_page with offset 12000 reads on" not in said[1]:
            problems.append(f"the page's first part was {said[1][:600]!r}")
        if not said[2].startswith("https://www.example.com/page, from character 12000:\n\n") or not said[2].endswith("The end."):
            problems.append(f"the page's second part was {said[2][:300]!r}...{said[2][-100:]!r}")
        if (not said[3].startswith("https://www.example.com/paper.pdf\n\n## Page 1\nA letter to the council\nWhy I'm writing\n")
                or "Ünïcödé, 名前, café and 🙂.\n\n## Page 2\nYours faithfully," not in said[3]):
            problems.append(f"the PDF said {said[3][:600]!r}")
        if "says there's no such page (404)" not in said[4]:
            problems.append(f"the missing page said {said[4]!r}")
        if "didn't trust the site that answered for wrong.example.org" not in said[5]:
            problems.append(f"the wrong certificate said {said[5]!r}")
        pages = [a for a in asked if a[1] == "/page"]
        if len(pages) != 1:
            problems.append(f"the page was asked for {len(pages)} times, not once (the second part comes from what's kept)")
        if not asked or any("ChatComputer/1.0" not in ua for _, _, ua in asked):
            problems.append(f"the stand-in was asked {asked!r}")
        for note in ("web: Searched the web (DuckDuckGo) for “chat computer own system”",
                     "web: Read a page on www.example.com", "web: sent on to www.example.com"):
            if note not in text:
                problems.append(f"the log doesn't say {note!r}")
    finally:
        server.shutdown()
        shutil.rmtree(tmp, ignore_errors=True)
        build()  # never leave a test build in out/
    return problems


MEMORY_ID = 0x5EEDCAFE
LOG_ID = 0x10C0FFEE


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
    with Chat Computer's files, a stand-in for Windows' start-up file, a 4 MiB memory file, and (if the settings name
    one) a 512 KiB log file. Returns the files."""
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
    if f"log-id = {LOG_ID:08x}" in settings:
        files["EFI/chatcomputer-own/log.bin"] = blank_memory(LOG_ID, 512 << 10)
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


# What the start-up partition's log file was last read as (esp_now).
LAST_LOG: list[bytes] = [b""]


def esp_now(path: Path, files: dict[str, bytes]) -> tuple[list[str], bytes]:
    """The disk's start-up partition checked: its file system clean (fsck.vfat, changing nothing), every file but the
    memory file and the log file as it was. Returns the problems, and the memory file (the log file: LAST_LOG)."""
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
        elif name.endswith("log.bin"):
            LAST_LOG[0] = now
            if len(now) != len(data):
                problems.append("the log file's size changed")
        elif now != data:
            problems.append(f"{name} changed")
    esp.unlink()
    return problems, memory


def remember(args) -> list[str]:
    """The conversation kept across a restart, with the start-up partition on an NVMe drive, then on a SATA one: a
    disk like a PC's (memory_disk), a question answered by a test build's script in place of the model, then the
    virtual PC started again: the conversation must be on the screen, and given to the model with the next question,
    and in the memory file in the format Windows' setup made it for. Before the restart the model notes something
    about the person (its remember tool): the note must be in the model's instructions after the restart, and after a
    new conversation (its tool), whose next question must be given alone, and only the new conversation kept. Last,
    the model is asked to turn the PC off. The partition's file system must stay clean and its other files
    untouched."""
    tmp = Path(tempfile.mkdtemp(prefix="cc-memory-"))
    note = "Their favourite word is pumpkin."
    script = {
        "hi": [{"text": "Hello! The word is pumpkin."}],
        "note": [{"calls": [["remember", {"note": note}]]}, {"text": "Noted."}],
        "new": [{"calls": [["start_new_conversation", {}]]}, {"text": "Starting afresh."}],
        "off": [{"calls": [["turn_off_computer", {}]]}, {"text": "Turning off now."}],
        "else": [{"text": "Pumpkin."}],
    }

    def kept_or_failed(log: Path, times: int) -> None:
        end = time.time() + 120
        while time.time() < end:
            text = log.read_text(errors="replace") if log.exists() else ""
            if text.count("memory: the conversation kept") >= times or "couldn't be kept" in text or "model: failed" in text:
                return
            time.sleep(0.5)

    script_build(script, tmp)
    problems: list[str] = []
    user = lambda t: {"role": "user", "content": t}  # noqa: E731
    answer = lambda t, *calls: {"role": "assistant", "content": t, "calls": list(calls)}  # noqa: E731
    try:
        for kind in ("nvme", "sata"):
            disk = tmp / f"{kind}.img"
            settings = SETTINGS_HEAD + f"clock-offset-minutes = 0\r\nmemory-id = {MEMORY_ID:08x}\r\nlog-id = {LOG_ID:08x}\r\n"
            files = memory_disk(disk, BUILT, settings, MEMORY_ID)
            texts = []
            turned_off = False
            for start in (1, 2):
                pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, disk=(disk, kind))
                try:
                    if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
                        print(pc.text())
                        return problems + [f"{kind}, start {start}: the kernel didn't say it was ready"]
                    time.sleep(1)
                    if start == 2:
                        pc.settled_picture(f"memory-{kind}-restored")
                    # The first start: a question, then a note. The second: a question after the restart, then a new
                    # conversation asked for, then a question in it, then the model asked to turn the PC off (ACPI's
                    # S5, which ends the virtual PC).
                    for n, question in enumerate(["hi", "note"] if start == 1 else ["word", "new", "hi", "off"]):
                        monitor_many(pc.mon, [f"sendkey {k}" for k in question] + ["sendkey ret"])
                        kept_or_failed(pc.log, n + 1)
                        if question == "off":
                            pc.picture(f"memory-{kind}-{start}-{question}")  # (at once: it turns off 5 s on)
                        else:
                            pc.settled_picture(f"memory-{kind}-{start}-{question}")
                    if start == 2 and wait_for(pc.log, ("kernel: turning off",), 30):
                        try:
                            pc.qemu.wait(timeout=20)
                            turned_off = True
                        except subprocess.TimeoutExpired:
                            pass
                finally:
                    pc.stop()
                texts.append(pc.text())
                print(pc.text())
                fs_problems, memory_file = esp_now(disk, files)
                problems += [f"{kind}, start {start}: {p}" for p in fs_problems]
                # The log kept on the start-up partition, as Windows' setup reads it back: this start's, whole (the
                # same as the serial port's, but for its line ends), to the end of what it did.
                try:
                    logged_run = kept_in(LAST_LOG[0], LOG_ID)
                except ValueError as e:
                    logged_run = None
                    problems.append(f"{kind}, start {start}: the log file: {e}")
                serial = pc.text().replace("\r\n", "\n")
                log_text = logged_run[1].decode(errors="replace") if logged_run else ""
                want = ["Chat Computer kernel: started", "next: the network", "Chat Computer: ready",
                        f"processor cores: {CORES} in use"] + (["kernel: turning off"] if start == 2 else [])
                missing = [w for w in want if w not in log_text]
                if missing or log_text[:2000] not in serial:
                    problems.append(f"{kind}, start {start}: the log kept on the start-up partition lacks {missing!r} "
                                    f"({len(log_text)} bytes kept)")
                try:
                    kept = kept_in(memory_file, MEMORY_ID)
                except ValueError as e:
                    problems.append(f"{kind}, start {start}: {e}")
                    continue
                # Kept once after each answer, with the note: the conversation, then (the second time) the new one,
                # which began with "hi" again and ended with the model turning the PC off.
                held = json.loads(kept[1]) if kept else {}
                chat = [{k: v for k, v in c.items() if k != "at"} for c in held.get("chat") or []]
                times = [c.get("at") for c in held.get("chat") or [] if "you" in c]
                last = [{"you": "note"}, {"said": "Noted."}] if start == 1 else [{"you": "off"}, {"said": "Turning off now."}]
                if (kept is None or kept[0] != (2 if start == 1 else 6) or held.get("notes") != [note]
                        or chat != [{"you": "hi"}, {"said": "Hello! The word is pumpkin."}] + last
                        or not all(isinstance(t, int) and t > 1_700_000_000 for t in times)):
                    problems.append(f"{kind}, start {start}: the memory file holds {kept!r}")
            drive = "NVMe SSD" if kind == "nvme" else "SATA drive"
            if f"memory: {drive}" not in texts[0]:
                problems.append(f"{kind}: the memory file wasn't found on the {drive}")
            if "the conversation from before the restart (4 parts)" not in texts[1] or "model: 1 note from before" not in texts[1]:
                problems.append(f"{kind}: the conversation or the note didn't come back after the restart")
            given = [plain(r) for r in logged(texts[0], "asked") + logged(texts[1], "asked")]
            if len(given) != 9:
                problems.append(f"{kind}: the model was given {len(given)} rounds, not 9: {given!r}"[:2000])
            else:
                systems = [r[0]["content"] for r in given]
                before = [user("hi"), answer("Hello! The word is pumpkin."), user("note"), answer("Noted.")]
                if given[3][1:] != before + [user("word")]:
                    problems.append(f"{kind}: the question after the restart wasn't given with the conversation before it: {given[3]!r}")
                elif given[4][1:] != before + [user("word"), answer("Pumpkin."), user("new")]:
                    problems.append(f"{kind}: asking for a new conversation wasn't given with the conversation: {given[4]!r}")
                elif given[6][1:] != [user("hi")]:
                    problems.append(f"{kind}: the question after a new conversation began wasn't given alone: {given[6]!r}")
                result = given[2][-1]
                if given[2][-2:-1] != [answer("", {"name": "remember", "args": {"note": note}})] or result.get("role") != "tool" \
                        or not result.get("content", "").startswith("Noted: "):
                    problems.append(f"{kind}: the note's tool result was {result!r}")
                if note in systems[0] or note in systems[2] or note not in systems[3] or note not in systems[6]:
                    problems.append(f"{kind}: the note wasn't in the model's instructions after the restart and the new conversation")
            if "model: uses start_new_conversation" not in texts[1] or "model: a new conversation starts" not in texts[1]:
                problems.append(f"{kind}: the new conversation didn't start")
            if "turning off: sleep type 0 by the PM1 control registers" not in texts[0]:
                problems.append(f"{kind}: the way to turn off wasn't found in the firmware's tables")
            if "model: uses turn_off_computer" not in texts[1] or not turned_off:
                problems.append(f"{kind}: asked to, the PC didn't turn off (the virtual PC ended: {turned_off})")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
        build()  # never leave a test build in out/
    return problems


def fnv1a(data: bytes) -> int:
    """FNV-1a, 64 bits: the hash the ntfs crate's test volumes list each file's bytes by."""
    h = 0xCBF29CE484222325
    for x in data:
        h = ((h ^ x) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


# The partitions Windows' registry in the test volume (registry/tests/make.py's SYSTEM hive) gives drive letters:
# C: and D: by their GPT partitions' own GUIDs, E: by its MBR disk's signature and where it starts.
WINDOWS_PARTITION = "b0e1f3a2-5c4d-4e6f-8a9b-0c1d2e3f4a5b"
DATA_PARTITION = "c1f2a4b3-6d5e-4f70-9bac-1d2e3f4a5b6c"
MBR_SIGNATURE, MBR_OFFSET = 0x5EEDCAFE, 1 << 20


def windows_volume() -> bytes:
    """The ntfs crate's test volume: Windows' folders, a person's in Users, and more."""
    return subprocess.run(["xz", "-dc", str(HERE / "ntfs/tests/small.img.xz")], check=True, capture_output=True).stdout


def windows_disk(path: Path, efi: Path, settings: str, memory_id: int, volume: bytes | None = None) -> dict[str, bytes]:
    """A disk like a PC's (memory_disk), with Windows' drive after the start-up partition: a basic data partition
    holding `volume` (else the ntfs crate's test volume). Returns the start-up partition's files."""
    files = memory_disk(path, efi, settings, memory_id)
    volume = windows_volume() if volume is None else volume
    with open(path, "r+b") as f:
        f.truncate((96 << 20) + len(volume) + (4 << 20))
    subprocess.run(["sgdisk", "-e", str(path)], check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["sgdisk", "-n", f"2:0:+{len(volume) >> 20}M", "-t", "2:0700", "-c", "2:Windows", "-u", f"2:{WINDOWS_PARTITION}",
                    str(path)], check=True, stdout=subprocess.DEVNULL)
    info = subprocess.run(["sgdisk", "-i", "2", str(path)], check=True, capture_output=True, text=True).stdout
    first = int(next(line for line in info.splitlines() if line.startswith("First sector:")).split()[2])
    with open(path, "r+b") as f:
        f.seek(first * 512)
        f.write(volume)
    return files


def files_check(args) -> list[str]:
    """Windows' drive, read: a test's script in place of the model asks for folders, files and a search, each by its
    tool, and what comes back is checked: a folder listed (a hidden system file left out), files read whatever the case of the path
    (UTF-8 and UTF-16 text; a Word file said to be one; a long file in two parts), links followed (a junction, "My
    Documents", and a symbolic link on the Desktop), OneDrive's files read when they're on the PC and said to be
    online only when they aren't, a file Windows compressed itself said to be one, and a search that finds the
    shopping list (and the link to it) but not the one in AppData (a hidden folder). Word, Excel and PowerPoint files
    and a PDF come back as their words; a ZIP archive as its list; a photo and a wide picture as what they are (the model
    reads words only); what can't be read (a damaged Word file, one from before 2007, an iPhone's photo) says why.
    computer_status says what the drive is. Last, a photo shown on the screen (show_picture): its sun's yellow must be
    there in the picture of the screen."""
    import re

    tmp = Path(tempfile.mkdtemp(prefix="cc-files-"))
    calls = {
        "list": ("list_files", {"path": "C:\\Users\\Sam\\Documents"}),
        "read": ("read_file", {"path": "c:/users/sam/documents/SHOPPING LIST.txt"}),
        "note": ("read_file", {"path": "C:\\Users\\Sam\\Documents\\Old note.txt"}),
        "word": ("read_file", {"path": "C:\\Users\\Sam\\Documents\\Letter.docx"}),
        "find": ("find_files", {"words": "shopping LIST"}),
        "big": ("read_file", {"path": "\\Documents\\Report 2026.txt"}),
        "more": ("read_file", {"path": "C:\\Documents\\Report 2026.txt", "offset": 60000}),
        "gone": ("read_file", {"path": "C:\\Users\\Sam\\nothing.txt"}),
        "mydocs": ("list_files", {"path": "C:\\Users\\Sam\\My Documents"}),
        "desk": ("list_files", {"path": "C:\\Users\\Sam\\Desktop"}),
        "linked": ("read_file", {"path": "C:\\Users\\Sam\\Desktop\\Shopping list.txt"}),
        "od": ("list_files", {"path": "C:\\Users\\Sam\\OneDrive\\Documents"}),
        "budget": ("read_file", {"path": "C:\\Users\\Sam\\OneDrive\\Documents\\Budget.txt"}),
        "plan": ("read_file", {"path": "C:\\Users\\Sam\\OneDrive\\Documents\\Plan.docx"}),
        "compact": ("read_file", {"path": "C:\\Windows\\System32\\notepad.exe"}),
        "docx": ("read_file", {"path": "C:\\Users\\Sam\\Documents\\Work\\Letter to the council.docx"}),
        "xlsx": ("read_file", {"path": "C:\\Users\\Sam\\Documents\\Work\\Budget.xlsx"}),
        "pptx": ("read_file", {"path": "C:\\Users\\Sam\\Documents\\Work\\Talk.pptx"}),
        "photo": ("read_file", {"path": "C:\\Users\\Sam\\Pictures\\Beach.jpg"}),
        "pdf": ("read_file", {"path": "C:\\Users\\Sam\\Documents\\Work\\Receipt.pdf"}),
        "zip": ("read_file", {"path": "C:\\Users\\Sam\\Downloads\\photos.zip"}),
        "olddoc": ("read_file", {"path": "C:\\Users\\Sam\\Documents\\Work\\Old letter.doc"}),
        "heic": ("read_file", {"path": "C:\\Users\\Sam\\Pictures\\IMG_0001.HEIC"}),
        "wide": ("read_file", {"path": "C:\\Users\\Sam\\Pictures\\Panorama.png"}),
        "status": ("computer_status", {}),
        "nopic": ("show_picture", {"path": "C:\\Users\\Sam\\Documents\\Work\\Talk.pptx"}),
        "showpic": ("show_picture", {"path": "C:\\Users\\Sam\\Pictures\\Beach.jpg"}),
    }


    script_build(tool_script(calls), tmp)
    problems: list[str] = []
    results: dict[str, dict] = {}
    try:
        disk = tmp / "pc.img"
        settings = SETTINGS_HEAD + f"clock-offset-minutes = 0\r\nmemory-id = {MEMORY_ID:08x}\r\n"
        windows_disk(disk, BUILT, settings, MEMORY_ID)
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, disk=(disk, "nvme"))
        try:
            if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
                print(pc.text())
                return ["the kernel didn't say it was ready"]
            time.sleep(1)
            for n, question in enumerate(calls):
                monitor_many(pc.mon, [f"sendkey {k}" for k in question] + ["sendkey ret"])
                wait_answered(pc, n + 1)
            pc.settled_picture("files")
        finally:
            pc.stop()
        text = pc.text()
        print(text)
        problems += stack_problems(text)
        results = tool_results(text, list(calls))
        if "files: C: Windows' drive (NTFS, 33.5 MB), read only" not in text:
            problems.append("Windows' drive wasn't found as it should be")
        missing = [q for q in calls if q not in results]
        if missing:
            problems.append(f"no tool result came back for {missing}")
            return problems
        said = {q: r["said"] for q, r in results.items()}
        errors = {q for q, r in results.items() if r["error"]}
        day = r"\d{4}-\d\d-\d\d"
        want = {
            "list": rf"C:\\Users\\Sam\\Documents: 1 folder, 3 files\nLetter\.docx \(4 KB, {day}\)\n"
                    rf"Old note\.txt \(76 bytes, {day}\)\nShopping list\.txt \(43 bytes, {day}\)\nWork\\",
            "read": rf"C:\\Users\\Sam\\Documents\\Shopping list\.txt \(43 bytes, changed {day}\):\n"
                    r"Milk, eggs, tea — and café au lait\.\r\n",
            "note": rf"C:\\Users\\Sam\\Documents\\Old note\.txt \(76 bytes, changed {day}\):\n"
                    r"A note saved by Notepad, in UTF-16\.\r\n",
            "word": r"C:\\Users\\Sam\\Documents\\Letter\.docx \(4 KB\) should be a Word, Excel or PowerPoint file, but "
                    r"Chat Computer's own system can't read it: it may be damaged\.",
            "find": r"Under C:\\Users, 2 found:\n"
                    r"C:\\Users\\Sam\\Desktop\\Shopping list\.txt \(a link to \.\.\\Documents\\Shopping list\.txt\)\n"
                    rf"C:\\Users\\Sam\\Documents\\Shopping list\.txt \(43 bytes, {day}\)\n"
                    r"\[Looked in \d+ folders: all of them\.\]",
            "gone": r"C:\\Users\\Sam\\nothing\.txt: there's no such file or folder",
            "mydocs": r"C:\\Users\\Sam\\Documents \(reached by the link C:\\Users\\Sam\\My Documents\): 1 folder, "
                      rf"3 files\nLetter\.docx \(4 KB, {day}\)\n.*",
            "desk": r"C:\\Users\\Sam\\Desktop: 0 folders, 1 file\n"
                    r"Shopping list\.txt \(a link to \.\.\\Documents\\Shopping list\.txt\)",
            "linked": r"C:\\Users\\Sam\\Documents\\Shopping list\.txt \(reached by the link "
                      rf"C:\\Users\\Sam\\Desktop\\Shopping list\.txt\) \(43 bytes, changed {day}\):\n"
                      r"Milk, eggs, tea — and café au lait\.\r\n",
            "od": rf"C:\\Users\\Sam\\OneDrive\\Documents: 0 folders, 4 files\nBudget\.txt \(19 bytes, {day}\)\n"
                  rf"Half\.bin \(301 KB, {day}, online only: not on this PC\)\n"
                  rf"Notes\.txt \(6 bytes, {day}, online only: not on this PC\)\n"
                  rf"Plan\.docx \(50 KB, {day}, online only: not on this PC\)",
            "budget": rf"C:\\Users\\Sam\\OneDrive\\Documents\\Budget\.txt \(19 bytes, changed {day}\):\n"
                      r"Rent 1200\nFood 400\n",
            "plan": r"C:\\Users\\Sam\\OneDrive\\Documents\\Plan\.docx: this file is online only: it's in OneDrive, "
                    r"not downloaded to this PC \(Windows downloads it when it's opened there\)",
            "compact": r"C:\\Windows\\System32\\notepad\.exe: Windows compressed this file itself \(Compact OS\), which "
                       r"Chat Computer's own system can't read yet",
            "docx": rf"C:\\Users\\Sam\\Documents\\Work\\Letter to the council\.docx \(a Word document, 38 KB, changed {day}\), "
                    r"its words:\n# A letter to the council\n\n# Why I'm writing\n.*\| Week \| Bins \| Emptied \|\n.*Yours faithfully,",
            "xlsx": rf"C:\\Users\\Sam\\Documents\\Work\\Budget\.xlsx \(an Excel workbook, 8 KB, changed {day}\), its words:\n"
                    r"## Budget\nrow,A,B,C,D,E,F,G\n.*\n4,Total,1612\.5 \(=SUM\(B2:B3\)\),,,,,\n.*## Notes\n.*",
            "pptx": rf"C:\\Users\\Sam\\Documents\\Work\\Talk\.pptx \(a PowerPoint presentation, 35 KB, changed {day}\), its "
                    r"words:\n## Slide 1: Chat Computer\n.*## Slide 2: What it does\n.*Notes: Say hello first\.\n.*",
            "photo": rf"C:\\Users\\Sam\\Pictures\\Beach\.jpg \(10 KB, 640×480 pixels, changed {day}\): the picture is "
                     r"here \(as you see it, 640×480 pixels\)\.",
            "pdf": rf"C:\\Users\\Sam\\Documents\\Work\\Receipt\.pdf \(a PDF, 1 page, 600 bytes, changed {day}\), its words:\n"
                   r"## Page 1\nReceipt: 3 coffees, 12\.50",
            "zip": r"C:\\Users\\Sam\\Downloads\\photos\.zip is a ZIP archive \(4 KB\) of 2 files:\nBeach\.jpg \(10 KB\)\n"
                   r"notes\.txt \(23 bytes\)\n\[What's in it isn't opened here\.\]",
            "olddoc": r"C:\\Users\\Sam\\Documents\\Work\\Old letter\.doc \(5 KB\) is in Office's format from before 2007 "
                      r"\(\.doc, \.xls, \.ppt\), which Chat Computer's own system can't read inside yet\.",
            "heic": r"C:\\Users\\Sam\\Pictures\\IMG_0001\.HEIC \(3 KB\) is a HEIC picture \(as iPhones take them\), which "
                    r"Chat Computer's own system can't open yet\.",
            "wide": rf"C:\\Users\\Sam\\Pictures\\Panorama\.png \(\d+ KB, 9000×40 pixels, changed {day}\): the picture "
                    r"is here \(as you see it, 8992×32 pixels\)\.",
            "nopic": r"C:\\Users\\Sam\\Documents\\Work\\Talk\.pptx isn't a picture this screen shows \(JPEG, PNG or GIF\)\.",
            "showpic": r"Shown on the screen: C:\\Users\\Sam\\Pictures\\Beach\.jpg \(10 KB, 640×480 pixels\)\.",
        }
        for q, pattern in want.items():
            if not re.fullmatch(pattern, said[q], re.S):
                problems.append(f"{q}: {said[q][:400]!r}")
        if errors != {"gone", "plan", "compact", "nopic"}:
            problems.append(f"errors came back for {sorted(errors)}, not just for the files that can't be read")
        # The photo on the screen: the sun's yellow (250, 220, 90 as it was drawn), in a good few of its pixels.
        from PIL import Image
        screen = Image.open(HERE / "out" / "files.png").convert("RGB")
        px = screen.tobytes()
        sun = sum(1 for i in range(0, len(px), 3) if abs(px[i] - 250) < 30 and abs(px[i + 1] - 220) < 30 and abs(px[i + 2] - 90) < 40)
        if sun < 2_000:
            problems.append(f"the photo wasn't on the screen: {sun} pixels of its sun's yellow")
        # The memory file: the conversation's words (not what the tools said).
        kept = [int(m) for m in re.findall(r"memory: the conversation kept \((\d+) bytes\)", text)]
        if not kept or max(kept) > 10_000:
            problems.append(f"the memory file kept {kept} bytes: the tools' results in it?")
        report = (HERE / "ntfs/tests/small.txt").read_text()
        size = int(next(line for line in report.splitlines() if line.startswith("Documents/Report 2026.txt\t")).split("\t")[2])
        big, more = said["big"], said["more"]
        if not (big.endswith("[The file goes on: read_file with offset 60000 reads the next part.]")
                and len(big.split(":\n", 1)[1].rsplit("\n[The file", 1)[0]) == 60_000):
            problems.append(f"the long file's first part wasn't 60,000 bytes and where to go on: {big[-200:]!r}")
        if "The file goes on" in more or len(more.split(":\n", 1)[1]) != size - 60_000:
            problems.append(f"the long file's second part wasn't the rest of it: {more[-200:]!r}")
        if "Files: C: Windows' drive (NTFS, 33.5 MB), read only" not in said["status"]:
            problems.append("computer_status didn't say what files it can read")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
        build()  # never leave a test build in out/
    return problems


def gpt_disk(path: Path, volume: bytes, guid: str | None = None) -> None:
    """A disk with a GPT and one basic data partition (its own GUID `guid`, else a random one) holding `volume`."""
    with open(path, "wb") as f:
        f.truncate(len(volume) + (2 << 20))
    unique = ["-u", f"1:{guid}"] if guid else []
    subprocess.run(["sgdisk", "-n", f"1:2048:+{len(volume) >> 20}M", "-t", "1:0700", *unique, str(path)], check=True,
                   stdout=subprocess.DEVNULL)
    with open(path, "r+b") as f:
        f.seek(2048 * 512)
        f.write(volume)


def mbr_disk(path: Path, volume: bytes, signature: int, kind: int = 0x07) -> None:
    """A disk as older PCs partition one (and USB sticks and memory cards are): an MBR with the disk's signature, and
    one partition of type `kind` (7: NTFS or exFAT; 0Ch: FAT32) from 1 MiB holding `volume`."""
    import struct

    sectors = len(volume) // 512
    mbr = bytearray(512)
    mbr[0x1B8:0x1BC] = struct.pack("<I", signature)
    mbr[0x1BE:0x1CE] = bytes([0, 0xFE, 0xFF, 0xFF, kind, 0xFE, 0xFF, 0xFF]) + struct.pack("<II", 2048, sectors)
    mbr[510:512] = b"\x55\xAA"
    with open(path, "wb") as f:
        f.write(mbr)
        f.seek(2048 * 512)
        f.write(volume)


def empty_ntfs(tmp: Path, label: str, mib: int) -> bytes:
    """An empty NTFS volume of `mib` MiB named `label`, made by mkntfs."""
    img = tmp / f"{label}.ntfs"
    with open(img, "wb") as f:
        f.truncate(mib << 20)
    subprocess.run(["mkntfs", "-F", "-Q", "-q", "-L", label, "-s", "512", "-c", "4096", str(img)], check=True,
                   stderr=subprocess.DEVNULL)
    data = img.read_bytes()
    img.unlink()
    return data


def drives_check(args) -> list[str]:
    """The PC's other drives, read: Windows' drive on the start-up NVMe SSD, a second NVMe SSD (GPT) with the ntfs
    crate's other test volume, a SATA drive partitioned the old way (MBR) with an empty volume named Photos, and a
    second SATA drive (GPT) with one named Spare. Windows' registry (the SYSTEM hive in Windows' test volume) names the
    first three C:, D: and E:, by their partitions; it gives F: to a USB stick, so the fourth, which Windows hasn't
    seen, gets G:. A test's script in place of the model asks for the computer's status (each drive, its letter and its
    name), lists D:'s and E:'s folders, reads a file on D:, searches D:, and asks for a drive there isn't."""
    import re

    tmp = Path(tempfile.mkdtemp(prefix="cc-drives-"))
    calls = {
        "status": ("computer_status", {}),
        "dlist": ("list_files", {"path": "D:\\"}),
        "dread": ("read_file", {"path": "d:/hello.txt"}),
        "elist": ("list_files", {"path": "E:"}),
        "dfind": ("find_files", {"words": "hello again", "in": "D:\\"}),
        "cread": ("read_file", {"path": "Users\\Sam\\Documents\\Shopping list.txt"}),
        "nodrive": ("read_file", {"path": "Z:\\note.txt"}),
    }


    script_build(tool_script(calls), tmp)
    problems: list[str] = []
    try:
        settings = SETTINGS_HEAD + f"clock-offset-minutes = 0\r\nmemory-id = {MEMORY_ID:08x}\r\n"
        disk = tmp / "pc.img"
        windows_disk(disk, BUILT, settings, MEMORY_ID)
        data = subprocess.run(["xz", "-dc", str(HERE / "ntfs/tests/big-sectors.img.xz")], check=True, capture_output=True).stdout
        gpt_disk(tmp / "data.img", data, DATA_PARTITION)
        mbr_disk(tmp / "photos.img", empty_ntfs(tmp, "Photos", 16), MBR_SIGNATURE)
        gpt_disk(tmp / "spare.img", empty_ntfs(tmp, "Spare", 8))
        extra = ("-drive", f"if=none,id=d1,format=raw,file={tmp / 'data.img'}", "-device", "nvme,serial=ccdata1,drive=d1",
                 "-drive", f"if=none,id=d2,format=raw,file={tmp / 'photos.img'}", "-device", "ide-hd,drive=d2,bus=ide.1",
                 "-drive", f"if=none,id=d3,format=raw,file={tmp / 'spare.img'}", "-device", "ide-hd,drive=d3,bus=ide.3")
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra, disk=(disk, "nvme"))
        try:
            if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
                print(pc.text())
                return ["the kernel didn't say it was ready"]
            time.sleep(1)
            for n, question in enumerate(calls):
                monitor_many(pc.mon, keys_for(question) + ["sendkey ret"])
                wait_answered(pc, n + 1)
        finally:
            pc.stop()
        text = pc.text()
        print(text)
        problems += stack_problems(text)
        results = tool_results(text, list(calls))
        missing = [q for q in calls if q not in results]
        if missing:
            return [f"no tool result came back for {missing}"]
        said = {q: r["said"] for q, r in results.items()}
        drives = ("C: Windows' drive (NTFS, 33.5 MB); D: \u201cbig-sectors\u201d (NTFS, 67.1 MB); E: \u201cPhotos\u201d (NTFS, "
                  "16.7 MB); G: \u201cSpare\u201d (NTFS, 8.3 MB, its letter given here, as Windows hasn't named it), read only")
        if f"files: {drives}" not in text:
            problems.append("the log didn't list the drives as they are")
        if not re.search(r"other drives: NVMe SSD .*; SATA drive .* on port 1 .*; SATA drive .* on port 3 ", text):
            problems.append("the other drives weren't started")
        day = r"\d{4}-\d\d-\d\d"
        # D:'s first folder, as ntfs-3g left it: its folders, and its files (links among them).
        top = [line.split("\t") for line in (HERE / "ntfs/tests/big-sectors.txt").read_text().splitlines() if "/" not in line.split("\t")[0]]
        folders, files = sum(1 for t in top if t[1] == "dir"), sum(1 for t in top if t[1] != "dir")
        want = {
            "status": rf"(?s).*Files: {re.escape(drives)}\n.*",
            "dlist": rf"(?s)D:\\: {folders} folders, {files} files\n.*hello\.txt \(17 bytes, .*",
            "dread": rf"D:\\hello\.txt \(17 bytes, changed {day}\):\nHello from NTFS\.\n",
            "elist": r"E:\\: 0 folders, 0 files",
            "dfind": rf"Under D:\\, 1 found:\nD:\\Documents\\hello again\.txt \(17 bytes, {day}\)\n\[Looked in \d+ folders: all of them\.\]",
            "cread": rf"C:\\Users\\Sam\\Documents\\Shopping list\.txt \(43 bytes, changed {day}\):\nMilk, eggs, tea — and café au lait\.\r\n",
            "nodrive": r"There's no drive Z: here \(its drives are C:, D:, E:, G:\)",
        }
        for q, pattern in want.items():
            if not re.fullmatch(pattern, said[q]):
                problems.append(f"{q}: {said[q][:600]!r}")
        if not results["nodrive"]["error"]:
            problems.append("asking for a drive there isn't wasn't an error")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
        build()  # never leave a test build in out/
    return problems


def fat_volume(tmp: Path, label: str, mib: int, bits: int, cluster: int, files: dict[str, bytes]) -> bytes:
    """A FAT`bits` volume of `mib` MiB named `label`, with `cluster` sectors a cluster (mkfs.vfat), and `files` put on
    it (mtools), as a PC would write them."""
    img = tmp / f"{label}.vfat"
    with open(img, "wb") as f:
        f.truncate(mib << 20)
    subprocess.run(["mkfs.vfat", "-F", str(bits), "-s", str(cluster), "-n", label, str(img)], check=True, stdout=subprocess.DEVNULL)
    env = dict(os.environ, TZ="UTC", LANG="C.UTF-8", LC_ALL="C.UTF-8")
    local = tmp / "file"
    for path, data in files.items():
        parts = path.split("/")
        for i in range(1, len(parts)):
            # (Already there, for a second file in a folder: mmd says so, which is fine.)
            subprocess.run(["mmd", "-i", str(img), "::/" + "/".join(parts[:i])], env=env, capture_output=True)
        local.write_bytes(data)
        subprocess.run(["mcopy", "-o", "-i", str(img), str(local), "::/" + path], check=True, env=env)
    data = img.read_bytes()
    img.unlink()
    return data


# The USB stick Windows' registry (the SYSTEM hive in Windows' test volume) gives F:, by its serial number.
STICK_SERIAL = "4C530001234567891234"


def usb_check(args) -> list[str]:
    """USB drives, read as they come and go. At the start: a USB stick (an MBR, a FAT32 partition named MY STICK) whose
    serial number Windows' registry gives F:, and a card reader with two slots (two logical units), one with a FAT16
    card that has no partition table (G:, the next letter Windows hasn't given), the other empty. Then an exFAT card
    (the exfat crate's SD card volume, in an MBR) is put in the empty slot (H:), and a stick with an NTFS volume of 4
    KiB sectors and no partition table, on a drive of 4 KiB blocks (the ntfs crate's big-sectors volume), is plugged in
    (I:); then the first stick is unplugged and the exFAT card taken out. A test's script in place of the model asks
    for the computer's status, lists folders, reads files (Chat Computer's own folder on the stick refused), finds one,
    all along."""
    import re

    tmp = Path(tempfile.mkdtemp(prefix="cc-usb-"))
    calls = {
        "status": ("computer_status", {}),
        "flist": ("list_files", {"path": "F:\\"}),
        "fread": ("read_file", {"path": "F:\\Documents\\Shopping list.txt"}),
        "fown": ("read_file", {"path": "F:\\EFI\\chatcomputer-own\\settings.txt"}),
        "gread": ("read_file", {"path": "g:/notes/TODO.TXT"}),
        "hlist": ("list_files", {"path": "H:\\Documents"}),
        "hread": ("read_file", {"path": "H:\\hello.txt"}),
        "hbig": ("read_file", {"path": "H:\\Big.bin", "offset": 500000}),
        "hfind": ("find_files", {"words": "lower", "in": "H:\\"}),
        "iread": ("read_file", {"path": "I:\\hello.txt"}),
        "gone": ("read_file", {"path": "F:\\hello.txt"}),
        "status2": ("computer_status", {}),
    }


    script_build(tool_script(calls), tmp)
    problems: list[str] = []
    try:
        settings = SETTINGS_HEAD + f"clock-offset-minutes = 0\r\nmemory-id = {MEMORY_ID:08x}\r\n"
        disk = tmp / "pc.img"
        windows_disk(disk, BUILT, settings, MEMORY_ID)
        stick_files = {
            "hello.txt": b"Hello from a USB stick.\r\n",
            "Documents/Shopping list.txt": "Bread, cheese \u2014 and olives.\r\n".encode(),
            "Photos/Holiday/Notes.txt": b"Sunny.\n",
            "EFI/chatcomputer-own/settings.txt": b"model = not-to-be-read\r\n",
        }
        mbr_disk(tmp / "stick.img", fat_volume(tmp, "MY STICK", 300, 32, 8, stick_files), 0x1234ABCD, 0x0C)
        (tmp / "old.img").write_bytes(fat_volume(tmp, "OLD CARD", 16, 16, 4, {"Notes/todo.txt": b"Buy stamps.\n"}))
        sd = subprocess.run(["xz", "-dc", str(HERE / "exfat/tests/sd.img.xz")], check=True, capture_output=True).stdout
        mbr_disk(tmp / "card.img", sd, 0x5D5D5D5D, 0x07)
        ntfs4k = subprocess.run(["xz", "-dc", str(HERE / "ntfs/tests/big-sectors.img.xz")], check=True, capture_output=True).stdout
        (tmp / "ntfs4k.img").write_bytes(ntfs4k)
        extra = ("-device", "qemu-xhci,id=xhci",
                 "-drive", f"if=none,id=stick,format=raw,file={tmp / 'stick.img'}",
                 "-device", f"usb-storage,bus=xhci.0,port=1,drive=stick,id=stick1,removable=on,serial={STICK_SERIAL}",
                 "-device", "usb-bot,bus=xhci.0,port=2,id=reader",
                 "-drive", "if=none,id=slot0",
                 "-device", "scsi-hd,bus=reader.0,scsi-id=0,lun=0,drive=slot0,removable=on,vendor=Generic,product=SD/MMC",
                 "-drive", f"if=none,id=slot1,format=raw,file={tmp / 'old.img'}",
                 "-device", "scsi-hd,bus=reader.0,scsi-id=0,lun=1,drive=slot1,removable=on,vendor=Generic,product=MS/MS-Pro",
                 "-drive", f"if=none,id=ssd,format=raw,file={tmp / 'ntfs4k.img'}")
        # (Drives there at the start are in the first list of drives, or said to have come just after, depending on
        # how soon they're ready.)
        stick = "F: \u201cMY STICK\u201d (FAT32, 314.5 MB"
        old = "G: \u201cOLD CARD\u201d (FAT16, 16.7 MB"
        card = "files: USB drive \u201cGeneric SD/MMC\u201d: H: \u201cSD CARD\u201d (exFAT, 100.6 MB). Its files can be read."
        ssd = "files: USB drive \u201cQEMU QEMU HARDDISK\u201d: I: \u201cbig-sectors\u201d (NTFS, 67.1 MB). Its files can be read."
        steps: list[tuple[str, str, str]] = [
            ("ask", "status", ""), ("ask", "flist", ""), ("ask", "fread", ""), ("ask", "fown", ""), ("ask", "gread", ""),
            ("do", f"change slot0 {tmp / 'card.img'} raw", card),
            ("ask", "hlist", ""), ("ask", "hread", ""), ("ask", "hbig", ""), ("ask", "hfind", ""),
            ("do", "device_add usb-storage,bus=xhci.0,port=3,drive=ssd,id=stick3,logical_block_size=4096,physical_block_size=4096", ssd),
            ("ask", "iread", ""),
            ("do", "device_del stick1", "files: F: (\u201cMY STICK\u201d) is gone: its USB drive was unplugged, or its card taken out."),
            ("do", "eject -f slot0", "files: H: (\u201cSD CARD\u201d) is gone: its USB drive was unplugged, or its card taken out."),
            ("ask", "gone", ""), ("ask", "status2", ""),
        ]
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra, disk=(disk, "nvme"))
        try:
            if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
                print(pc.text())
                return ["the kernel didn't say it was ready"]
            for note in (stick, old):
                if not wait_for(pc.log, (note,), 30):
                    problems.append(f"no {note!r} in the log")
            asked = 0
            for n, (kind, what, note) in enumerate(steps):
                if kind == "do":
                    monitor(pc.mon, what)
                    if not wait_for(pc.log, (note,), 30):
                        problems.append(f"after {what!r}: no {note!r} in the log")
                    time.sleep(1)
                    pc.picture(f"usb-{n}")  # the line saying what came or went
                    continue
                monitor_many(pc.mon, keys_for(what) + ["sendkey ret"])
                asked += 1
                wait_answered(pc, asked)
            time.sleep(1)
            pc.picture("usb")
        finally:
            pc.stop()
        text = pc.text()
        print(text)
        problems += stack_problems(text)
        results = tool_results(text, list(calls))
        missing = [q for q in calls if q not in results]
        if missing:
            return problems + [f"no tool result came back for {missing}"]
        said = {q: r["said"] for q, r in results.items()}
        day = r"\d{4}-\d\d-\d\d"
        usb_drive = "on the USB drive \u201c"
        drives = (f"C: Windows' drive (NTFS, 33.5 MB); F: \u201cMY STICK\u201d (FAT32, 314.5 MB, {usb_drive}QEMU QEMU HARDDISK\u201d); "
                  f"G: \u201cOLD CARD\u201d (FAT16, 16.7 MB, {usb_drive}Generic MS/MS-Pro\u201d, its letter given here, as Windows hasn't "
                  "named it), read only")
        later = (f"C: Windows' drive (NTFS, 33.5 MB); G: \u201cOLD CARD\u201d (FAT16, 16.7 MB, {usb_drive}Generic MS/MS-Pro\u201d, its "
                 f"letter given here, as Windows hasn't named it); I: \u201cbig-sectors\u201d (NTFS, 67.1 MB, {usb_drive}QEMU QEMU "
                 "HARDDISK\u201d, its letter given here, as Windows hasn't named it), read only")
        want = {
            "status": rf"(?s).*Files: {re.escape(drives)}\n.*",
            "flist": rf"(?s)F:\\: 3 folders, 1 file\n.*hello\.txt \(25 bytes, {day}\).*",
            "fread": rf"F:\\Documents\\Shopping list\.txt \(31 bytes, changed {day}\):\nBread, cheese \u2014 and olives\.\r\n",
            "fown": r"F:\\EFI\\chatcomputer-own: Chat Computer's own folder \(its settings and memory\), which isn't read here",
            "gread": rf"G:\\Notes\\todo\.txt \(12 bytes, changed {day}\):\nBuy stamps\.\n",
            "hlist": rf"(?s)H:\\Documents: 0 folders, 3 files\n.*\u00dcn\u00efc\u00f6d\u00e9 \u2014 \u540d\u524d \U0001f642\.txt \(\d+ bytes, {day}\).*",
            "hread": rf"H:\\hello\.txt \(18 bytes, changed {day}\):\nHello from exFAT\.\n",
            "hbig": r"(?s)H:\\Big\.bin \(1\.0 MB, changed .*\):\n0003:000007a120\n0003:000007a130\n.*\[The file goes on: read_file with offset 560000 reads the next part\.\]",
            "hfind": rf"Under H:\\, 1 found:\nH:\\MiXeD CaSe\\lower\.TXT \(5 bytes, {day}\)\n\[Looked in \d+ folders: all of them\.\]",
            "iread": rf"I:\\hello\.txt \(17 bytes, changed {day}\):\nHello from NTFS\.\n",
            "gone": r"There's no drive F: here \(its drives are C:, G:, I:\)",
            "status2": rf"(?s).*Files: {re.escape(later)}\n.*",
        }
        for q, pattern in want.items():
            if not re.fullmatch(pattern, said[q]):
                problems.append(f"{q}: {said[q][:600]!r}")
        for q in ("fown", "gone"):
            if not results[q]["error"]:
                problems.append(f"{q} wasn't an error")
        if "not-to-be-read" in text:
            problems.append("Chat Computer's own settings reached the model")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
        build()  # never leave a test build in out/
    return problems


def keys_for(text: str) -> list[str]:
    """The monitor's commands that type `text` (letters, capitals among them, digits, spaces, dashes, colons, full
    stops and backslashes)."""
    named = {" ": "spc", "-": "minus", ":": "shift-semicolon", ".": "dot", "\\": "backslash"}
    return [f"sendkey {named.get(c, f'shift-{c.lower()}' if c.isupper() else c)}" for c in text]


def encrypted(volume: bytes, clear: bool) -> tuple[bytes, dict]:
    """`volume` encrypted with BitLocker (bitlocker/tests/encrypt.py), and its recovery key and the key's ID."""
    import importlib.util

    spec = importlib.util.spec_from_file_location("encrypt", HERE / "bitlocker/tests/encrypt.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.encrypt(volume, clear=clear)


def bitlocker_check(args) -> list[str]:
    """Windows' drive encrypted with BitLocker (the ntfs crate's test volume, encrypted by bitlocker/tests/encrypt.py
    as Windows 10 and 11 do, with a TPM's protector and a recovery key), as on a PC with device encryption. Locked at
    first: the model's file tools say how to unlock it (its recovery key, typed in the message box, and the key's ID).
    A wrong recovery key typed is refused, naming the key ID that unlocks it; then a message with the right one in it
    (spaces between its groups) unlocks it, and the files read as from an unencrypted drive: a folder listed, a text
    file, a photo shown on the screen (its colours there), the computer's status. Neither key, nor the message with one
    in it, reaches the model (a test build logs all it's given), the log or the disk (the memory file). Then the same drive with BitLocker's key in the clear
    (BitLocker suspended): read at once, with nothing typed."""
    import re

    tmp = Path(tempfile.mkdtemp(prefix="cc-bitlocker-"))
    calls = {
        "locked": ("list_files", {"path": "C:\\Users\\Sam\\Documents"}),
        "list": ("list_files", {"path": "C:\\Users\\Sam\\Documents"}),
        "read": ("read_file", {"path": "C:\\Users\\Sam\\Documents\\Shopping list.txt"}),
        "photo": ("show_picture", {"path": "C:\\Users\\Sam\\Pictures\\Beach.jpg"}),
        "status": ("computer_status", {}),
    }



    script_build(tool_script(calls), tmp)
    problems: list[str] = []
    try:
        ntfs = windows_volume()
        size = f"{(len(ntfs) + (1 << 20)) / 1e6:.1f} MB"
        settings = SETTINGS_HEAD + f"clock-offset-minutes = 0\r\nmemory-id = {MEMORY_ID:08x}\r\n"
        # Locked, then unlocked with its recovery key.
        volume, info = encrypted(ntfs, clear=False)
        right, key_id = info["recovery_key"], info["recovery_id"][:8]
        groups = right.split("-")
        wrong = "-".join(groups[1:] + groups[:1])
        disk = tmp / "pc.img"
        windows_disk(disk, BUILT, settings, MEMORY_ID, volume)
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, disk=(disk, "nvme"))
        answered = 0
        asked: list[str] = []

        def ask(question: str) -> None:
            nonlocal answered
            answered += 1
            asked.append(question)
            monitor_many(pc.mon, keys_for(question) + ["sendkey ret"])
            wait_answered(pc, answered)

        try:
            if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
                print(pc.text())
                return ["the kernel didn't say it was ready"]
            time.sleep(1)
            ask("locked")
            monitor_many(pc.mon, keys_for(wrong) + ["sendkey ret"])
            if not wait_for(pc.log, ("files: That recovery key doesn't unlock",), 120):
                problems.append("a wrong recovery key typed wasn't refused")
            monitor_many(pc.mon, keys_for("here it is: " + right.replace("-", " ")) + ["sendkey ret"])
            if not wait_for(pc.log, ("files: C: (Windows' drive) is unlocked",), 120):
                problems.append("the right recovery key didn't unlock the drive")
            pc.settled_picture("bitlocker-unlocked")
            # (The photo last: a question after it would scroll it up, out of sight.)
            for question in ("list", "read", "status", "photo"):
                ask(question)
            pc.settled_picture("bitlocker")
        finally:
            pc.stop()
        text = pc.text()
        print(text)
        results = tool_results(text, asked)
        missing = [q for q in calls if q not in results]
        if missing:
            problems.append(f"no tool result came back for {missing}")
            return problems
        said = {q: r["said"] for q, r in results.items()}
        day = r"\d{4}-\d\d-\d\d"
        locked = (r"C: is encrypted with BitLocker\. To read it here, the person types its recovery key in the message "
                  rf"box: 48 digits, .*; its key ID starts {key_id}\. Chat Computer's own system unlocks the drive with "
                  r"the key itself: it isn't given to the model, or kept, so it's typed again after a restart")
        want = {
            "locked": locked,
            "list": rf"C:\\Users\\Sam\\Documents: 1 folder, 3 files\nLetter\.docx \(4 KB, {day}\)\n"
                    rf"Old note\.txt \(76 bytes, {day}\)\nShopping list\.txt \(43 bytes, {day}\)\nWork\\",
            "read": rf"C:\\Users\\Sam\\Documents\\Shopping list\.txt \(43 bytes, changed {day}\):\n"
                    r"Milk, eggs, tea — and café au lait\.\r\n",
            "photo": r"Shown on the screen: C:\\Users\\Sam\\Pictures\\Beach\.jpg \(10 KB, 640×480 pixels\)\.",
        }
        for q, pattern in want.items():
            if not re.fullmatch(pattern, said[q], re.S):
                problems.append(f"{q}: {said[q][:500]!r}")
        if not results["locked"]["error"]:
            problems.append("the locked drive's listing wasn't an error")
        if f"files: C: encrypted with BitLocker, locked ({size}; its recovery key's ID starts {key_id}), read only. To read a locked drive" not in text:
            problems.append("the log didn't say Windows' drive is locked with BitLocker")
        if f"files: That recovery key doesn't unlock a drive here. The keys that do have IDs starting {key_id} (C:)" not in text:
            problems.append("the wrong key's refusal didn't name the right key's ID")
        unlocked = f"Files: C: Windows' drive (NTFS, {size}, encrypted with BitLocker: unlocked with its recovery key until Chat Computer restarts), read only"
        if unlocked not in said["status"]:
            problems.append(f"computer_status didn't say the drive was unlocked: {said['status'][:600]!r}")
        # The photo, decrypted, on the screen: the sun's yellow (250, 220, 90 as it was drawn), in a good few pixels.
        from PIL import Image
        px = Image.open(HERE / "out" / "bitlocker.png").convert("RGB").tobytes()
        sun = sum(1 for i in range(0, len(px), 3) if abs(px[i] - 250) < 30 and abs(px[i + 1] - 220) < 30 and abs(px[i + 2] - 90) < 40)
        if sun < 2_000:
            problems.append(f"the photo wasn't on the screen: {sun} pixels of its sun's yellow")
        # The keys, as typed and as digits: nowhere but the keyboard (the log has all the model was given).
        on_disk = disk.read_bytes()
        for key in (right, wrong):
            for form in {key, key.replace("-", " "), key.replace("-", "")}:
                if form in text:
                    problems.append("a recovery key reached the model, or the log")
                if form.encode() in on_disk or form.encode("utf-16-le") in on_disk:
                    problems.append("a recovery key was written on the disk")
        if "here it is" in text:
            problems.append("the message with the recovery key in it was given to the model")

        # The key in the clear: read at once.
        volume, _ = encrypted(ntfs, clear=True)
        windows_disk(disk, BUILT, settings, MEMORY_ID, volume)
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, disk=(disk, "nvme"))
        answered = 0
        asked = []
        try:
            if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
                print(pc.text())
                return problems + ["the kernel didn't say it was ready, with BitLocker's key in the clear"]
            time.sleep(1)
            ask("read")
        finally:
            pc.stop()
        text = pc.text()
        clear = (f"files: C: Windows' drive (NTFS, {size}, encrypted with BitLocker, which keeps its key in the clear: "
                 "suspended, or not switched on yet), read only")
        if clear not in text:
            problems.append("with BitLocker's key in the clear, the drive wasn't read at once")
        results = tool_results(text, asked)
        if "read" not in results or not re.fullmatch(want["read"], results["read"]["said"], re.S):
            problems.append(f"with BitLocker's key in the clear, the file didn't read: {str(results.get('read'))[:400]}")
    finally:
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


# The timer's chime (the chime crate's, the main Chat Computer's): its two notes, how long each lasts with the gap
# after it, and its loudest sample.
CHIME_NOTES = (1046.5, 1318.5)
CHIME_NOTE_S = 0.42 + 0.07
CHIME_PEAK = 8194


def goertzel(samples: list[int], rate: int, freq: float) -> float:
    """How much of `freq` there is in `samples` (the power of that one frequency, by Goertzel's algorithm)."""
    import math

    k = 2 * math.cos(2 * math.pi * freq / rate)
    s1 = s2 = 0.0
    for x in samples:
        s1, s2 = x + k * s1 - s2, s1
    return (s1 * s1 + s2 * s2 - k * s1 * s2) / max(1, len(samples)) ** 2


def chimes_in(wav: Path) -> tuple[list[list[int]], int, list[str]]:
    """The chimes recorded (each its left channel's samples), the rate, and what's wrong with the recording. (QEMU
    records only while a stream plays, so chimes rung seconds apart come one after another, a moment apart.)"""
    import array
    import wave

    if not wav.exists() or wav.stat().st_size <= 44:
        return [], 0, [f"{wav.name}: nothing was recorded"]
    with wave.open(str(wav)) as w:
        rate, channels, width = w.getframerate(), w.getnchannels(), w.getsampwidth()
        raw = w.readframes(w.getnframes())
    if (channels, width) != (2, 2):
        return [], rate, [f"{wav.name}: {channels} channels of {8 * width} bits, not 2 of 16"]
    both = array.array("h", raw)
    left, right = both[0::2], both[1::2]
    problems = []
    if any(abs(a - b) > 1 for a, b in zip(left, right)):
        problems.append(f"{wav.name}: the channels differ")
    # Each chime: sound, with no more than its notes' gap (0.07 s) of silence in it.
    chimes: list[list[int]] = []
    start = last = None
    for i, x in enumerate(left):
        if abs(x) > 200:
            if start is None or i - last > rate // 10:
                if start is not None:
                    chimes.append(list(left[start:last + 1]))
                start = i
            last = i
    if start is not None:
        chimes.append(list(left[start:last + 1]))
    return chimes, rate, problems


def check_chime(name: str, samples: list[int], rate: int, loudness: float = 1.0) -> list[str]:
    """One chime: the timer's, its first note then its second, as loud as it should be (`loudness`: what the device's
    own volume makes of it)."""
    problems = []
    peak = max(abs(x) for x in samples)
    want = round(CHIME_PEAK * loudness)
    if abs(peak - want) > want * 0.05:
        problems.append(f"{name}: its loudest sample is {peak}, not about {want}")
    heard = len(samples) / rate
    if not 0.75 < heard < 1.0:
        problems.append(f"{name}: sound for {heard:.2f} s, not the chime's 0.9 s or so")
    note = int(rate * CHIME_NOTE_S)
    for n, freq in enumerate(CHIME_NOTES):
        part = samples[n * note:n * note + int(rate * 0.3)]
        other = CHIME_NOTES[1 - n]
        mine, theirs, below = (goertzel(part, rate, f) for f in (freq, other, freq / 2))
        if not (mine > 50 * theirs and mine > 50 * below):
            problems.append(f"{name}: note {n + 1} isn't {freq} Hz ({mine:.0f} there, {theirs:.0f} at {other} Hz, "
                            f"{below:.0f} at {freq / 2} Hz)")
    return problems


def sound(args) -> list[str]:
    """A 2-second timer, set by a test build once it's ready (as the model's reminders tool sets one), on each HD Audio
    controller and codec QEMU has, and on QEMU's USB sound card (alone, and plugged in beside HD Audio, which it then
    plays instead of): it rings (a line in the conversation, and the timer's chime), and rings again 4 seconds later
    until a key is pressed (the first time, and on the USB sound card's first, after two chimes; else after one).
    Every chime recorded is checked (on the USB sound card, at QEMU's own volume for it: 240 of 255)."""
    import re

    build("sound")
    problems = []
    first = True
    usb = ("-device", "qemu-xhci,id=xhci", "-device", "usb-audio,audiodev=snd0,bus=xhci.0")
    runs = [(controller, codec) for controller in ("intel-hda", "ich9-intel-hda") for codec in ("hda-output", "hda-duplex", "hda-micro")]
    for controller, codec in runs + [("usb", "usb-audio"), ("intel-hda", "hda-output and usb-audio")]:
        label = f"{controller} + {codec}"
        wav = OUT / f"sound-{controller}-{codec.replace(' ', '-')}.wav"
        wav.unlink(missing_ok=True)
        on_usb = "usb-audio" in codec
        if controller == "usb":
            devices = usb
        else:
            devices = ("-device", f"{controller},id=hda0", "-device", f"{codec.split()[0]},audiodev={'none0' if on_usb else 'snd0'},bus=hda0.0")
            devices += (("-audiodev", "none,id=none0") + usb) if on_usb else ()
        extra = ("-audiodev", f"wav,id=snd0,path={wav},out.frequency=48000", *devices)
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra)
        want = 2 if first or controller == "usb" else 1
        try:
            got = wait_for(pc.log, ("Your 2-second timer is done: tea.", "kernel stopped"), args.timeout)
            if got and "done" in got:
                # Rung: a key once it has rung `want` times stops it.
                wait_for(pc.log, ("test: chime at",), 10, times=want)
                time.sleep(1.5)
                if first:
                    pc.picture("sound-rung")
                monitor(pc.mon, "sendkey a")
                time.sleep(5.5)
        finally:
            pc.stop()
        text = pc.text()
        found = next((line for line in text.splitlines() if "kernel: sound" in line), "(no line about sound)")
        print(f"{label}: {found}")
        if "test: Set a 2-second timer for tea (rings in 2 seconds). [1]" not in text:
            problems.append(f"{label}: the timer wasn't set as the model's tool sets one")
        if got is None or "done" not in got:
            problems.append(f"{label}: the timer didn't ring ({got or 'nothing said in time'})")
            continue
        times = [int(t) for t in re.findall(r"test: chime at (\d+) ms", text)]
        if len(times) != want:
            problems.append(f"{label}: the chime rang {len(times)} times, not {want} (a key stops it)")
        elif want == 2 and not 3700 <= times[1] - times[0] <= 4400:
            problems.append(f"{label}: it rang again {times[1] - times[0]} ms after, not 4 s")
        if on_usb and text.count("on the USB sound card") != want:
            problems.append(f"{label}: it didn't ring on the USB sound card")
        chimes, rate, wrong = chimes_in(wav)
        problems += [f"{label}: {p}" for p in wrong]
        if len(chimes) != want:
            problems.append(f"{label}: {len(chimes)} chimes were recorded, not {want}")
        loudness = 240 / 255 if on_usb else 1.0
        for k, c in enumerate(chimes):
            problems += [f"{label}: {p}" for p in check_chime(f"{wav.name} chime {k + 1}", c, rate, loudness)]
        print(f"{label}: {len(chimes)} chime(s) recorded at {rate} Hz")
        first = False
    build()  # never leave a test build in out/
    return problems


TINY_LISTENING = HERE.parent / "tests/data/tiny-listening-model.ccm"


def listen_check(args) -> list[str]:
    """Chat Computer's own speech recognition, in a virtual PC: a test build, with a small speech recognition model of
    random weights (tests/data/tiny-listening-model.ccm) where Chat Computer's app keeps its speech models on Windows
    (its models folder's speech folder, here on a second drive's FAT32 volume), its microphone (QEMU's HD Audio
    microphone, fed from a file by QEMU's SDL audio) hearing a voice-like hum (harmonics of a pitch rising and falling)
    for 2.5 seconds every 6. Windows+H is pressed: it must listen, wait for the voice to begin and end, hear it, and say
    whether it sent it. What it heard (its samples, which the test build logs) must be heard the same by the engine on
    Linux (examples/listen_check, with the arithmetic the kernel used): the same tokens, and the same chance of no
    speech and log-likelihood to the last digit; and sent or not by the same rules (Chat Computer's app's)."""
    import base64
    import math
    import re
    import struct
    import wave

    build("listening")
    problems: list[str] = []
    tmp = Path(tempfile.mkdtemp(prefix="cc-listen-"))
    rate = 48000
    try:
        models = "Users/Test/AppData/Local/ai.chatcomputer.desktop/models"
        volume = fat_volume(tmp, "DATA", 64, 32, 1, {f"{models}/speech/whisper-test.ccm": TINY_LISTENING.read_bytes()})
        gpt_disk(tmp / "data.img", volume)
        # Four minutes of it: 3.5 s quiet (a little hiss), then the hum for 2.5 s; SDL reads the file as time goes by.
        frames = []
        phase = 0.0
        seed = 12345
        for i in range(rate * 240):
            t = (i % (rate * 6)) / rate
            seed = (seed * 1103515245 + 12345) & 0x7FFFFFFF
            v = 30 * (seed / 0x7FFFFFFF - 0.5)
            if t >= 3.5:
                pitch = 140 + 40 * math.sin(2 * math.pi * 0.7 * t)
                phase += 2 * math.pi * pitch / rate
                hum = sum(math.sin(phase * k) / k for k in range(1, 6))
                v += 5000 * hum * (0.6 + 0.4 * math.sin(2 * math.pi * 2 * t))
            s16 = max(-32768, min(32767, int(round(v))))
            frames.append(struct.pack("<hh", s16, s16))
        said = tmp / "said.raw"
        said.write_bytes(b"".join(frames))
        env = {"SDL_AUDIODRIVER": "disk", "SDL_DISKAUDIOFILEIN": str(said), "SDL_DISKAUDIOFILE": str(tmp / "played.raw")}
        extra = ("-drive", f"if=none,id=d1,format=raw,file={tmp / 'data.img'}", "-device", "nvme,serial=ccdata1,drive=d1",
                 "-audiodev", f"sdl,id=snd0,in.frequency={rate},in.channels=2,in.format=s16,out.frequency={rate}",
                 "-device", "intel-hda,id=hda0", "-device", "hda-micro,audiodev=snd0,bus=hda0.0")
        settings = SETTINGS_HEAD + "clock-offset-minutes = 0\r\n"
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra, settings=settings, env=env)
        try:
            if wait_for(pc.log, ("Chat Computer: ready", "stopped"), args.timeout) != "Chat Computer: ready":
                print(pc.text())
                return ["the kernel didn't say it was ready"]
            time.sleep(1)
            pc.settled_picture("listen-ready")
            monitor_many(pc.mon, ["sendkey meta_l-h"])
            if wait_for(pc.log, ("listening: to ", "can't listen", "stopped"), args.timeout) != "listening: to ":
                problems.append("it didn't start listening when Windows+H was pressed")
            else:
                time.sleep(0.5)
                pc.settled_picture("listen-listening")
                if wait_for(pc.log, ("hearing it", "didn't hear", "stopped"), 60) != "hearing it":
                    problems.append("it didn't hear the voice begin and end")
                elif wait_for(pc.log, ("test: sent", "test: not sent", "stopped"), args.timeout) is None:
                    problems.append("it didn't finish hearing what was said")
                pc.settled_picture("listen-heard")
        finally:
            pc.stop()
        text = pc.text()
        print("\n".join(line for line in text.splitlines() if "test: said" not in line))
        problems += stack_problems(text)
        samples = re.search(r"test: said (\d+) samples: ([A-Za-z0-9+/=]+)", text)
        heard = re.search(r"listening: heard (\d+) tokens in [\d.]+ s \(([\d.]+) s said\): (\{.*\})", text)
        ready = re.search(r"listening: whisper-test ready \(.*\), its arithmetic (.+)", text)
        sent = re.search(r"test: (sent as the message|not sent: no words)", text)
        if not samples or not heard or not ready or not sent:
            return problems + ["the log doesn't say what was heard, with what, and what came of it"]
        pcm = base64.b64decode(samples[2])
        n = int(samples[1])
        if len(pcm) != 2 * n:
            problems.append(f"{len(pcm)} bytes of samples logged, not {2 * n}")
        seconds = n / 16000
        # (The hum is 2.5 s, with 0.3 s kept before it and after: about 3.1 s, give or take a frame or two.)
        if not 2.6 <= seconds <= 3.6:
            problems.append(f"it heard {seconds:.2f} s said, not about 3.1")
        wav = tmp / "said.wav"
        with wave.open(str(wav), "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(16000)
            w.writeframes(pcm)
        kernel = json.loads(heard[3])
        engine = subprocess.run(["cargo", "run", "-q", "--release", "-p", "model", "--features", "std", "--example",
                                 "listen_check", "--", str(TINY_LISTENING), str(wav), "--level", LEVELS.get(ready[1], "plain")],
                                cwd=HERE, capture_output=True, text=True)
        try:
            linux = json.loads(engine.stdout)
        except ValueError:
            return problems + [f"the engine on Linux didn't say what it heard: {engine.stderr[-1500:]}"]
        same = [k for k in ("tokens", "no_speech", "avg_logprob") if kernel[k] != linux[k]]
        if same:
            problems.append(f"the kernel heard {kernel}, the engine on Linux {dict((k, linux[k]) for k in kernel)}"[:2000])
        else:
            print(f"The kernel heard what the engine on Linux hears: {len(kernel['tokens'])} tokens, no speech "
                  f"{kernel['no_speech']}, log-likelihood {kernel['avg_logprob']}, with {ready[1]} arithmetic.")
        # Sent or not, by Chat Computer's app's rules.
        words = linux["text"].strip().lower()
        made_up = {"thank you", "thank you.", "thanks for watching", "thanks for watching!", "thank you for watching", "you",
                   "bye", "bye.", "bye-bye", "okay.", "so", "uh", "um", "hmm", "mm", "the end", ".", "...",
                   "subtitles by the amara.org community", "please subscribe", "like and subscribe", "i'm sorry"}
        noise = (not words or not re.search(r"\w", words)
                 or (words in made_up and (linux["no_speech"] > 0.3 or seconds < 1.2 or linux["avg_logprob"] < -0.7))
                 or (linux["no_speech"] > 0.75 and linux["avg_logprob"] < -0.8))
        if (sent[1] == "sent as the message") == noise:
            problems.append(f"it {sent[1]!r}, but by Chat Computer's app's rules that's {'noise' if noise else 'words'}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
        build()  # never leave a test build in out/
    return problems


def microphone(args) -> list[str]:
    """What the microphone hears: a test build listens for 2.5 seconds once it's ready, on each HD Audio controller
    and codec QEMU has that has an input, which is fed a tone of its own (its frequency and its loudness), from a
    file, by QEMU's SDL audio (SDL's disk driver). What it heard after its first quarter second must be that tone:
    its frequency (by how often it crosses zero) within 1%, its loudness (root mean square) within 0.5 dB, its peak
    within 1%. A codec without an input says it can't listen."""
    import math
    import re
    import struct

    build("microphone")
    problems = []
    tmp = Path(tempfile.mkdtemp(prefix="cc-microphone-"))
    rate = 48000
    try:
        runs = [("intel-hda", "hda-duplex", "a line in", 440.0, 8000), ("intel-hda", "hda-micro", "a microphone jack", 1000.0, 3000),
                ("ich9-intel-hda", "hda-duplex", "a line in", 1500.0, 20000), ("ich9-intel-hda", "hda-micro", "a microphone jack", 700.0, 12000),
                ("intel-hda", "hda-output", None, 1000.0, 8000)]
        for controller, codec, what, freq, amp in runs:
            label = f"{controller} + {codec}"
            # A minute of the tone (a whole number of its waves a second), in both channels: the virtual PC listens a
            # few seconds after it starts, and SDL reads the file as the time goes by, from when QEMU starts.
            tone = tmp / f"tone-{controller}-{codec}.raw"
            one = b"".join(struct.pack("<hh", v, v) for v in
                           (int(round(amp * math.sin(2 * math.pi * freq * i / rate))) for i in range(rate)))
            tone.write_bytes(one * 60)
            env = {"SDL_AUDIODRIVER": "disk", "SDL_DISKAUDIOFILEIN": str(tone), "SDL_DISKAUDIOFILE": str(tmp / "played.raw")}
            extra = ("-audiodev", f"sdl,id=snd0,in.frequency={rate},in.channels=2,in.format=s16,out.frequency={rate}",
                     "-device", f"{controller},id=hda0", "-device", f"{codec},audiodev=snd0,bus=hda0.0")
            pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu, extra=extra, env=env)
            try:
                got = wait_for(pc.log, ("test: heard", "test: can't listen", "kernel stopped"), args.timeout)
                time.sleep(0.3)
            finally:
                pc.stop()
            text = pc.text()
            found = next((line for line in text.splitlines() if "kernel: sound" in line), "(no line about sound)")
            print(f"{label}: {found}")
            if what is None:
                if "test: can't listen: it has no microphone or line in" not in text:
                    problems.append(f"{label}: it didn't say it can't listen ({got or 'nothing said in time'})")
                continue
            if f"test: listening to {what}" not in text:
                problems.append(f"{label}: it didn't listen to {what}")
            heard = re.search(r"test: heard (\d+) samples from (.+?): root mean square (\d+), peak (\d+), (\d+) zero crossings", text)
            if not heard:
                problems.append(f"{label}: it didn't say what it heard ({got or 'nothing said in time'})")
                continue
            n, rms, peak, crossings = int(heard[1]), int(heard[3]), int(heard[4]), int(heard[5])
            seconds = n / rate
            heard_freq = crossings / 2 / seconds if n else 0
            print(f"{label}: heard {seconds:.2f} s of {heard[2]}: {heard_freq:.1f} Hz (fed {freq:g}), root mean square "
                  f"{rms} (fed {amp / math.sqrt(2):.0f}), peak {peak} (fed {amp})")
            if seconds < 2.0:
                problems.append(f"{label}: only {seconds:.2f} s heard")
            if abs(heard_freq - freq) > freq * 0.01:
                problems.append(f"{label}: heard {heard_freq:.1f} Hz, not {freq:g}")
            if rms == 0 or abs(20 * math.log10(rms / (amp / math.sqrt(2)))) > 0.5:
                problems.append(f"{label}: heard it at {rms} root mean square, not {amp / math.sqrt(2):.0f}")
            if abs(peak - amp) > amp * 0.01:
                problems.append(f"{label}: heard a peak of {peak}, not {amp}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
        build()  # never leave a test build in out/
    return problems


def power_check(args) -> list[str]:
    """The firmware's programs: a laptop's battery, AC adapter and lid (an extra table, whose readings change 15 seconds
    after the battery is first read), a processor plugged in (QEMU's own firmware handles that event with its _E02
    method, which notifies the processor's device), then the power button."""
    if not args.no_build:
        build()
    problems: list[str] = []
    tmp = Path(tempfile.mkdtemp())
    turned_off = False
    try:
        r = subprocess.run(["iasl", "-p", str(tmp / "power"), str(HERE / "aml/tests/power.asl")], capture_output=True, text=True)
        table = tmp / "power.aml"
        if r.returncode or not table.exists():
            return [f"iasl couldn't compile the test table: {r.stdout[-400:]}"]
        debug = tmp / "debugcon.txt"
        cpu_driver = f"{args.cpu}-x86_64-cpu"
        pc = VirtualPC(BUILT, size=args.size, memory=args.memory, cpu=args.cpu,
                       extra=("-acpitable", f"file={table}", "-smp", "1,maxcpus=2",
                              "-chardev", f"file,id=dbg,path={debug}", "-device", "isa-debugcon,iobase=0x403,chardev=dbg"))
        try:
            got = wait_for(pc.log, ("Chat Computer: ready", "stopped", "couldn't start"), args.timeout)
            if got != "Chat Computer: ready":
                problems.append("the kernel didn't say it was ready" + (f" (the log says {got!r})" if got else " in time"))
                return problems
            pc.settled_picture("power-1")
            # A processor plugged in: QEMU signals general-purpose event 2, which the firmware's _E02 handles.
            monitor(pc.mon, f"device_add {cpu_driver},id=cpu1,socket-id=0,core-id=1,thread-id=0")
            if not wait_for(pc.log, ("firmware: notified \\_SB_.CPUS.C001",), 20):
                problems.append("the processor plugged in wasn't handled (the firmware's _E02 didn't notify it)")
            # Unplugged, the battery low, the lid closed: read within the 10 s the kernel reads them every.
            if not wait_for(pc.log, ("The battery is low (9%)",), 60):
                problems.append("the battery's going low wasn't noticed")
            time.sleep(1)
            pc.settled_picture("power-2")
            monitor(pc.mon, "system_powerdown")
            if wait_for(pc.log, ("kernel: turning off",), 30):
                try:
                    pc.qemu.wait(timeout=20)
                    turned_off = True
                except subprocess.TimeoutExpired:
                    pass
        finally:
            pc.stop()
        text = pc.text()
        print(text)
        said = debug.read_text(errors="replace").split("\n") if debug.exists() else []
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    for words in ["firmware: 2 tables of programs", "power button: the fixed one",
                  "battery: 64%, charging (plugged in), full in about 1 h 26 min", "lid: open",
                  "firmware: unplugged", "firmware: Unplugged: on battery power, 9%.",
                  "firmware: The battery is low (9%): plug the PC in.", "firmware: lid closed",
                  "firmware: the power button was pressed",
                  "firmware: The power button was pressed: Chat Computer is turning the PC off."]:
        if words not in text:
            problems.append(f"the log doesn't say {words!r}")
    if "(0 problems)" not in text:
        problems.append("the firmware's programs had problems loading or starting (see the log)")
    if [l for l in said if l] != ["TTS 5", "PTS 5"]:
        problems.append(f"the firmware's _TTS(5) and _PTS(5) weren't run before turning off, in that order: {said!r}")
    if not turned_off:
        problems.append("the power button didn't turn the PC off (the virtual PC didn't end)")
    return problems


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--size", default="1920x1080", help="screen resolution the virtual monitor reports")
    ap.add_argument("--memory", default="512", help="memory in MiB")
    ap.add_argument("--cpu", default="max", help="QEMU CPU model")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--timeout", type=float, default=240)
    ap.add_argument("--crash", choices=["page-fault", "panic", "stack", "core"])
    ap.add_argument("--cores", type=int, default=2, help="the virtual PC's processor cores")
    ap.add_argument("--keyboard", action="store_true", help="USB keyboards: on a port, behind a hub, plugged in and out")
    ap.add_argument("--network", action="store_true", help="each network card there's a driver for: DHCP, DNS and TCP")
    ap.add_argument("--card", action="append", choices=[c[0] for c in CARDS], help="with --network: only this card")
    ap.add_argument("--model", action="store_true", help="the own model: a tiny one found, read, asked, its answer the engine's")
    ap.add_argument("--chat", action="store_true", help="a conversation answered by a test's script in place of the model")
    ap.add_argument("--web", action="store_true", help="the model's web tools, against a stand-in for its sites")
    ap.add_argument("--remember", action="store_true", help="the conversation kept across a restart, on NVMe and SATA")
    ap.add_argument("--sound", action="store_true", help="the timer's chime on QEMU's HD Audio controllers and codecs")
    ap.add_argument("--microphone", action="store_true", help="what QEMU's HD Audio inputs hear, fed a tone")
    ap.add_argument("--listen", action="store_true", help="speech recognition: Windows+H, a voice heard, the same as the engine on Linux")
    ap.add_argument("--files", action="store_true", help="Windows' drive (NTFS) read by the model's file tools")
    ap.add_argument("--bitlocker", action="store_true", help="Windows' drive encrypted with BitLocker, unlocked")
    ap.add_argument("--drives", action="store_true", help="the PC's other drives (NVMe and SATA) read, by Windows' letters")
    ap.add_argument("--usb", action="store_true", help="USB drives (sticks, a card reader) read as they come and go")
    ap.add_argument("--power", action="store_true", help="the firmware's programs: a battery, an AC adapter, a lid, the power button")
    ap.add_argument("--secure-boot", action="store_true")
    ap.add_argument("--signed", help="a signed BOOTX64.EFI to start with Secure Boot on")
    ap.add_argument("--cert", help="the certificate (PEM or DER) that signed it")
    args = ap.parse_args()
    global CORES
    CORES = max(1, args.cores)

    if args.crash:
        problems = crash(args.crash, args)
    elif args.keyboard:
        problems = keyboard(args)
    elif args.network:
        problems = network(args)
    elif args.model:
        problems = model_check(args)
    elif args.web:
        problems = web_check(args)
    elif args.chat:
        problems = chat(args) + chat_limits(args)
    elif args.remember:
        problems = remember(args)
    elif args.sound:
        problems = sound(args)
    elif args.microphone:
        problems = microphone(args)
    elif args.listen:
        problems = listen_check(args)
    elif args.files:
        problems = files_check(args)
    elif args.bitlocker:
        problems = bitlocker_check(args)
    elif args.drives:
        problems = drives_check(args)
    elif args.usb:
        problems = usb_check(args)
    elif args.power:
        problems = power_check(args)
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
