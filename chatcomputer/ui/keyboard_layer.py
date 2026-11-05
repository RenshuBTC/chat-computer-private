"""The onscreen keyboard, drawn over the screen.

Drawn with cairo on the window's top layer (a drawing area that lets clicks through to what's under it), in
Chat Computer's look: black on white. The keyboard is the iPad's (ui/keyboard.py has its measurements): its
keys where the iPad's are, as big as the iPad's are to one another, the letter under the small second character,
and the keys that do things marked in their bottom corners, as on the iPad, with symbols drawn here (so they
look the same whatever fonts the system has). It is drawn once into a picture of its own and copied from
there, so a key pressed or touched redraws only its own small square.
"""
from __future__ import annotations

import math
import time
from typing import Any, Callable

import gi

try:
    import cairo            # pycairo, with GTK's bridge to it (python3-gi-cairo): without them nothing is drawn
except ImportError:         # here, but Chat Computer still starts
    cairo = None

gi.require_version("Pango", "1.0")
gi.require_version("PangoCairo", "1.0")
from gi.repository import GLib, Pango, PangoCairo  # noqa: E402

from .keyboard import CORNER, ROWS, UNITS, Keyboard  # noqa: E402

FONT = "Lora"
FLASH_MS = 160
WHITE, BLACK = (1.0, 1.0, 1.0), (0.0, 0.0, 0.0)


