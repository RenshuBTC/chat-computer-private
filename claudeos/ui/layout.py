"""How big ClaudeOS draws things, on any screen.

Text is the usual desktop size (16 pixels at 100%) in a column at most 48 rem wide (768 pixels at 100%).
That is drawn 1:1 on a standard desktop monitor (96 dots per inch, viewed from about 60 cm). Sharper
screens, and screens farther away, are scaled up the way the system's display scaling does, so the text
looks the same size on a 4K monitor or on a TV across the room. Websites get the same zoom.

The screen's real size comes from the monitor (EDID). The viewing distance is estimated from the
screen size unless you tell Claude how far you sit: about 50 cm for a laptop, 60-80 cm for a desk
monitor, and for a TV the distance at which it fills about 30 degrees of view. "Bigger" and "smaller"
adjust on top (text_scale).
"""
from __future__ import annotations

import math
from dataclasses import asdict, dataclass
from typing import Any

REF_PX_PER_MM = 96 / 25.4      # a standard desktop monitor...
REF_DISTANCE_MM = 600.0        # ...at arm's length
BASE_FONT_PX = 16.0            # the usual desktop text size at 100%
COLUMN_REM = 48.0              # the widest the conversation column gets
MARGIN_REM = 1.5               # space kept at each side on narrow screens
SCALE_STEP = 0.125
SCALE_RANGE = (0.75, 4.0)
TV_VIEW_DEG = 30.0
ASSUMED_PX_PER_MM = 96 / 25.4


@dataclass
class Screen:
    """The area ClaudeOS draws in, plus the monitor it is on."""
    view_w_px: float                 # CSS pixels
    view_h_px: float
    px_per_mm: float                 # CSS pixels per millimetre on this monitor
    monitor_w_mm: float
    monitor_h_mm: float
    size_known: bool                 # False when the monitor didn't report a believable size


@dataclass
class Layout:
    scale: float
    font_px: float          # root text size (1 rem)
    column_px: float
    web_zoom: float
    distance_mm: float
    distance_estimated: bool
    size_known: bool
    diagonal_in: float

    def to_dict(self) -> dict[str, Any]:
        d = asdict(self)
        return {k: (round(v, 3) if isinstance(v, float) else v) for k, v in d.items()}

    def describe(self) -> str:
        where = f"about {self.distance_mm / 10:.0f} cm away" + (" (estimated)" if self.distance_estimated else "")
        size = f"{self.diagonal_in:.0f}-inch screen" + ("" if self.size_known else " (size assumed)")
        return (f"{size}, viewed from {where}: drawn at {self.scale * 100:.0f}% "
                f"(text {self.font_px:.0f} px), websites at {self.web_zoom * 100:.0f}%.")


BOGUS_MM = {(160, 90), (160, 100), (16, 9), (16, 10), (150, 90), (0, 0)}


def screen_from_monitor(view_w_px: float, view_h_px: float, mon_w_px: float, mon_h_px: float,
                        mon_w_mm: float, mon_h_mm: float, scale: float = 1.0,
                        diagonal_in: float = 0.0) -> Screen:
    """Physical size from the monitor (EDID via the display server), checked for believability.

    `diagonal_in` (a setting) overrides what the monitor reports: some TVs and projectors report
    nothing, or an aspect ratio instead of a size.
    """
    mon_w_px = max(1.0, float(mon_w_px))
    mon_h_px = max(1.0, float(mon_h_px))
    known = True
    if diagonal_in and diagonal_in > 0:
        diag_px = math.hypot(mon_w_px, mon_h_px)
        mon_w_mm = diagonal_in * 25.4 * mon_w_px / diag_px
        mon_h_mm = diagonal_in * 25.4 * mon_h_px / diag_px
    else:
        w, h = float(mon_w_mm or 0), float(mon_h_mm or 0)
        # Rotated monitors can report width and height the other way round.
        if w > 0 and h > 0 and (w > h) != (mon_w_px > mon_h_px):
            w, h = h, w
        ok = w >= 150 and h >= 80 and (round(w), round(h)) not in BOGUS_MM
        if ok:
            aspect_err = abs((w / h) / (mon_w_px / mon_h_px) - 1)
            density = mon_w_px * scale / w          # device pixels per mm
            ok = aspect_err < 0.15 and 0.8 <= density <= 25
        if ok:
            mon_w_mm, mon_h_mm = w, h
        else:
            known = False
            mon_w_mm = mon_w_px * scale / ASSUMED_PX_PER_MM
            mon_h_mm = mon_h_px * scale / ASSUMED_PX_PER_MM
    px_per_mm = mon_w_px / mon_w_mm                  # CSS (logical) pixels per mm
    return Screen(float(view_w_px), float(view_h_px), px_per_mm, mon_w_mm, mon_h_mm, known)


def estimate_distance_mm(w_mm: float, h_mm: float) -> float:
    diag_in = math.hypot(w_mm, h_mm) / 25.4
    monitor = min(1000.0, max(500.0, 0.6 * w_mm + 300.0))
    tv = w_mm / (2 * math.tan(math.radians(TV_VIEW_DEG / 2)))
    if diag_in <= 17.5:
        return 500.0
    if diag_in <= 34.5:
        return monitor
    if diag_in >= 40:
        return max(monitor, tv)
    t = (diag_in - 34.5) / (40 - 34.5)
    return (1 - t) * monitor + t * max(monitor, tv)


def _round_step(x: float) -> float:
    return round(x / SCALE_STEP) * SCALE_STEP


def compute(screen: Screen, *, distance_cm: float = 0, text_px: float = 0, text_scale: float = 1.0,
            web_zoom: float = 0) -> Layout:
    estimated = not (distance_cm and distance_cm > 0)
    d = estimate_distance_mm(screen.monitor_w_mm, screen.monitor_h_mm) if estimated else float(distance_cm) * 10
    # How many times bigger a pixel must be drawn to look like one on the reference monitor.
    base = (screen.px_per_mm / REF_PX_PER_MM) * (d / REF_DISTANCE_MM)
    base = min(SCALE_RANGE[1], max(SCALE_RANGE[0], _round_step(base)))
    user = min(3.0, max(0.5, float(text_scale or 1.0)))
    scale = min(SCALE_RANGE[1] * 1.5, max(0.5, base * user))
    font_px = float(text_px) if text_px and text_px > 0 else BASE_FONT_PX * scale
    column_px = min(COLUMN_REM * font_px, max(200.0, screen.view_w_px - 2 * MARGIN_REM * font_px))
    zoom = float(web_zoom) if web_zoom and web_zoom > 0 else base
    zoom = round(min(5.0, max(0.5, zoom)) * 20) / 20
    return Layout(
        scale=scale,
        font_px=round(font_px, 1),
        column_px=round(column_px),
        web_zoom=zoom,
        distance_mm=round(d),
        distance_estimated=estimated,
        size_known=screen.size_known,
        diagonal_in=math.hypot(screen.monitor_w_mm, screen.monitor_h_mm) / 25.4,
    )


def css_vars(layout: Layout) -> str:
    return f":root{{--co-size:{layout.font_px:.1f}px;--co-column:{layout.column_px:.0f}px}}"
