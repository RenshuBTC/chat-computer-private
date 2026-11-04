"""A stand-in for the Asahi installer (v0.9.2) for tests/test_mac_setup.py: the same questions, in the same
order and with the same colour codes as the real one's main.py and util.py (which this mirrors by hand), read
from the terminal the same way (/dev/tty, clearing anything typed ahead), on a pretend Mac. It records the
answers it got in $FAKE_INSTALLER_LOG, one JSON object per line.

  python3 mac_fake_installer.py [--no-resize] [--two-areas] [--small] [--bless-fails-once] [--crash] [--pretend-mac]

With --pretend-mac it also changes the pretend Mac's disk the way the real one would (tests/mac_pretend.py,
$PRETEND_MAC_STATE), and writes installer.log naming the partitions it made, as the real one does.
"""
import json
import os
import sys
import termios
import time

ARGS = set(sys.argv[1:])
PRETEND = "--pretend-mac" in ARGS
if PRETEND:
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import mac_pretend
TTYR = open("/dev/tty", "r")          # the terminal, as the real installer's install.sh makes stdin/stdout
TTY = open("/dev/tty", "w")
LOG = os.environ.get("FAKE_INSTALLER_LOG", "/dev/null")


def col(*c):
    return "\033[" + ";".join(map(str, c)) + "m"


def p_style(*args, color=(), end="\n"):
    TTY.write(col(*color) + " ".join(map(str, args)) + col() + end)
    TTY.flush()


def record(**kw):
    with open(LOG, "a") as f:
        f.write(json.dumps(kw) + "\n")


def flush():
    termios.tcflush(TTYR, termios.TCIFLUSH)


def read_line():
    line = TTYR.readline()
    if not line:
        sys.exit(1)
    return line.rstrip("\r\n")


def input_prompt(*args):
    p_style(f"{col(1, 37)}»{col(1, 36)}", *args, end="")
    return read_line()


def choice(prompt, options, default=None):
    for k, v in options.items():
        p_style(f"  {col(1)}{k}{col(22)}: {v}")
    if default:
        prompt += f" ({default})"
    while True:
        flush()
        res = input_prompt(prompt + ": ").strip() or (default or "")
        if res in options:
            TTY.write("\n")
            return res
        p_style(f"Enter one of the following: {', '.join(options)}", color=(1, 33))


def yesno(prompt):
    flush()
    res = input_prompt(prompt + " (y/N): ").strip()
    return res.lower() in ("y", "yes")


def enter(msg, color=(1, 36)):
    p_style(msg, color=color)
    flush()
    read_line()


def menu(actions, default):
    p_style("Collecting partition information...", color=(1, 35))
    p_style("  1: APFS [Macintosh HD] (494.38 GB, 6 volumes)")
    p_style("    OS: [B*] [Macintosh HD] macOS v15.6 [disk3s1, 1234]")
    p_style("Choose what to do:", color=(1, 36))
    act = choice("Action", actions, default)
    record(q="action", a=act)
    return act


p_style("Welcome to the Chat Computer installer!", color=(1,))
enter("Press enter to continue.")
time.sleep(0.05)
if "--crash" in ARGS:
    TTY.write("Traceback (most recent call last):\n  File \"main.py\", line 1\nOSError: something broke\n")
    TTY.flush()
    sys.exit(1)

resized = "--no-resize" in ARGS
while True:
    if resized:
        act = menu({"f": "Install an OS into free space", "r": "Resize an existing partition", "q": "Quit"}, "f")
    else:
        act = menu({"r": "Resize an existing partition to make space for a new OS", "q": "Quit"}, "r")
    if act == "q":
        sys.exit(0)
    if act == "r":
        p_style("We're going to resize this partition:", color=(1,))
        p_style("Enter the new size for your existing partition:", color=(1, 36))
        while True:
            flush()
            v = input_prompt("New size (50%): ").strip()
            record(q="new size", a=v)
            if v.endswith("B") and v[:-1].isdigit():
                break
            p_style(f"Invalid size '{v}'.", color=(1, 31))
            sys.exit(2)
        p_style("Resizing will free up 120.00 GB of space.", color=(1,))
        ok = yesno("Continue?")
        record(q="continue", a=ok)
        if not ok:
            continue
        if PRETEND:
            st = mac_pretend.load()
            mac_pretend.resize_macos(st, int(v[:-1]))
            mac_pretend.save(st)
        enter("Resize complete. Press enter to continue.", color=(1, 32))
        resized = True
        continue
    p_style("Choose an OS to install:", color=(1, 36))
    os_idx = choice("OS", {"1": "Chat Computer"})
    record(q="os", a=os_idx)
    p_style("Minimum required space for this OS: 14.13 GB", color=(1,))
    if "--small" in ARGS:
        p_style("There is not enough free space to install this OS.", color=(1, 31))
        enter("Press enter to go back to the main menu.", color=(1,))
        continue
    if "--two-areas" in ARGS:
        p_style("Choose a free area to install into:", color=(1, 36))
        choice("Target area", {"0": "(free space: 40 GB)", "3": "(free space: 120 GB)"})
    p_style("How much space should be allocated to the new OS?", color=(1, 36))
    flush()
    size = input_prompt("New OS size (max): ").strip()
    record(q="os size", a=size)
    p_style("Enter a name for your OS", color=(1, 36))
    flush()
    name = input_prompt("OS name (Chat Computer): ").strip() or "Chat Computer"
    record(q="name", a=name)
    p_style("Using macOS 14.8.3 for OS firmware", color=(1,))
    if PRETEND:
        st = mac_pretend.load()
        made = mac_pretend.install_os(st, int(size[:-1]), name)
        mac_pretend.save(st)
        with open("installer.log", "w") as f:
            for part, ptype in (("EFI", "EFI"), ("Boot", "Linux"), ("Root", "Linux")):
                f.write(f"10-16 21:00 root         INFO     Installing partition {{'name': '{part}', 'type': '{ptype}'}} "
                        f"-> {made[part]}\n")
    p_style("Installing stub macOS into disk0s3 (Chat Computer)", color=(1, 35))
    tries = 0
    while True:
        p_style("To continue the installation, you will need to enter your macOS", color=(1,))
        p_style("admin credentials.", color=(1,))
        # As Python's getpass does: echo off (which also clears what was typed ahead), then the question.
        old = termios.tcgetattr(TTYR)
        new = termios.tcgetattr(TTYR)
        new[3] &= ~termios.ECHO
        termios.tcsetattr(TTYR, termios.TCSAFLUSH, new)
        try:
            TTY.write("Password for sammy: ")
            TTY.flush()
            pw = read_line()
        finally:
            termios.tcsetattr(TTYR, termios.TCSAFLUSH, old)
            TTY.write("\n")
            TTY.flush()
        record(q="password", a=pw)
        tries += 1
        if "--bless-fails-once" in ARGS and tries == 1:
            enter("Failed to run bless. Press enter to try again.", color=(1, 31))
            continue
        break
    p_style("Installation successful!", color=(1, 32))
    enter("Press enter to continue.")
    p_style("When the system shuts down, follow these steps:", color=(1,))
    enter("Press enter to shut down the system.")
    record(q="shutdown", a=True)
    sys.exit(0)
