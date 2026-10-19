"""A pretend Mac's disk, for tests/test_mac_setup.py: stand-ins for diskutil and plutil that keep the disk's
partitions in a JSON file ($PRETEND_MAC_STATE), and the disk changes the Asahi installer makes (used by
mac_fake_installer.py --pretend-mac). Each partition with a file system has a folder (its contents) under the
state's volumes folder; "mounted" only decides whether diskutil reports a mount point.

Run as `python3 mac_pretend.py diskutil|plutil ARGS...` (the tests' stand-ins do), or imported."""
import json
import os
import plistlib
import sys
import uuid

GB = 1000 ** 3
STUB = int(2.5 * GB)


def load():
    with open(os.environ["PRETEND_MAC_STATE"]) as f:
        return json.load(f)


def save(st):
    with open(os.environ["PRETEND_MAC_STATE"], "w") as f:
        json.dump(st, f, indent=1)


def new_mac(path, volumes, total=494 * GB, free=300 * GB, min_pref=150 * GB, recovery=5 * GB):
    """A Mac with just macOS: its APFS container (disk3, on disk0s2) and Apple's recovery container."""
    st = {"volumes": str(volumes), "next": 4, "calls": [],
          "macos": {"container": "disk3", "store": "disk0s2", "system": "disk3s1", "free": free, "min_pref": min_pref},
          "parts": [{"id": "disk0s1", "content": "Apple_APFS_ISC", "size": 524288000},
                    {"id": "disk0s2", "content": "Apple_APFS", "size": total, "apfs": "macos"},
                    {"id": "disk0s3", "content": "Apple_APFS_Recovery", "size": recovery}]}
    with open(path, "w") as f:
        json.dump(st, f, indent=1)
    return st


def _new_id(st):
    n = st["next"]
    st["next"] = n + 1
    return f"disk0s{n}"


def _index(st, pid):
    for i, p in enumerate(st["parts"]):
        if p.get("id") == pid:
            return i
    raise SystemExit(f"pretend diskutil: no partition {pid}")


def _merge_free(st):
    out = []
    for p in st["parts"]:
        if p.get("free") and out and out[-1].get("free"):
            out[-1]["size"] += p["size"]
        elif not (p.get("free") and p["size"] <= 0):
            out.append(p)
    st["parts"] = out


def resize_macos(st, new_size):
    i = _index(st, st["macos"]["store"])
    p = st["parts"][i]
    delta = p["size"] - new_size
    p["size"] = new_size
    st["macos"]["free"] -= delta
    st["parts"].insert(i + 1, {"free": True, "size": delta})
    _merge_free(st)


def folder(st, name):
    d = os.path.join(st["volumes"], name)
    os.makedirs(d, exist_ok=True)
    return d


def install_os(st, os_size, name="Chat Computer"):
    """What the Asahi installer does with 'f': a stub APFS container, then EFI, Boot and Root (Root takes the
    rest of the OS's space) at the start of the free space after macOS. Returns {part name: id}."""
    i = _index(st, st["macos"]["store"]) + 1
    gap = st["parts"][i]
    assert gap.get("free") and gap["size"] >= os_size, ("no room", gap, os_size)
    made = []
    for part, content, size, extra in (("Stub", "Apple_APFS", STUB, {"apfs": "stub", "stub_name": name}),
                                       ("EFI", "EFI", 524288000, {"name": "EFI - CHAT", "fs": True}),
                                       ("Boot", "Linux Filesystem", 1073741824, {}),
                                       ("Root", "Linux Filesystem", os_size - STUB - 524288000 - 1073741824, {})):
        p = {"id": _new_id(st), "content": content, "size": size, "uuid": str(uuid.uuid4()).upper(), **extra}
        made.append((part, p))
    gap["size"] -= os_size
    st["parts"][i:i] = [p for _, p in made]
    _merge_free(st)
    esp = dict(made)["EFI"]
    d = folder(st, esp["name"])
    os.makedirs(os.path.join(d, "EFI/fedora"), exist_ok=True)
    with open(os.path.join(d, "EFI/fedora/grub.cfg"), "w") as f:
        f.write("search --fs-uuid --set=root 65e9d770-9bc9-4b3f-9877-10347fb284e5\nset prefix=($root)/grub2\n"
                "configfile ($root)/grub2/grub.cfg\n")
    esp["mounted"] = True
    return {k: p["id"] for k, p in made}


