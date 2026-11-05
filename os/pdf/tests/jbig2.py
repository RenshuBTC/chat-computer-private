#!/usr/bin/env python3
"""Makes scan-jbig2.pdf and scan-jbig2.png, for the pdf crate's JBIG2 decoder (src/jbig2.rs): a scanned page (black
and white, drawn here) coded as JBIG2 in each of the ways the decoder reads, one page each, every page the same
picture (scan-jbig2.png). The JBIG2 is written here (T.88's MQ coder and its integer, symbol ID, generic and
refinement coding, as Annex E, A, 6.2 and 6.3 have them), and each page is checked against Poppler's own decoder
(pdfimages), so the writing is right whatever this crate's reading does.

    python3 tests/jbig2.py

The pages:
 1. generic regions, the page in eight bands: templates 0 to 3, each without and with typical prediction (TPGDON),
    template 0's adaptive pixels moved for one band;
 2. one generic region coded as MMR (fax's Group 4, by libtiff through Pillow);
 3. a symbol dictionary (the page's 16x16 tiles that aren't white, template 1) and a text region placing them (two
    strips, the bottom-left corner, a DS offset);
 4. the same with the dictionary in JBIG2Globals, the top-right corner, transposed, four strips;
 5. a text region whose instances are refined: every symbol placed is another tile, refined into the right one;
 6. a second dictionary whose symbols are refinements of the first's (one instance) and aggregates of two of them;
 7. a generic region of the page with noise (an intermediate region), refined into the page by a refinement region
    (template 1);
 8. the noisy page drawn, then refined where it is with typical prediction (TPGRON), by template 0;
 9. the same by template 1;
10. a striped page of unknown height, three stripes, each a generic region with an end of stripe after it.

Poppler (24.02) and jbig2dec (0.20) agree with the picture on every page but these: Poppler's MMR (2) and its typical
prediction in refinements (8, 9) differ from jbig2dec's, which agrees; jbig2dec doesn't read refinements in
aggregate symbols (6) or intermediate regions (7), which Poppler does as the picture has them.
"""
from __future__ import annotations

import io
import random
import shutil
import struct
import subprocess
import tempfile
import zlib
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
W, H = 480, 320

QE = [
    (0x5601, 1, 1, 1), (0x3401, 2, 6, 0), (0x1801, 3, 9, 0), (0x0AC1, 4, 12, 0), (0x0521, 5, 29, 0), (0x0221, 38, 33, 0),
    (0x5601, 7, 6, 1), (0x5401, 8, 14, 0), (0x4801, 9, 14, 0), (0x3801, 10, 14, 0), (0x3001, 11, 17, 0), (0x2401, 12, 18, 0),
    (0x1C01, 13, 20, 0), (0x1601, 29, 21, 0), (0x5601, 15, 14, 1), (0x5401, 16, 14, 0), (0x5101, 17, 15, 0), (0x4801, 18, 16, 0),
    (0x3801, 19, 17, 0), (0x3401, 20, 18, 0), (0x3001, 21, 19, 0), (0x2801, 22, 19, 0), (0x2401, 23, 20, 0), (0x2201, 24, 21, 0),
    (0x1C01, 25, 22, 0), (0x1801, 26, 23, 0), (0x1601, 27, 24, 0), (0x1401, 28, 25, 0), (0x1201, 29, 26, 0), (0x1101, 30, 27, 0),
    (0x0AC1, 31, 28, 0), (0x09C1, 32, 29, 0), (0x08A1, 33, 30, 0), (0x0521, 34, 31, 0), (0x0441, 35, 32, 0), (0x02A1, 36, 33, 0),
    (0x0221, 37, 34, 0), (0x0141, 38, 35, 0), (0x0111, 39, 36, 0), (0x0085, 40, 37, 0), (0x0049, 41, 38, 0), (0x0025, 42, 39, 0),
    (0x0015, 43, 40, 0), (0x0009, 44, 41, 0), (0x0005, 45, 42, 0), (0x0001, 45, 43, 0), (0x5601, 46, 46, 0),
]


