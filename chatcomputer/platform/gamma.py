"""The night light: warmer colours on every screen, by scaling each colour's ramp (X RandR's gamma ramps) the way a
lower colour temperature does, as redshift and Night Shift do. White becomes warm white, not grey: each channel is
scaled, not curved (xrandr --gamma curves the middle and leaves white white, so it can't do this).

Through libX11 and libXrandr with ctypes: they're on every Chat Computer (Xorg's own), so nothing to install."""
from __future__ import annotations

import ctypes
import ctypes.util
import math
import os

NORMAL_K = 6500          # daylight: the colours as they are
WARMEST_K = 2700         # warmth 100: like a warm light bulb


def kelvin(warmth: int) -> int:
    """Warmth 0-100 as a colour temperature: 0 is daylight (6500 K), 50 about 4600 K, 100 a warm bulb (2700 K)."""
    w = max(0, min(100, int(warmth)))
    return round(NORMAL_K - (NORMAL_K - WARMEST_K) * w / 100)


def _rgb(k: float) -> tuple[float, float, float]:
    """A colour temperature's white, 0-255 per channel (Tanner Helland's fit to the black-body colours)."""
    t = k / 100.0
    r = 255.0 if t <= 66 else 329.698727446 * (t - 60) ** -0.1332047592
    g = 99.4708025861 * math.log(t) - 161.1195681661 if t <= 66 else 288.1221695283 * (t - 60) ** -0.0755148492
    b = 255.0 if t >= 66 else (0.0 if t <= 19 else 138.5177312231 * math.log(t - 10) - 305.0447927307)
    return tuple(max(0.0, min(255.0, c)) for c in (r, g, b))  # type: ignore[return-value]


def whitepoint(k: float) -> tuple[float, float, float]:
    """How much of each colour to keep at temperature k, relative to daylight (so 6500 K keeps all of each)."""
    if k >= NORMAL_K:
        return (1.0, 1.0, 1.0)
    ref, now = _rgb(NORMAL_K), _rgb(k)
    return tuple(min(1.0, n / r) for n, r in zip(now, ref, strict=True))  # type: ignore[return-value]


class _Resources(ctypes.Structure):
    _fields_ = [("timestamp", ctypes.c_ulong), ("configTimestamp", ctypes.c_ulong),
                ("ncrtc", ctypes.c_int), ("crtcs", ctypes.POINTER(ctypes.c_ulong)),
                ("noutput", ctypes.c_int), ("outputs", ctypes.POINTER(ctypes.c_ulong)),
                ("nmode", ctypes.c_int), ("modes", ctypes.c_void_p)]


class _Gamma(ctypes.Structure):
    _fields_ = [("size", ctypes.c_int), ("red", ctypes.POINTER(ctypes.c_ushort)),
                ("green", ctypes.POINTER(ctypes.c_ushort)), ("blue", ctypes.POINTER(ctypes.c_ushort))]


def _libs() -> tuple[ctypes.CDLL, ctypes.CDLL]:
    x11 = ctypes.CDLL(ctypes.util.find_library("X11") or "libX11.so.6")
    xrr = ctypes.CDLL(ctypes.util.find_library("Xrandr") or "libXrandr.so.2")
    x11.XOpenDisplay.argtypes, x11.XOpenDisplay.restype = [ctypes.c_char_p], ctypes.c_void_p
    x11.XDefaultRootWindow.argtypes, x11.XDefaultRootWindow.restype = [ctypes.c_void_p], ctypes.c_ulong
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
    x11.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
    xrr.XRRGetScreenResourcesCurrent.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    xrr.XRRGetScreenResourcesCurrent.restype = ctypes.POINTER(_Resources)
    xrr.XRRFreeScreenResources.argtypes = [ctypes.POINTER(_Resources)]
    xrr.XRRGetCrtcGammaSize.argtypes, xrr.XRRGetCrtcGammaSize.restype = [ctypes.c_void_p, ctypes.c_ulong], ctypes.c_int
    xrr.XRRGetCrtcGamma.argtypes, xrr.XRRGetCrtcGamma.restype = [ctypes.c_void_p, ctypes.c_ulong], ctypes.POINTER(_Gamma)
    xrr.XRRAllocGamma.argtypes, xrr.XRRAllocGamma.restype = [ctypes.c_int], ctypes.POINTER(_Gamma)
    xrr.XRRSetCrtcGamma.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(_Gamma)]
    xrr.XRRFreeGamma.argtypes = [ctypes.POINTER(_Gamma)]
    return x11, xrr


def _each_crtc(fn) -> int:   # noqa: ANN001
    """fn(xrr, display, crtc, gamma size) for every screen output (CRTC) with colour ramps; how many there were."""
    x11, xrr = _libs()
    dpy = x11.XOpenDisplay((os.environ.get("DISPLAY") or ":0").encode())
    if not dpy:
        raise RuntimeError("The screen can't be reached (no display).")
    try:
        res = xrr.XRRGetScreenResourcesCurrent(dpy, x11.XDefaultRootWindow(dpy))
        if not res:
            raise RuntimeError("The screen doesn't say what it's made of (no RandR).")
        n = 0
        try:
            for i in range(res.contents.ncrtc):
                crtc = res.contents.crtcs[i]
                size = xrr.XRRGetCrtcGammaSize(dpy, crtc)
                if size > 1:
                    fn(xrr, dpy, crtc, size)
                    n += 1
        finally:
            xrr.XRRFreeScreenResources(res)
        x11.XSync(dpy, 0)
        return n
    finally:
        x11.XCloseDisplay(dpy)


def set_temperature(k: float) -> int:
    """Every screen at colour temperature k (6500 or more: the normal colours). Returns how many were set."""
    r, g, b = whitepoint(k)

    def apply(xrr, dpy, crtc, size) -> None:   # noqa: ANN001
        ramp = xrr.XRRAllocGamma(size)
        if not ramp:
            raise RuntimeError("Out of memory for the screen's colours.")
        try:
            for j in range(size):
                v = j * 65535.0 / (size - 1)
                ramp.contents.red[j] = int(v * r)
                ramp.contents.green[j] = int(v * g)
                ramp.contents.blue[j] = int(v * b)
            xrr.XRRSetCrtcGamma(dpy, crtc, ramp)
        finally:
            xrr.XRRFreeGamma(ramp)

    n = _each_crtc(apply)
    if n == 0:
        raise RuntimeError("This screen's colours can't be changed (its driver has no colour ramps).")
    return n


def current() -> list[tuple[int, int, int]]:
    """Each screen's white now (the top of its ramps, 0-65535 per colour): (65535, 65535, 65535) is normal."""
    out: list[tuple[int, int, int]] = []

    def read(xrr, dpy, crtc, size) -> None:   # noqa: ANN001
        g = xrr.XRRGetCrtcGamma(dpy, crtc)
        if g:
            try:
                top = g.contents.size - 1
                out.append((g.contents.red[top], g.contents.green[top], g.contents.blue[top]))
            finally:
                xrr.XRRFreeGamma(g)

    _each_crtc(read)
    return out
