#!/usr/bin/env python3
"""Makes the hives the registry crate's tests read (tests/*.hiv), with hivex (the registry library libguestfs edits
Windows' registries with): a hive with nothing in it is written here (a header, a bin, a security cell and the root
key), then hivex adds keys and values to it, as Windows' SYSTEM hive has them: MountedDevices with drive letters
(GPT partitions, MBR partitions, a USB stick's, volumes' names), a control set with values of every type, names in
other alphabets and of every case, a key with hundreds of subkeys (so its list is big), and data longer than a cell.
And a hive written here with data in pieces, as Windows keeps data that long. Each hive's keys and values, as hivex
reads them back, are listed in tests/<hive>.txt for the tests to check against; the hives are kept compressed (xz).

    python3 tests/make.py

Needs python3-hivex (Python's hivex module; on Ubuntu 24.04, for its own Python 3.12)."""
from __future__ import annotations

import struct
import subprocess
import sys
import time
from pathlib import Path

import hivex

HERE = Path(__file__).resolve().parent


def filetime() -> int:
    return int(time.time() * 10_000_000) + 116_444_736_000_000_000


def cell(body: bytes) -> bytes:
    """A cell in use: its size (negative), then what it holds, to a multiple of 8 bytes."""
    size = (4 + len(body) + 7) // 8 * 8
    return struct.pack("<i", -size) + body.ljust(size - 4, b"\0")


def empty_hive(name: str) -> bytes:
    """A hive with only its root key: the header, one bin with a security cell, the root key, and the rest free."""
    sk_at = 0x20
    descriptor = bytes([1, 0, 0x04, 0x80]) + bytes(16)  # self-relative; no owner, group or lists
    sk = cell(b"sk\0\0" + struct.pack("<IIII", sk_at, sk_at, 1, len(descriptor)) + descriptor)
    nk_at = sk_at + len(sk)
    root = name.encode()
    nk = cell(b"nk" + struct.pack("<HQII", 0x2C, filetime(), 0, 0)
              + struct.pack("<IIIIIIII", 0, 0, 0xFFFFFFFF, 0xFFFFFFFF, 0, 0xFFFFFFFF, sk_at, 0xFFFFFFFF)
              + struct.pack("<IIIII", 0, 0, 0, 0, 0) + struct.pack("<HH", len(root), 0) + root)
    bin_ = bytearray(4096)
    bin_[:32] = b"hbin" + struct.pack("<II", 0, 4096) + bytes(20)
    bin_[sk_at:sk_at + len(sk)] = sk
    bin_[nk_at:nk_at + len(nk)] = nk
    free_at = nk_at + len(nk)
    bin_[free_at:free_at + 4] = struct.pack("<i", 4096 - free_at)
    base = bytearray(4096)
    base[:0x30] = b"regf" + struct.pack("<IIQIIIIIII", 1, 1, filetime(), 1, 5, 0, 1, nk_at, 4096, 1)
    base[0x30:0x30 + 64] = "\\SystemRoot\\System32\\Config\\SYSTEM".encode("utf-16-le")[:64]
    checksum = 0
    for i in range(0, 508, 4):
        checksum ^= struct.unpack_from("<I", base, i)[0]
    base[0x1FC:0x200] = struct.pack("<I", checksum)
    return bytes(base) + bytes(bin_)


def guid(text: str) -> bytes:
    """A GUID's 16 bytes as Windows stores one (its first three parts little-endian)."""
    a, b, c, d, e = text.split("-")
    return struct.pack("<IHH", int(a, 16), int(b, 16), int(c, 16)) + bytes.fromhex(d + e)


def sz(text: str) -> bytes:
    return (text + "\0").encode("utf-16-le")


# The partitions the drive letters are for: check.py's disks use the same GUIDs and MBR signature.
WINDOWS_PARTITION = "b0e1f3a2-5c4d-4e6f-8a9b-0c1d2e3f4a5b"
DATA_PARTITION = "c1f2a4b3-6d5e-4f70-9bac-1d2e3f4a5b6c"
MBR_SIGNATURE, MBR_OFFSET = 0x5EEDCAFE, 1 << 20


