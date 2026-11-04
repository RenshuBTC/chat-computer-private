"""Chat Computer's icon for Android: the letter C of the word logo (Atkinson Hyperlegible Next at 800), black on the
screen's off-white, as the Windows and Mac app's (apps/desktop/icon). An adaptive icon (Android 8 and later: the
off-white behind, the letter in front, inside the circle every launcher keeps) and the older square ones.
Run once:  python apps/android/make_icons.py"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
RES = HERE / "app" / "src" / "main" / "res"
FONT = HERE.parent.parent / "docs" / "demo" / "fonts" / "AtkinsonHyperlegibleNext.ttf"
GROUND, INK = (0xF8, 0xF5, 0xF0, 255), (0, 0, 0, 255)
DENSITIES = {"mdpi": 1, "hdpi": 1.5, "xhdpi": 2, "xxhdpi": 3, "xxxhdpi": 4}


def letter(size: int, height: float, ground: bool, radius: float = 0) -> Image.Image:
    """The C, `height` of the picture tall, centred; on the off-white (rounded by `radius` of it) or on nothing."""
    k = 4
    img = Image.new("RGBA", (size * k, size * k), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    if ground:
        d.rounded_rectangle((0, 0, size * k - 1, size * k - 1), radius=int(radius * size * k), fill=GROUND)
    font = ImageFont.truetype(str(FONT), int(height * size * k * 1.36))
    font.set_variation_by_axes([800])
    l, t, r, b = d.textbbox((0, 0), "C", font=font)
    d.text(((size * k - (r - l)) / 2 - l, (size * k - (b - t)) / 2 - t), "C", font=font, fill=INK)
    return img.resize((size, size), Image.LANCZOS)


def main() -> None:
    for name, x in DENSITIES.items():
        folder = RES / f"mipmap-{name}"
        folder.mkdir(parents=True, exist_ok=True)
        letter(round(48 * x), 0.5, True, 0.22).save(folder / "ic_launcher.png")
        letter(round(108 * x), 0.36, False).save(folder / "ic_launcher_foreground.png")
    any_dpi = RES / "mipmap-anydpi-v26"
    any_dpi.mkdir(parents=True, exist_ok=True)
    (any_dpi / "ic_launcher.xml").write_text(
        '<?xml version="1.0" encoding="utf-8"?>\n'
        '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
        '    <background android:drawable="@color/ground" />\n'
        '    <foreground android:drawable="@mipmap/ic_launcher_foreground" />\n'
        '</adaptive-icon>\n')


if __name__ == "__main__":
    main()
