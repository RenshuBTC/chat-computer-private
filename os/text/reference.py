"""Prints the tables in src/tests.rs: where HarfBuzz sets each letter of some lines of text in Lora, at its regular
weight and its bold (700), kerning and all: each glyph's start along the line, in font units (1000 to the em), and
the line's width (SET); and how it sets accents typed as characters of their own (MARKS): each glyph, and where it's
drawn (along the line, and up from the baseline).

Made with uharfbuzz 0.57.0:
    python reference.py
"""
from pathlib import Path

import uharfbuzz as hb

FONT = Path(__file__).resolve().parent.parent.parent / "chatcomputer" / "ui" / "assets" / "fonts" / "Lora.ttf"
LINES = ["AVATAR", "Today, Tokyo.", "WAVY lines: P. F, y. r.", "“Quoted” — LT AV To", "Hello, world", " T A Y "]
# Accents typed after their letters: composed into one letter ("e" and an acute, "é"; a dot below and a circumflex in
# either order, "ệ"; a tilde past a dot below that doesn't compose, "ñ" with it under), or placed on the letter (or on
# a letter they partly composed into) where the font's anchors put them, a mark on a mark among them; an i and a j
# losing their dots. Ones where HarfBuzz would change a mark's glyph for another (Lora's own glyph composition, which
# the own system leaves out) aren't among them.
MARKED = ["e\u0301", "e\u0323\u0302", "e\u0302\u0323", "n\u0303\u0323", "y\u030a", "k\u0323\u0307", "j\u0301", "i\u0308",
          "a\u0328\u0301", "a\u0304\u0301", "w\u030a", "h\u0306", "o\u031b\u0309\u0323", "xe\u0301x", "Te\u0301"]


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
    print("const MARKS: [(&str, bool, &[(u16, i32, i32)]); %d] = [" % (2 * len(MARKED)))
    for text in MARKED:
        for bold in (False, True):
            font = hb.Font(face)
            font.set_variations({"wght": 700 if bold else 400})
            buf = hb.Buffer()
            buf.add_str(text)
            buf.guess_segment_properties()
            hb.shape(font, buf)
            x, glyphs = 0, []
            for info, p in zip(buf.glyph_infos, buf.glyph_positions):
                glyphs.append((info.codepoint, x + p.x_offset, p.y_offset))
                x += p.x_advance
            escaped = "".join(c if ord(c) < 0x300 else f"\\u{{{ord(c):x}}}" for c in text)
            print(f'    ("{escaped}", {str(bold).lower()}, &{glyphs}),')
    print("];")


if __name__ == "__main__":
    main()
