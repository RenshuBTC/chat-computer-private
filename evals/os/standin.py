"""Commands of the simulated computer that answer from its hardware (world.py) rather than this machine's: what a
real Chat Computer's would say about the same machine. Each is a tiny script in the PATH (machine.py) that runs

    python /opt/chatcomputer/app/evals/os/standin.py COMMAND ARGS...

Only the parts of each command people (and models) commonly use are here; anything else answers as the real
command does to an option it doesn't know."""
from __future__ import annotations

import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import world  # noqa: E402

GB = 1_000_000_000


def _human(nbytes: float) -> str:
    v = float(nbytes)
    for unit in ("B", "K", "M", "G", "T"):
        if v < 1024 or unit == "T":
            return (f"{v:.1f}" if v < 10 and unit != "B" else f"{v:.0f}") + unit
        v /= 1024
    return str(nbytes)


def _devices(m: dict) -> list[dict]:
    """The block devices, as lsblk sees them: the PC's own disk (Windows and Chat Computer on it) and any stick."""
    disks = [{"name": "nvme0n1", "size": 512 * GB, "rm": 0, "hotplug": 0, "tran": "nvme", "type": "disk",
              "model": "SAMSUNG MZVL2512HCJQ", "vendor": "", "label": None, "fstype": None, "mount": None,
              "children": [
                  {"name": "nvme0n1p1", "size": 260 * 2**20, "type": "part", "fstype": "vfat", "label": "SYSTEM",
                   "mount": "/boot/efi"},
                  {"name": "nvme0n1p2", "size": 16 * 2**20, "type": "part", "fstype": None, "label": None, "mount": None},
                  {"name": "nvme0n1p3", "size": 400 * GB, "type": "part", "fstype": "ntfs", "label": "Windows",
                   "mount": "/mnt/windows"},
                  {"name": "nvme0n1p4", "size": 64 * GB, "type": "part", "fstype": "ext4", "label": "ChatComputer",
                   "mount": "/"},
              ]}]
    for d in m.get("drives", []):
        if d.get("ejected"):
            continue          # powered off: gone from the list until it's plugged in again
        disks.append({"name": d["disk"], "size": d["size_gb"] * GB, "rm": 1, "hotplug": 1, "tran": "usb",
                      "type": "disk", "model": d.get("model", ""), "vendor": d.get("vendor", ""), "label": None,
                      "fstype": None, "mount": None,
                      "children": [{"name": d["id"], "size": d["size_gb"] * GB - 2**20, "type": "part",
                                    "fstype": d.get("fstype", "vfat"), "label": d["label"],
                                    "mount": str(world.MEDIA / d["label"]) if d.get("mounted") else None,
                                    "free": d["size_gb"] * GB - world.used_gb(d) * GB}]})
    for disk in disks:
        for c in disk.get("children", []):
            c.setdefault("rm", disk["rm"])
            c.setdefault("hotplug", disk["hotplug"])
            c.setdefault("tran", None)
            c.setdefault("model", None)
            c.setdefault("vendor", None)
            c["pkname"] = disk["name"]
    return disks


COLS = {"NAME", "KNAME", "PATH", "MAJ:MIN", "RM", "SIZE", "RO", "TYPE", "MOUNTPOINT", "MOUNTPOINTS", "LABEL",
        "FSTYPE", "UUID", "MODEL", "VENDOR", "TRAN", "HOTPLUG", "FSAVAIL", "FSUSE%", "FSSIZE", "PKNAME", "SERIAL",
        "PARTLABEL", "FSVER"}


