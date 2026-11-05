#!/usr/bin/env python3
"""Makes scan-jpx.pdf and scan-jpx.png, for the pdf crate's JPEG 2000 decoder (src/jpx.rs): a colour picture (drawn
here, as a scan would be) coded by OpenJPEG's opj_compress (Ubuntu's libopenjp2-tools) each way the decoder reads, a
page each; scan-jpx.png is each page's picture as opj_decompress decodes it, the pages one under another.

    python3 tests/jpx.py

The pages (opj_compress's options):
 1. lossless (5/3, the colour transform RCT), four resolutions;
 2. lossy (9/7, ICT), a tenth of the size;
 3-8. each code-block style: arithmetic coding bypassed (-M 1), contexts reset (2), every pass terminated (4),
    vertically causal contexts (8), predictable termination (16), segmentation symbols (32);
 9. all of them at once, with SOP and EPH markers;
10. 32x32 tiles, the image and its tiles offset, a tile-part for each resolution;
11-13. precincts (32x32, then 16x16) in RPCL, PCRL and CPRL order;
14. three layers (the last lossless);
15. a progression order change (POC) in the middle;
16. a region of interest (max-shift, component 0);
17. no colour transform;
18. gray;
19. gray of 12 bits a sample;
20. YCbCr with its chroma at half the size each way (no colour space in the PDF: the decoder makes it RGB);
21. lossy with bypass and termination, three layers.
"""
from __future__ import annotations

import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
W, H = 80, 56
LOSSY = {2, 21}


