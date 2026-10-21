#!/usr/bin/env python3
"""Makes the NTFS volumes the ntfs crate's tests read (tests/*.img.xz), with Linux's NTFS tools: mkntfs makes each, and
ntfs-3g (mounted through FUSE, so this needs root) fills it, as Windows would leave a drive: folders nested, mixed-case
and Unicode names, long names and 8.3 short names, small files kept in their file records, big ones in clusters, one
fragmented into hundreds of pieces (so its file record needs an attribute list and others to hold them all), a sparse
file, compressed files (in a compressed folder), named streams, a hard link, symbolic links, a junction, empty files, a
folder of 600 files (an index of several blocks, in two levels), OneDrive's files (downloaded, or "online only") and a
file Windows compressed itself. Each volume's contents are listed in tests/<name>.txt: each file's path, what it is
(a file, a folder, a link, a file "online" only or "compacted" by Windows), its size and a hash of its bytes (FNV-1a,
64 bits), for the tests to check against.

    sudo python3 tests/make.py
"""
from __future__ import annotations

import lzma
import os
import random
import struct
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
    """Bytes that differ everywhere, so a wrong cluster shows: each 16 the file's number and their place in it (which
    gzip keeps small here)."""
    return b"".join(b"%04x:%010x\n" % (seed, i) for i in range(0, n + 15, 16))[:n]


def noise(n: int, seed: int) -> bytes:
    """Bytes nothing compresses (so a compressed file has clusters stored as they are)."""
    r = random.Random(seed)
    return bytes(r.getrandbits(8) for _ in range(n))


def text(n: int, seed: int) -> bytes:
    """Words, which compress well (a compressed file's clusters really are compressed)."""
    r = random.Random(seed)
    words = ["chat", "computer", "kernel", "file", "record", "cluster", "index", "Claude", "tea", "timer", "the", "a"]
    out = []
    size = 0
    while size < n:
        w = r.choice(words) + (" " if r.random() < 0.9 else ".\n")
        out.append(w)
        size += len(w)
    return "".join(out).encode()[:n]