class Contexts(dict):
    """Contexts' states (index in QE, the more probable symbol), all starting at (0, 0)."""

    def __missing__(self, k):
        return (0, 0)


class MQ:
    """The MQ encoder (T.88 E.2), its output with a byte before it that's dropped."""

    def __init__(self):
        self.a, self.c, self.ct, self.buf = 0x8000, 0, 12, bytearray([0])

    def byteout(self):
        if self.buf[-1] == 0xFF:
            self.buf.append((self.c >> 20) & 0xFF)
            self.c &= 0xFFFFF
            self.ct = 7
        elif self.c < 0x8000000:
            self.buf.append((self.c >> 19) & 0xFF)
            self.c &= 0x7FFFF
            self.ct = 8
        else:
            self.buf[-1] += 1
            if self.buf[-1] == 0xFF:
                self.c &= 0x7FFFFFF
                self.buf.append((self.c >> 20) & 0xFF)
                self.c &= 0xFFFFF
                self.ct = 7
            else:
                self.buf.append((self.c >> 19) & 0xFF)
                self.c &= 0x7FFFF
                self.ct = 8

    def renorm(self):
        while True:
            self.a <<= 1
            self.c <<= 1
            self.ct -= 1
            if self.ct == 0:
                self.byteout()
            if self.a & 0x8000:
                break

    def encode(self, cx: Contexts, i: int, d: int):
        index, mps = cx[i]
        qe, nmps, nlps, switch = QE[index]
        self.a -= qe
        if d == mps:
            if self.a & 0x8000 == 0:
                if self.a < qe:
                    self.a = qe
                else:
                    self.c += qe
                cx[i] = (nmps, mps)
                self.renorm()
            else:
                self.c += qe
        else:
            if self.a < qe:
                self.c += qe
            else:
                self.a = qe
            cx[i] = (nlps, 1 - mps if switch else mps)
            self.renorm()

    def flush(self) -> bytes:
        temp = self.c + self.a
        self.c |= 0xFFFF
        if self.c >= temp:
            self.c -= 0x8000
        self.c <<= self.ct
        self.byteout()
        self.c <<= self.ct
        self.byteout()
        assert self.buf[0] == 0
        out = bytes(self.buf[1:])
        if not out.endswith(b"\xff"):
            out += b"\xff"
        return out + b"\xac"


def encode_int(mq: MQ, cx: Contexts, v):
    """An integer (None: OOB) by an IAx procedure (T.88 A.2)."""
    prev = 1

    def bit(b):
        nonlocal prev
        mq.encode(cx, prev, b)
        prev = (prev << 1) | b if prev < 256 else ((((prev << 1) | b) & 511) | 256)

    def bits(value, n):
        for i in range(n - 1, -1, -1):
            bit((value >> i) & 1)

    if v is None:
        bit(1), bit(0), bits(0, 2)
        return
    bit(1 if v < 0 else 0)
    m = abs(v)
    for prefix, base, n in [((0,), 0, 2), ((1, 0), 4, 4), ((1, 1, 0), 20, 6), ((1, 1, 1, 0), 84, 8), ((1, 1, 1, 1, 0), 340, 12)]:
        if m < base + (1 << n):
            for b in prefix:
                bit(b)
            bits(m - base, n)
            return
    for b in (1, 1, 1, 1, 1):
        bit(b)
    bits(m - 4436, 32)


def encode_id(mq: MQ, cx: Contexts, v: int, length: int):
    prev = 1
    for i in range(length - 1, -1, -1):
        b = (v >> i) & 1
        mq.encode(cx, prev, b)
        prev = (prev << 1) | b


class Bits:
    """A bitmap: rows of 0 and 1 (1 black)."""

    def __init__(self, w, h, rows=None):
        self.w, self.h = w, h
        self.rows = rows if rows is not None else [[0] * w for _ in range(h)]

    def get(self, x, y):
        return self.rows[y][x] if 0 <= x < self.w and 0 <= y < self.h else 0

    def crop(self, x, y, w, h):
        return Bits(w, h, [[self.get(x + i, y + j) for i in range(w)] for j in range(h)])

    def key(self):
        return tuple(tuple(r) for r in self.rows)