def _containers(st):
    """The APFS containers, as `diskutil apfs list -plist` lists them: macOS's, then each start-up volume's."""
    m = st["macos"]
    mac = st["parts"][_index(st, m["store"])]
    out = [{"ContainerReference": m["container"], "DesignatedPhysicalStore": m["store"],
            "PhysicalStores": [{"DeviceIdentifier": m["store"], "Size": mac["size"]}],
            "CapacityCeiling": mac["size"], "CapacityFree": m["free"],
            "Volumes": [{"Name": "Macintosh HD", "DeviceIdentifier": m["system"], "Roles": ["System"]},
                        {"Name": "Preboot", "DeviceIdentifier": "disk3s2", "Roles": ["Preboot"]},
                        {"Name": "Macintosh HD - Data", "DeviceIdentifier": "disk3s5", "Roles": ["Data"]}]}]
    n = 5
    for p in st["parts"]:
        if p.get("apfs") == "stub":
            ref = f"disk{n}"
            out.append({"ContainerReference": ref, "DesignatedPhysicalStore": p["id"],
                        "PhysicalStores": [{"DeviceIdentifier": p["id"], "Size": p["size"]}],
                        "CapacityCeiling": p["size"], "CapacityFree": 0,
                        "Volumes": [{"Name": p["stub_name"] + " - Data", "DeviceIdentifier": ref + "s2", "Roles": ["Data"]},
                                    {"Name": p["stub_name"], "DeviceIdentifier": ref + "s1", "Roles": ["System"]}]})
            n += 1
    return out


def _plist(obj):
    sys.stdout.buffer.write(plistlib.dumps(obj))


def _mount_point(st, p):
    if p.get("fs") and p.get("mounted"):
        return os.path.join(st["volumes"], p["name"])
    return None


