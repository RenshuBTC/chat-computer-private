"""Writes what tests.rs checks the font engine against: Atkinson Hyperlegible Next's kerning and advances, as
HarfBuzz (and so a browser) has them.

src/kern_reference.txt: the kerning of every pair of some characters, as HarfBuzz shapes them (kern.rs).

A pair's kerning is how much HarfBuzz moves the second character closer (negative) or further away: the first glyph's
advance in the pair, less its advance alone. It's worked out at four weights (400, the font's default; 700, bold; and
the axis's ends, 200 and 800), where the font's kerning varies.

The file's first line lists the characters (their code points, in hexadecimal); then each pair that kerns at any of
the weights: its two characters and its kerning at each weight, in font units (1000 to the em). A pair of them not
listed doesn't kern.

src/advance_reference.txt: every glyph's advance at the weights other than the default (from the advances' own
variations, HVAR, as HarfBuzz reads them), a line for each glyph in order.

Made with uharfbuzz 0.56.3 (HarfBuzz 14.6.0) and fontTools 4.65.0:
    python reference.py
"""
from pathlib import Path

import uharfbuzz as hb
from fontTools.ttLib import TTFont

HERE = Path(__file__).resolve().parent
FONT = HERE / "testdata" / "AtkinsonHyperlegibleNext.ttf"
WEIGHTS = (400, 700, 200, 800)
# Printable ASCII with the space, and letters and punctuation from other Latin languages and typesetting.
CHARS = [chr(c) for c in range(0x20, 0x7F)] + list("àáâäåçèéêëíîïñóôöøúûüÿÀÁÂÄÅÇÈÉÊËÍÎÏÑÓÔÖØÚÛÜßæÆœŒ‘’“”«»–—…")


def main() -> None:
    face = hb.Face(hb.Blob.from_file_path(str(FONT)))
    cmap = TTFont(FONT).getBestCmap()
    order = TTFont(FONT).getGlyphOrder()
    gid = {c: order.index(cmap[ord(c)]) for c in CHARS if ord(c) in cmap}
    chars = [c for c in CHARS if c in gid]
    fonts = {}
    for w in WEIGHTS:
        fonts[w] = hb.Font(face)
        fonts[w].set_variations({"wght": w})

    def shape(text: str, w: int) -> tuple[list[int], list[int]]:
        buf = hb.Buffer()
        buf.add_str(text)
        buf.guess_segment_properties()
        hb.shape(fonts[w], buf)
        return [i.codepoint for i in buf.glyph_infos], [p.x_advance for p in buf.glyph_positions]

    alone = {(c, w): shape(c, w)[1][0] for c in chars for w in WEIGHTS}
    lines = ["chars " + " ".join(f"{ord(c):04X}" for c in chars)]
    for a in chars:
        for b in chars:
            kerns = []
            for w in WEIGHTS:
                glyphs, advances = shape(a + b, w)
                # (No other glyphs: the font substitutes none of these, so the pair is its two characters' glyphs.)
                assert glyphs == [gid[a], gid[b]], (a, b, glyphs)
                kerns.append(advances[0] - alone[(a, w)])
            if any(kerns):
                lines.append(f"{ord(a):04X} {ord(b):04X} " + " ".join(str(k) for k in kerns))
    (HERE / "src" / "kern_reference.txt").write_text("\n".join(lines) + "\n")
    print(len(lines) - 1, "pairs kern, of", len(chars) ** 2)
    glyphs = len(order)
    advances = [" ".join(str(fonts[w].get_glyph_h_advance(g)) for w in WEIGHTS[1:]) for g in range(glyphs)]
    (HERE / "src" / "advance_reference.txt").write_text("\n".join(advances) + "\n")
    print(glyphs, "glyphs' advances at", WEIGHTS[1:])


if __name__ == "__main__":
    main()
