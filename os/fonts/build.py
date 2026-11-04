#!/usr/bin/env python3
"""Builds the own system's fallback fonts: the characters its fonts (Lora, and Atkinson Hyperlegible Mono for code) lack
that Claude writes (arrows, check marks, maths, Greek, emoji, and box drawing in code; Chinese, Japanese and Korean),
from Google's Noto fonts. See README.md.

    ./build.py              downloads the Noto fonts (checking their SHA-256), builds the three, and prints their SHA-256
    ./build.py --from DIR   uses the Noto fonts already in DIR (named as in google/fonts)

Needs fontTools (pip install fonttools). The output is the same for the same fontTools (4.65.0 built these).
"""
from __future__ import annotations

import argparse
import hashlib
import io
import sys
import urllib.request
from pathlib import Path

from fontTools import subset
from fontTools.merge import Merger
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont
from fontTools.ttLib.scaleUpem import scale_upem
from fontTools.varLib import instancer

HERE = Path(__file__).resolve().parent
FONTS = HERE.parent.parent / "chatcomputer/ui/assets/fonts"
# google/fonts at the commit Atkinson Hyperlegible's files came from (the Noto fonts the fallbacks are made from).
COMMIT = "47d54ca1ebb14fd864695f1d39dc734f39017c33"
SOURCES = {
    "ofl/notosans/NotoSans[wdth,wght].ttf": "bfb7bb691513f12e734dc346c03a03f784912432d7e3fa8e56efcf906fe86b3d",
    "ofl/notosansmath/NotoSansMath-Regular.ttf": "3f495fe933c06786e4d5f6d86b8ee70b6753a68ee3b9d87528726de0f6e2c47d",
    "ofl/notosanssymbols2/NotoSansSymbols2-Regular.ttf": "7d5fb73b7ca67a6798101741f5d280a3d016a56a197afcd4199dbb57b4b82a21",
    "ofl/notosanssymbols/NotoSansSymbols[wght].ttf": "f7e7e04b4a24b6c78893d50cbfd2b2f6cae49617ab047bfef668d252adb128f7",
    "ofl/notoemoji/NotoEmoji[wght].ttf": "de6c18832938afc99caf132b39d6a30a19bac7f2e812e28db2535b4608d27551",
    "ofl/notosansmono/NotoSansMono[wdth,wght].ttf": "2cb2adb378a8f574213e23df697050b83c54c27df465a2015552740b2769a081",
    "ofl/notosanssc/NotoSansSC[wght].ttf": "a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da",
    "ofl/notosanskr/NotoSansKR[wght].ttf": "194018e6b2b293a7964f037b25c0249ce1418bc9ab3c971060a03aa57861e252",
}


def blocks(*ranges: tuple[int, int]) -> set[int]:
    return {c for first, last in ranges for c in range(first, last + 1)}


# What each source gives, by Unicode block (those of its characters in them). Where two give the same character, the
# one merged first (in `build`) wins.
LATIN_GREEK_CYRILLIC = blocks(
    (0x00A0, 0x024F),  # Latin-1 and Latin Extended-A and -B (accented letters)
    (0x0250, 0x02FF),  # IPA and spacing modifiers
    (0x0370, 0x03FF),  # Greek
    (0x0400, 0x052F),  # Cyrillic
    (0x1E00, 0x1EFF),  # Latin Extended Additional (Vietnamese)
    (0x2000, 0x206F),  # general punctuation (′ ″ ‣ ‰)
    (0x2070, 0x209F),  # superscripts and subscripts
    (0x20A0, 0x20CF),  # currency (₹ ₽ ₿)
    (0x2100, 0x218F),  # letterlike symbols and number forms (™ ℃ № ⅓)
)
SYMBOLS = blocks(
    (0x2190, 0x21FF),  # arrows
    (0x2200, 0x22FF),  # mathematical operators
    (0x2300, 0x23FF),  # miscellaneous technical (⌘ ⌥ ⏎)
    (0x2460, 0x24FF),  # enclosed numbers (①)
    (0x25A0, 0x25FF),  # geometric shapes
    (0x2600, 0x27BF),  # miscellaneous symbols and dingbats (☐ ☑ ★ ✓ ✗ ♫)
    (0x27C0, 0x27FF),  # maths symbols and long arrows
    (0x2B00, 0x2BFF),  # more arrows and stars (⬆ ⭐)
)
EMOJI = blocks(
    (0x2190, 0x21FF),
    (0x2300, 0x23FF),
    (0x2600, 0x27BF),
    (0x2B00, 0x2BFF),
    (0x1F000, 0x1FAFF),  # emoji: faces, people, animals, food, objects, symbols, flags' letters
)
MONO = blocks(
    (0x00A0, 0x052F),
    (0x2000, 0x22FF),
    (0x2500, 0x25FF),  # box drawing, blocks and shapes, for diagrams and trees in code
)