class KeyboardLayer:
    def __init__(self, area: Any, rem: Callable[[], float]) -> None:
        self.area = area                       # the Gtk.DrawingArea over everything
        self.rem = rem                         # the screen's text size (pixels)
        self.kb = Keyboard()
        self.up = False
        self.panel = (0.0, 0.0, 0.0, 0.0)     # the keyboard: x, y, width, height (window pixels)
        self.touched: set[int] = set()         # the key a finger is on, on a touchscreen
        self.flash: int | None = None          # the key just pressed
        self._flash_id = 0
        self._cache: Any = None
        self._cache_for: tuple = ()

    # ---------------------------------------------------------------- the keyboard
    def show_keyboard(self, width: float, height: float) -> tuple[float, float, float, float]:
        """Put the keyboard along the bottom of a window this big, as big as fits in its width and 42% of its
        height (the iPad's proportions kept, so on a wide screen it sits in the middle); returns where its keys
        are (window pixels)."""
        ph = round(Keyboard.size_for(width, 0.42 * height)[1])
        self.panel = (0.0, float(height - ph), float(width), float(ph))
        self.kb.layout(width, ph)
        self.up, self.flash, self.touched = True, None, set()
        self._cache = None
        self.area.queue_draw()
        ax, ay, aw, ah = self.kb.area()
        return (ax, self.panel[1] + ay, aw, ah)

    def hide_keyboard(self) -> None:
        self.up, self.flash, self._cache, self.touched = False, None, None, set()
        self.area.queue_draw()

    def touch_key(self, at: tuple[float, float] | None) -> None:
        """A finger on the keyboard, on a touchscreen (window pixels): the key under it shows pressed, as an iPad's
        does. None: no finger on it now."""
        touched: set[int] = set()
        if at is not None and self.up:
            k = self.kb.hit(at[0] - self.panel[0], at[1] - self.panel[1])
            if k is not None:
                touched.add(self.kb.keys.index(k))
        for i in touched ^ self.touched:
            self._redraw_key(i)
        self.touched = touched

    @staticmethod
    def spec(area: tuple[float, float, float, float]) -> tuple[float, ...]:
        return tuple(area) + (UNITS, ROWS)

    def press(self, x: float, y: float) -> tuple | None:
        """A finger pressed at x, y (window pixels): what to type, if anything (see Keyboard.press)."""
        if not self.up:
            return None
        key = self.kb.hit(x - self.panel[0], y - self.panel[1])
        if key is None:
            return None
        i = self.kb.keys.index(key)
        state = (self.kb.page, self.kb.shift)
        action = self.kb.press(key, time.monotonic())
        if (self.kb.page, self.kb.shift) != state:       # new labels (the keys stay where they were)
            self.kb.layout(self.panel[2], self.panel[3])
            self._cache = None
            self.area.queue_draw_area(*self._int_rect(self.panel))
        self._flash(i)
        return action

    def _flash(self, i: int | None) -> None:
        old, self.flash = self.flash, i
        for j in (old, i):
            if j is not None:
                self._redraw_key(j)
        if self._flash_id:
            GLib.source_remove(self._flash_id)
        self._flash_id = GLib.timeout_add(FLASH_MS, self._unflash) if i is not None else 0

    def _unflash(self) -> bool:
        self._flash_id = 0
        old, self.flash = self.flash, None
        if old is not None:
            self._redraw_key(old)
        return False

    def _redraw_key(self, i: int) -> None:
        if 0 <= i < len(self.kb.keys):
            k = self.kb.keys[i]
            pad = 4 + self._line() * 2
            self.area.queue_draw_area(*self._int_rect((self.panel[0] + k.x - pad, self.panel[1] + k.y - pad,
                                                       k.w + 2 * pad, k.h + 2 * pad)))

    def _line(self) -> float:
        return max(1.5, round((self.rem() or 24.0) * 0.07, 1))

    @staticmethod
    def _int_rect(r: tuple[float, float, float, float]) -> tuple[int, int, int, int]:
        x, y, w, h = r
        return int(math.floor(x)), int(math.floor(y)), int(math.ceil(w)) + 2, int(math.ceil(h)) + 2

    # ---------------------------------------------------------------- drawing
    def draw(self, _widget: Any, cr: Any) -> bool:
        if self.up:
            self._draw_keyboard(cr)
        return False

    def _draw_keyboard(self, cr: Any) -> None:
        px, py, pw, ph = self.panel
        target = cr.get_target()
        sx, sy = target.get_device_scale() if hasattr(target, "get_device_scale") else (1.0, 1.0)
        key = (pw, ph, sx, self.kb.page, self.kb.shift, round(self.rem() or 24.0, 1))
        if cairo is None:
            cr.save()
            cr.translate(px, py)
            self.render_panel(cr)
            cr.restore()
        elif self._cache is None or self._cache_for != key:
            surf = cairo.ImageSurface(cairo.FORMAT_RGB24, max(1, int(math.ceil(pw * sx))), max(1, int(math.ceil(ph * sy))))
            surf.set_device_scale(sx, sy)
            c = cairo.Context(surf)
            self.render_panel(c)
            self._cache, self._cache_for = surf, key
        if cairo is not None:
            cr.save()
            cr.set_source_surface(self._cache, px, py)
            cr.rectangle(px, py, pw, ph)
            cr.fill()
            cr.restore()
        for i in sorted(self.touched | ({self.flash} if self.flash is not None else set())):
            if 0 <= i < len(self.kb.keys):
                cr.save()
                cr.translate(px, py)
                self._draw_key(cr, self.kb.keys[i], pressed=True)
                cr.restore()

    def render_panel(self, cr: Any) -> None:
        """The keyboard as it rests (no finger on it), in its own pixels."""
        _px, _py, pw, ph = self.panel
        cr.set_source_rgb(*WHITE)
        cr.rectangle(0, 0, pw, ph)
        cr.fill()
        cr.set_source_rgb(*BLACK)
        cr.set_line_width(self._line())
        cr.move_to(0, self._line() / 2)
        cr.line_to(pw, self._line() / 2)
        cr.stroke()
        for k in self.kb.keys:
            self._draw_key(cr, k)

    def _draw_key(self, cr: Any, k: Any, pressed: bool = False) -> None:
        line = self._line()
        lit = (k.action == ("caps",) and self.kb.shift == "lock")
        inverted = pressed or lit
        ink, paper = (WHITE, BLACK) if inverted else (BLACK, WHITE)
        _rounded(cr, k.x, k.y, k.w, k.h, CORNER * k.h)
        cr.set_source_rgb(*paper)
        cr.fill_preserve()
        cr.set_source_rgb(*BLACK)
        cr.set_line_width(line)
        cr.stroke()
        cr.set_source_rgb(*ink)
        h = k.h
        if k.symbol:                                   # tab, delete, Caps Lock, return, Shift, emoji, mic, hide
            cx = k.x + 0.36 * h if k.corner == "bl" else k.x + k.w - 0.36 * h if k.corner == "br" else k.x + k.w / 2
            cy = k.y + 0.72 * h
            filled = k.symbol == "shift" and self.kb.shift != "off"
            SYMBOLS[k.symbol](cr, cx, cy, 0.34 * h, max(1.2, line * 0.9), filled)
            if k.symbol == "shift" and self.kb.shift == "lock":
                cr.set_line_width(max(1.2, line * 0.9))
                cr.move_to(cx - 0.1 * h, cy + 0.24 * h)
                cr.line_to(cx + 0.1 * h, cy + 0.24 * h)
                cr.stroke()
            return
        if k.label == "space":
            return
        if k.corner:                                   # .?123, ABC, #+=: small, in the bottom corner
            _text(cr, k.label, 0.26 * h, k.x + 0.16 * h if k.corner == "bl" else k.x + k.w - 0.16 * h,
                  k.y + 0.82 * h, "left" if k.corner == "bl" else "right")
            return
        if k.label in (",", ".") and k.sub:            # the iPad's two-character punctuation keys
            _text(cr, k.sub, 0.3 * h, k.x + k.w / 2, k.y + 0.5 * h)
            _text(cr, k.label, 0.3 * h, k.x + k.w / 2, k.y + 0.72 * h)
            return
        if k.sub:                                      # the second character, small, above the letter
            _text(cr, k.sub, 0.18 * h, k.x + k.w / 2, k.y + 0.34 * h)
            _text(cr, k.label, 0.36 * h, k.x + k.w / 2, k.y + 0.77 * h)
            return
        if k.action[0] == "text" and len(k.label) == 1 and k.label.isascii():
            _text(cr, k.label, 0.38 * h, k.x + k.w / 2, k.y + 0.66 * h)
            return
        _text(cr, k.label, 0.38 * h, k.x + k.w / 2, k.y + 0.52 * h, middle=True)      # emoji: in the middle