def generic_context(b: Bits, x, y, template, at):
    p = b.get
    if template == 0:
        bits = [(-1, 0), (-2, 0), (-3, 0), (-4, 0), at[0], (2, -1), (1, -1), (0, -1), (-1, -1), (-2, -1), at[1], at[2],
                (1, -2), (0, -2), (-1, -2), at[3]]
    elif template == 1:
        bits = [(-1, 0), (-2, 0), (-3, 0), at[0], (2, -1), (1, -1), (0, -1), (-1, -1), (-2, -1), (2, -2), (1, -2), (0, -2), (-1, -2)]
    elif template == 2:
        bits = [(-1, 0), (-2, 0), at[0], (1, -1), (0, -1), (-1, -1), (-2, -1), (1, -2), (0, -2), (-1, -2)]
    else:
        bits = [(-1, 0), (-2, 0), (-3, 0), (-4, 0), at[0], (1, -1), (0, -1), (-1, -1), (-2, -1), (-3, -1)]
    c = 0
    for k, (dx, dy) in enumerate(bits):
        c |= p(x + dx, y + dy) << k
    return c


DEFAULT_AT = {0: [(3, -1), (-3, -1), (2, -2), (-2, -2)], 1: [(3, -1)], 2: [(2, -1)], 3: [(2, -1)]}
SLTP = [0x9B25, 0x0795, 0x00E5, 0x0195]


def encode_generic(mq: MQ, cx: Contexts, b: Bits, template, tpgdon, at):
    ltp = 0
    for y in range(b.h):
        if tpgdon:
            same = b.rows[y] == (b.rows[y - 1] if y > 0 else [0] * b.w)
            mq.encode(cx, SLTP[template], ltp ^ int(same))
            ltp = int(same)
            if ltp:
                continue
        for x in range(b.w):
            mq.encode(cx, generic_context(b, x, y, template, at), b.rows[y][x])


def refine_context(b: Bits, ref: Bits, x, y, dx, dy, template, at):
    p, r = b.get, (lambda ox, oy: ref.get(x - dx + ox, y - dy + oy))
    if template == 0:
        own = [(-1, 0), (1, -1), (0, -1), at[0]]
        theirs = [(1, 1), (0, 1), (-1, 1), (1, 0), (0, 0), (-1, 0), (1, -1), (0, -1), at[1]]
    else:
        own = [(-1, 0), (1, -1), (0, -1), (-1, -1)]
        theirs = [(1, 1), (0, 1), (1, 0), (0, 0), (-1, 0), (0, -1)]
    c = 0
    for k, (ox, oy) in enumerate(own):
        c |= p(x + ox, y + oy) << k
    for k, (ox, oy) in enumerate(theirs):
        c |= r(ox, oy) << (k + len(own))
    return c


def encode_refine(mq: MQ, cx: Contexts, b: Bits, ref: Bits, dx, dy, template, at, tpgron=False):
    ltp = 0

    def uniform(x, y):
        s = sum(ref.get(x - dx + ox, y - dy + oy) for oy in (-1, 0, 1) for ox in (-1, 0, 1))
        return 0 if s == 0 else 1 if s == 9 else None

    for y in range(b.h):
        if tpgron:
            ok = all(uniform(x, y) is None or uniform(x, y) == b.rows[y][x] for x in range(b.w))
            mq.encode(cx, 0x100 if template == 0 else 0x040, ltp ^ int(ok))
            ltp = int(ok)
        for x in range(b.w):
            if ltp and uniform(x, y) is not None:
                continue
            mq.encode(cx, refine_context(b, ref, x, y, dx, dy, template, at), b.rows[y][x])


# ----------------------------------------------------------------------------------------------- segments


