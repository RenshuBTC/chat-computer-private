"""The app's icon: the letter C of the word logo (Atkinson Hyperlegible Next at its heaviest, 800, as the word logo
is), black on the screen's off-white (#F8F5F0), in a square with round corners as a Mac's icons are (824 of 1024,
corners 185). No picture: a letter, as Chrome shows for an extension without one. Run once; `tauri icon` makes
every size from it:  python apps/desktop/icon/make_icon.py && npx @tauri-apps/cli icon apps/desktop/icon/icon.png"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
FONT = HERE.parent.parent.parent / "docs" / "demo" / "fonts" / "AtkinsonHyperlegibleNext.ttf"
SIZE, BOX, RADIUS = 1024, 824, 185
GROUND, INK = (0xF8, 0xF5, 0xF0, 255), (0, 0, 0, 255)


def main() -> None:
    scale = 4                                       # drawn 4 times bigger, then made smaller: smooth edges
    big = Image.new("RGBA", (SIZE * scale, SIZE * scale), (0, 0, 0, 0))
    d = ImageDraw.Draw(big)
    pad = (SIZE - BOX) // 2 * scale
    d.rounded_rectangle((pad, pad, SIZE * scale - pad, SIZE * scale - pad), radius=RADIUS * scale, fill=GROUND)
    font = ImageFont.truetype(str(FONT), 600 * scale)
    font.set_variation_by_axes([800])
    l, t, r, b = d.textbbox((0, 0), "C", font=font)
    d.text(((SIZE * scale - (r - l)) / 2 - l, (SIZE * scale - (b - t)) / 2 - t), "C", font=font, fill=INK)
    big.resize((SIZE, SIZE), Image.LANCZOS).save(HERE / "icon.png")


if __name__ == "__main__":
    main()