def _text(cr: Any, text: str, size: float, x: float, y: float, align: str = "centre", middle: bool = False) -> None:
    """Text in Chat Computer's font with its baseline at y (letters with tails hang below it, as on any keyboard), or
    with middle its ink's middle there; centred on x, or starting or ending there."""
    layout = PangoCairo.create_layout(cr)
    fd = Pango.FontDescription.from_string(FONT)
    fd.set_absolute_size(max(1.0, size) * Pango.SCALE)
    layout.set_font_description(fd)
    layout.set_text(text, -1)
    ink, logical = layout.get_pixel_extents()
    left = x - ink.width / 2 - ink.x if align == "centre" else x - ink.x if align == "left" else x - ink.width - ink.x
    top = y - ink.height / 2 - ink.y if middle else y - layout.get_baseline() / Pango.SCALE
    cr.move_to(left, top)
    PangoCairo.show_layout(cr, layout)


def _rounded(cr: Any, x: float, y: float, w: float, h: float, r: float) -> None:
    """A rectangle with the iPad's smooth corners: each corner a curve that starts further along the side than a
    circle's would and bends more gently into it (a cubic Bezier through the corner's reach, 1.3 times r)."""
    r = min(r, w / 2, h / 2)
    e = min(1.3 * r, w / 2, h / 2)       # where the curve leaves the straight side
    c = 0.45 * e                         # how far its handles reach towards the corner
    cr.new_sub_path()
    cr.move_to(x + e, y)
    cr.line_to(x + w - e, y)
    cr.curve_to(x + w - c, y, x + w, y + c, x + w, y + e)
    cr.line_to(x + w, y + h - e)
    cr.curve_to(x + w, y + h - c, x + w - c, y + h, x + w - e, y + h)
    cr.line_to(x + e, y + h)
    cr.curve_to(x + c, y + h, x, y + h - c, x, y + h - e)
    cr.line_to(x, y + e)
    cr.curve_to(x, y + c, x + c, y, x + e, y)
    cr.close_path()


# ------------------------------------------------------------------ the keys' symbols, drawn: centre cx, cy;
# u is their size (about a third of a key's height); lw the line's width.
def _tab(cr: Any, cx: float, cy: float, u: float, lw: float, filled: bool = False) -> None:
    cr.set_line_width(lw)
    cr.move_to(cx - 0.5 * u, cy)
    cr.line_to(cx + 0.32 * u, cy)
    cr.move_to(cx + 0.05 * u, cy - 0.27 * u)
    cr.line_to(cx + 0.32 * u, cy)
    cr.line_to(cx + 0.05 * u, cy + 0.27 * u)
    cr.move_to(cx + 0.48 * u, cy - 0.32 * u)
    cr.line_to(cx + 0.48 * u, cy + 0.32 * u)
    cr.stroke()


def _delete(cr: Any, cx: float, cy: float, u: float, lw: float, filled: bool = False) -> None:
    cr.set_line_width(lw)
    cr.move_to(cx - 0.55 * u, cy)
    cr.line_to(cx - 0.22 * u, cy - 0.34 * u)
    cr.line_to(cx + 0.5 * u, cy - 0.34 * u)
    cr.line_to(cx + 0.5 * u, cy + 0.34 * u)
    cr.line_to(cx - 0.22 * u, cy + 0.34 * u)
    cr.close_path()
    cr.move_to(cx + 0.02 * u, cy - 0.13 * u)
    cr.line_to(cx + 0.28 * u, cy + 0.13 * u)
    cr.move_to(cx + 0.28 * u, cy - 0.13 * u)
    cr.line_to(cx + 0.02 * u, cy + 0.13 * u)
    cr.stroke()


