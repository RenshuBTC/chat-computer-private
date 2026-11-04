#!/usr/bin/env python3
"""Makes what the zip crate's tests read, in the folder given: DEFLATE streams from zlib (every level and strategy,
small windows, flushes in the middle; on words, noise, runs and nothing), and ZIP archives made by Python's zipfile
and by Info-ZIP's zip: files stored and compressed, a folder, UTF-8 and code page 437 names, data descriptors, ZIP64,
a comment, and ones that can't be read here (a password's, bzip2's, LZMA's). Each archive's files are listed in
<archive>.txt (name, size and FNV-1a hash), for the tests to check against.

    python3 tests/cases.py <folder>
"""
from __future__ import annotations

import io
import random
import shutil
import struct
import subprocess
import sys
import zipfile
import zlib
from pathlib import Path


def fnv(data: bytes) -> int:
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def words(n: int, seed: int) -> bytes:
    r = random.Random(seed)
    vocabulary = ["the", "a", "chat", "computer", "kernel", "deflate", "huffman", "window", "Claude", "zip", "of", "tea"]
    out, size = [], 0
    while size < n:
        w = r.choice(vocabulary) + (" " if r.random() < 0.9 else ".\n")
        out.append(w)
        size += len(w)
    return "".join(out).encode()[:n]


def inputs() -> list[bytes]:
    r = random.Random(3)
    return [
        b"",
        b"a",
        words(150_000, 1),
        bytes(r.getrandbits(8) for _ in range(40_000)),
        b"abc" * 20_000 + bytes(70_000) + b"xyz" * 500,
        words(20_000, 2) + bytes(r.getrandbits(8) for _ in range(5_000)) + words(20_000, 3) * 2,
    ]


def deflate_cases(out: Path) -> None:
    """input-<i>.bin, and deflate.bin: for each stream, which input, then the stream's length and bytes."""
    data = inputs()
    for i, d in enumerate(data):
        (out / f"input-{i}.bin").write_bytes(d)
    cases = bytearray()

    def add(i: int, packed: bytes) -> None:
        cases.extend(struct.pack("<BI", i, len(packed)))
        cases.extend(packed)

    strategies = [zlib.Z_DEFAULT_STRATEGY, zlib.Z_FILTERED, zlib.Z_HUFFMAN_ONLY, zlib.Z_RLE, zlib.Z_FIXED]
    for i, d in enumerate(data):
        for level in range(10):
            for strategy in strategies:
                c = zlib.compressobj(level, zlib.DEFLATED, -15, 9, strategy)
                add(i, c.compress(d) + c.flush())
        # The smallest window and least memory; and flushes every 1,000 bytes (each leaves an empty stored block).
        c = zlib.compressobj(9, zlib.DEFLATED, -9, 1)
        add(i, c.compress(d) + c.flush())
        c = zlib.compressobj(6, zlib.DEFLATED, -15)
        parts = [c.compress(d[k:k + 1000]) + c.flush(zlib.Z_SYNC_FLUSH) for k in range(0, len(d), 1000)]
        add(i, b"".join(parts) + c.flush())
    (out / "deflate.bin").write_bytes(bytes(cases))
    (out / "zlib.bin").write_bytes(zlib.compress(data[2], 9))


def listing(out: Path, name: str, files: dict[str, bytes]) -> None:
    lines = [f"{n}\t{len(d)}\t{fnv(d):016x}" for n, d in files.items()]
    (out / f"{name}.txt").write_text("\n".join(lines) + "\n")


def zip_cases(out: Path) -> None:
    data = inputs()
    files = {
        "hello.txt": b"Hello from a ZIP.\n",
        "folder/": b"",
        "folder/stored.txt": words(5_000, 4),
        "folder/words.txt": data[2],
        "名前 — ünïcödé.txt": "Unicode names\n".encode(),
        "noise.bin": data[3],
        "empty.txt": b"",
    }
    # As zipfile writes them: compressed, a file stored, a folder, a comment at the end.
    with zipfile.ZipFile(out / "python.zip", "w", zipfile.ZIP_DEFLATED) as z:
        for n, d in files.items():
            if n.endswith("/"):
                z.writestr(zipfile.ZipInfo(n), b"")
            else:
                z.writestr(n, d, compress_type=zipfile.ZIP_STORED if "stored" in n else zipfile.ZIP_DEFLATED)
        z.comment = b"an archive comment"
    listing(out, "python", files)

    # Written where it can't go back (a stream): each file's sizes and CRC in a "data descriptor" after its bytes, the
    # local header's left as zeros.
    class Stream(io.RawIOBase):
        def __init__(self) -> None:
            self.data = bytearray()

        def writable(self) -> bool:
            return True

        def write(self, b) -> int:
            self.data.extend(b)
            return len(b)

    s = Stream()
    with zipfile.ZipFile(s, "w", zipfile.ZIP_DEFLATED) as z:
        for n, d in files.items():
            if not n.endswith("/"):
                with z.open(n, "w") as f:
                    f.write(d)
    (out / "stream.zip").write_bytes(bytes(s.data))
    listing(out, "stream", {n: d for n, d in files.items() if not n.endswith("/")})

    # ZIP64: every file's header with 64-bit sizes; and more than 65,535 files, so the end record's count is full and
    # ZIP64's end record has the real one.
    many = {f"m/{k:05d}": b"%d\n" % k for k in range(65_540)}
    with zipfile.ZipFile(out / "zip64.zip", "w", zipfile.ZIP_STORED) as z:
        with z.open("first.txt", "w", force_zip64=True) as f:
            f.write(files["folder/words.txt"])
        for n, d in many.items():
            z.writestr(n, d)
    listing(out, "zip64", {"first.txt": files["folder/words.txt"], **many})

    # A name in code page 437 (no UTF-8 flag): "café.txt", é being 0x82 there.
    with zipfile.ZipFile(out / "cp437.zip", "w") as z:
        z.writestr("cafX.txt", b"old names\n")
    raw = (out / "cp437.zip").read_bytes().replace(b"cafX", b"caf\x82")
    (out / "cp437.zip").write_bytes(raw)
    listing(out, "cp437", {"café.txt": b"old names\n"})

    # Compressed some other way.
    with zipfile.ZipFile(out / "other.zip", "w") as z:
        z.writestr("bzip2.txt", words(3_000, 5), compress_type=zipfile.ZIP_BZIP2)
        z.writestr("lzma.txt", words(3_000, 6), compress_type=zipfile.ZIP_LZMA)

    # Info-ZIP's: its own DEFLATE (not zlib's), and a file with a password.
    if shutil.which("zip"):
        src = out / "infozip-src"
        (src / "sub").mkdir(parents=True)
        (src / "a.txt").write_bytes(data[2])
        (src / "sub" / "b.bin").write_bytes(data[4])
        subprocess.run(["zip", "-q", "-9", "-r", str(out / "infozip.zip"), "."], cwd=src, check=True)
        listing(out, "infozip", {"a.txt": data[2], "sub/": b"", "sub/b.bin": data[4]})
        subprocess.run(["zip", "-q", "-P", "secret", str(out / "password.zip"), "a.txt"], cwd=src, check=True)


if __name__ == "__main__":
    folder = Path(sys.argv[1])
    folder.mkdir(parents=True, exist_ok=True)
    deflate_cases(folder)
    zip_cases(folder)
