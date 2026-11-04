"""Writes src/compose.rs: what the text needs of Unicode to set accents typed as characters of their own, from
Python's own Unicode database: each mark's combining class (ranges of characters with the same one; the others are
0, letters), and the letter-and-mark pairs Unicode composes into one character (its canonical compositions whose
second part is a mark, less those it excludes), each sorted for searching.

    python compose.py
"""
import unicodedata
from pathlib import Path

OUT = Path(__file__).resolve().parent / "src" / "compose.rs"


def main() -> None:
    ranges = []
    for c in range(0x110000):
        cc = unicodedata.combining(chr(c))
        if cc == 0:
            continue
        if ranges and ranges[-1][1] == c - 1 and ranges[-1][2] == cc:
            ranges[-1][1] = c
        else:
            ranges.append([c, c, cc])
    pairs = []
    for c in range(0x110000):
        d = unicodedata.decomposition(chr(c))
        if not d or d.startswith("<"):
            continue
        parts = [int(p, 16) for p in d.split()]
        # (Unicode's exclusions and singletons don't compose again: NFC tells.)
        if len(parts) == 2 and unicodedata.combining(chr(parts[1])) and \
                unicodedata.normalize("NFC", chr(parts[0]) + chr(parts[1])) == chr(c):
            pairs.append((parts[0], parts[1], c))
    pairs.sort()
    lines = [
        "//! What the text needs of Unicode to set accents typed as characters of their own (Unicode "
        f"{unicodedata.unidata_version}, written by",
        "//! compose.py from Python's own database): marks' combining classes (ranges of characters with the same one;",
        "//! the others' is 0), and the letter-and-mark pairs that compose into one character, each sorted for searching.",
        "",
        f"pub static CLASSES: [(u32, u32, u8); {len(ranges)}] = [",
        *(f"    (0x{a:X}, 0x{b:X}, {cc})," for a, b, cc in ranges),
        "];",
        "",
        f"pub static COMPOSE: [(u32, u32, u32); {len(pairs)}] = [",
        *(f"    (0x{a:X}, 0x{b:X}, 0x{c:X})," for a, b, c in pairs),
        "];",
    ]
    OUT.write_text("\n".join(lines) + "\n")
    print(f"{len(ranges)} ranges of classes, {len(pairs)} pairs that compose")


if __name__ == "__main__":
    main()
