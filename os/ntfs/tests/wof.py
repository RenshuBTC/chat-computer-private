#!/usr/bin/env python3
"""Makes wof-chunks.bin, for the ntfs crate's decompressors of Windows' own compression (src/wof.rs): chunks as WOF
keeps them, compressed by wimlib (Ubuntu's libwim15, a library for Windows' WIM files, which use the same XPRESS
and LZX), each way WOF compresses: XPRESS with Huffman codes in chunks of 4, 8 and 16 KiB, and LZX in chunks of 32
KiB (LZX at wimlib's most thorough too), of words, of x86 code (calls whose addresses LZX translates), of
records (offsets in steps of 8, which LZX's aligned-offset blocks are for), of zeros (the longest matches), of bytes
repeated from far back, and of chunks shorter than their size (a file's last). Each checked by wimlib's own
decompressor first.

    python3 tests/wof.py

Each chunk in the file: its algorithm (WOF's: 0 XPRESS 4K, 1 LZX, 2 XPRESS 8K, 3 XPRESS 16K, a byte), its size and a
hash of its bytes (FNV-1a, 64 bits), its compressed size, then its compressed bytes; sizes as 32-bit little-endian.
"""
from __future__ import annotations

import ctypes
import random
import struct
from pathlib import Path

HERE = Path(__file__).resolve().parent

XPRESS, LZX = 1, 2  # wimlib's compression types
WOF = {(XPRESS, 4096): 0, (LZX, 32768): 1, (XPRESS, 8192): 2, (XPRESS, 16384): 3}


def fnv(data: bytes) -> int:
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def words(n: int, seed: int) -> bytes:
    r = random.Random(seed)
    vocab = ["chat", "computer", "kernel", "file", "record", "cluster", "index", "Claude", "tea", "timer", "the", "a",
             "Windows", "compressed", "this", "itself", "to", "save", "room", "on", "its", "drive"]
    out = []
    size = 0
    while size < n:
        w = r.choice(vocab) + (" " if r.random() < 0.9 else ".\n")
        out.append(w)
        size += len(w)
    return "".join(out).encode()[:n]


def code(n: int, seed: int) -> bytes:
    """Bytes like x86 code: instructions from a few patterns, and calls (E8, then the target's distance) to a few
    functions, so the same target is reached from many places (LZX makes those addresses absolute, to match)."""
    r = random.Random(seed)
    targets = [r.randrange(0, n) for _ in range(12)] + [n + 5000, -3000]
    out = bytearray()
    patterns = [b"\x55\x48\x89\xe5", b"\x48\x83\xec\x20", b"\x8b\x45\xfc", b"\x89\xc7", b"\x5d\xc3", b"\x31\xc0",
                b"\x48\x8d\x3d", b"\x0f\x1f\x44\x00\x00", b"\x90"]
    while len(out) < n:
        k = r.random()
        if k < 0.3:
            t = r.choice(targets)
            out += b"\xe8" + struct.pack("<i", t - (len(out) + 5))
        elif k < 0.85:
            out += r.choice(patterns)
        else:
            out += bytes(r.getrandbits(8) for _ in range(r.randrange(1, 6)))
    # (An E8 in the last 10 bytes, which isn't translated.)
    out = out[:n]
    if n > 12:
        out[n - 7] = 0xE8
    return bytes(out)


def records(n: int, seed: int) -> bytes:
    """Records of 16 bytes, many alike but for a counter: matches whose offsets are multiples of 8."""
    r = random.Random(seed)
    kinds = [bytes(r.getrandbits(8) for _ in range(12)) for _ in range(5)]
    out = bytearray()
    i = 0
    while len(out) < n:
        out += struct.pack("<I", i * 3 if r.random() < 0.7 else r.getrandbits(32)) + r.choice(kinds)
        i += 1
    return bytes(out[:n])


def far(n: int, seed: int) -> bytes:
    """Noise, with stretches of it repeated from far back."""
    r = random.Random(seed)
    out = bytearray()
    while len(out) < n:
        if len(out) > 64 and r.random() < 0.5:
            start = r.randrange(0, len(out) - 32)
            out += out[start:start + r.randrange(3, 300)]
        else:
            out += bytes(r.getrandbits(8) for _ in range(r.randrange(1, 80)))
    return bytes(out[:n])


class Wimlib:
    def __init__(self):
        lib = ctypes.CDLL("libwim.so.15")
        lib.wimlib_create_compressor.argtypes = [ctypes.c_int, ctypes.c_size_t, ctypes.c_uint, ctypes.POINTER(ctypes.c_void_p)]
        lib.wimlib_compress.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p]
        lib.wimlib_compress.restype = ctypes.c_size_t
        lib.wimlib_create_decompressor.argtypes = [ctypes.c_int, ctypes.c_size_t, ctypes.POINTER(ctypes.c_void_p)]
        lib.wimlib_decompress.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p]
        self.lib = lib
        self.made: dict = {}

    def compress(self, kind: int, chunk: int, level: int, data: bytes) -> bytes | None:
        """`data` compressed as one chunk of `chunk` bytes (None if that's no smaller), checked by wimlib's own
        decompressor."""
        key = (kind, chunk, level)
        if key not in self.made:
            c, d = ctypes.c_void_p(), ctypes.c_void_p()
            assert self.lib.wimlib_create_compressor(kind, chunk, level, ctypes.byref(c)) == 0
            assert self.lib.wimlib_create_decompressor(kind, chunk, ctypes.byref(d)) == 0
            self.made[key] = (c, d)
        c, d = self.made[key]
        out = ctypes.create_string_buffer(len(data))
        n = self.lib.wimlib_compress(data, len(data), out, len(data) - 1, c)
        if n == 0:
            return None
        packed = out.raw[:n]
        back = ctypes.create_string_buffer(len(data))
        assert self.lib.wimlib_decompress(packed, n, back, len(data), d) == 0 and back.raw == data
        return packed


def chunks(w: Wimlib):
    for (kind, chunk), algorithm in WOF.items():
        for make, seed in ((words, 1), (code, 2), (records, 3), (far, 4), (lambda n, s: bytes(n), 0)):
            for size in (chunk, chunk // 3 + 7):
                data = make(size, seed + size)
                packed = w.compress(kind, chunk, 50, data)
                if packed is not None:
                    yield algorithm, data, packed
            # (LZX at its most thorough too: near-optimal parsing, aligned-offset blocks where they're smaller.)
            if kind == LZX and make in (code, records):
                data = make(chunk, seed + 1)
                packed = w.compress(kind, chunk, 100, data)
                if packed is not None:
                    yield algorithm, data, packed
        # A file's last chunks: short ones.
        for size in (1, 37, 300):
            data = words(size, 9) if size > 1 else b"A"
            packed = w.compress(kind, chunk, 50, data)
            if packed is not None:
                yield algorithm, data, packed


def main():
    out = bytearray()
    count = 0
    for algorithm, data, packed in chunks(Wimlib()):
        out += struct.pack("<BIQI", algorithm, len(data), fnv(data), len(packed)) + packed
        count += 1
    (HERE / "wof-chunks.bin").write_bytes(bytes(out))
    print(f"{count} chunks, {len(out)} bytes")


if __name__ == "__main__":
    main()