def lsblk(args: list[str]) -> int:
    m = world.load()
    cols = ["NAME", "MAJ:MIN", "RM", "SIZE", "RO", "TYPE", "MOUNTPOINTS"]
    as_json = nbytes = noheadings = fullpath = nodeps = as_list = False
    wanted: list[str] = []
    it = iter(args)
    for a in it:
        if a in ("-J", "--json"):
            as_json = True
        elif a in ("-b", "--bytes"):
            nbytes = True
        elif a in ("-n", "--noheadings"):
            noheadings = True
        elif a in ("-p", "--paths"):
            fullpath = True
        elif a in ("-d", "--nodeps"):
            nodeps = True
        elif a in ("-l", "--list", "-r", "--raw", "-P", "--pairs"):
            as_list = True
        elif a in ("-f", "--fs"):
            cols = ["NAME", "FSTYPE", "FSVER", "LABEL", "UUID", "FSAVAIL", "FSUSE%", "MOUNTPOINTS"]
        elif a in ("-o", "--output") or a.startswith(("-o", "--output=")):
            val = a.split("=", 1)[1] if a.startswith("--output=") else (a[2:] if a.startswith("-o") and len(a) > 2 else next(it, ""))
            if val.startswith("+"):
                cols = cols + val[1:].split(",")
            else:
                cols = val.split(",")
        elif a in ("-a", "--all", "-e", "-x", "-T", "-i", "--ascii", "-S", "--scsi", "-A", "--noempty", "-s"):
            continue
        elif a.startswith("-"):
            print(f"lsblk: invalid option -- '{a.lstrip('-')[:1]}'\nTry 'lsblk --help' for more information.",
                  file=sys.stderr)
            return 1
        else:
            wanted.append(a)
    cols = [c.strip().upper() for c in cols if c.strip()]
    for c in cols:
        if c not in COLS:
            print(f"lsblk: unknown column: {c}", file=sys.stderr)
            return 1
    devs = _devices(m)

    def value(dev: dict, col: str):
        path = "/dev/" + dev["name"]
        free = dev.get("free")
        mnt = dev.get("mount")
        return {
            "NAME": path if fullpath else dev["name"], "KNAME": dev["name"], "PATH": path,
            "MAJ:MIN": "8:16" if dev["name"].startswith("sd") else "259:0",
            "RM": bool(dev["rm"]) if as_json else str(dev["rm"]), "HOTPLUG": bool(dev["hotplug"]) if as_json else str(dev["hotplug"]),
            "SIZE": int(dev["size"]) if nbytes else _human(dev["size"]), "RO": False if as_json else "0",
            "TYPE": dev["type"], "MOUNTPOINT": mnt, "MOUNTPOINTS": [mnt] if as_json else (mnt or ""),
            "LABEL": dev.get("label"), "PARTLABEL": None, "FSTYPE": dev.get("fstype"), "FSVER": "FAT32" if dev.get("fstype") == "vfat" else None,
            "UUID": ("5A1B-" + dev["name"][-2:].upper().rjust(4, "0")) if dev.get("fstype") else None,
            "MODEL": dev.get("model"), "VENDOR": dev.get("vendor"), "TRAN": dev.get("tran"), "SERIAL": None,
            "FSAVAIL": (_human(free) if free is not None and mnt else None), "FSUSE%": None, "FSSIZE": None,
            "PKNAME": dev.get("pkname"),
        }[col]

    rows: list[tuple[int, dict]] = []
    for disk in devs:
        if wanted and not any(w in ("/dev/" + disk["name"], disk["name"]) for w in wanted) and not any(
                w in ("/dev/" + c["name"], c["name"]) for c in disk.get("children", []) for w in wanted):
            continue
        rows.append((0, disk))
        if not nodeps:
            rows += [(1, c) for c in disk.get("children", [])]
    if wanted and not rows:
        print(f"lsblk: {wanted[0]}: not a block device", file=sys.stderr)
        return 32
    if as_json:
        def obj(dev: dict) -> dict:
            o = {c.lower(): value(dev, c) for c in cols}
            if dev.get("children") and not nodeps:
                o["children"] = [obj(c) for c in dev["children"]]
            return o
        print(json.dumps({"blockdevices": [obj(d) for lvl, d in rows if lvl == 0]}, indent=3))
        return 0
    table = [] if noheadings else [cols]
    kids_seen: dict[str, int] = {}
    for lvl, dev in rows:
        cells = []
        for c in cols:
            v = value(dev, c)
            v = "" if v is None else str(v)
            if c == "NAME" and lvl and not as_list:
                parent = dev["pkname"]
                siblings = next(d for d in devs if d["name"] == parent)["children"]
                kids_seen[parent] = kids_seen.get(parent, 0) + 1
                v = ("└─" if kids_seen[parent] == len(siblings) else "├─") + v
            cells.append(v)
        table.append(cells)
    widths = [max(len(r[i]) for r in table) for i in range(len(cols))]
    for r in table:
        print(" ".join(cell.ljust(widths[i]) if i < len(cols) - 1 else cell for i, cell in enumerate(r)).rstrip())
    return 0


