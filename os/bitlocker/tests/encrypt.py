#!/usr/bin/env python3
"""Encrypts an NTFS volume as BitLocker does on Windows 10 and 11 (XTS-AES-128), for check.py --bitlocker: a Windows
drive that Chat Computer's own system must unlock, with its recovery key, or by itself when BitLocker keeps its key in
the clear (suspended, or not switched on yet). Written from the format as the bitlocker crate reads it, which its tests
check against volumes Windows encrypted (src/tests.rs).

Where Windows keeps BitLocker's metadata (three copies) and NTFS's first sectors in files inside the volume, this puts
them after the NTFS volume's end, in the last megabyte of the partition, which NTFS never reads (unlocked, it reads as
zeros). A TPM's protector is there too, as on a PC with device encryption, whose sealed key nothing here can open.

    python3 tests/encrypt.py ntfs.img bitlocker.img [--clear]

Needs Python's cryptography package (python3-cryptography)."""
from __future__ import annotations

import hashlib
import json
import os
import struct
import sys
import time
import zlib

from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
from cryptography.hazmat.primitives.ciphers.aead import AESCCM

SECTOR = 512
METADATA = 64 * 1024
# NTFS's first sectors, kept elsewhere (encrypted where they're kept); and the room after the NTFS volume.
HEADER = 8192
ROOM = 1 << 20
# The identifier of a volume encrypted throughout; XTS-AES-128; key protectors' kinds.
NORMAL = bytes([0x3B, 0xD6, 0x67, 0x49, 0x29, 0x2E, 0xD8, 0x4A, 0x83, 0x99, 0xF6, 0xA3, 0x39, 0xE3, 0xD0, 0x01])
XTS_128 = 0x8004
CLEAR, TPM, RECOVERY = 0x0000, 0x0100, 0x0800


def filetime() -> int:
    return int(time.time() * 10_000_000) + 116_444_736_000_000_000


def entry(kind: int, value: int, body: bytes) -> bytes:
    """A metadata entry: its size, its type, its value's type, version 1, then the value."""
    return struct.pack("<HHHH", 8 + len(body), kind, value, 1) + body


def key_entry(method: int, key: bytes) -> bytes:
    return entry(0, 1, struct.pack("<I", method) + key)


def sealed(key: bytes, plain: bytes, kind: int = 0) -> bytes:
    """An entry holding `plain` sealed with AES-CCM (a 12-byte nonce, a 16-byte tag): nonce, tag, then the bytes."""
    nonce = struct.pack("<QI", filetime(), int.from_bytes(os.urandom(4), "little"))
    out = AESCCM(key, tag_length=16).encrypt(nonce, plain, None)
    return entry(kind, 5, nonce + out[-16:] + out[:-16])


def stretch(first: bytes, salt: bytes) -> bytes:
    """BitLocker's stretching of a key: a million rounds of SHA-256 over the last hash, the first, the salt and the
    round's number."""
    last = bytes(32)
    for i in range(0x100000):
        last = hashlib.sha256(last + first + salt + struct.pack("<Q", i)).digest()
    return last


def new_recovery_key() -> tuple[bytes, str]:
    """A recovery key's 16 bytes, and its 48 digits as Windows shows them (8 groups, each 11 times two of the bytes)."""
    raw = os.urandom(16)
    groups = [struct.unpack_from("<H", raw, 2 * i)[0] * 11 for i in range(8)]
    return raw, "-".join(f"{g:06d}" for g in groups)


def protector(kind: int, vmk: bytes, key: bytes, salt: bytes | None = None, clear: bytes | None = None) -> tuple[bytes, bytes]:
    """A key protector's entry (the VMK sealed with `key`), and its ID."""
    guid = os.urandom(16)
    body = guid + struct.pack("<QHH", filetime(), 0, kind)
    if salt is not None:
        body += entry(0, 3, struct.pack("<I", 0x1000) + salt)
    if clear is not None:
        body += key_entry(0x2000, clear)
    body += sealed(key, key_entry(0x2000, vmk))
    return entry(2, 8, body), guid


def guid_text(g: bytes) -> str:
    a, b, c = struct.unpack_from("<IHH", g)
    return f"{a:08X}-{b:04X}-{c:04X}-{g[8:10].hex().upper()}-{g[10:].hex().upper()}"


