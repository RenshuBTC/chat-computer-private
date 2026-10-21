#!/usr/bin/env python3
"""Makes the exFAT volumes the exfat crate's tests read (tests/*.img.xz), with Linux's exFAT tools: mkfs.exfat
(exfatprogs) makes each, named, and exfat-fuse (mounted through FUSE, so this needs root) fills it, as a USB stick
or an SD card is filled: folders nested, names long (several entries' worth) and in other alphabets, a folder of 600
files (its entries in many clusters), a file written in one go (its clusters in one run, not in the FAT) and two
written a little at a time each in turn (their clusters in chains in the FAT, interleaved), empty files and folders,
a photo. Each volume's contents are listed in tests/<name>.txt: each file's path, what it is (a file or a folder),
its size, a hash of its bytes (FNV-1a, 64 bits) and when it was changed (seconds since 1970), for the tests to check
against.

    sudo python3 tests/make.py

Needs exfatprogs, exfat-fuse, and a loop device (losetup)."""
from __future__ import annotations

import lzma
import os
import random
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent


def fnv(data: bytes) -> int:
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def pattern(n: int, seed: int) -> bytes:
    """Bytes that differ everywhere, so a wrong cluster shows."""
    return b"".join(b"%04x:%010x\n" % (seed, i) for i in range(0, n + 15, 16))[:n]


def fill(mnt: Path) -> None:
    def put(rel: str, data: bytes, when: int | None = None) -> None:
        p = mnt / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
        if when is not None:
            os.utime(p, (when, when))

    put("hello.txt", b"Hello from exFAT.\n", 1_700_000_000)
    put("Documents/Shopping list.txt", "Milk, eggs, tea — and café au lait.\r\n".encode(), 1_750_000_000)
    put("Documents/A name longer than fifteen characters, by far (several entries).txt", pattern(3000, 1))
    put("Documents/Ünïcödé — 名前 🙂.txt", "Ünïcödé — 名前 🙂\n".encode())
    noise = random.Random(2)
    put("Photos/2024/Beach.jpg", bytes(noise.getrandbits(8) for _ in range(20_000)))
    put("Empty.txt", b"")
    (mnt / "Empty folder").mkdir()
    (mnt / "MiXeD CaSe").mkdir()
    put("MiXeD CaSe/lower.TXT", b"case\n")
    # Written in one go: its clusters in one run.
    put("Big.bin", pattern(1_000_000, 3))
    # Two files written a little at a time each in turn: their clusters interleaved, in chains in the FAT.
    with open(mnt / "Chain A.bin", "wb") as a, open(mnt / "Chain B.bin", "wb") as b:
        for i in range(40):
            a.write(pattern(5000, 100 + i))
            a.flush()
            os.fsync(a.fileno())
            b.write(pattern(7000, 200 + i))
            b.flush()
            os.fsync(b.fileno())
    for i in range(600):
        put(f"Many/file {i:03}.txt", b"%d\n" % i)


def make(name: str, label: str, size_mb: int, cluster: int) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        img, mnt = Path(tmp) / "v.img", Path(tmp) / "mnt"
        mnt.mkdir()
        with open(img, "wb") as f:
            f.truncate(size_mb << 20)
        subprocess.run(["mkfs.exfat", "-q", "-L", label, "-c", str(cluster), str(img)], check=True, stdout=subprocess.DEVNULL)
        # (exfat-fuse, run by root, wants a block device: the image's loop device.)
        loop = subprocess.run(["losetup", "-f", "--show", str(img)], check=True, capture_output=True, text=True).stdout.strip()
        try:
            subprocess.run(["mount.exfat-fuse", loop, str(mnt)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                fill(mnt)
            finally:
                subprocess.run(["fusermount", "-u", str(mnt)], check=True)
            # Listed as the tools read it back, mounted again.
            subprocess.run(["mount.exfat-fuse", "-o", "ro", loop, str(mnt)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except BaseException:
            subprocess.run(["losetup", "-d", loop])
            raise
        try:
            lines = []
            for root, dirs, files in os.walk(mnt):
                dirs.sort()
                rel_root = Path(root).relative_to(mnt)
                for d in dirs:
                    lines.append(f"{(rel_root / d).as_posix()}\tdir\t0\t0\t{int(os.stat(Path(root) / d).st_mtime)}")
                for f in sorted(files):
                    p = Path(root) / f
                    data = p.read_bytes()
                    lines.append(f"{(rel_root / f).as_posix()}\tfile\t{len(data)}\t{fnv(data):016x}\t{int(p.stat().st_mtime)}")
        finally:
            subprocess.run(["fusermount", "-u", str(mnt)], check=True)
            subprocess.run(["losetup", "-d", loop], check=True)
        (HERE / f"{name}.txt").write_text("\n".join(sorted(lines)) + "\n")
        (HERE / f"{name}.img.xz").write_bytes(lzma.compress(img.read_bytes(), preset=9 | lzma.PRESET_EXTREME))
        print(name, (HERE / f"{name}.img.xz").stat().st_size, "bytes compressed,", len(lines), "entries")


if __name__ == "__main__":
    if os.geteuid() != 0:
        sys.exit("exfat-fuse needs root to mount the volumes it fills: run with sudo")
    os.environ["TZ"] = "UTC"
    make("usb", "USB STICK", 64, 4096)
    make("sd", "SD CARD", 96, 32768)