def diskutil(args):
    st = load()
    st["calls"].append(" ".join(args))
    save(st)
    a = list(args)
    m = st["macos"]
    if a[:3] == ["info", "-plist", "/"]:
        _plist({"APFSContainerReference": m["container"], "APFSPhysicalStores": [{"APFSPhysicalStore": m["store"]}],
                "DeviceIdentifier": m["system"] + "s1", "MountPoint": "/"})
        return 0
    if a[:2] == ["info", "-plist"]:
        p = st["parts"][_index(st, a[2])]
        out = {"DeviceIdentifier": p["id"], "DiskUUID": p.get("uuid", ""), "Size": p["size"]}
        mp = _mount_point(st, p)
        if mp:
            out["MountPoint"] = mp
            out["FreeSpace"] = p["size"] // 2
        if p.get("name"):
            out["VolumeName"] = p["name"]
        _plist(out)
        return 0
    if a[:3] == ["apfs", "list", "-plist"]:
        # Like the real one (seen on GitHub's Macs): every container, or one by its own name (disk3), not by
        # the partition it's on.
        found = _containers(st)
        if len(a) == 3:
            _plist({"Containers": found})
            return 0
        found = [c for c in found if c["ContainerReference"] == a[3]]
        if not found:
            print(f"pretend diskutil: {a[3]} is not an APFS Container", file=sys.stderr)
            return 1
        _plist({"Containers": found})
        return 0
    if a[:2] == ["apfs", "resizeContainer"] and a[3:5] == ["limits", "-plist"]:
        i = _index(st, m["store"])
        mac = st["parts"][i]
        gap = st["parts"][i + 1]["size"] if i + 1 < len(st["parts"]) and st["parts"][i + 1].get("free") else 0
        _plist({"CurrentSize": mac["size"], "MinimumSizePreferred": m["min_pref"], "MaximumSize": mac["size"] + gap,
                "Type": "APFSPhysicalStore"})
        return 0
    if a[:2] == ["apfs", "resizeContainer"] and a[3] == "0":
        if st.get("resize_fails", 0) > 0:
            st["resize_fails"] -= 1
            save(st)
            print("Error: -69519: The target disk is too small for this operation", file=sys.stderr)
            return 1
        i = _index(st, a[2])
        if i + 1 < len(st["parts"]) and st["parts"][i + 1].get("free"):
            grow = st["parts"][i + 1]["size"]
            st["parts"][i]["size"] += grow
            m["free"] += grow
            del st["parts"][i + 1]
        save(st)
        return 0
    if a[:2] == ["apfs", "deleteContainer"]:
        p = st["parts"][_index(st, a[2])]
        st["parts"][_index(st, a[2])] = {"free": True, "size": p["size"]}
        _merge_free(st)
        save(st)
        return 0
    if a[0] == "addPartition":
        prev, fmt, name, size = a[1:5]
        i = _index(st, prev) + 1
        gap = st["parts"][i] if i < len(st["parts"]) else {}
        if not gap.get("free") or size != "0":
            print("pretend diskutil: no free space after", prev, file=sys.stderr)
            return 1
        p = {"id": _new_id(st), "content": "Microsoft Basic Data", "size": gap["size"], "name": name, "fs": True,
             "mounted": True, "uuid": str(uuid.uuid4()).upper()}
        st["parts"][i] = p
        folder(st, name)
        save(st)
        print(f"Finished partitioning on disk0\n{p['id']} {name}")
        return 0
    if a[0] == "list" and a[1] == "-plist":
        parts = []
        for p in st["parts"]:
            if p.get("free"):
                continue
            e = {"Content": p["content"], "DeviceIdentifier": p["id"], "Size": p["size"]}
            if p.get("uuid"):
                e["DiskUUID"] = p["uuid"]
            if p.get("name"):
                e["VolumeName"] = p["name"]
            parts.append(e)
        _plist({"AllDisksAndPartitions": [{"DeviceIdentifier": "disk0", "Partitions": parts}]})
        return 0
    if a[0] == "mount":
        p = st["parts"][_index(st, a[1])]
        if not p.get("fs"):
            return 1
        p["mounted"] = True
        folder(st, p["name"])
        save(st)
        return 0
    if a[0] == "unmount":
        pid = a[-1]
        st["parts"][_index(st, pid)]["mounted"] = False
        save(st)
        return 0
    if a[:3] == ["eraseVolume", "free", "free"]:
        i = _index(st, a[3])
        st["parts"][i] = {"free": True, "size": st["parts"][i]["size"]}
        _merge_free(st)
        save(st)
        return 0
    print("pretend diskutil: unknown " + " ".join(a), file=sys.stderr)
    return 1


def plutil(args):
    # plutil -extract KEY.PATH raw -o - FILE
    if len(args) != 6 or args[0] != "-extract" or args[2] != "raw" or args[3:5] != ["-o", "-"]:
        print("pretend plutil: unsupported " + " ".join(args), file=sys.stderr)
        return 1
    with open(args[5], "rb") as f:
        try:
            v = plistlib.load(f)
        except Exception:
            return 1
    for k in args[1].split("."):
        if isinstance(v, list):
            if not k.isdigit() or int(k) >= len(v):
                return 1
            v = v[int(k)]
        elif isinstance(v, dict) and k in v:
            v = v[k]
        else:
            return 1
    if isinstance(v, (dict, list)):
        return 1
    print(str(v).lower() if isinstance(v, bool) else v)
    return 0


if __name__ == "__main__":
    # mac_pretend.py diskutil|plutil ARGS...
    tool, rest = sys.argv[1], sys.argv[2:]
    sys.exit(diskutil(rest) if tool == "diskutil" else plutil(rest))
