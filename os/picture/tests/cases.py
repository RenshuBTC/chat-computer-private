#!/usr/bin/env python3
"""Makes what the picture crate's tests read, in the folder given: pictures, and the RGB pixels each should come out
as (on the tests' background, where they're transparent), with the box each is decoded to fit and how near it must
come (cases.txt: name, file, width, height, box, most a sample may differ, least PSNR in dB).

JPEGs are written and decoded by Pillow (libjpeg-turbo): every kind of subsampling, grey, progressive, restart
markers, odd sizes, every quality from 5 to 100, optimized Huffman tables, Adobe CMYK, EXIF's eight orientations, and
pictures decoded smaller than they are. PNGs by Pillow, and by a writer here for what Pillow doesn't write (every
depth of every colour type, 16 bits, transparency, Adam7 interlacing, each filter), whose pixels are worked out here.
GIFs by Pillow, interlaced or not, with a transparent colour. And pictures for the crate's JPEG writer, with
Pillow's JPEG of each, which it must write the same.

    python3 tests/cases.py <folder>
"""
from __future__ import annotations

import io
import random
import struct
import sys
import zlib
from pathlib import Path

from PIL import Image, ImageDraw, ImageOps

BACKGROUND = (250, 249, 246)


def photo(w: int, h: int, seed: int = 1) -> Image.Image:
    """Something like a photo: gradients, shapes with sharp and soft edges, lines and a little noise."""
    r = random.Random(seed)
    im = Image.new("RGB", (w, h))
    px = im.load()
    for y in range(h):
        for x in range(w):
            px[x, y] = ((x * 255) // max(w - 1, 1), (y * 255) // max(h - 1, 1), ((x + y) * 128) // max(w + h, 1) + 60)
    d = ImageDraw.Draw(im)
    for _ in range(12):
        x0, y0 = r.randrange(w), r.randrange(h)
        x1, y1 = x0 + r.randrange(1, max(2, w // 3)), y0 + r.randrange(1, max(2, h // 3))
        colour = (r.randrange(256), r.randrange(256), r.randrange(256))
        (d.ellipse if r.random() < 0.5 else d.rectangle)([x0, y0, x1, y1], fill=colour)
    for _ in range(8):
        d.line([(r.randrange(w), r.randrange(h)), (r.randrange(w), r.randrange(h))], fill=(0, 0, 0), width=1)
    for _ in range(w * h // 20):
        x, y = r.randrange(w), r.randrange(h)
        c = px[x, y]
        px[x, y] = tuple(max(0, min(255, v + r.randrange(-30, 31))) for v in c)
    return im


def on_background(im: Image.Image) -> Image.Image:
    """An RGBA picture on the background, rounded as the crate rounds: (c·a + b·(255 − a) + 127) / 255."""
    rgba = im.convert("RGBA")
    out = Image.new("RGB", rgba.size)
    src, dst = rgba.load(), out.load()
    for y in range(rgba.height):
        for x in range(rgba.width):
            r, g, b, a = src[x, y]
            dst[x, y] = tuple((c * a + bg * (255 - a) + 127) // 255 for c, bg in zip((r, g, b), BACKGROUND))
    return out


class Cases:
    def __init__(self, folder: Path) -> None:
        self.folder = folder
        self.lines: list[str] = []

    def add(self, name: str, ext: str, data: bytes, expected: Image.Image, box: tuple[int, int] | None = None,
            diff: int = 0, psnr: float = 0.0) -> None:
        (self.folder / f"{name}.{ext}").write_bytes(data)
        (self.folder / f"{name}.rgb").write_bytes(expected.convert("RGB").tobytes())
        bw, bh = box or (100_000, 100_000)
        self.lines.append(f"{name}\t{name}.{ext}\t{expected.width}\t{expected.height}\t{bw}\t{bh}\t{diff}\t{psnr}")

    def save(self) -> None:
        (self.folder / "cases.txt").write_text("\n".join(self.lines) + "\n")


def jpeg(im: Image.Image, **options) -> bytes:
    out = io.BytesIO()
    im.save(out, "JPEG", **options)
    return out.getvalue()


def jpeg_cases(c: Cases) -> None:
    big = photo(320, 240, 2)
    for name, im, options in [
        ("jpeg-420", big, dict(quality=85, subsampling=2)),
        ("jpeg-422", big, dict(quality=85, subsampling=1)),
        ("jpeg-444", big, dict(quality=90, subsampling=0)),
        ("jpeg-grey", big.convert("L"), dict(quality=85)),
        ("jpeg-progressive", big, dict(quality=85, progressive=True)),
        ("jpeg-progressive-444", big, dict(quality=95, progressive=True, subsampling=0)),
        ("jpeg-progressive-grey", big.convert("L"), dict(quality=75, progressive=True)),
        ("jpeg-restarts", big, dict(quality=85, restart_marker_blocks=3)),
        ("jpeg-restarts-progressive", big, dict(quality=85, progressive=True, restart_marker_rows=1)),
        ("jpeg-optimized", big, dict(quality=85, optimize=True)),
        ("jpeg-q100", big, dict(quality=100, subsampling=0)),
        ("jpeg-q5", big, dict(quality=5)),
        ("jpeg-odd-420", photo(33, 17, 3), dict(quality=90, subsampling=2)),
        ("jpeg-odd-422", photo(31, 9, 4), dict(quality=90, subsampling=1)),
        ("jpeg-tiny", photo(1, 1, 5), dict(quality=90)),
        ("jpeg-tall", photo(7, 61, 6), dict(quality=80, subsampling=2)),
    ]:
        data = jpeg(im, **options)
        if "restart" in "".join(options) and b"\xff\xdd" not in data:
            print(f"(this Pillow doesn't write restart markers: {name} left out)")
            continue
        c.add(name, "jpg", data, Image.open(io.BytesIO(data)).convert("RGB"), diff=1)
    # Adobe's CMYK (stored inverted): as the crate makes it RGB, (255 − C)·(255 − K) / 255 and so on.
    cmyk = photo(64, 48, 7).convert("CMYK")
    data = jpeg(cmyk, quality=90)
    back = Image.open(io.BytesIO(data))
    assert back.mode == "CMYK"
    rgb = Image.new("RGB", back.size)
    src, dst = back.load(), rgb.load()
    for y in range(back.height):
        for x in range(back.width):
            cc, m, yy, k = src[x, y]
            dst[x, y] = tuple((255 - v) * (255 - k) // 255 for v in (cc, m, yy))
    c.add("jpeg-cmyk", "jpg", data, rgb, diff=1)
    # A photo as a phone saves it, turned by EXIF's orientation: as Pillow's exif_transpose turns it.
    wide = photo(48, 32, 8)
    for orientation in range(1, 9):
        exif = Image.Exif()
        exif[0x0112] = orientation
        data = jpeg(wide, quality=90, exif=exif.tobytes())
        c.add(f"jpeg-orientation-{orientation}", "jpg", data, ImageOps.exif_transpose(Image.open(io.BytesIO(data))).convert("RGB"),
              diff=1)
    # Decoded smaller (half, a quarter, an eighth, and a size between): near enough to the whole picture averaged down.
    data = jpeg(photo(400, 300, 9), quality=90)
    whole = Image.open(io.BytesIO(data)).convert("RGB")
    # (An eighth is each block's average, as libjpeg makes it, then shrunk a little more: blurrier than the rest.)
    for name, box, psnr in [("half", (200, 150), 30.0), ("quarter", (100, 75), 30.0), ("eighth", (50, 37), 26.0),
                            ("between", (130, 130), 30.0)]:
        w, h = fitted(400, 300, *box)
        c.add(f"jpeg-smaller-{name}", "jpg", data, whole.resize((w, h), Image.Resampling.BOX), box=box, psnr=psnr)


def fitted(w: int, h: int, mw: int, mh: int) -> tuple[int, int]:
    if w <= mw and h <= mh:
        return w, h
    if w * mh >= h * mw:
        return mw, max(1, h * mw // w)
    return max(1, w * mh // h), mh


def png_chunk(kind: bytes, body: bytes) -> bytes:
    return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))


def filtered(row: bytes, prev: bytes, bpp: int, f: int) -> bytes:
    out = bytearray(len(row))
    for i in range(len(row)):
        a = row[i - bpp] if i >= bpp else 0
        b = prev[i]
        cc = prev[i - bpp] if i >= bpp else 0
        if f == 0:
            p = 0
        elif f == 1:
            p = a
        elif f == 2:
            p = b
        elif f == 3:
            p = (a + b) // 2
        else:
            pp = a + b - cc
            pa, pb, pc = abs(pp - a), abs(pp - b), abs(pp - cc)
            p = a if pa <= pb and pa <= pc else (b if pb <= pc else cc)
        out[i] = (row[i] - p) & 0xFF
    return bytes([f]) + bytes(out)


ADAM7 = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]


def png(width: int, height: int, color: int, depth: int, pixels: list[list[tuple[int, ...]]], palette: bytes = b"",
        trns: bytes = b"", interlace: bool = False) -> bytes:
    """A PNG written here: the samples given (each pixel's, at `depth` bits), each row filtered a different way."""
    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[color]
    bits = channels * depth
    bpp = max(1, bits // 8)

    def pack(row: list[tuple[int, ...]]) -> bytes:
        samples = [v for px in row for v in px]
        if depth == 16:
            return b"".join(struct.pack(">H", v) for v in samples)
        if depth == 8:
            return bytes(samples)
        out, acc, n = bytearray(), 0, 0
        for v in samples:
            acc = acc << depth | v
            n += depth
            if n == 8:
                out.append(acc)
                acc, n = 0, 0
        if n:
            out.append(acc << (8 - n))
        return bytes(out)

    raw = bytearray()
    passes = ADAM7 if interlace else [(0, 0, 1, 1)]
    k = 0
    for x0, y0, dx, dy in passes:
        rows = [[pixels[y][x] for x in range(x0, width, dx)] for y in range(y0, height, dy)]
        rows = [r for r in rows if r]
        prev = bytes(len(pack(rows[0]))) if rows else b""
        for r in rows:
            packed = pack(r)
            raw += filtered(packed, prev, bpp, k % 5)
            prev = packed
            k += 1
    data = zlib.compress(bytes(raw), 9)
    half = len(data) // 2
    out = b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, depth, color, 0, 0, int(interlace)))
    if palette:
        out += png_chunk(b"PLTE", palette)
    if trns:
        out += png_chunk(b"tRNS", trns)
    # The image data in two chunks (they're joined), and a chunk the crate passes over.
    return out + png_chunk(b"IDAT", data[:half]) + png_chunk(b"tEXt", b"Comment\0made by hand") + png_chunk(b"IDAT", data[half:]) + \
        png_chunk(b"IEND", b"")


def png_cases(c: Cases) -> None:
    im = photo(45, 31, 10)
    out = io.BytesIO()
    im.save(out, "PNG")
    c.add("png-pillow-rgb", "png", out.getvalue(), im)
    rgba = im.convert("RGBA")
    a = rgba.load()
    for y in range(rgba.height):
        for x in range(rgba.width):
            r_, g_, b_, _ = a[x, y]
            a[x, y] = (r_, g_, b_, (x * 255) // (rgba.width - 1))
    out = io.BytesIO()
    rgba.save(out, "PNG")
    c.add("png-pillow-rgba", "png", out.getvalue(), on_background(rgba))
    pal = im.convert("P", palette=Image.Palette.ADAPTIVE, colors=16)
    out = io.BytesIO()
    pal.save(out, "PNG", transparency=3)
    c.add("png-pillow-palette", "png", out.getvalue(), on_background(Image.open(io.BytesIO(out.getvalue()))))
    grey = im.convert("L")
    out = io.BytesIO()
    grey.save(out, "PNG")
    c.add("png-pillow-grey", "png", out.getvalue(), grey)

    # Written here: every colour type at every depth it can have, interlaced and not.
    w, h = 13, 11
    r = random.Random(11)
    for color, depths in [(0, [1, 2, 4, 8, 16]), (2, [8, 16]), (3, [1, 2, 4, 8]), (4, [8, 16]), (6, [8, 16])]:
        for depth in depths:
            for interlace in (False, True):
                top = (1 << depth) - 1
                channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[color]
                count = min(1 << depth, 256) if color == 3 else 0
                pixels = [[tuple(r.randrange(count if color == 3 else top + 1) for _ in range(channels)) for _ in range(w)]
                          for _ in range(h)]
                palette = bytes(r.randrange(256) for _ in range(3 * count)) if color == 3 else b""
                trns = b""
                transparent = None
                if color == 3:
                    trns = bytes(r.randrange(256) for _ in range(count // 2 + 1))
                elif color in (0, 2) and depth >= 2:
                    transparent = pixels[2][3]
                    trns = b"".join(struct.pack(">H", v) for v in transparent)
                data = png(w, h, color, depth, pixels, palette, trns, interlace)
                expected = Image.new("RGB", (w, h))
                e = expected.load()
                eight = lambda v: v >> 8 if depth == 16 else (v if depth == 8 else v * 255 // top)  # noqa: E731
                for y in range(h):
                    for x in range(w):
                        px = pixels[y][x]
                        if color == 3:
                            rgb = tuple(palette[3 * px[0]:3 * px[0] + 3])
                            alpha = trns[px[0]] if px[0] < len(trns) else 255
                        elif color in (0, 2):
                            rgb = tuple(eight(v) for v in px) * (3 if color == 0 else 1)
                            alpha = 0 if transparent is not None and px == transparent else 255
                        elif color == 4:
                            rgb, alpha = (eight(px[0]),) * 3, eight(px[1])
                        else:
                            rgb, alpha = tuple(eight(v) for v in px[:3]), eight(px[3])
                        e[x, y] = tuple((v * alpha + bg * (255 - alpha) + 127) // 255 for v, bg in zip(rgb, BACKGROUND))
                c.add(f"png-{color}-{depth}{'-interlaced' if interlace else ''}", "png", data, expected)
    # A big one shrunk to fit.
    big = photo(300, 200, 12)
    out = io.BytesIO()
    big.save(out, "PNG")
    c.add("png-shrunk", "png", out.getvalue(), big.resize((150, 100), Image.Resampling.BOX), box=(150, 120), psnr=45.0)


def gif_cases(c: Cases) -> None:
    im = photo(37, 23, 13).convert("P", palette=Image.Palette.ADAPTIVE, colors=200)
    for interlace in (True, False):
        out = io.BytesIO()
        im.save(out, "GIF", interlace=interlace, transparency=7)
        back = Image.open(io.BytesIO(out.getvalue()))
        c.add(f"gif{'-interlaced' if interlace else ''}", "gif", out.getvalue(), on_background(back))
    few = photo(20, 16, 14).convert("P", palette=Image.Palette.ADAPTIVE, colors=4)
    out = io.BytesIO()
    few.save(out, "GIF")
    c.add("gif-four-colours", "gif", out.getvalue(), Image.open(io.BytesIO(out.getvalue())).convert("RGB"))


def encoder_cases(folder: Path) -> None:
    """Pictures for the crate's JPEG writer, and Pillow's JPEG of each (4:2:0, its standard tables, as Pillow saves by
    default): encode.txt lists each one's name, width, height and quality. Sizes of every kind: whole MCUs, a block
    over, a pixel over, odd ones, a line, a dot."""
    lines = []
    sizes = [(64, 48), (65, 49), (72, 40), (105, 77), (16, 1), (1, 16), (1, 1), (37, 23), (200, 133), (2576, 11)]
    for n, ((w, h), q) in enumerate(zip(sizes, [85, 85, 75, 90, 85, 50, 85, 95, 85, 85])):
        im = photo(w, h, 30 + n)
        name = f"enc-{w}x{h}-q{q}"
        (folder / f"{name}.rgb").write_bytes(im.tobytes())
        out = io.BytesIO()
        im.save(out, "JPEG", quality=q, subsampling=2)
        (folder / f"{name}.jpg").write_bytes(out.getvalue())
        lines.append(f"{name}\t{w}\t{h}\t{q}")
    for q in (1, 10, 30, 60, 100):
        im = photo(50, 30, 99)
        name = f"enc-50x30-q{q}"
        (folder / f"{name}.rgb").write_bytes(im.tobytes())
        out = io.BytesIO()
        im.save(out, "JPEG", quality=q, subsampling=2)
        (folder / f"{name}.jpg").write_bytes(out.getvalue())
        lines.append(f"{name}\t50\t30\t{q}")
    (folder / "encode.txt").write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    folder = Path(sys.argv[1])
    folder.mkdir(parents=True, exist_ok=True)
    cases = Cases(folder)
    jpeg_cases(cases)
    png_cases(cases)
    gif_cases(cases)
    cases.save()
    encoder_cases(folder)