def legacy(codec: str, firsts: range, seconds: list[int]) -> set[int]:
    """The characters of a two-byte character set, as Python's codec for it maps each pair of bytes."""
    out = set()
    for a in firsts:
        for b in seconds:
            try:
                text = bytes([a, b]).decode(codec)
            except UnicodeDecodeError:
                continue
            if len(text) == 1:
                out.add(ord(text))
    return out


def ideographs(chars: set[int]) -> set[int]:
    return {c for c in chars if 0x3400 <= c <= 0x9FFF or 0xF900 <= c <= 0xFAFF}


# Chinese and Japanese: the ideographs of the character sets their text has long been written in, China's GB 2312
# (6,763), Taiwan's Big5 (its first level, the commoner 5,401) and Japan's JIS X 0208 (6,356): 10,269 together. With
# kana, Bopomofo, CJK punctuation and full-width forms. From Noto Sans SC, so in the forms mainland China writes them.
HAN = (ideographs(legacy("gb2312", range(0xB0, 0xF8), list(range(0xA1, 0xFF))))
       | ideographs(legacy("big5", range(0xA4, 0xC7), list(range(0x40, 0x7F)) + list(range(0xA1, 0xFF))) & set(range(0x4E00, 0xA000)))
       | ideographs(legacy("euc_jp", range(0xB0, 0xF5), list(range(0xA1, 0xFF)))))
CJK = HAN | blocks(
    (0x3000, 0x303F),  # CJK punctuation (、。「」)
    (0x3040, 0x30FF),  # hiragana and katakana
    (0x3100, 0x312F),  # Bopomofo
    (0x31F0, 0x31FF),  # small katakana for Ainu
    (0xFF00, 0xFFEF),  # full-width forms (，：！？（）) and half-width katakana
)
# Korean: every Hangul syllable (11,172) and Hangul's letters on their own, from Noto Sans KR.
HANGUL = blocks((0xAC00, 0xD7A3), (0x3130, 0x318F))


def fetch(source: str, where: Path | None) -> bytes:
    if where is not None:
        data = (where / Path(source).name).read_bytes()
    else:
        url = f"https://raw.githubusercontent.com/google/fonts/{COMMIT}/" + source.replace("[", "%5B").replace("]", "%5D")
        with urllib.request.urlopen(url, timeout=120) as r:
            data = r.read()
    digest = hashlib.sha256(data).hexdigest()
    if digest != SOURCES[source]:
        sys.exit(f"{source}: SHA-256 {digest}, not {SOURCES[source]}")
    return data


UNREAD = ("GSUB", "GPOS", "GDEF", "MATH", "STAT", "BASE", "fpgm", "prep", "cvt ", "gasp", "vhea", "vmtx", "HVAR", "MVAR")


def static(data: bytes, **axes: float) -> TTFont:
    """The font at `axes` (a variable one made static), without what Chat Computer's engine doesn't read."""
    font = TTFont(io.BytesIO(data))
    if "fvar" in font:
        font = instancer.instantiateVariableFont(font, axes)
    for table in UNREAD:
        if table in font:
            del font[table]
    return font


def static_cut(data: bytes, wanted: set[int], skip: set[int], **axes: float) -> TTFont:
    """As cut(static(...)), but cut first: a CJK font has some 30,000 glyphs, and only those kept are made static."""
    font = TTFont(io.BytesIO(data))
    for table in UNREAD:
        if table in font:
            del font[table]
    font = cut(font, wanted, skip)
    return instancer.instantiateVariableFont(font, axes)


def cut(font: TTFont, wanted: set[int], skip: set[int]) -> TTFont:
    """Only the characters in `wanted` the font has, but not those in `skip` (the main font's own)."""
    cmap = font.getBestCmap()
    keep = sorted(c for c in wanted if c in cmap and c not in skip)
    options = subset.Options()
    options.layout_features = []
    options.name_IDs = ["*"]
    options.notdef_outline = True
    options.hinting = False
    options.glyph_names = False
    s = subset.Subsetter(options)
    s.populate(unicodes=keep)
    s.subset(font)
    return font


