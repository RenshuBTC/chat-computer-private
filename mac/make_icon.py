"""Draws Chat Computer's icon in the Mac's startup options (mac/chatcomputer.icns): its word logo, "Chat" over
"Computer", in black on a white tile, like the screen itself (the same font and weight as the logo on the screen).
Run once when the look changes: python mac/make_icon.py [font.ttf]"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
FONT = sys.argv[1] if len(sys.argv) > 1 else str(HERE.parent / "chatcomputer/ui/assets/fonts/AtkinsonHyperlegibleNext.ttf")
WEIGHT = 800                                  # as #wordmark in chat.css
OUT = HERE / "chatcomputer.icns"
SIZE, BODY, RADIUS = 1024, 824, 185          # Apple's icon grid: an 824-pixel tile on a 1024 canvas
WORDS = "Chat\nComputer"


def font_at(size: int) -> ImageFont.FreeTypeFont:
    font = ImageFont.truetype(FONT, size)
    try:
        font.set_variation_by_axes([WEIGHT])  # the variable font's weight axis
    except (OSError, AttributeError):
        pass                                  # a font without one: as it is
    return font


def draw() -> Image.Image:
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    m = (SIZE - BODY) // 2
    d.rounded_rectangle((m, m, m + BODY - 1, m + BODY - 1), radius=RADIUS, fill=(255, 255, 255, 255),
                        outline=(217, 217, 217, 255), width=6)
    size = 240
    while True:                               # the largest size that leaves a margin on both sides
        font = font_at(size)
        spacing = size * 0.08
        left, top, right, bottom = d.multiline_textbbox((0, 0), WORDS, font=font, spacing=spacing, align="center")
        if right - left <= BODY * 0.76 or size <= 40:
            break
        size -= 4
    x = (SIZE - (right - left)) / 2 - left
    y = (SIZE - (bottom - top)) / 2 - top
    d.multiline_text((x, y), WORDS, font=font, fill=(0, 0, 0, 255), spacing=spacing, align="center")
    return img


if __name__ == "__main__":
    draw().save(OUT, sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)])
    print(OUT)
