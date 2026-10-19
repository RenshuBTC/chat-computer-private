"""The touchscreen, as on an iPad: what one or more fingers do before the pages under them see them.

Most touches go straight to the page under the finger, and WebKit does there what an iPad does: a tap taps, a
finger moved scrolls (a quick swipe scrolls on, a touch stops it), two fingers pinch to zoom a website. The
conversation page adds the iPad's own ways for it (chat.js): pinch for the text size, swipe a sidebar item to pin
it, hold a message to select it, double and triple taps to select a word or a paragraph.

What's here is for the whole screen: the iPad's system gestures, which no page sees (Apple's iPad User Guide,
"Learn gestures for iPad models with Face ID", "View and respond to notifications", "Select and edit text"):
- swipe up from the bottom edge: Home: what's open closes and the conversation is back (or the keyboard goes);
- swipe left or right along the bottom edge: switch between open apps: here, back or forward through what was
  opened;
- swipe in from the left edge: back, as in Safari; from the right edge: forward;
- swipe down from the top centre: Notification Center: here, Messages, when the screen is too narrow to show it;
- three fingers: pinch closed to copy (twice to cut), pinch open to paste, swipe left to undo and right to redo;
- four or five fingers: pinch closed or swipe up for Home, swipe right or left for back or forward.
And the onscreen keyboard (shown for a touch on a box to type in): its keys are pressed here, so nothing under
it gets those touches.

TouchGestures is the arithmetic, with no GTK in it (tests feed it fingers); ui/gtk_app.py gives it every touch
and does what it says: hand the touch to the page, hold it back for now, take it, or press a key.
"""
from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Any, Callable

EDGE_PX = 22            # a swipe starting this near an edge may be the system's (the iPad's edges are about as wide)
SWIPE_PX = 48           # how far a finger goes for a swipe to count
GIVE_UP_PX = 110        # further than this, the wrong way, and it was the page's after all
HOLD_S = 0.6            # an edge touch that hasn't gone anywhere by then is the page's (a press, not a swipe)
PINCH_IN, PINCH_OUT = 0.65, 1.5     # fingers this much closer together or further apart: a pinch
CUT_WITHIN_S = 1.2      # three-finger pinch closed twice within this long: cut (the first one copied)

# Verdicts for a touch's events.
PASS = "pass"           # to the page under it, as it is
HOLD = "hold"           # kept back: maybe the system's (an edge swipe); later "release" (to the page) or "take"
RELEASE = "release"     # it wasn't the system's: give the page what was held back, then this
TAKE = "take"           # the system's: the page never sees it
KEY = "key"             # on the onscreen keyboard


@dataclass
class Finger:
    start: tuple[float, float]
    now: tuple[float, float]
    t0: float
    verdict: str
    edge: str = ""                       # bottom, left, right, top: where an edge swipe started
    path: list[tuple[float, float]] = field(default_factory=list)