def fill(mnt: Path, big_cluster: bool) -> None:
    def put(rel: str, data: bytes) -> None:
        p = mnt / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)

    put("hello.txt", b"Hello from NTFS.\n")
    # Windows' own folders (how C: is told from other drives), and a person's, with what people keep there: a note in
    # UTF-8 with its byte order mark, one in UTF-16 as Notepad saved it, a Word file (not text), desktop.ini (a hidden
    # system file, left out of lists) and AppData (a hidden folder, left out of searches).
    put("Windows/System32/drivers/etc/hosts", b"# The hosts file.\r\n127.0.0.1 localhost\r\n")
    put("Users/Sam/Documents/Shopping list.txt", "\ufeffMilk, eggs, tea — and café au lait.\r\n".encode())
    put("Users/Sam/Documents/Old note.txt", "\ufeffA note saved by Notepad, in UTF-16.\r\n".encode("utf-16-le"))
    put("Users/Sam/Documents/Letter.docx", b"PK\x03\x04" + pattern(3_000, 20))
    put("Users/Sam/Desktop/desktop.ini", b"[.ShellClassInfo]\r\n")
    os.setxattr(mnt / "Users/Sam/Desktop/desktop.ini", "system.ntfs_attrib_be", (0x02 | 0x04 | 0x20).to_bytes(4, "big"))
    put("Users/Sam/AppData/Local/Temp/shopping list (old).txt", b"in a hidden folder\n")
    os.setxattr(mnt / "Users/Sam/AppData", "system.ntfs_attrib_be", (0x10 | 0x02).to_bytes(4, "big"))
    put("Empty.txt", b"")
    put("Documents/Report 2026.txt", text(100_000, 1))
    put("Documents/Taxes/Letter from IRAS (March).txt", text(6_000, 2))
    put("Documents/Taxes/receipt.bin", pattern(70_001, 3))
    put("Pictures/Holiday/Beach.raw", pattern(300_000, 4))
    put("Ünïcödé — 名前 🙂.txt", "Unicode names: Ünïcödé, 名前, 🙂\n".encode())
    put("Deep/a/b/c/d/e/f/g/Deepest.txt", b"seven folders down\n")
    long_name = "L" + "o" * 240 + "ng name.txt"
    put(f"Deep/{long_name}", b"a name of 255 characters\n")
    put("MiXeD CaSe/ReadMe.TXT", b"case kept, found either way\n")
    # A name with an 8.3 short name too, as Windows makes them (only the long one is listed).
    put("Program Files/Some Program.exe", pattern(5_000, 5))
    os.setxattr(mnt / "Program Files/Some Program.exe", "system.ntfs_dos_name", b"SOMEPR~1.EXE")
    os.setxattr(mnt / "Program Files", "system.ntfs_dos_name", b"PROGRA~1")
    # Named streams (the unnamed one is the file): Windows' "downloaded from the internet" mark, say.
    put("Downloads/setup.exe", pattern(9_000, 6))
    with open(mnt / "Downloads/setup.exe:Zone.Identifier", "wb") as f:
        f.write(b"[ZoneTransfer]\r\nZoneId=3\r\n")
    # A sparse file: a hole, then data, then a hole to its end.
    with open(mnt / "sparse.bin", "wb") as f:
        f.seek(1_000_000)
        f.write(pattern(20_000, 7))
        f.truncate(3_000_000)
    # A hard link, and a symbolic link as Windows makes one (a reparse point, IO_REPARSE_TAG_SYMLINK, relative).
    os.link(mnt / "hello.txt", mnt / "Documents/hello again.txt")
    target = "hello.txt".encode("utf-16-le")
    body = struct.pack("<HHHHI", 0, len(target), len(target), len(target), 1) + target + target
    (mnt / "link to hello").write_bytes(b"")
    os.setxattr(mnt / "link to hello", "system.ntfs_reparse_data", struct.pack("<IHH", 0xA000000C, len(body), 0) + body)
    # A compressed folder: files made in it are compressed (16 clusters at a time).
    (mnt / "Compressed").mkdir()
    os.setxattr(mnt / "Compressed", "system.ntfs_attrib_be", (0x10 | 0x800).to_bytes(4, "big"))
    put("Compressed/words.txt", text(200_000, 8))
    put("Compressed/random.bin", noise(70_000, 9))
    put("Compressed/mixed.bin", text(30_000, 10) + noise(30_000, 11) + bytes(40_000) + text(25_000, 12))
    # 600 files in a folder: its index is a B-tree of several blocks.
    for i in range(600):
        put(f"Many/file {i:04d} with a longish name.txt", f"file {i}\n".encode())
    # A file in hundreds of pieces: the volume filled, then every other one of many one-cluster files deleted, and the
    # file written into the gaps.
    cluster = 65536 if big_cluster else 4096
    pieces = 60 if big_cluster else 1200
    for i in range(pieces):
        put(f"Gaps/{i:04d}", bytes(cluster))
    with open(mnt / "filler", "wb") as f:
        try:
            while True:
                f.write(bytes(cluster))
                f.flush()
        except OSError:
            pass
    for i in range(0, pieces, 2):
        (mnt / f"Gaps/{i:04d}").unlink()
    put("Fragmented.bin", pattern(cluster * (pieces // 2 - 20), 13))
    (mnt / "filler").unlink()
    # OneDrive's folder, as Windows keeps it for a person whose Documents and Pictures it backs up (each file and
    # folder a placeholder: tag() makes them so): files downloaded to the PC, a file "online only" (sparse: none of its
    # clusters there, and marked offline), one partly downloaded (a hole in the middle), and one its attributes alone
    # say isn't here.
    put("Users/Sam/OneDrive/Documents/Budget.txt", b"Rent 1200\nFood 400\n")
    put("Users/Sam/OneDrive/Pictures/Big.raw", pattern(100_000, 14))
    with open(mnt / "Users/Sam/OneDrive/Documents/Plan.docx", "wb") as f:
        f.truncate(50_000)
    os.setxattr(mnt / "Users/Sam/OneDrive/Documents/Plan.docx", "system.ntfs_attrib_be", (0x20 | 0x1000).to_bytes(4, "big"))
    with open(mnt / "Users/Sam/OneDrive/Documents/Half.bin", "wb") as f:
        f.write(pattern(8_192, 15))
        f.seek(300_000)
        f.write(b"end")
    put("Users/Sam/OneDrive/Documents/Notes.txt", b"notes\n")
    os.setxattr(mnt / "Users/Sam/OneDrive/Documents/Notes.txt", "system.ntfs_attrib_be", (0x20 | 0x1000).to_bytes(4, "big"))
    # A file Windows compressed itself ("Compact OS"): sparse, its bytes in a stream of their own (tag() marks it).
    with open(mnt / "Windows/System32/notepad.exe", "wb") as f:
        f.truncate(200_000)
    with open(mnt / "Windows/System32/notepad.exe:WofCompressedData", "wb") as f:
        f.write(noise(5_000, 16))
    # "My Documents", a junction to Documents as Windows has in each person's folder (hidden and system; tag() makes
    # it one), and a symbolic link on the Desktop to a file in Documents, relative to its folder.
    (mnt / "Users/Sam/My Documents").mkdir()
    os.setxattr(mnt / "Users/Sam/My Documents", "system.ntfs_attrib_be", (0x10 | 0x02 | 0x04).to_bytes(4, "big"))
    target = "..\\Documents\\Shopping list.txt".encode("utf-16-le")
    body = struct.pack("<HHHHI", 0, len(target), len(target), len(target), 1) + target + target
    (mnt / "Users/Sam/Desktop/Shopping list.txt").write_bytes(b"")
    os.setxattr(mnt / "Users/Sam/Desktop/Shopping list.txt", "system.ntfs_reparse_data",
                struct.pack("<IHH", 0xA000000C, len(body), 0) + body)


def tag(mnt: Path) -> dict[str, str]:
    """The reparse points that ntfs-3g can't read through (so they're made after the volume's contents are listed):
    each path's kind in the listing, now."""
    def reparse(rel: str, tag: int, body: bytes) -> None:
        os.setxattr(mnt / rel, "system.ntfs_reparse_data", struct.pack("<IHH", tag, len(body), 0) + body)

    # OneDrive's (IO_REPARSE_TAG_CLOUD_6): files first, then their folders (from the inside, as ntfs-3g can't go
    # through one once it's marked).
    cloud = b"\x01\x00\x00\x00" + bytes(range(12))
    for rel in ["Documents/Budget.txt", "Documents/Plan.docx", "Documents/Half.bin", "Documents/Notes.txt",
                "Pictures/Big.raw", "Documents", "Pictures", ""]:
        reparse(f"Users/Sam/OneDrive/{rel}".rstrip("/"), 0x9000601A, cloud)
    # Compact OS's (IO_REPARSE_TAG_WOF): WOF's version and provider (a file's), the provider's version and algorithm
    # (LZX), no flags.
    reparse("Windows/System32/notepad.exe", 0x80000017, struct.pack("<IIIII", 1, 2, 1, 1, 0))
    # A junction (IO_REPARSE_TAG_MOUNT_POINT): the name Windows follows, and the one shown, each ending in a NUL.
    sub = "\\??\\C:\\Users\\Sam\\Documents".encode("utf-16-le")
    shown = "C:\\Users\\Sam\\Documents".encode("utf-16-le")
    body = struct.pack("<HHHH", 0, len(sub), len(sub) + 2, len(shown)) + sub + b"\0\0" + shown + b"\0\0"
    reparse("Users/Sam/My Documents", 0xA0000003, body)
    return {
        "Users/Sam/OneDrive/Documents/Plan.docx": "online",
        "Users/Sam/OneDrive/Documents/Half.bin": "online",
        "Users/Sam/OneDrive/Documents/Notes.txt": "online",
        "Windows/System32/notepad.exe": "compacted",
        "Users/Sam/My Documents": "link",
    }


def make(name: str, size_mb: int, sector: int, cluster: int) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        img, mnt = Path(tmp) / "v.img", Path(tmp) / "mnt"
        mnt.mkdir()
        with open(img, "wb") as f:
            f.truncate(size_mb << 20)
        subprocess.run(["mkntfs", "-F", "-Q", "-q", "-L", name, "-s", str(sector), "-c", str(cluster), str(img)], check=True,
                       stderr=subprocess.DEVNULL)
        subprocess.run(["ntfs-3g", "-o", "streams_interface=windows", str(img), str(mnt)], check=True)
        try:
            fill(mnt, cluster > 4096)
            lines = []
            for root, dirs, files in os.walk(mnt):
                dirs.sort()
                rel_root = Path(root).relative_to(mnt)
                for d in dirs:
                    p = Path(root) / d
                    kind = "link" if p.is_symlink() else "dir"
                    lines.append(f"{(rel_root / d).as_posix()}\t{kind}\t0\t0")
                for f in sorted(files):
                    p = Path(root) / f
                    if p.is_symlink():
                        lines.append(f"{(rel_root / f).as_posix()}\tlink\t0\t0")
                        continue
                    data = p.read_bytes()
                    lines.append(f"{(rel_root / f).as_posix()}\tfile\t{len(data)}\t{fnv(data):016x}")
            kinds = tag(mnt)
            for i, line in enumerate(lines):
                path = line.split("\t")[0]
                if path in kinds:
                    lines[i] = f"{path}\t{kinds[path]}\t0\t0"
        finally:
            subprocess.run(["fusermount", "-u", str(mnt)], check=True)
        (HERE / f"{name}.txt").write_text("\n".join(sorted(lines)) + "\n")
        (HERE / f"{name}.img.xz").write_bytes(lzma.compress(img.read_bytes(), preset=9 | lzma.PRESET_EXTREME))
        print(name, (HERE / f"{name}.img.xz").stat().st_size, "bytes compressed,", len(lines), "entries")


if __name__ == "__main__":
    if os.geteuid() != 0:
        sys.exit("ntfs-3g needs root to mount the volumes it fills: run with sudo")
    make("small", 32, 512, 4096)
    make("big-sectors", 64, 4096, 65536)