def segment(number, kind, data, refers=(), page=1, retain=0):
    """A segment: its header (T.88 7.2) and data."""
    assert len(refers) <= 4
    head = struct.pack(">IB", number, kind) + bytes([(len(refers) << 5) | retain])
    size = 1 if number <= 256 else 2 if number <= 65536 else 4
    for r in refers:
        head += r.to_bytes(size, "big")
    head += bytes([page]) + struct.pack(">I", len(data))
    return head + data


def page_info(w, h, striped=None):
    stripes = 0x8000 | striped if striped else 0
    return struct.pack(">IIIIBH", w, h, 300, 300, 0, stripes)


def region_info(w, h, x, y, op=0):
    return struct.pack(">IIIIB", w, h, x, y, op)


def at_bytes(at):
    return b"".join(struct.pack(">bb", dx, dy) for dx, dy in at)


def generic_segment(number, b: Bits, x, y, template, tpgdon, at=None, kind=38, op=0):
    at = at or DEFAULT_AT[template]
    mq = MQ()
    encode_generic(mq, Contexts(), b, template, tpgdon, at)
    flags = (template << 1) | (8 if tpgdon else 0)
    return segment(number, kind, region_info(b.w, b.h, x, y, op) + bytes([flags]) + at_bytes(at) + mq.flush())


# -------------------------------------------------------------------------------------------- the picture


