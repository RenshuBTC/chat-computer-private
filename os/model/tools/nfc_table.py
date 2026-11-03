"""Writes src/nfc_table.rs: what NFC (Unicode's composed normal form, which the models' tokenizer puts text in
before reading it) needs, from Python's own Unicode database: each character's canonical decomposition (one step),
the combining classes, and the pairs that compose (Hangul syllables are worked out, not listed)."""

import sys
import unicodedata


def canonical(cp):
    d = unicodedata.decomposition(chr(cp))
    if not d or d.startswith("<"):
        return None
    return [int(x, 16) for x in d.split()]


decomp, compose = [], []
for cp in range(0x110000):
    if 0xD800 <= cp <= 0xDFFF or 0xAC00 <= cp <= 0xD7A3:
        continue
    d = canonical(cp)
    if d is None:
        continue
    assert 1 <= len(d) <= 2, (hex(cp), d)
    decomp.append((cp, d[0], d[1] if len(d) == 2 else 0))
    # A primary composite: two characters that NFC puts back together (not an exclusion, not a singleton).
    if len(d) == 2 and unicodedata.normalize("NFC", chr(cp)) == chr(cp):
        compose.append((d[0], d[1], cp))
compose.sort()

ccc = []
start, cur = None, 0
for cp in range(0x110001):
    c = unicodedata.combining(chr(cp)) if cp <= 0x10FFFF and not 0xD800 <= cp <= 0xDFFF else 0
    if c != cur:
        if cur:
            ccc.append((start, cp - 1, cur))
        start, cur = cp, c

out = [
    "//! What NFC needs (Unicode " + unicodedata.unidata_version + ", written by tools/nfc_table.py from Python's own",
    "//! database): canonical decompositions (one step: the character, its first and second parts, 0 for none),",
    "//! combining classes (ranges) and the pairs that compose, each sorted for searching.",
    "",
    f"pub static DECOMPOSE: [(u32, u32, u32); {len(decomp)}] = [",
]
out += [f"    (0x{a:X}, 0x{b:X}, 0x{c:X})," for a, b, c in decomp]
out += ["];", "", f"pub static CLASSES: [(u32, u32, u8); {len(ccc)}] = ["]
out += [f"    (0x{a:X}, 0x{b:X}, {c})," for a, b, c in ccc]
out += ["];", "", f"pub static COMPOSE: [(u32, u32, u32); {len(compose)}] = ["]
out += [f"    (0x{a:X}, 0x{b:X}, 0x{c:X})," for a, b, c in compose]
out += ["];", ""]
open(sys.argv[1], "w").write("\n".join(out))
print(len(decomp), "decompositions,", len(ccc), "class ranges,", len(compose), "compositions")