def picture() -> Image.Image:
    """A small colour scan: a tinted page with words, a coloured box and a line."""
    im = Image.new("RGB", (W, H))
    px = im.load()
    for y in range(H):
        for x in range(W):
            px[x, y] = (235 - y // 2, 230 - x // 4, 210 + (x + y) % 23)
    d = ImageDraw.Draw(im)
    try:
        font = ImageFont.load_default(size=13)
    except TypeError:
        font = ImageFont.load_default()
    d.text((4, 3), "Receipt", font=font, fill=(20, 30, 90))
    d.text((4, 20), "12.50 SGD", font=font, fill=(10, 10, 10))
    d.rectangle((52, 34, 74, 50), fill=(200, 40, 30), outline=(40, 40, 40))
    d.line((2, 52, 48, 38), fill=(0, 120, 60), width=2)
    return im


def pages(tmp: Path, im: Image.Image) -> list[tuple[bytes, str | None, Image.Image]]:
    """Each page's codestream, its PDF colour space, and its picture as opj_decompress decodes it."""
    src = tmp / "in.ppm"
    im.save(src)
    gray = tmp / "gray.pgm"
    im.convert("L").save(gray)
    # 12-bit gray, as raw samples (big-endian 16-bit words).
    raw12 = tmp / "gray12.raw"
    raw12.write_bytes(b"".join(int(v * 4095 / 255).to_bytes(2, "big") for v in im.convert("L").getdata()))
    # YCbCr with half-size chroma, as raw: Y, then Cb and Cr at 40x28.
    ycc = im.convert("YCbCr")
    y, cb, cr = ycc.split()
    small = [c.resize((W // 2, H // 2), Image.BOX) for c in (cb, cr)]
    raw420 = tmp / "ycc.raw"
    raw420.write_bytes(y.tobytes() + small[0].tobytes() + small[1].tobytes())
    variants = [
        (src, ["-n", "4"], "/DeviceRGB"),
        (src, ["-n", "4", "-I", "-r", "10"], "/DeviceRGB"),
        (src, ["-n", "4", "-M", "1"], "/DeviceRGB"),
        (src, ["-n", "4", "-M", "2"], "/DeviceRGB"),
        (src, ["-n", "4", "-M", "4"], "/DeviceRGB"),
        (src, ["-n", "4", "-M", "8"], "/DeviceRGB"),
        (src, ["-n", "4", "-M", "16"], "/DeviceRGB"),
        (src, ["-n", "4", "-M", "32"], "/DeviceRGB"),
        (src, ["-n", "4", "-M", "63", "-SOP", "-EPH"], "/DeviceRGB"),
        (src, ["-n", "3", "-t", "32,32", "-T", "3,2", "-d", "5,7", "-TP", "R"], "/DeviceRGB"),
        (src, ["-n", "3", "-c", "[32,32],[16,16]", "-p", "RPCL"], "/DeviceRGB"),
        (src, ["-n", "3", "-c", "[16,16]", "-p", "PCRL"], "/DeviceRGB"),
        (src, ["-n", "3", "-c", "[16,16]", "-p", "CPRL"], "/DeviceRGB"),
        (src, ["-n", "4", "-r", "20,10,1"], "/DeviceRGB"),
        (src, ["-n", "4", "-POC", "T0=0,0,1,2,3,CPRL/T0=2,0,1,4,3,LRCP"], "/DeviceRGB"),
        (src, ["-n", "4", "-ROI", "c=0,U=8"], "/DeviceRGB"),
        (src, ["-n", "4", "-mct", "0"], "/DeviceRGB"),
        (gray, ["-n", "4"], "/DeviceGray"),
        (raw12, ["-n", "4", "-F", f"{W},{H},1,12,u"], "/DeviceGray"),
        (raw420, ["-n", "3", "-mct", "0", "-F", f"{W},{H},3,8,u@1x1:2x2:2x2"], None),
        (src, ["-n", "4", "-I", "-r", "20,10,5", "-M", "5"], "/DeviceRGB"),
    ]
    out = []
    for k, (inp, args, space) in enumerate(variants, 1):
        j2k = tmp / f"p{k}.j2k"
        subprocess.run(["opj_compress", "-i", str(inp), "-o", str(j2k)] + args, check=True, capture_output=True)
        decoded = tmp / f"p{k}.{'pgm' if space == '/DeviceGray' else 'ppm'}"
        extra = ["-upsample", "-force-rgb"] if space is None else []
        subprocess.run(["opj_decompress", "-i", str(j2k), "-o", str(decoded)] + extra, check=True, capture_output=True)
        pic = Image.open(decoded)
        if pic.mode in ("I;16", "I;16B", "I"):
            # (12 bits, which opj_decompress writes as 16: made 8, as the decoder makes them.)
            values = [round(v * 4095 / 65535) for v in pic.convert("I").getdata()]
            pic = Image.new("L", pic.size)
            pic.putdata([int(v * 255 / 4095 + 0.5) for v in values])
        out.append((j2k.read_bytes(), space, pic.convert("RGB")))
    return out


def pdf(streams) -> bytes:
    objects = [(1, b"<< /Type /Catalog /Pages 2 0 R >>")]
    kids = []
    num = 3
    plan = []
    for data, space in streams:
        plan.append((num, num + 1, num + 2, data, space))
        kids.append(f"{num} 0 R")
        num += 3
    objects.append((2, f"<< /Type /Pages /Kids [{' '.join(kids)}] /Count {len(streams)} >>".encode()))
    for page_n, contents_n, image_n, data, space in plan:
        objects.append((page_n, f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {W} {H}] /Resources << /XObject << /Im0 {image_n} 0 R >> >> /Contents {contents_n} 0 R >>".encode()))
        content = f"q {W} 0 0 {H} 0 0 cm /Im0 Do Q".encode()
        objects.append((contents_n, b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream"))
        cs = f" /ColorSpace {space}" if space else ""
        head = f"<< /Type /XObject /Subtype /Image /Width {W} /Height {H}{cs} /BitsPerComponent 8 /Filter /JPXDecode /Length {len(data)} >>"
        objects.append((image_n, head.encode() + b"\nstream\n" + data + b"\nendstream"))
    out = bytearray(b"%PDF-1.5\n%\xe2\xe3\xcf\xd3\n")
    offsets = {}
    for n, body in sorted(objects):
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
    im = picture()
    with tempfile.TemporaryDirectory() as tmp:
        made = pages(Path(tmp), im)
    (HERE / "scan-jpx.pdf").write_bytes(pdf([(d, s) for d, s, _ in made]))
    sheet = Image.new("RGB", (W, H * len(made)))
    for k, (_, _, pic) in enumerate(made):
        sheet.paste(pic, (0, k * H))
    sheet.save(HERE / "scan-jpx.png")
    # (How far each lossless page is from the picture itself: nothing.)
    for k, (_, space, pic) in enumerate(made, 1):
        want = im.convert("L").convert("RGB") if space == "/DeviceGray" else im
        if k not in LOSSY and space is not None and k != 19:
            same = pic.tobytes() == want.tobytes()
            print(f"page {k}: {'the picture exactly' if same else 'NOT the picture'}")


if __name__ == "__main__":
    main()
