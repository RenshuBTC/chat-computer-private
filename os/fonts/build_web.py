#!/usr/bin/env python3
"""Builds the fonts web pages are drawn in on Chat Computer's own system: the metric twins of Chrome's defaults on
Windows (Times New Roman, Arial and Courier New), so a page's words take the room and the lines the height they take
in Chrome: Tinos, Arimo and Cousine (Google's, from Steve Matteson's designs, OFL), at 1000 units to the em (the
engine's) and without their hinting or the layout features it doesn't read. See README.md.

    ./build_web.py              downloads them (checking their SHA-256), builds the ten, and prints their SHA-256
    ./build_web.py --from DIR   uses the fonts already in DIR (named as in google/fonts)

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
from fontTools.ttLib import TTFont
from fontTools.ttLib.scaleUpem import scale_upem

HERE = Path(__file__).resolve().parent
# google/fonts at the commit the other fonts here came from.
COMMIT = "47d54ca1ebb14fd864695f1d39dc734f39017c33"
SOURCES = {
    "ofl/tinos/Tinos-Regular.ttf": "60a0e8ef0c04dd5dd69ffe91025fa2ae5836cbd35600a82ba031977557e2cb61",
    "ofl/tinos/Tinos-Bold.ttf": "393269dbab8899f938db19783eca5eac92eb431f7ae0ab45b8349ca895f1a06b",
    "ofl/tinos/Tinos-Italic.ttf": "5942266ed398b155d7dc23e36833e7ec6be988f2439bdbeb8ef1bede808eaa91",
    "ofl/tinos/Tinos-BoldItalic.ttf": "a5de79f0fe863ea0954757acb3d47b3ccd0a930ce3dd5b97230cd3866790a06e",
    "ofl/arimo/Arimo[wght].ttf": "e43898b143ec826ac8cb4034816458a7047fbe0836558de2a1f8c6223ae3e0ca",
    "ofl/arimo/Arimo-Italic[wght].ttf": "a80fc54fd0233c1dfe298577c4d00f5ae81d5bb83510975e473c47e699b7f4ed",
    "ofl/cousine/Cousine-Regular.ttf": "1da22250675fc4c42fcf3a9736c44bc0570516105331443b663fd5cfbd1412fe",
    "ofl/cousine/Cousine-Bold.ttf": "17c8a7245156d2253531c9e529474937b09d9f641c5ae7695c5e33f22822eef4",
    "ofl/cousine/Cousine-Italic.ttf": "ea2a76ae3d0ece9cd59f0d30fdc08dd70e8f5f457beee5b0852a7b50c2286c7c",
    "ofl/cousine/Cousine-BoldItalic.ttf": "848e858726fee0ae27b754e4cd6a2755209bf1428a8c91f747696d58c33906c3",
}
# What each becomes: its file here, its family's name, and its style's.
OUT = {
    "ofl/tinos/Tinos-Regular.ttf": ("ChatComputerWebSerif-Regular.ttf", "Chat Computer Web Serif", "Regular"),
    "ofl/tinos/Tinos-Bold.ttf": ("ChatComputerWebSerif-Bold.ttf", "Chat Computer Web Serif", "Bold"),
    "ofl/tinos/Tinos-Italic.ttf": ("ChatComputerWebSerif-Italic.ttf", "Chat Computer Web Serif", "Italic"),
    "ofl/tinos/Tinos-BoldItalic.ttf": ("ChatComputerWebSerif-BoldItalic.ttf", "Chat Computer Web Serif", "Bold Italic"),
    "ofl/arimo/Arimo[wght].ttf": ("ChatComputerWebSans.ttf", "Chat Computer Web Sans", "Regular"),
    "ofl/arimo/Arimo-Italic[wght].ttf": ("ChatComputerWebSans-Italic.ttf", "Chat Computer Web Sans", "Italic"),
    "ofl/cousine/Cousine-Regular.ttf": ("ChatComputerWebMono-Regular.ttf", "Chat Computer Web Mono", "Regular"),
    "ofl/cousine/Cousine-Bold.ttf": ("ChatComputerWebMono-Bold.ttf", "Chat Computer Web Mono", "Bold"),
    "ofl/cousine/Cousine-Italic.ttf": ("ChatComputerWebMono-Italic.ttf", "Chat Computer Web Mono", "Italic"),
    "ofl/cousine/Cousine-BoldItalic.ttf": ("ChatComputerWebMono-BoldItalic.ttf", "Chat Computer Web Mono", "Bold Italic"),
}


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


def made(data: bytes) -> TTFont:
    """At 1000 units to the em, every character kept, with only kerning and mark positioning of its layout features,
    and no hinting (Chrome lays text out with the fonts' unhinted advances too)."""
    font = TTFont(io.BytesIO(data))
    scale_upem(font, 1000)
    options = subset.Options()
    options.layout_features = ["kern", "mark", "mkmk"]
    options.name_IDs = ["*"]
    options.notdef_outline = True
    options.hinting = False
    options.glyph_names = False
    options.drop_tables += ["Debg", "STAT", "gasp", "DSIG", "GSUB"]
    s = subset.Subsetter(options)
    s.populate(unicodes=font.getBestCmap().keys())
    s.subset(font)
    return font


def named(font: TTFont, family: str, style: str) -> None:
    """A name of its own (it isn't the original any more, only made from it), and the same file every time."""
    name = font["name"]
    for record in list(name.names):
        if record.nameID in (1, 2, 3, 4, 6, 16, 17, 21, 22, 25):
            name.removeNames(nameID=record.nameID)
    plain = style in ("Regular", "Bold", "Italic", "Bold Italic")
    name.setName(family, 1, 3, 1, 0x409)
    name.setName(style if plain else "Regular", 2, 3, 1, 0x409)
    name.setName(f"{family} {style}", 3, 3, 1, 0x409)
    name.setName(f"{family} {style}" if style != "Regular" else family, 4, 3, 1, 0x409)
    name.setName(family.replace(" ", "") + "-" + style.replace(" ", ""), 6, 3, 1, 0x409)
    font["head"].created = font["head"].modified = 3_818_534_400  # 1 January 2025, in seconds since 1904
    font.recalcTimestamp = False


def build(where: Path | None) -> None:
    for source, (file, family, style) in OUT.items():
        font = made(fetch(source, where))
        named(font, family, style)
        font.save(HERE / file)
    for file, _, _ in OUT.values():
        out = (HERE / file).read_bytes()
        print(f"{hashlib.sha256(out).hexdigest()}  {file} ({len(out)} bytes, {len(TTFont(HERE / file).getBestCmap())} characters)")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--from", dest="where", type=Path, help="a folder with the fonts already downloaded")
    build(ap.parse_args().where)


if __name__ == "__main__":
    main()
