"""What the eval's sandbox looks like from the inside, so the model meets the commands a real Chat Computer has (Ubuntu
Server 24.04 and the packages that version's linux/install-chatcomputer.sh adds) and not this machine's (no
ImageMagick, scrot, ffmpeg, Node, compilers...). Commands are linked into a folder that is the whole PATH. A few
that a real one has and this machine doesn't are stand-ins that answer like a healthy home network and PC would,
and those about the hardware (drives, printers, keyboard layout, backlight) answer from the simulated machine
(standin.py, world.py)."""
from __future__ import annotations

import os
import re
import shutil
from pathlib import Path

# Ubuntu Server's own, and Chat Computer's packages (x11-xserver-utils, xdotool, wmctrl, xinput, pipewire, bluez,
# network-manager, poppler-utils, espeak-ng...).
APPLIANCE = """
bash sh dash ls cat cp mv rm mkdir rmdir ln touch chmod chown chgrp stat du df date sleep echo printf env which
head tail sort uniq wc cut tr tee seq basename dirname realpath readlink mktemp id whoami groups uname hostname nproc
yes true false test [ expr xargs find grep egrep fgrep sed awk mawk diff cmp comm paste join fold fmt nl od hexdump
file tar gzip gunzip zcat bzip2 bunzip2 xz unxz less more nano vi ps top kill killall pkill pgrep uptime free vmstat
watch lsof timeout nohup nice ionice flock install split tac rev md5sum sha256sum base64
systemctl journalctl hostnamectl loginctl busctl systemd-run resolvectl
mount umount findmnt blkid lscpu dmesg logger
curl wget
xset xsetroot xrandr xinput xdotool wmctrl
wpctl pw-cli pw-play pw-record pw-cat espeak-ng
pdftoppm pdftotext pdfinfo
python3 git
apt apt-get apt-cache dpkg dpkg-query
""".split()

STANDINS = {
    "ping": "#!/bin/sh\nhost=\"$(eval echo \\${$#})\"\necho \"PING $host 56(84) bytes of data.\"\n"
            "echo \"--- $host ping statistics ---\"\necho \"4 packets transmitted, 4 received, 0% packet loss, time 3004ms\"\n"
            "echo \"rtt min/avg/max/mdev = 8.112/9.420/11.031/1.101 ms\"\n",
    "ip": "#!/bin/sh\ncase \"$*\" in\n *route*) echo 'default via 192.168.1.1 dev wlan0 proto dhcp metric 600'; "
          "echo '192.168.1.0/24 dev wlan0 proto kernel scope link src 192.168.1.23 metric 600';;\n"
          " *) echo '2: wlan0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 state UP'; echo '    inet 192.168.1.23/24 scope global wlan0';;\nesac\n",
    "lspci": "#!/bin/sh\necho '00:02.0 VGA compatible controller: Intel Corporation Alder Lake-P GT2 [Iris Xe Graphics]'\n"
             "echo '00:14.3 Network controller: Intel Corporation Alder Lake-P PCH CNVi WiFi'\n",
    "sudo": "#!/bin/sh\necho 'sudo: a password is required' >&2\nexit 1\n",
}

# Commands that answer from the simulated machine, and the package each comes in (None: every Chat Computer has it).
HARDWARE = {"lsblk": None, "setxkbmap": None, "localectl": None, "timedatectl": None,
            "udisksctl": "udisks2", "lp": "cups", "lpstat": "cups", "cancel": "cups", "ddcutil": "ddcutil",
            "brightnessctl": "brightnessctl"}
# A package installed during a case (EvalPlatform.install) brings these commands.
PACKAGE_COMMANDS = {"udisks2": ["udisksctl"], "cups": ["lp", "lpstat", "cancel"], "cups-client": ["lp", "lpstat", "cancel"],
                    "ddcutil": ["ddcutil"], "brightnessctl": ["brightnessctl"]}
STANDIN_PY = "/opt/chatcomputer/app/evals/os/standin.py"


def hardware_command(cmd: str) -> str:
    return f"#!/bin/sh\nexec /opt/chatcomputer/venv/bin/python {STANDIN_PY} {cmd} \"$@\"\n"


def installer_packages(app: Path) -> set[str]:
    """The packages that version of Chat Computer's Linux installer puts on a PC (its Ubuntu list)."""
    try:
        text = (app / "linux" / "install-chatcomputer.sh").read_text()
    except OSError:
        return set()
    blocks = re.findall(r"^PACKAGES=\((.*?)^\)", text, re.S | re.M)
    ubuntu = blocks[-1] if blocks else ""
    return {pkg for line in ubuntu.splitlines() for pkg in line.split("#")[0].split()}


def build_bin(folder: Path, app: Path | None = None) -> list[str]:
    """Fill `folder` with the appliance's commands (links to this machine's) and the stand-ins, for the Chat
    Computer in `app`. Returns what's missing here (a real Chat Computer has it; the eval's machine doesn't)."""
    folder.mkdir(parents=True, exist_ok=True)
    missing = []
    for cmd in APPLIANCE:
        target = shutil.which(cmd, path="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
        link = folder / cmd
        if link.exists() or link.is_symlink():
            continue
        if target:
            os.symlink(target, link)
        else:
            missing.append(cmd)
    for cmd, script in STANDINS.items():
        p = folder / cmd
        p.write_text(script)
        p.chmod(0o755)
    packages = installer_packages(app) if app else set()
    for cmd, pkg in HARDWARE.items():
        p = folder / cmd
        if p.exists() or p.is_symlink():
            p.unlink()
        if pkg is None or pkg in packages:
            p.write_text(hardware_command(cmd))
            p.chmod(0o755)
    return missing