def widen(font: TTFont, advance: int) -> None:
    """Every glyph stretched across to `advance` units (Noto Sans Mono's 600 to Atkinson Hyperlegible Mono's 632), so
    box drawing lines meet in code's cells."""
    glyphs = font.getGlyphSet()
    glyf = font["glyf"]
    for name in font.getGlyphOrder():
        width, _ = font["hmtx"][name]
        if width == 0:
            continue
        factor = advance / width
        recorder = DecomposingRecordingPen(glyphs)
        glyphs[name].draw(recorder)
        pen = TTGlyphPen(None)
        recorder.replay(TransformPen(pen, (factor, 0, 0, 1, 0, 0)))
        glyph = pen.glyph()
        glyph.recalcBounds(glyf)
        glyf[name] = glyph
        font["hmtx"][name] = (advance, getattr(glyph, "xMin", 0))


def named(font: TTFont, family: str) -> None:
    """A name of its own (it isn't Noto any more, only made from it), and the same file every time."""
    name = font["name"]
    for record in list(name.names):
        if record.nameID in (1, 3, 4, 6, 16, 17, 21, 22, 25):
            name.removeNames(nameID=record.nameID)
    name.setName(family, 1, 3, 1, 0x409)
    name.setName("Regular", 2, 3, 1, 0x409)
    name.setName(f"{family} Regular", 3, 3, 1, 0x409)
    name.setName(family, 4, 3, 1, 0x409)
    name.setName(family.replace(" ", "") + "-Regular", 6, 3, 1, 0x409)
    font["head"].created = font["head"].modified = 3_818_534_400  # 1 January 2025, in seconds since 1904
    font.recalcTimestamp = False


def build(where: Path | None) -> None:
    main = set(TTFont(FONTS / "Lora.ttf").getBestCmap())
    mono_font = set(TTFont(FONTS / "AtkinsonHyperlegibleMono.ttf").getBestCmap())
    data = {source: fetch(source, where) for source in SOURCES}

    sans = cut(static(data["ofl/notosans/NotoSans[wdth,wght].ttf"], wght=400, wdth=100), LATIN_GREEK_CYRILLIC, main)
    maths = cut(static(data["ofl/notosansmath/NotoSansMath-Regular.ttf"]), SYMBOLS, main)
    symbols = cut(static(data["ofl/notosanssymbols2/NotoSansSymbols2-Regular.ttf"]), SYMBOLS, main)
    more_symbols = cut(static(data["ofl/notosanssymbols/NotoSansSymbols[wght].ttf"], wght=400), SYMBOLS, main)
    emoji = static(data["ofl/notoemoji/NotoEmoji[wght].ttf"], wght=400)
    scale_upem(emoji, 1000)
    emoji = cut(emoji, EMOJI, main)
    parts = []
    for font in (sans, maths, symbols, more_symbols, emoji):
        buffer = io.BytesIO()
        font.save(buffer)
        parts.append(buffer)
    for buffer in parts:
        buffer.seek(0)
    merged = Merger().merge(parts)
    named(merged, "Chat Computer Fallback")
    merged.save(HERE / "ChatComputerFallback.ttf")

    mono = cut(static(data["ofl/notosansmono/NotoSansMono[wdth,wght].ttf"], wght=400, wdth=100), MONO, mono_font)
    widen(mono, 632)
    named(mono, "Chat Computer Fallback Mono")
    mono.save(HERE / "ChatComputerFallbackMono.ttf")

    # (Characters the other fallback has come from it first: they're left out here.)
    before = main | set(merged.getBestCmap())
    chinese = static_cut(data["ofl/notosanssc/NotoSansSC[wght].ttf"], CJK, before, wght=400)
    korean = static_cut(data["ofl/notosanskr/NotoSansKR[wght].ttf"], HANGUL, before, wght=400)
    parts = []
    for font in (chinese, korean):
        buffer = io.BytesIO()
        font.save(buffer)
        buffer.seek(0)
        parts.append(buffer)
    cjk = Merger().merge(parts)
    named(cjk, "Chat Computer CJK")
    cjk.save(HERE / "ChatComputerCJK.ttf")

    for name in ("ChatComputerFallback.ttf", "ChatComputerFallbackMono.ttf", "ChatComputerCJK.ttf"):
        out = (HERE / name).read_bytes()
        print(f"{hashlib.sha256(out).hexdigest()}  {name} ({len(out)} bytes, {len(TTFont(HERE / name).getBestCmap())} characters)")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--from", dest="where", type=Path, help="a folder with the Noto fonts already downloaded")
    build(ap.parse_args().where)


if __name__ == "__main__":
    main()