class TouchGestures:
    def __init__(self, act: Callable[[str], None]) -> None:
        self.act = act                   # what a gesture means: home, back, forward, messages, copy, cut,...
        self.size = (1920.0, 1080.0)     # the window's (pixels)
        self.keyboard: tuple[float, float, float, float] | None = None   # the onscreen keyboard while it's up
        self.fingers: dict[Any, Finger] = {}
        self.multi: dict[Any, tuple[tuple[float, float], tuple[float, float]]] | None = None  # 3+ fingers: start, now
        self.multi_peak = 0
        self.last_copy = -10.0

    # ---------------------------------------------------------------- where a touch starts
    def _edge(self, x: float, y: float) -> str:
        w, h = self.size
        if y >= h - EDGE_PX:
            return "bottom"
        if x <= EDGE_PX:
            return "left"
        if x >= w - EDGE_PX:
            return "right"
        if y <= EDGE_PX and 0.3 * w <= x <= 0.7 * w:
            return "top"
        return ""

    def _on_keyboard(self, x: float, y: float) -> bool:
        k = self.keyboard
        return bool(k) and k[0] <= x <= k[0] + k[2] and k[1] <= y <= k[1] + k[3]

    def begin(self, seq: Any, x: float, y: float, t: float) -> tuple[str, list[Any]]:
        """A finger down. Returns what to do with its touch, and touches already given to the page that it must
        now be told are cancelled (a third finger makes it a gesture of the screen's)."""
        cancel: list[Any] = []
        if self._on_keyboard(x, y) and self.multi is None:
            self.fingers[seq] = Finger((x, y), (x, y), t, KEY)
            return KEY, cancel
        others = [f for s, f in self.fingers.items() if f.verdict != KEY]
        if self.multi is None and len(others) >= 2:
            # A third finger: a gesture with several fingers (from where they are now). The page had the first
            # two: they're cancelled there.
            self.multi = {s: (f.now, f.now) for s, f in self.fingers.items() if f.verdict != KEY}
            for s, f in self.fingers.items():
                if f.verdict == PASS:
                    cancel.append(s)
                if f.verdict != KEY:
                    f.verdict = TAKE
        if self.multi is not None:
            self.multi[seq] = ((x, y), (x, y))
            self.multi_peak = max(self.multi_peak, len(self.multi))
            self.fingers[seq] = Finger((x, y), (x, y), t, TAKE)
            return TAKE, cancel
        held = [s for s, f in self.fingers.items() if f.verdict == HOLD]
        edge = self._edge(x, y) if not others else ""
        if edge:
            self.fingers[seq] = Finger((x, y), (x, y), t, HOLD, edge, [(x, y)])
            return HOLD, cancel
        for s in held:                   # a second finger: what was held back was the page's after all
            self.fingers[s].verdict = PASS
        self.fingers[seq] = Finger((x, y), (x, y), t, PASS)
        return (RELEASE if held else PASS), cancel     # RELEASE: what was held back goes to the page first

    # ---------------------------------------------------------------- moving
    def _swipe(self, f: Finger) -> str:
        """What an edge swipe has become so far: a gesture's name, "" (not yet), or "no" (not a swipe)."""
        dx, dy = f.now[0] - f.start[0], f.now[1] - f.start[1]
        if f.edge == "bottom":
            if -dy >= SWIPE_PX and -dy >= abs(dx):
                return "home"
            if abs(dx) >= SWIPE_PX and abs(dx) > -dy:
                return "back" if dx > 0 else "forward"
            return "no" if dy > SWIPE_PX else ""
        if f.edge == "left":
            if dx >= SWIPE_PX and dx > abs(dy):
                return "back"
            return "no" if abs(dy) > GIVE_UP_PX or dx < -SWIPE_PX else ""
        if f.edge == "right":
            if -dx >= SWIPE_PX and -dx > abs(dy):
                return "forward"
            return "no" if abs(dy) > GIVE_UP_PX or dx > SWIPE_PX else ""
        if f.edge == "top":
            if dy >= SWIPE_PX and dy > abs(dx):
                return "messages"
            return "no" if abs(dx) > GIVE_UP_PX else ""
        return "no"

    def move(self, seq: Any, x: float, y: float, t: float) -> str:
        f = self.fingers.get(seq)
        if f is None:
            return PASS
        f.now = (x, y)
        if self.multi is not None and seq in self.multi:
            self.multi[seq] = (self.multi[seq][0], (x, y))
        if f.verdict == HOLD:
            f.path.append((x, y))
            if self._swipe(f) == "no" or (t - f.t0 > HOLD_S and not self._swipe(f)
                                          and math.dist(f.start, f.now) < SWIPE_PX / 2):
                f.verdict = PASS
                return RELEASE
        return f.verdict

    # ---------------------------------------------------------------- lifting
    def end(self, seq: Any, x: float, y: float, t: float) -> tuple[str, str]:
        """A finger up: what to do with its touch, and the gesture it finished ("" for none)."""
        f = self.fingers.pop(seq, None)
        if f is None:
            return PASS, ""
        f.now = (x, y)
        if self.multi is not None and seq in self.multi:
            self.multi[seq] = (self.multi[seq][0], (x, y))
            if not any(s in self.multi for s in self.fingers):
                done, self.multi = self.multi, None
                peak, self.multi_peak = self.multi_peak, 0
                gesture = self._fingers_gesture(done, peak, t)
                if gesture:
                    self.act(gesture)
                return TAKE, gesture
            return TAKE, ""
        if f.verdict == HOLD:
            gesture = self._swipe(f)
            if gesture and gesture != "no":
                self.act(gesture)
                return TAKE, gesture
            return RELEASE, ""
        return f.verdict, ""

    def cancel(self, seq: Any) -> str:
        f = self.fingers.pop(seq, None)
        if self.multi is not None:
            self.multi.pop(seq, None)
            if not self.multi:
                self.multi, self.multi_peak = None, 0
        return f.verdict if f else PASS

    def _fingers_gesture(self, done: dict[Any, tuple[tuple[float, float], tuple[float, float]]], peak: int,
                         t: float) -> str:
        """Three, four or five fingers, lifted: what they did."""
        starts = [s for s, _ in done.values()]
        ends = [e for _, e in done.values()]
        if len(starts) < 3:
            return ""

        def centre(ps: list[tuple[float, float]]) -> tuple[float, float]:
            return sum(p[0] for p in ps) / len(ps), sum(p[1] for p in ps) / len(ps)

        def spread(ps: list[tuple[float, float]]) -> float:
            c = centre(ps)
            return sum(math.dist(p, c) for p in ps) / len(ps)
        c0, c1 = centre(starts), centre(ends)
        dx, dy = c1[0] - c0[0], c1[1] - c0[1]
        ratio = spread(ends) / max(1.0, spread(starts))
        swipe_x = abs(dx) >= SWIPE_PX and abs(dx) > 1.5 * abs(dy)
        swipe_up = -dy >= SWIPE_PX and -dy > 1.5 * abs(dx)
        if peak >= 4:
            if ratio < PINCH_IN or swipe_up:
                return "home"
            if swipe_x:
                return "back" if dx > 0 else "forward"
            return ""
        if ratio < PINCH_IN:
            if t - self.last_copy <= CUT_WITHIN_S:
                self.last_copy = -10.0
                return "cut"
            self.last_copy = t
            return "copy"
        if ratio > PINCH_OUT:
            return "paste"
        if swipe_x:
            return "undo" if dx < 0 else "redo"
        return ""
