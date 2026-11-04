"""Writes src/marks_reference.txt, what tests.rs checks marks.rs against: where HarfBuzz (and so a browser) puts the
marks (accents typed as characters of their own) on letters and on each other in Lora, the own system's text font,
at its regular weight and its bold (700).

Each line: the text (its characters' code points, in hexadecimal, joined by "+"), the weight, then each glyph
HarfBuzz sets for it: its id, and where it's drawn from the start (x, y: font units, y up). The first is the letter
(or the letter with marks composed into it), at 0, 0; the marks after it are drawn where the font's GPOS puts them.

Made with uharfbuzz 0.57.0:
    python marks_reference.py
"""
from pathlib import Path

import uharfbuzz as hb

HERE = Path(__file__).resolve().parent
FONT = HERE.parent.parent / "chatcomputer" / "ui" / "assets" / "fonts" / "Lora.ttf"
# Letters and marks that don't compose into one character (or compose only partly), so the marks are placed: on the
# letter, on a letter with marks composed into it, and on each other; one with no anchors for its letter (q́).
TEXTS = ["ẙ", "ḳ̇", "q́", "j́", "ą́", "ā́", "ṇ̃",
         "k̈́", "x̣̂", "ẘ", "m̂", "E̱"[:1] + "̣̄", "g̃̀",
         "ț", "v̧", "ợ̉", "h̆"]


def main() -> None:
    face = hb.Face(hb.Blob.from_file_path(str(FONT)))
    lines = []
    for text in TEXTS:
        for weight in (400, 700):
            font = hb.Font(face)
            font.set_variations({"wght": weight})
            buf = hb.Buffer()
            buf.add_str(text)
            buf.guess_segment_properties()
            hb.shape(font, buf)
            x, placed = 0, []
            for info, pos in zip(buf.glyph_infos, buf.glyph_positions):
                placed.append(f"{info.codepoint}:{x + pos.x_offset}:{pos.y_offset}")
                x += pos.x_advance
            lines.append("+".join(f"{ord(c):04X}" for c in text) + f" {weight} " + " ".join(placed))
    (HERE / "src" / "marks_reference.txt").write_text("\n".join(lines) + "\n")
    print(len(lines), "lines")


if __name__ == "__main__":
    main()