def system(path: Path) -> None:
    path.write_bytes(empty_hive("ROOT"))
    h = hivex.Hivex(str(path), write=True)
    root = h.root()

    def key(parent: int, name: str) -> int:
        return h.node_add_child(parent, name)

    mounted = key(root, "MountedDevices")
    usb = sz("_??_USBSTOR#Disk&Ven_SanDisk&Prod_Ultra&Rev_1.00#4C530001234567891234&0#{53f56307-b6bf-11d0-94f2-00a0c91efb8b}")
    h.node_set_values(mounted, [
        {"key": "\\DosDevices\\C:", "t": 3, "value": b"DMIO:ID:" + guid(WINDOWS_PARTITION)},
        {"key": "\\DosDevices\\D:", "t": 3, "value": b"DMIO:ID:" + guid(DATA_PARTITION)},
        {"key": "\\DosDevices\\E:", "t": 3, "value": struct.pack("<IQ", MBR_SIGNATURE, MBR_OFFSET)},
        {"key": "\\DosDevices\\F:", "t": 3, "value": usb},
        {"key": "\\??\\Volume{6f1e2d3c-4b5a-11ef-9a8b-806e6f6e6963}", "t": 3, "value": b"DMIO:ID:" + guid(WINDOWS_PARTITION)},
        {"key": "\\??\\Volume{6f1e2d3d-4b5a-11ef-9a8b-806e6f6e6963}", "t": 3, "value": usb},
        {"key": "#{8a2c5d10-3f4e-11ef-9a8b-806e6f6e6963}", "t": 3, "value": bytes(range(16))},
    ])
    select = key(root, "Select")
    h.node_set_values(select, [{"key": k, "t": 4, "value": struct.pack("<I", n)}
                               for k, n in (("Current", 1), ("Default", 1), ("Failed", 0), ("LastKnownGood", 1))])
    control = key(key(root, "ControlSet001"), "Control")
    tz = key(control, "TimeZoneInformation")
    h.node_set_values(tz, [
        {"key": "Bias", "t": 4, "value": struct.pack("<i", -60)},
        {"key": "TimeZoneKeyName", "t": 1, "value": sz("W. Europe Standard Time")},
        {"key": "DynamicDaylightTimeDisabled", "t": 4, "value": struct.pack("<I", 0)},
        {"key": "StandardStart", "t": 3, "value": bytes([0, 0, 10, 0, 5, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0])},
        {"key": "ActiveTimeBias", "t": 4, "value": struct.pack("<i", -120)},
    ])
    kinds = key(control, "Every kind")
    h.node_set_values(kinds, [
        {"key": "", "t": 1, "value": sz("the default value")},
        {"key": "none", "t": 0, "value": b""},
        {"key": "expand", "t": 2, "value": sz("%SystemRoot%\\System32")},
        {"key": "binary", "t": 3, "value": bytes(range(256)) * 3},
        {"key": "dword", "t": 4, "value": struct.pack("<I", 0xDEADBEEF)},
        {"key": "dword big-endian", "t": 5, "value": struct.pack(">I", 7)},
        {"key": "multi", "t": 7, "value": sz("one") + sz("two") + sz("three") + b"\0\0"},
        {"key": "qword", "t": 11, "value": struct.pack("<Q", 1 << 40)},
        {"key": "tiny", "t": 3, "value": b"ab"},
        {"key": "Ünïcödé ναμε 名前", "t": 1, "value": sz("Grüße, κόσμε, 世界")},
        {"key": "long", "t": 3, "value": bytes((i * 7) & 0xFF for i in range(40_000))},
    ])
    key(control, "Ελληνικά")
    key(control, "MiXeD CaSe")
    many = key(control, "Class")
    for i in range(700):
        key(many, f"{{4d36e{i:03x}-e325-11ce-bfc1-08002be10318}}")
    h.commit(str(path))


def big_data_hive(path: Path) -> None:
    """A hive written here whose value of 40,000 bytes is kept as Windows keeps data that long: in pieces of at most
    16,344 bytes ("db": how many, and where the list of them is), which hivex reads (but writes in one cell)."""
    data = bytes((i * 13 + 5) & 0xFF for i in range(40_000))
    pieces = [data[i:i + 16_344] for i in range(0, len(data), 16_344)]
    cells: list[bytes] = []
    at = 0x20

    def put(body: bytes) -> int:
        nonlocal at
        c = cell(body)
        cells.append(c)
        where = at
        at += len(c)
        return where

    descriptor = bytes([1, 0, 0x04, 0x80]) + bytes(16)
    sk_at = put(b"sk\0\0" + struct.pack("<IIII", 0x20, 0x20, 1, len(descriptor)) + descriptor)
    piece_at = [put(p) for p in pieces]
    list_at = put(b"".join(struct.pack("<I", o) for o in piece_at))
    db_at = put(b"db" + struct.pack("<HI", len(pieces), list_at))
    name = b"big"
    vk_at = put(b"vk" + struct.pack("<HIIIHH", len(name), len(data), db_at, 3, 1, 0) + name)
    values_at = put(struct.pack("<I", vk_at))
    root = b"ROOT"
    nk_at = put(b"nk" + struct.pack("<HQII", 0x2C, filetime(), 0, 0)
                + struct.pack("<IIIIIIII", 0, 0, 0xFFFFFFFF, 0xFFFFFFFF, 1, values_at, sk_at, 0xFFFFFFFF)
                + struct.pack("<IIIII", 0, 0, 6, len(data), 0) + struct.pack("<HH", len(root), 0) + root)
    size = (at + 4 + 4095) // 4096 * 4096
    bin_ = bytearray(size)
    bin_[:32] = b"hbin" + struct.pack("<II", 0, size) + bytes(20)
    bin_[0x20:at] = b"".join(cells)
    bin_[at:at + 4] = struct.pack("<i", size - at)
    base = bytearray(4096)
    base[:0x30] = b"regf" + struct.pack("<IIQIIIIIII", 1, 1, filetime(), 1, 5, 0, 1, nk_at, size, 1)
    checksum = 0
    for i in range(0, 508, 4):
        checksum ^= struct.unpack_from("<I", base, i)[0]
    base[0x1FC:0x200] = struct.pack("<I", checksum)
    path.write_bytes(bytes(base) + bytes(bin_))


def listing(path: Path) -> str:
    """Every key (its path, how many subkeys and values) and value (name, type, data in hex), as hivex reads them,
    keys in the order their lists have them."""
    h = hivex.Hivex(str(path))
    lines = []

    def walk(node: int, where: str) -> None:
        children = h.node_children(node)
        values = h.node_values(node)
        lines.append(f"K\t{where}\t{len(children)}\t{len(values)}")
        for v in values:
            kind, data = h.value_value(v)
            lines.append(f"V\t{where}\t{h.value_key(v)}\t{kind}\t{data.hex()}")
        for c in children:
            walk(c, f"{where}\\{h.node_name(c)}" if where else h.node_name(c))

    walk(h.root(), "")
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    for name, make in (("system", system), ("big", big_data_hive)):
        out = HERE / f"{name}.hiv"
        make(out)
        (HERE / f"{name}.txt").write_text(listing(out), encoding="utf-8")
        subprocess.run(["xz", "-9", "-f", str(out)], check=True)
        print(f"{out}.xz", file=sys.stderr)
