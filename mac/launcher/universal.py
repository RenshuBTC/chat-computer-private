"""Joins Mac programs built for single processors into one universal program (what Apple's lipo -create
does): a fat header, then each program at an aligned offset. Usage: universal.py OUT IN...

Format (mach-o/fat.h): big-endian magic 0xCAFEBABE and the number of programs, then per program its
cputype, cpusubtype, offset, size and alignment (as a power of two), all 32-bit big-endian."""
import struct
import sys

ALIGN = {0x0100000C: 14, 0x01000007: 12}      # arm64: 16 KB pages; x86_64: 4 KB


def thin_header(data: bytes) -> tuple[int, int]:
    magic, cputype, cpusubtype = struct.unpack_from("<IiI", data, 0)
    if magic != 0xFEEDFACF:
        raise SystemExit("not a 64-bit Mac program")
    return cputype, cpusubtype


def join(parts: list[bytes]) -> bytes:
    heads = [thin_header(p) for p in parts]
    offset = 8 + 20 * len(parts)
    entries, body = [], b""
    for (cputype, cpusubtype), data in zip(heads, parts):
        align = ALIGN.get(cputype, 14)
        start = (offset + (1 << align) - 1) // (1 << align) * (1 << align)
        body += b"\0" * (start - offset) + data
        entries.append(struct.pack(">iIIII", cputype, cpusubtype, start, len(data), align))
        offset = start + len(data)
    head = struct.pack(">II", 0xCAFEBABE, len(parts)) + b"".join(entries)
    return head + body


if __name__ == "__main__":
    out, *ins = sys.argv[1:]
    with open(out, "wb") as f:
        f.write(join([open(p, "rb").read() for p in ins]))