def xts(key: bytes, data: bytearray, at: int) -> None:
    """Encrypts `data` (whole sectors) in place, at byte `at` of the volume: each sector's tweak its number."""
    for i in range(0, len(data), SECTOR):
        tweak = ((at + i) // SECTOR).to_bytes(16, "little")
        enc = Cipher(algorithms.AES(key), modes.XTS(tweak)).encryptor()
        data[i:i + SECTOR] = enc.update(bytes(data[i:i + SECTOR])) + enc.finalize()


def encrypt(ntfs: bytes, clear: bool = False, description: str = "DESKTOP-CHAT C: 21/10/2026") -> tuple[bytes, dict]:
    """The NTFS volume encrypted, in a partition a megabyte bigger; and what a test needs to know (the recovery key,
    its ID, the protectors)."""
    assert len(ntfs) % SECTOR == 0 and ntfs[3:11] == b"NTFS    "
    size = len(ntfs) + ROOM
    header_at = len(ntfs)
    metadata_at = [header_at + HEADER + i * METADATA for i in range(3)]
    fvek, vmk = os.urandom(32), os.urandom(32)

    # The protectors: the TPM's (a key nothing here has), the recovery key's, and, if asked, the clear key.
    raw, recovery = new_recovery_key()
    salt = os.urandom(16)
    entries = entry(7, 2, (description + "\0").encode("utf-16-le"))
    tpm, _ = protector(TPM, vmk, os.urandom(32))
    rec, rec_id = protector(RECOVERY, vmk, stretch(hashlib.sha256(raw).digest(), salt), salt=salt)
    entries += tpm + rec
    if clear:
        key = os.urandom(32)
        entries += protector(CLEAR, vmk, key, clear=key)[0]
    entries += sealed(vmk, key_entry(XTS_128, fvek), kind=3)
    entries += entry(0xF, 0xF, struct.pack("<QQ", header_at, HEADER))

    # The metadata: its header (64 bytes), the metadata's own (48), the entries; then its CRC, and its hash sealed
    # with the VMK.
    guid = os.urandom(16)
    meta = struct.pack("<IIII", 48 + len(entries), 1, 48, 48 + len(entries)) + guid + struct.pack("<IHHQ", 1, XTS_128, 0, filetime())
    used = 64 + len(meta) + len(entries)
    padded = (used + 15) // 16 * 16
    head = b"-FVE-FS-" + struct.pack("<HHHHQII", padded >> 4, 2, 4, 4, size, 0, HEADER // SECTOR) + struct.pack("<3QQ", *metadata_at, header_at)
    block = (head + meta + entries).ljust(padded, b"\0")
    hashed = struct.pack("<HHHHHH", 44, 0, 1, 0, 0x2005, 0) + hashlib.sha256(block).digest()
    check = sealed(vmk, hashed)
    assert len(check) == 80
    block += struct.pack("<HHI", 8 + len(check), 2, zlib.crc32(block)) + check

    # The volume: BitLocker's boot sector first; NTFS's sectors encrypted where they are, but its first ones (in
    # their place, BitLocker's boot sector) encrypted where they're kept; the metadata three times.
    out = bytearray(size)
    data = bytearray(ntfs)
    first = bytearray(data[:HEADER])
    xts(fvek, data, 0)
    out[:len(ntfs)] = data
    xts(fvek, first, header_at)
    out[header_at:header_at + HEADER] = first
    boot = bytearray(SECTOR)
    boot[:11] = b"\xEB\x58\x90-FVE-FS-"
    boot[11:13] = struct.pack("<H", SECTOR)
    boot[13] = 8
    boot[160:176] = NORMAL
    boot[176:200] = struct.pack("<3Q", *metadata_at)
    boot[510:512] = b"\x55\xAA"
    out[:SECTOR] = boot
    for at in metadata_at:
        out[at:at + len(block)] = block
    return bytes(out), {"recovery_key": recovery, "recovery_id": guid_text(rec_id), "clear": clear}


if __name__ == "__main__":
    src, dst = sys.argv[1], sys.argv[2]
    volume, info = encrypt(open(src, "rb").read(), clear="--clear" in sys.argv)
    open(dst, "wb").write(volume)
    print(json.dumps(info))
