"""Moving the pointer, clicking and scrolling, as if with a mouse: through the X server's XTEST extension
(libXtst, which xdotool uses too), called directly so a move costs no new process. The pointer's arrow is
hidden and shown with XFIXES (libXfixes)."""
from __future__ import annotations

import ctypes
import ctypes.util
from typing import Any

MOUSE_YIELD_S = 3.0          # a mouse that moved the pointer keeps it until it has been still this long


class XInput:
    def __init__(self, display: str | None = None) -> None:
        self.x11 = ctypes.CDLL(ctypes.util.find_library("X11") or "libX11.so.6")
        self.xtst = ctypes.CDLL(ctypes.util.find_library("Xtst") or "libXtst.so.6")
        x11, xtst = self.x11, self.xtst
        x11.XOpenDisplay.restype = ctypes.c_void_p
        x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
        x11.XFlush.argtypes = [ctypes.c_void_p]
        x11.XDefaultScreen.argtypes = [ctypes.c_void_p]
        x11.XDisplayWidth.argtypes = [ctypes.c_void_p, ctypes.c_int]
        x11.XDisplayHeight.argtypes = [ctypes.c_void_p, ctypes.c_int]
        x11.XDefaultRootWindow.restype = ctypes.c_ulong
        x11.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
        x11.XQueryPointer.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong),
                                      ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int),
                                      ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int),
                                      ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_uint)]
        xtst.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
        xtst.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        self.dpy = x11.XOpenDisplay(display.encode() if display else None)
        if not self.dpy:
            raise OSError("can't reach the X display")
        self.xfixes: Any = None
        try:
            xf = ctypes.CDLL(ctypes.util.find_library("Xfixes") or "libXfixes.so.3")
            xf.XFixesQueryVersion.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int)]
            xf.XFixesHideCursor.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
            xf.XFixesShowCursor.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
            major, minor = ctypes.c_int(5), ctypes.c_int(0)
            if xf.XFixesQueryVersion(self.dpy, ctypes.byref(major), ctypes.byref(minor)) and major.value >= 4:
                self.xfixes = xf
        except OSError:
            pass

    def screen_size(self) -> tuple[int, int]:
        s = self.x11.XDefaultScreen(self.dpy)
        return self.x11.XDisplayWidth(self.dpy, s), self.x11.XDisplayHeight(self.dpy, s)

    def position(self) -> tuple[int, int]:
        root = self.x11.XDefaultRootWindow(self.dpy)
        r, c = ctypes.c_ulong(), ctypes.c_ulong()
        rx, ry, wx, wy, mask = ctypes.c_int(), ctypes.c_int(), ctypes.c_int(), ctypes.c_int(), ctypes.c_uint()
        self.x11.XQueryPointer(self.dpy, root, ctypes.byref(r), ctypes.byref(c), ctypes.byref(rx), ctypes.byref(ry),
                               ctypes.byref(wx), ctypes.byref(wy), ctypes.byref(mask))
        return rx.value, ry.value

    def move(self, x: float, y: float) -> None:
        self.xtst.XTestFakeMotionEvent(self.dpy, -1, int(round(x)), int(round(y)), 0)
        self.x11.XFlush(self.dpy)

    def click(self, x: float, y: float, button: int = 1) -> None:
        self.xtst.XTestFakeMotionEvent(self.dpy, -1, int(round(x)), int(round(y)), 0)
        self.xtst.XTestFakeButtonEvent(self.dpy, button, 1, 0)
        self.xtst.XTestFakeButtonEvent(self.dpy, button, 0, 0)
        self.x11.XFlush(self.dpy)

    def scroll(self, dx: int, dy: int) -> None:
        """Wheel clicks: dy > 0 down (button 5), < 0 up (4); dx > 0 right (7), < 0 left (6)."""
        for steps, more, less in ((dy, 5, 4), (dx, 7, 6)):
            button = more if steps > 0 else less
            for _ in range(min(abs(steps), 20)):
                self.xtst.XTestFakeButtonEvent(self.dpy, button, 1, 0)
                self.xtst.XTestFakeButtonEvent(self.dpy, button, 0, 0)
        self.x11.XFlush(self.dpy)

    def show_cursor(self, show: bool) -> None:
        """Hide the pointer's arrow, or show it again. The X server shows it only once every program that hid
        it has shown it again (or closed), so this doesn't fight the idle-hider (unclutter)."""
        if self.xfixes is not None:
            root = self.x11.XDefaultRootWindow(self.dpy)
            (self.xfixes.XFixesShowCursor if show else self.xfixes.XFixesHideCursor)(self.dpy, root)
            self.x11.XFlush(self.dpy)

    def close(self) -> None:
        if self.dpy:
            self.x11.XCloseDisplay(self.dpy)
            self.dpy = None

    def __enter__(self) -> "XInput":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()


class SharedPointer:
    """AirTouch's use of the display's pointer, shared with a real mouse. While AirTouch moves it the arrow is
    hidden (the dots show where the hands are); when something else moves it (it isn't where AirTouch left it:
    a mouse, or a click Claude made) the arrow comes back and AirTouch leaves the pointer alone until that has
    been still for MOUSE_YIELD_S. Clicks and scrolls always happen: a pinch is meant."""

    def __init__(self, x: Any) -> None:
        self.x = x
        self.left_at: tuple[int, int] | None = None
        self.mouse_at: tuple[int, int] | None = None       # where the mouse last put it
        self.hidden = False
        self.mouse_t = -1e9

    def check(self, t: float) -> bool:
        """Whether a mouse has the pointer (call this often: it notices one moving it, and one that goes on moving
        it keeps it)."""
        at = self.left_at if self.left_at is not None else self.mouse_at
        if at is not None:
            px, py = self.x.position()
            if abs(px - at[0]) > 2 or abs(py - at[1]) > 2:
                self.mouse_t, self.left_at, self.mouse_at = t, None, (px, py)
                self._arrow(True)
        return t - self.mouse_t < MOUSE_YIELD_S

    def move(self, t: float, x: float, y: float) -> None:
        if self.check(t):
            return
        self._arrow(False)
        self.x.move(x, y)
        self.left_at = (int(round(x)), int(round(y)))

    def click(self, t: float, x: float, y: float) -> None:
        self.check(t)
        self._arrow(False)
        self.x.click(x, y)
        self.left_at = (int(round(x)), int(round(y)))

    def scroll(self, t: float, dx: int, dy: int) -> None:
        self.check(t)
        self.x.scroll(dx, dy)

    def release(self) -> None:
        """AirTouch is done with the pointer: the arrow shows again (the idle-hider still hides it)."""
        self._arrow(True)
        self.left_at = None

    def _arrow(self, show: bool) -> None:
        if show == (not self.hidden):
            return
        self.hidden = not show
        try:
            self.x.show_cursor(show)
        except Exception:
            pass
