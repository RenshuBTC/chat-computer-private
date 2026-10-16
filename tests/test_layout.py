"""How big ClaudeOS draws things: the usual desktop size, scaled for each screen like display scaling does."""
import math

import pytest

from claudeos.ui import layout as L


def screen(w_px, h_px, inches=0.0, w_mm=None, h_mm=None, view=None):
    if w_mm is None:
        diag = math.hypot(w_px, h_px)
        w_mm, h_mm = inches * 25.4 * w_px / diag, inches * 25.4 * h_px / diag
    vw, vh = view or (w_px, h_px)
    return L.screen_from_monitor(vw, vh, w_px, h_px, w_mm, h_mm)


CASES = {
    "24in 1080p monitor": (screen(1920, 1080, 24), 1.0),
    "27in 1440p monitor": (screen(2560, 1440, 27), 1.25),
    "27in 4K monitor": (screen(3840, 2160, 27), 1.875),
    "14in 1080p laptop": (screen(1920, 1080, 14), 1.375),
    "55in 4K TV": (screen(3840, 2160, 55), 3.125),
}


@pytest.mark.parametrize("name", list(CASES))
def test_scale_like_desktop_display_scaling(name):
    scr, expected = CASES[name]
    lay = L.compute(scr)
    assert lay.scale == pytest.approx(expected), lay.describe()
    assert lay.font_px == pytest.approx(16 * expected)
    assert lay.web_zoom == pytest.approx(round(expected * 20) / 20)
    assert lay.column_px <= scr.view_w_px


def test_text_looks_the_same_size_on_every_screen():
    # Visual angle of 1 rem: pixels -> mm -> degrees at the viewing distance.
    angles = []
    for scr, _ in CASES.values():
        lay = L.compute(scr)
        mm = lay.font_px / scr.px_per_mm
        angles.append(math.degrees(2 * math.atan(mm / 2 / lay.distance_mm)))
    assert max(angles) / min(angles) < 1.08, angles


def test_column_is_48_rem_or_the_screen():
    lay = L.compute(CASES["27in 1440p monitor"][0])
    assert lay.column_px == pytest.approx(48 * lay.font_px)
    narrow = L.compute(screen(1920, 1080, 14, view=(700, 600)))
    assert narrow.column_px == pytest.approx(700 - 2 * L.MARGIN_REM * narrow.font_px, abs=1)


def test_distance_estimates():
    assert L.estimate_distance_mm(310, 174) == 500                 # 14in laptop
    assert 580 <= L.estimate_distance_mm(531, 299) <= 650          # 24in monitor
    assert 2100 <= L.estimate_distance_mm(1218, 685) <= 2400       # 55in TV: fills about 30 degrees


def test_told_distance_and_bigger_smaller():
    scr = CASES["27in 1440p monitor"][0]
    far = L.compute(scr, distance_cm=200)
    assert not far.distance_estimated and far.distance_mm == 2000 and far.scale == pytest.approx(3.75)
    big = L.compute(scr, text_scale=1.2)
    assert big.font_px == pytest.approx(L.compute(scr).font_px * 1.2)
    assert big.web_zoom == L.compute(scr).web_zoom               # websites follow the screen, not "bigger"


def test_fixed_pixel_size_and_zoom_are_respected():
    lay = L.compute(CASES["24in 1080p monitor"][0], text_px=18, web_zoom=1.25)
    assert lay.font_px == 18 and lay.web_zoom == 1.25


@pytest.mark.parametrize("w_mm,h_mm", [(0, 0), (160, 90), (16, 9), (700, 100), (5000, 2800)])
def test_unbelievable_monitor_sizes_fall_back_to_96_dpi(w_mm, h_mm):
    s = screen(1920, 1080, w_mm=w_mm, h_mm=h_mm)
    assert not s.size_known
    assert s.px_per_mm == pytest.approx(96 / 25.4)
    assert L.compute(s).scale == 1.0


def test_rotated_monitor_size_is_swapped():
    s = screen(1080, 1920, w_mm=531, h_mm=299)
    assert s.size_known and s.monitor_w_mm == 299


def test_diagonal_setting_overrides_monitor_report():
    s = L.screen_from_monitor(1920, 1080, 1920, 1080, 0, 0, diagonal_in=55)
    assert s.size_known
    assert math.hypot(s.monitor_w_mm, s.monitor_h_mm) / 25.4 == pytest.approx(55)


def test_css_vars():
    css = L.css_vars(L.compute(CASES["24in 1080p monitor"][0]))
    assert css == ":root{--co-size:16.0px;--co-column:768px}"


def test_print_table():
    for n, (s, _) in CASES.items():
        print(n.ljust(20), L.compute(s).describe())