def picture() -> Bits:
    """The scanned page: lines of words, a box, a line and a dot pattern."""
    im = Image.new("1", (W, H), 1)
    d = ImageDraw.Draw(im)
    try:
        font = ImageFont.load_default(size=22)
    except TypeError:
        font = ImageFont.load_default()
    for i, line in enumerate(["Chat Computer reads", "scanned pages, JBIG2", "and all: 0123456789."]):
        d.text((20, 20 + 40 * i), line, font=font, fill=0)
    d.rectangle((300, 160, 440, 280), outline=0, width=3)
    d.line((20, 200, 260, 300), fill=0, width=2)
    for yy in range(200, 290, 6):
        for xx in range(320, 420, 6):
            d.point((xx + (yy // 6) % 2 * 3, yy), fill=0)
    return Bits(W, H, [[1 - im.getpixel((x, y)) for x in range(W)] for y in range(H)])


def tiles(b: Bits, size=16):
    """The page's tiles that aren't white: (x, y, tile)."""
    out = []
    for ty in range(0, b.h, size):
        for tx in range(0, b.w, size):
            t = b.crop(tx, ty, min(size, b.w - tx), min(size, b.h - ty))
            if any(any(r) for r in t.rows):
                out.append((tx, ty, t))
    return out


def symbol_dictionary(number, symbols, template=1, refers=(), inputs=0, exported=None, refagg=None, extra_flags=0, page=1):
    """A symbol dictionary (arithmetic): `symbols` coded in height classes (each symbol a generic region; or, with
    `refagg`, as refagg[i] says: ("refine", id, rdx, rdy) or ("aggregate", [(id, x, y), ...])); then which of the
    `inputs` input symbols and new ones it exports (all new ones, unless `exported` lists them)."""
    mq = MQ()
    gb, gr = Contexts(), Contexts()
    ints = {k: Contexts() for k in ["dh", "dw", "ex", "ai", "dt", "fs", "ds", "it", "ri", "rdw", "rdh", "rdx", "rdy"]}
    idcx = Contexts()
    at = DEFAULT_AT[template]
    total = inputs + len(symbols)
    code_len = (total - 1).bit_length() if total > 1 else 0
    order = sorted(range(len(symbols)), key=lambda i: symbols[i][0].h)
    assert order == list(range(len(symbols))), "symbols must come in height order"
    height = 0
    i = 0
    while i < len(symbols):
        h = symbols[i][0].h
        encode_int(mq, ints["dh"], h - height)
        height = h
        width = 0
        while i < len(symbols) and symbols[i][0].h == h:
            b, how = symbols[i]
            encode_int(mq, ints["dw"], b.w - width)
            width = b.w
            if refagg is None:
                encode_generic(mq, gb, b, template, False, at)
            elif how[0] == "refine":
                _, ref_id, rdx, rdy, ref = how
                encode_int(mq, ints["ai"], 1)
                encode_id(mq, idcx, ref_id, code_len)
                encode_int(mq, ints["rdx"], rdx)
                encode_int(mq, ints["rdy"], rdy)
                encode_refine(mq, gr, b, ref, rdx, rdy, 0, [(-1, -1), (-1, -1)])
            else:
                _, parts = how
                encode_int(mq, ints["ai"], len(parts))
                encode_text(mq, ints, idcx, gr, parts, code_len, strips=1, corner=1, transposed=False, ds_offset=0, refine=True)
            i += 1
        encode_int(mq, ints["dw"], None)
    flags = exported if exported is not None else [0] * inputs + [1] * len(symbols)
    runs, cur, n = [], 0, 0
    for f in flags + [None]:
        if f == cur:
            n += 1
        else:
            runs.append(n)
            cur, n = f, 1
    for r in runs:
        encode_int(mq, ints["ex"], r)
    template_bits = template << 10
    sd_flags = template_bits | (2 if refagg is not None else 0) | extra_flags
    head = struct.pack(">H", sd_flags) + at_bytes(at)
    if refagg is not None:
        head += at_bytes([(-1, -1), (-1, -1)])
    head += struct.pack(">II", sum(flags), len(symbols))
    return segment(number, 0, head + mq.flush(), refers, page)


def encode_text(mq, ints, idcx, gr, instances, code_len, strips, corner, transposed, ds_offset, refine=False):
    """Symbol instances placed (T.88 6.4.5's other side): each (id, x, y) of its top-left corner, with its bitmap, and
    (if refined) the bitmap it's refined into: [(id, x, y, bitmap, refined_to)]. Grouped in strips by T, in order of
    S along each."""
    right = corner & 2 != 0
    bottom = corner & 1 == 0

    def st(inst):
        _, x, y, b, *_ = inst
        final = inst[4] if len(inst) > 4 and inst[4] is not None else b
        s = (y if transposed else x) + ((final.h - 1) if transposed and bottom else (final.w - 1) if not transposed and right else 0)
        t = (x if transposed else y) + ((final.h - 1) if not transposed and bottom else (final.w - 1) if transposed and right else 0)
        return s, t

    placed = sorted(instances, key=lambda i: (st(i)[1] // strips, st(i)[0]))
    strip_t = 0
    encode_int(mq, ints["dt"], 0)
    first_s = 0
    k = 0
    while k < len(placed):
        t0 = st(placed[k])[1] // strips * strips
        encode_int(mq, ints["dt"], (t0 - strip_t) // strips)
        strip_t = t0
        group = [p for p in placed[k:] if st(p)[1] // strips * strips == t0]
        cur_s = None
        for n, inst in enumerate(group):
            s, t = st(inst)
            ident, b = inst[0], inst[3]
            final = inst[4] if len(inst) > 4 and inst[4] is not None else b
            # (CURS before the symbol: its reference corner's S, less what the decoder adds first.)
            extent = (final.h - 1) if transposed else (final.w - 1)
            pre = extent if (right if not transposed else bottom) else 0
            target = s - pre
            if n == 0:
                encode_int(mq, ints["fs"], target - first_s)
                first_s = target
            else:
                encode_int(mq, ints["ds"], target - cur_s - ds_offset)
            cur_s = target
            if strips > 1:
                encode_int(mq, ints["it"], t - t0)
            encode_id(mq, idcx, ident, code_len)
            if refine:
                if final is b:
                    encode_int(mq, ints["ri"], 0)
                else:
                    rdw, rdh, rdx, rdy = final.w - b.w, final.h - b.h, 0, 0
                    encode_int(mq, ints["ri"], 1)
                    for name, v in [("rdw", rdw), ("rdh", rdh), ("rdx", rdx), ("rdy", rdy)]:
                        encode_int(mq, ints[name], v)
                    encode_refine(mq, gr, final, b, (rdw >> 1) + rdx, (rdh >> 1) + rdy, 0, [(-1, -1), (-1, -1)])
            # (CURS moves on past the symbol: its extent less one, as the decoder adds it, before or after.)
            cur_s += extent
        encode_int(mq, ints["ds"], None)
        k += len(group)


def text_segment(number, w, h, instances, refers, code_len, strips=1, corner=1, transposed=False, ds_offset=0, refine=False, op=0):
    mq = MQ()
    ints = {k: Contexts() for k in ["dt", "fs", "ds", "it", "ri", "rdw", "rdh", "rdx", "rdy"]}
    encode_text(mq, ints, Contexts(), Contexts(), instances, code_len, strips, corner, transposed, ds_offset, refine)
    log_strips = {1: 0, 2: 1, 4: 2, 8: 3}[strips]
    flags = (2 if refine else 0) | (log_strips << 2) | (corner << 4) | (0x40 if transposed else 0) | ((ds_offset & 0x1F) << 10)
    head = region_info(w, h, 0, 0, op) + struct.pack(">H", flags)
    if refine:
        head += at_bytes([(-1, -1), (-1, -1)])
    head += struct.pack(">I", len(instances))
    return segment(number, 6, head + mq.flush(), refers)


# ---------------------------------------------------------------------------------------------------- pages


def pages(page: Bits) -> list[tuple[bytes, bytes | None]]:
    """Each page's JBIG2 stream (embedded: its segments), and its JBIG2Globals' if it has one."""
    out = []
    # 1. Generic regions in eight bands.
    s = segment(0, 48, page_info(W, H))
    band = H // 8
    for k in range(8):
        template, tpgdon = k // 2, k % 2 == 1
        at = [(4, -1), (-3, -2), (2, -2), (-2, -1)] if k == 0 else None
        s += generic_segment(k + 1, page.crop(0, k * band, W, band), 0, k * band, template, tpgdon, at)
    out.append((s + segment(9, 49, b""), None))
    # 2. MMR: Group 4 as libtiff writes it.
    im = Image.new("1", (W, H))
    im.putdata([v for r in page.rows for v in r])
    buf = io.BytesIO()
    im.save(buf, "TIFF", compression="group4")
    tif = Image.open(io.BytesIO(buf.getvalue()))
    offsets, counts = tif.tag_v2[273], tif.tag_v2[279]
    raw = b"".join(buf.getvalue()[o:o + c] for o, c in zip(offsets, counts))
    s = segment(0, 48, page_info(W, H)) + segment(1, 38, region_info(W, H, 0, 0) + bytes([1]) + raw)
    out.append((s + segment(2, 49, b""), None))
    # 3. A symbol dictionary of the tiles, and a text region placing them: two strips, the bottom-left corner.
    found = tiles(page)
    unique = []
    index = {}
    for _, _, t in found:
        if t.key() not in index:
            index[t.key()] = len(unique)
            unique.append(t)
    order = sorted(range(len(unique)), key=lambda i: unique[i].h)
    remap = {old: new for new, old in enumerate(order)}
    symbols = [unique[i] for i in order]
    code_len = (len(symbols) - 1).bit_length()
    instances = [(remap[index[t.key()]], x, y, t) for x, y, t in found]
    sd = symbol_dictionary(1, [(b, None) for b in symbols])
    s = segment(0, 48, page_info(W, H)) + sd + text_segment(2, W, H, instances, [1], code_len, strips=2, corner=0, ds_offset=-2)
    out.append((s + segment(3, 49, b""), None))
    # 4. The dictionary in JBIG2Globals; the top-right corner, transposed, four strips.
    globals_ = symbol_dictionary(1, [(b, None) for b in symbols], page=0)
    s = segment(2, 48, page_info(W, H)) + text_segment(3, W, H, instances, [1], code_len, strips=4, corner=3, transposed=True)
    out.append((s + segment(4, 49, b""), globals_))
    # 5. Refined instances: each placed as the next symbol, refined into its own.
    wrong = [(i[0] + 1) % len(symbols) for i in instances]
    refined = [(w_, x, y, symbols[w_], t) for w_, (_, x, y, t) in zip(wrong, instances)]
    s = segment(0, 48, page_info(W, H)) + sd + text_segment(2, W, H, refined, [1], code_len, strips=1, corner=1, refine=True)
    out.append((s + segment(3, 49, b""), None))
    # 6. A second dictionary: its symbols refinements of the first's, or two of them side by side (aggregates).
    small = symbols[: max(4, len(symbols) // 3)]
    rest = symbols[len(small):]
    first = symbol_dictionary(1, [(b, None) for b in small])
    how = []
    made = []
    for k, b in enumerate(rest):
        if k % 5 == 4 and b.w == 16:
            # (A symbol as two halves placed side by side: each half one of the first dictionary's, refined.)
            parts = [(0, 0, 0, small[0], b.crop(0, 0, 8, b.h)), (1, 8, 0, small[1], b.crop(8, 0, b.w - 8, b.h))]
            how.append(("aggregate", parts))
        else:
            ref = small[k % len(small)]
            how.append(("refine", k % len(small), 0, 0, ref))
        made.append(b)
    pairs = sorted(zip(made, how), key=lambda p: p[0].h)
    second = symbol_dictionary_refagg(2, pairs, len(small))
    all_syms = small + [b for b, _ in pairs]
    ids = {b.key(): k for k, b in enumerate(all_syms)}
    inst = [(ids[t.key()], x, y, t) for x, y, t in found]
    s = segment(0, 48, page_info(W, H)) + first + second + text_segment(3, W, H, inst, [1, 2], (len(all_syms) - 1).bit_length())
    out.append((s + segment(4, 49, b""), None))
    # 7. The page with noise (an intermediate generic region), refined into it by a refinement region (template 1).
    rng = random.Random(7)
    noisy = Bits(W, H, [r[:] for r in page.rows])
    for _ in range(600):
        x, y = rng.randrange(W), rng.randrange(H)
        noisy.rows[y][x] ^= 1
    mq = MQ()
    encode_refine(mq, Contexts(), page, noisy, 0, 0, 1, None)
    s = segment(0, 48, page_info(W, H)) + generic_segment(1, noisy, 0, 0, 0, True, kind=36)
    s += segment(2, 42, region_info(W, H, 0, 0, 4) + bytes([1]) + mq.flush(), [1])
    out.append((s + segment(3, 49, b""), None))
    # 8 and 9. The noisy page on the page, refined where it is with typical prediction (TPGRON), by template 0 and
    #    by template 1.
    for template in (0, 1):
        s = segment(0, 48, page_info(W, H)) + generic_segment(1, noisy, 0, 0, 0, True)
        mq = MQ()
        encode_refine(mq, Contexts(), page, noisy, 0, 0, template, [(-1, -1), (-1, -1)], tpgron=True)
        at = at_bytes([(-1, -1), (-1, -1)]) if template == 0 else b""
        s += segment(2, 42, region_info(W, H, 0, 0, 4) + bytes([template | 2]) + at + mq.flush())
        out.append((s + segment(3, 49, b""), None))
    # 10. Striped, of unknown height: three stripes.
    s = segment(0, 48, page_info(W, 0xFFFFFFFF, striped=128))
    n = 1
    for top, h in [(0, 128), (128, 128), (256, 64)]:
        s += generic_segment(n, page.crop(0, top, W, h), 0, top, 1, True)
        s += segment(n + 1, 50, struct.pack(">I", top + h - 1))
        n += 2
    out.append((s + segment(n, 49, b""), None))
    return out


def symbol_dictionary_refagg(number, pairs, inputs):
    return symbol_dictionary(number, [(b, how) for b, how in pairs], template=0, refers=(1,), inputs=inputs, refagg=True)


# ------------------------------------------------------------------------------------------------------ PDF


def pdf(streams) -> bytes:
    objects = []
    kids = []
    n_pages = len(streams)
    # Objects: 1 catalog, 2 pages, then per page: page, contents, image (and globals).
    num = 3
    plan = []
    for data, glob in streams:
        page_n, contents_n, image_n = num, num + 1, num + 2
        glob_n = num + 3 if glob else None
        num += 4 if glob else 3
        plan.append((page_n, contents_n, image_n, glob_n, data, glob))
        kids.append(f"{page_n} 0 R")
    objects.append((1, b"<< /Type /Catalog /Pages 2 0 R >>"))
    objects.append((2, f"<< /Type /Pages /Kids [{' '.join(kids)}] /Count {n_pages} >>".encode()))
    for page_n, contents_n, image_n, glob_n, data, glob in plan:
        objects.append((page_n, f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {W} {H}] /Resources << /XObject << /Im0 {image_n} 0 R >> >> /Contents {contents_n} 0 R >>".encode()))
        content = f"q {W} 0 0 {H} 0 0 cm /Im0 Do Q".encode()
        objects.append((contents_n, b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream"))
        parms = f" /DecodeParms << /JBIG2Globals {glob_n} 0 R >>" if glob_n else ""
        head = f"<< /Type /XObject /Subtype /Image /Width {W} /Height {H} /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /JBIG2Decode{parms} /Length {len(data)} >>"
        objects.append((image_n, head.encode() + b"\nstream\n" + data + b"\nendstream"))
        if glob_n:
            z = zlib.compress(glob)
            objects.append((glob_n, b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(z) + z + b"\nendstream"))
    objects.sort()
    out = bytearray(b"%PDF-1.5\n%\xe2\xe3\xcf\xd3\n")
    offsets = {}
    for n, body in objects:
        offsets[n] = len(out)
        out += f"{n} 0 obj\n".encode() + body + b"\nendobj\n"
    xref = len(out)
    count = max(offsets) + 1
    out += f"xref\n0 {count}\n0000000000 65535 f \n".encode()
    for n in range(1, count):
        out += f"{offsets[n]:010d} 00000 n \n".encode()
    out += f"trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    return bytes(out)


def main():
    page = picture()
    streams = pages(page)
    data = pdf(streams)
    (HERE / "scan-jbig2.pdf").write_bytes(data)
    im = Image.new("L", (W, H))
    im.putdata([0 if v else 255 for r in page.rows for v in r])
    im.convert("RGB").save(HERE / "scan-jbig2.png")
    # Each page as Poppler and jbig2dec decode it: the picture (Poppler's TPGRON isn't jbig2dec's, and jbig2dec
    # doesn't read intermediate regions).
    want = list(im.getdata())
    with tempfile.TemporaryDirectory() as tmp:
        for k, (stream, glob) in enumerate(streams):
            said = []
            if shutil.which("pdfimages"):
                single = pdf([(stream, glob)])
                Path(f"{tmp}/one.pdf").write_bytes(single)
                subprocess.run(["pdfimages", "-png", f"{tmp}/one.pdf", f"{tmp}/p"], check=True, capture_output=True)
                got = Image.open(f"{tmp}/p-000.png").convert("L")
                bad = sum(1 for a, b in zip(got.getdata(), want) if (a < 128) != (b < 128))
                said.append(f"Poppler: {'the same' if bad == 0 else f'{bad} pixels differ'}")
            if shutil.which("jbig2dec"):
                Path(f"{tmp}/page.jb2").write_bytes(stream)
                args = ["jbig2dec", "-q", "-e", "-t", "pbm", "-o", f"{tmp}/o.pbm"]
                if glob:
                    Path(f"{tmp}/glob.jb2").write_bytes(glob)
                    args.append(f"{tmp}/glob.jb2")
                subprocess.run(args + [f"{tmp}/page.jb2"], capture_output=True)
                try:
                    got = Image.open(f"{tmp}/o.pbm").convert("L")
                    bad = sum(1 for a, b in zip(got.getdata(), want) if (a < 128) != (b < 128))
                    said.append(f"jbig2dec: {'the same' if bad == 0 else f'{bad} pixels differ'}")
                except OSError:
                    said.append("jbig2dec: nothing")
            print(f"page {k + 1}: " + ", ".join(said))


if __name__ == "__main__":
    main()
