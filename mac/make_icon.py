"""Draws ClaudeOS's icon in the Mac's startup options (mac/claudeos.icns): the word ClaudeOS in black on a white tile,
like the screen itself. Run once when the look changes: python mac/make_icon.py [font.ttf]
(Noto Sans by default; any plain sans-serif works)."""
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

FONT = sys.argv[1] if len(sys.argv) > 1 else "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf"
OUT = Path(__file__).resolve().parent / "claudeos.icns"
SIZE, BODY, RADIUS = 1024, 824, 185          # Apple's icon grid: an 824-pixel tile on a 1024 canvas


def draw() -> Image.Image:
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    m = (SIZE - BODY) // 2
    d.rounded_rectangle((m, m, m + BODY - 1, m + BODY - 1), radius=RADIUS, fill=(255, 255, 255, 255),
                        outline=(217, 217, 217, 255), width=6)
    text = "ClaudeOS"
    size = 200
    while True:                               # the largest size that leaves a margin on both sides
        font = ImageFont.truetype(FONT, size)
        left, top, right, bottom = d.textbbox((0, 0), text, font=font)
        if right - left <= BODY * 0.78 or size <= 40:
            break
        size -= 4
    x = (SIZE - (right - left)) / 2 - left
    y = (SIZE - (bottom - top)) / 2 - top
    d.text((x, y), text, font=font, fill=(0, 0, 0, 255))
    return img


if __name__ == "__main__":
    draw().save(OUT, sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)])
    print(OUT)
