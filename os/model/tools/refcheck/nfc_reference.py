"""Strings to put in NFC (every character alone, decomposed, and random runs of letters and marks), and a check of
the engine's answers against Python's unicodedata.

    python3 -I nfc_reference.py make IN.json
    (cargo run --release --features std --example nfc_check -- IN.json > OURS.json)
    python3 -I nfc_reference.py compare IN.json OURS.json

(Python's Unicode database must be the version tools/nfc_table.py made src/nfc_table.rs from.)
"""
import json, random, sys, unicodedata

def make(path):
    rng = random.Random(7)
    cps = [c for c in range(0x110000) if not 0xD800 <= c <= 0xDFFF]
    out = []
    # Every character in blocks of 64, as it is and decomposed.
    for i in range(0, len(cps), 64):
        s = "".join(map(chr, cps[i:i + 64]))
        out.append(s)
        out.append(unicodedata.normalize("NFD", s))
    marks = [c for c in cps if unicodedata.combining(chr(c))]
    decomposable = [c for c in cps if unicodedata.decomposition(chr(c)) and not unicodedata.decomposition(chr(c)).startswith("<")]
    jamo = list(range(0x1100, 0x1113)) + list(range(0x1161, 0x1176)) + list(range(0x11A8, 0x11C3))
    starters = [ord(c) for c in "aeiouAEIOUnNcCsSzZyYkKhH"] + decomposable[:400] + list(range(0xAC00, 0xAC40)) + jamo
    for _ in range(20000):
        n = rng.randint(1, 12)
        s = []
        for _ in range(n):
            r = rng.random()
            s.append(rng.choice(marks) if r < 0.45 else rng.choice(jamo) if r < 0.6 else rng.choice(starters) if r < 0.9 else rng.choice(cps))
        out.append("".join(map(chr, s)))
    json.dump(out, open(path, "w"), ensure_ascii=False)
    print(len(out), "strings")

def compare(inp, ours):
    a = json.load(open(inp)); b = json.load(open(ours))
    bad = [(x, y) for x, y in zip(a, b) if unicodedata.normalize("NFC", x) != y]
    for x, y in bad[:5]:
        print("DIFF", [hex(ord(c)) for c in x], [hex(ord(c)) for c in unicodedata.normalize("NFC", x)], [hex(ord(c)) for c in y])
    print(len(a) - len(bad), "of", len(a), "the same")
    return len(bad) == 0 and len(a) == len(b)

if __name__ == "__main__":
    if sys.argv[1] == "make": make(sys.argv[2])
    else: sys.exit(0 if compare(sys.argv[2], sys.argv[3]) else 1)