def udisksctl(args: list[str]) -> int:
    if not args or args[0] in ("help", "--help", "-h"):
        print("Usage:\n  udisksctl COMMAND\n\nCommands:\n  help, info, dump, status, monitor, mount, unmount, unlock, "
              "lock, loop-setup, loop-delete, power-off, smart-simulate")
        return 0
    verb, rest = args[0], args[1:]
    dev = ""
    for i, a in enumerate(rest):
        if a in ("-b", "--block-device") and i + 1 < len(rest):
            dev = rest[i + 1]
        elif a.startswith("--block-device="):
            dev = a.split("=", 1)[1]
    with world.changing() as m:
        if verb == "status":
            print("MODEL                     REVISION  SERIAL               DEVICE\n" + "-" * 76)
            print("SAMSUNG MZVL2512HCJQ      GXA7801Q  S675NX0T123456       nvme0n1")
            for d in m.get("drives", []):
                if not d.get("ejected"):
                    print(f"{(d.get('vendor', '') + ' ' + d.get('model', '')).strip():<25} 1.00      "
                          f"{'4C530001230101112345':<20} {d['disk']}")
            return 0
        d = world.find_drive(m, dev) if dev else None
        if verb in ("mount", "unmount", "power-off", "info") and not dev:
            print(f"Error: Unknown or missing arguments for {verb}", file=sys.stderr)
            return 1
        if d is None or d.get("ejected"):
            print(f"Error looking up object for device {dev}", file=sys.stderr)
            return 1
        part = "/dev/" + d["id"]
        if verb == "mount":
            if d.get("mounted"):
                print(f"Error mounting {part}: GDBus.Error:org.freedesktop.UDisks2.Error.AlreadyMounted: Device "
                      f"{part} is already mounted at `{world.MEDIA / d['label']}'.", file=sys.stderr)
                return 1
            print(f"Mounted {part} at {world.mount(d)}")
            return 0
        if verb == "unmount":
            if not d.get("mounted"):
                print(f"Error unmounting {part}: GDBus.Error:org.freedesktop.UDisks2.Error.NotMounted: Device "
                      f"`{part}' is not mounted", file=sys.stderr)
                return 1
            world.unmount(d)
            print(f"Unmounted {part}.")
            return 0
        if verb == "power-off":
            if d.get("mounted"):
                print(f"Error powering off drive: GDBus.Error:org.freedesktop.UDisks2.Error.Failed: The drive in use: "
                      f"Device {part} is mounted", file=sys.stderr)
                return 1
            d["ejected"] = True
            return 0
        if verb == "info":
            print(f"/org/freedesktop/UDisks2/block_devices/{d['id']}:\n  org.freedesktop.UDisks2.Block:\n"
                  f"    Device:                     {part}\n    IdLabel:                    {d['label']}\n"
                  f"    IdType:                     {d.get('fstype', 'vfat')}\n    Size:                       {int(d['size_gb'] * GB)}\n"
                  f"  org.freedesktop.UDisks2.Filesystem:\n    MountPoints:                "
                  f"{world.MEDIA / d['label'] if d.get('mounted') else ''}")
            return 0
    print(f"Unknown command `{verb}'", file=sys.stderr)
    return 1


def _printers(m: dict) -> list[dict]:
    return m.get("printers", [])


def lpstat(args: list[str]) -> int:
    m = world.load()
    ps = _printers(m)
    flags = set("".join(a[1:] for a in args if a.startswith("-") and not a.startswith("--"))) or {"o"}
    if "t" in flags:
        flags |= set("rdvapo")
    if "s" in flags:
        flags |= set("dv")
    default = ps[0]["queue"] if len(ps) == 1 else None
    since = time.strftime("%a %d %b %Y %H:%M:%S", time.localtime(time.time() - 3600))
    lines: list[str] = []
    if "r" in flags:
        lines.append("scheduler is running")
    if "d" in flags:
        lines.append(f"system default destination: {default}" if default else "no system default destination")
    if not ps and flags & set("pave"):
        print("\n".join(lines)) if lines else None
        print("lpstat: No destinations added.", file=sys.stderr)
        return 1
    if "v" in flags:
        lines += [f"device for {p['queue']}: implicitclass://{p['queue']}/" for p in ps]
    if "a" in flags:
        lines += [f"{p['queue']} accepting requests since {since}" for p in ps]
    if "p" in flags:
        lines += [f"printer {p['queue']} is {p['state']}.  enabled since {since}" for p in ps]
    if "e" in flags:
        lines += [p["queue"] for p in ps]
    if "o" in flags or "W" in flags:
        lines += [f"{j['queue']}-{j['id']}  sam  {j.get('bytes', 20480)}   {since}" for j in m.get("print_jobs", [])
                  if not j.get("done")]
    if lines:
        print("\n".join(lines))
    return 0