def _arrow_up(cr: Any, cx: float, cy: float, u: float) -> None:
    cr.move_to(cx, cy - 0.5 * u)
    cr.line_to(cx + 0.46 * u, cy)
    cr.line_to(cx + 0.2 * u, cy)
    cr.line_to(cx + 0.2 * u, cy + 0.38 * u)
    cr.line_to(cx - 0.2 * u, cy + 0.38 * u)
    cr.line_to(cx - 0.2 * u, cy)
    cr.line_to(cx - 0.46 * u, cy)
    cr.close_path()


def _shift(cr: Any, cx: float, cy: float, u: float, lw: float, filled: bool = False) -> None:
    cr.set_line_width(lw)
    _arrow_up(cr, cx, cy, u)
    if filled:
        cr.fill_preserve()
    cr.stroke()


def _caps(cr: Any, cx: float, cy: float, u: float, lw: float, filled: bool = False) -> None:
    cr.set_line_width(lw)
    _arrow_up(cr, cx, cy - 0.1 * u, 0.85 * u)
    cr.stroke()
    cr.rectangle(cx - 0.17 * u, cy + 0.36 * u, 0.34 * u, 0.12 * u)
    cr.stroke()


def _return(cr: Any, cx: float, cy: float, u: float, lw: float, filled: bool = False) -> None:
    cr.set_line_width(lw)
    cr.move_to(cx + 0.45 * u, cy - 0.38 * u)
    cr.line_to(cx + 0.45 * u, cy + 0.12 * u)
    cr.line_to(cx - 0.45 * u, cy + 0.12 * u)
    cr.move_to(cx - 0.2 * u, cy - 0.14 * u)
    cr.line_to(cx - 0.45 * u, cy + 0.12 * u)
    cr.line_to(cx - 0.2 * u, cy + 0.38 * u)
    cr.stroke()


def _emoji(cr: Any, cx: float, cy: float, u: float, lw: float, filled: bool = False) -> None:
    cr.set_line_width(lw)
    cr.new_sub_path()
    cr.arc(cx, cy, 0.48 * u, 0, 2 * math.pi)
    cr.stroke()
    for ex in (-0.17, 0.17):
        cr.new_sub_path()
        cr.arc(cx + ex * u, cy - 0.12 * u, 0.06 * u, 0, 2 * math.pi)
        cr.fill()
    cr.new_sub_path()
    cr.arc(cx, cy + 0.02 * u, 0.26 * u, math.radians(25), math.radians(155))
    cr.stroke()


def _mic(cr: Any, cx: float, cy: float, u: float, lw: float, filled: bool = False) -> None:
    cr.set_line_width(lw)
    _rounded(cr, cx - 0.14 * u, cy - 0.55 * u, 0.28 * u, 0.62 * u, 0.14 * u)
    cr.stroke()
    cr.new_sub_path()
    cr.arc(cx, cy - 0.08 * u, 0.3 * u, 0, math.pi)
    cr.move_to(cx, cy + 0.22 * u)
    cr.line_to(cx, cy + 0.45 * u)
    cr.move_to(cx - 0.16 * u, cy + 0.45 * u)
    cr.line_to(cx + 0.16 * u, cy + 0.45 * u)
    cr.stroke()


def _hide(cr: Any, cx: float, cy: float, u: float, lw: float, filled: bool = False) -> None:
    cr.set_line_width(lw)
    _rounded(cr, cx - 0.55 * u, cy - 0.5 * u, 1.1 * u, 0.62 * u, 0.08 * u)
    cr.stroke()
    for row, n in ((-0.33, 5), (-0.17, 5)):
        for i in range(n):
            cr.rectangle(cx + (-0.4 + 0.2 * i) * u - 0.04 * u, cy + row * u - 0.03 * u, 0.08 * u, 0.06 * u)
    cr.fill()
    cr.move_to(cx - 0.25 * u, cy - 0.02 * u)
    cr.line_to(cx + 0.25 * u, cy - 0.02 * u)
    cr.move_to(cx - 0.14 * u, cy + 0.28 * u)
    cr.line_to(cx, cy + 0.42 * u)
    cr.line_to(cx + 0.14 * u, cy + 0.28 * u)
    cr.stroke()


SYMBOLS = {"tab": _tab, "delete": _delete, "shift": _shift, "caps": _caps, "return": _return, "emoji": _emoji,
           "mic": _mic, "hide": _hide}
