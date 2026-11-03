"""Prints the table in src/tests.rs: where HarfBuzz sets each letter of some lines of text in Lora, at its regular
weight and its bold (700), kerning and all: each glyph's start along the line, in font units (1000 to the em), and
the line's width.

Made with uharfbuzz 0.57.0:
    python reference.py
"""
from pathlib import Path

import uharfbuzz as hb

FONT = Path(__file__).resolve().parent.parent.parent / "chatcomputer" / "ui" / "assets" / "fonts" / "Lora.ttf"
LINES = ["AVATAR", "Today, Tokyo.", "WAVY lines: P. F, y. r.", "“Quoted” — LT AV To", "Hello, world", " T A Y "]


def main() -> None:
    face = hb.Face(hb.Blob.from_file_path(str(FONT)))
    print("const SET: [(&str, bool, &[i32], i32); %d] = [" % (2 * len(LINES)))
    for text in LINES:
        for bold in (False, True):
            font = hb.Font(face)
            font.set_variations({"wght": 700 if bold else 400})
            buf = hb.Buffer()
            buf.add_str(text)
            buf.guess_segment_properties()
            hb.shape(font, buf)
            x, starts = 0, []
            for p in buf.glyph_positions:
                starts.append(x)
                x += p.x_advance
            quoted = text.replace("\\", "\\\\").replace('"', '\\"')
            print(f'    ("{quoted}", {str(bold).lower()}, &{starts}, {x}),')
    print("];")


if __name__ == "__main__":
    main()