def lp(args: list[str]) -> int:
    dest, copies, files, opts, pages = None, 1, [], [], ""
    it = iter(args)
    for a in it:
        if a == "-d":
            dest = next(it, None)
        elif a.startswith("-d") and len(a) > 2:
            dest = a[2:]
        elif a == "-n":
            copies = int(next(it, "1") or 1)
        elif a.startswith("-n") and len(a) > 2:
            copies = int(a[2:])
        elif a == "-o":
            opts.append(next(it, ""))
        elif a.startswith("-o") and len(a) > 2:
            opts.append(a[2:])
        elif a == "-P":
            pages = next(it, "")
        elif a in ("-t", "-H", "-q", "-U", "-h"):
            next(it, None)
        elif a in ("-c", "-m", "-s", "-w", "-E"):
            continue
        elif a == "--":
            files += list(it)
        elif a.startswith("-"):
            print(f"lp: Error - unknown option \"{a[1:]}\".", file=sys.stderr)
            return 1
        else:
            files.append(a)
    if not files:
        print("lp: Error - no files (this simulated computer doesn't print from a pipe).", file=sys.stderr)
        return 1
    with world.changing() as m:
        ps = _printers(m)
        if dest is None:
            if len(ps) != 1:
                print("lp: Error - No default destination.", file=sys.stderr)
                return 1
            dest = ps[0]["queue"]
        p = next((p for p in ps if p["queue"] == dest or p["name"] == dest), None)
        if p is None:
            print("lp: The printer or class does not exist.", file=sys.stderr)
            return 1
        for f in files:
            if not os.path.isfile(f):
                print(f"lp: Error - unable to access \"{f}\" - No such file or directory", file=sys.stderr)
                return 1
        n = len(m.setdefault("print_jobs", [])) + 1
        for f in files:
            m["print_jobs"].append({"id": n, "queue": p["queue"], "printer": p["name"], "file": os.path.basename(f),
                                    "path": os.path.abspath(f), "copies": copies, "options": opts, "pages": pages,
                                    "via": "lp", "bytes": os.path.getsize(f)})
        print(f"request id is {p['queue']}-{n} ({len(files)} file(s))")
    return 0


def cancel(args: list[str]) -> int:
    with world.changing() as m:
        jobs = m.get("print_jobs", [])
        if "-a" in args:
            for j in jobs:
                j["done"] = True
            return 0
        for a in args:
            for j in jobs:
                if a in (f"{j['queue']}-{j['id']}", str(j["id"])):
                    j["done"] = True
                    return 0
        print(f"cancel: Job #{args[-1] if args else ''} does not exist.", file=sys.stderr)
        return 1


def setxkbmap(args: list[str]) -> int:
    if "-query" in args or "-print" in args:
        m = world.load()
        print(f"rules:      evdev\nmodel:      pc105\nlayout:     {m.get('keyboard_layout', 'us')}")
        if m.get("keyboard_variant"):
            print(f"variant:    {m['keyboard_variant']}")
        return 0
    layout, variant = None, ""
    it = iter(args)
    rest = []
    for a in it:
        if a == "-layout":
            layout = next(it, None)
        elif a == "-variant":
            variant = next(it, "") or ""
        elif a in ("-display", "-model", "-option", "-rules", "-keycodes", "-geometry", "-symbols", "-types",
                   "-compat", "-keymap", "-device", "-I", "-v"):
            next(it, None)
        elif a.startswith("-"):
            continue
        else:
            rest.append(a)
    if layout is None and rest:
        layout = rest[0]
        variant = rest[1] if len(rest) > 1 else variant
    if not layout:
        return 0
    base = layout.split(",")[0].split("(")[0]
    if base not in world.KEYBOARD_LAYOUTS:
        print("Error loading new keyboard description", file=sys.stderr)
        return 1
    with world.changing() as m:
        m["keyboard_layout"], m["keyboard_variant"] = base, variant
    return 0


def localectl(args: list[str]) -> int:
    if not args or args[0] == "status":
        print("System Locale: LANG=en_GB.UTF-8\n    VC Keymap: (unset)\n   X11 Layout: us\n    X11 Model: pc105")
        return 0
    if args[0] == "list-x11-keymap-layouts":
        print("\n".join(sorted(world.KEYBOARD_LAYOUTS)))
        return 0
    if args[0].startswith("set-"):
        print("Failed to set keymap: Access denied", file=sys.stderr)    # needs an administrator (polkit)
        return 1
    print(f"Unknown command verb '{args[0]}'.", file=sys.stderr)
    return 1


def timedatectl(args: list[str]) -> int:
    zone = world.load().get("timezone", "Asia/Singapore")
    verb = args[0] if args and not args[0].startswith("-") else "status"
    if verb == "status":
        now = time.localtime()
        utc = time.gmtime()
        print(f"               Local time: {time.strftime('%a %Y-%m-%d %H:%M:%S %Z', now)}\n"
              f"           Universal time: {time.strftime('%a %Y-%m-%d %H:%M:%S UTC', utc)}\n"
              f"                 RTC time: {time.strftime('%a %Y-%m-%d %H:%M:%S', utc)}\n"
              f"                Time zone: {zone} ({time.strftime('%Z, %z', now)})\n"
              "System clock synchronized: yes\n              NTP service: active\n          RTC in local TZ: no")
        return 0
    if verb == "show":
        if "--value" in args:
            print(zone)
        else:
            print(f"Timezone={zone}\nLocalRTC=no\nCanNTP=yes\nNTP=yes\nNTPSynchronized=yes")
        return 0
    if verb == "list-timezones":
        zones = []
        for tab in ("zone1970.tab", "zone.tab"):
            try:
                zones = [ln.split("\t")[2] for ln in (world.ZONEINFO / tab).read_text().splitlines()
                         if ln and not ln.startswith("#")]
                break
            except (OSError, IndexError):
                continue
        print("\n".join(sorted(set(zones + ["UTC"]))))
        return 0
    if verb.startswith("set-"):
        print("Failed to set time zone: Access denied", file=sys.stderr)    # an administrator's (polkit)
        return 1
    print(f"Unknown command verb '{verb}'.", file=sys.stderr)
    return 1


def brightnessctl(args: list[str]) -> int:
    with world.changing() as m:
        b = m.get("backlight")
        if b is None:
            print("Failed to find a backlight device.", file=sys.stderr)       # a desktop: the monitor has its own
            return 1
        mx = 19200
        pos = [a for a in args if not a.startswith("-")]
        if pos and pos[0] in ("s", "set") and len(pos) > 1:
            v = pos[1]
            num = float(v.rstrip("%+-").lstrip("+-") or 0)
            pct = num if v.endswith("%") or "%" in v else num * 100 / mx
            if v.endswith("-") or v.startswith("-"):
                b = b - pct
            elif v.endswith("+") or v.startswith("+"):
                b = b + pct
            else:
                b = pct
            m["backlight"] = int(round(max(1, min(100, b))))
        cur = int(round(m["backlight"] * mx / 100))
        if "-m" in args or "--machine-readable" in args:
            print(f"intel_backlight,backlight,{cur},{m['backlight']}%,{mx}")
        else:
            print(f"Device 'intel_backlight' of class 'backlight':\n\tCurrent brightness: {cur} ({m['backlight']}%)\n"
                  f"\tMax brightness: {mx}")
        return 0


def ddcutil(args: list[str]) -> int:
    print("No displays found.")          # a laptop's own panel doesn't speak DDC/CI, nor does this case's monitor
    return 1 if args and args[0] in ("getvcp", "setvcp") else 0


COMMANDS = {"lsblk": lsblk, "udisksctl": udisksctl, "lpstat": lpstat, "lp": lp, "cancel": cancel,
            "setxkbmap": setxkbmap, "localectl": localectl, "timedatectl": timedatectl,
            "brightnessctl": brightnessctl, "ddcutil": ddcutil}

if __name__ == "__main__":
    sys.exit(COMMANDS[sys.argv[1]](sys.argv[2:]))
