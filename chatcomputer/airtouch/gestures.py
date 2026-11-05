"""From hand joints, picture after picture, to what the hands mean.

AirTouch works like a touchscreen in the air, with a dot on the screen for each index finger:
- A dot follows each index fingertip, of either hand or both, where it is in front of the screen.
- Pinch thumb and index finger together and let go: a tap where that hand's dot is.
- Pinch and move up or down: scroll, as if dragging the page.
- Hold up an open hand, palm towards the screen: stop (Chat Computer stops talking; a video pauses) or, asleep, wake.
- Thumbs up or down, held a moment: yes or no to a question Chat Computer asked.
- Beckon up (palm up, all four fingers curled towards you once, "come here"): the keyboard comes up. Beckon
  down (the same with the palm down): it goes away.
- With the keyboard up, a hand's four fingers rest on four keys side by side, as on a real keyboard: its index
  finger's dot shows where, and the other fingers are on the next keys out. Moving the hand moves them; a
  finger tapping down (any of the four) types the key it's on.

Positions are absolute: a place in front of the screen is a place on the screen. The space the hands work in
is measured in hand sizes (wrist to middle knuckle), so it is the same close to the camera or across the room,
and it moves along when a hand goes past its edge. With the keyboard up it is a keyboard's size: a key per
19 mm, like a real one.

Everything here is geometry on the 21 joints (hands.py's numbering), in three dimensions (the landmark
network's depth), so it is the same for any camera.
"""
from __future__ import annotations

import math
from collections import deque
from dataclasses import dataclass, field
from typing import Any, Mapping

import numpy as np

from .keyboard import HOME, HOME_ROW

FINGERS = {"index": (5, 6, 7, 8), "middle": (9, 10, 11, 12), "ring": (13, 14, 15, 16), "little": (17, 18, 19, 20)}
TIPS = {name: joints[3] for name, joints in FINGERS.items()}
PALM = (0, 5, 9, 13, 17)

# Pinching, in hand sizes (wrist to middle knuckle).
PINCH_ON = 0.32          # thumb tip to index tip
PINCH_OFF = 0.48
PINCH_REACH = 0.55       # index tip to knuckle: 0.4 in a fist, 0.7 pinching, 0.9 or more pointing
TAP_MAX_S = 0.8          # a pinch let go of sooner, without moving, is a tap
DRAG_START = 0.18        # hand movement that turns a pinch into scrolling
SCROLL_STEP = 0.07       # hand movement per wheel click
# Held gestures.
HOLD_S = 0.6             # an open hand or thumb held this long counts
HOLD_STILL = 0.5         # ...if it moved less than this meanwhile
FACING = -0.3            # "stop" wants the palm towards the camera: its direction's depth part under this
THUMB_RISE = 0.45        # thumbs up: its tip this far above its base (hand sizes); down, as far below
# Beckoning.
UP = -0.45               # the palm's direction, upwards part (image y grows downwards, so -1 is straight up)
OPEN_DEG = 50.0          # fingers bent less than this are open...
CURLED_DEG = 95.0        # ...and more than this, curled
BECKON_S = 0.8           # curled within this long of being open
BECKON_STILL = 0.6       # while the hand itself stays put
BECKON_AGAIN_S = 1.0     # and not again until open for this long
# Where the dots go.
SPAN = 5.0               # hand sizes across the whole screen
KEY_PITCH = 0.21         # hand sizes from one key to the next: a real keyboard's 19 mm (with both hands in view
KEY_PITCH_RANGE = (0.15, 0.35)     # when the keyboard comes up, the hands' own spread sets it, within these)
ROW_PITCH = 0.2
KNUCKLES = (5, 9, 13, 17)          # with the keyboard up, a hand's four fingers' keys go where its knuckles are...
FINGER_KEYS = {"index": -1.5, "middle": -0.5, "ring": 0.5, "little": 1.5}     # ...a key apart (index to little)
# (The hands arrive on the keyboard where they rest on a real one: the left on a s d f, the right on j k l and the
# next key, HOME and HOME_ROW in keyboard.py.)
HAND = -1                # the knuckles' point, among a hand's points
LOST_S = 0.6             # a hand is gone after this long unseen
FORGET_S = 10.0          # the working space is placed afresh after this long without hands
GLIDE_S = 0.2            # a dot held still for a pinch glides back to its finger
ACTIVE_MOVE = 3.0        # pixels: the hand moving its dot more than this drives the pointer
CALM = 0.3               # hand sizes a second: slower is resting
DOT_CUTOFF = 0.8         # the dots' 1-euro filter, in hand sizes
DOT_BETA = 0.8
# Typing: how far a finger bends towards the palm (degrees), compared with where it rests.
TAP_ON = 18.0            # a key press
TAP_OFF = 8.0            # back under this: let go
TAP_LEAD = 6.0           # and more than any other finger of that hand (they move together a little)
TAP_HOVER_S = 0.1        # the key typed is the one under the dot just before the finger moved
TAP_STILL = 2.0          # not while the hand moves faster than this (hand sizes a second)
TAP_HOLD_S = 1.0         # bent longer than this: that's where it rests now
REST_FOLLOW = 0.08


class OneEuro:
    """The 1-euro filter (Casiez, Roussel and Vogel, CHI 2012): smooths a moving value, more when it moves
    slowly (less jitter) and less when it moves fast (less lag)."""

    def __init__(self, min_cutoff: float = 1.0, beta: float = 0.05, d_cutoff: float = 1.0) -> None:
        self.min_cutoff, self.beta, self.d_cutoff = min_cutoff, beta, d_cutoff
        self.x: np.ndarray | None = None
        self.dx: np.ndarray | None = None
        self.t = 0.0

    @staticmethod
    def _alpha(cutoff: float, dt: float) -> float:
        tau = 1.0 / (2 * math.pi * cutoff)
        return 1.0 / (1.0 + tau / dt)

    def __call__(self, x: np.ndarray, t: float) -> np.ndarray:
        x = np.asarray(x, dtype=np.float64)
        if self.x is None or t <= self.t:
            self.x, self.dx, self.t = x, np.zeros_like(x), t
            return x
        dt = t - self.t
        self.t = t
        dx = (x - self.x) / dt
        a_d = self._alpha(self.d_cutoff, dt)
        self.dx = a_d * dx + (1 - a_d) * self.dx
        cutoff = self.min_cutoff + self.beta * float(np.linalg.norm(self.dx))
        a = self._alpha(cutoff, dt)
        self.x = a * x + (1 - a) * self.x
        return self.x

    def reset(self) -> None:
        self.x = None

    def rescale(self, k: float) -> None:
        """The same positions in new units (value * k)."""
        if self.x is not None:
            self.x = self.x * k
            self.dx = self.dx * k


@dataclass
class Pose:
    size: float              # wrist to middle knuckle, pixels (in three dimensions, so tilting doesn't shrink it)
    extended: dict[str, bool]
    thumb_out: bool
    thumb_up: bool           # thumb tip clearly above its base
    thumb_down: bool
    pinch: float             # thumb tip to index tip, in hand sizes
    reach: float             # index tip to its own knuckle, in hand sizes: under about half in a fist
    curl: dict[str, float]   # each finger's bend, degrees: 0 straight on from the palm, 180 folded back
    flex: dict[str, float]   # each finger's bend towards the palm side, degrees (for typing)
    normal: np.ndarray | None = None    # the way the palm faces (unit, camera axes: x right, y down, z away);
                                        # None when which hand it is isn't known

    @property
    def fist(self) -> bool:
        return not any(self.extended.values())

    @property
    def open(self) -> bool:
        return all(self.extended.values()) and self.thumb_out

    @property
    def thumb_only(self) -> bool:
        return self.thumb_out and not any(self.extended.values())

    @property
    def facing(self) -> bool:
        """The palm towards the camera, not up (or not known which way)."""
        return self.normal is None or (self.normal[2] < FACING and self.normal[1] >= UP)

    @property
    def palm_up(self) -> bool:
        return self.normal is not None and self.normal[1] < UP

    @property
    def palm_down(self) -> bool:
        return self.normal is not None and self.normal[1] > -UP


def _angle(a: np.ndarray, b: np.ndarray) -> float:
    na, nb = float(np.linalg.norm(a)), float(np.linalg.norm(b))
    if na < 1e-9 or nb < 1e-9:
        return 0.0
    return math.degrees(math.acos(max(-1.0, min(1.0, float(np.dot(a, b)) / (na * nb)))))


def pose_of(joints: np.ndarray, right: float | None = None) -> Pose:
    """The hand's shape from its joints (pixels, with depth): which fingers are out and how bent, the thumb's
    direction, how close thumb and index tips are, and (knowing which hand it is: `right` is how sure it's a
    right hand) which way the palm faces."""
    p = joints.astype(np.float64)
    size = float(np.linalg.norm(p[9] - p[0])) or 1.0
    wrist = p[0]
    ext, curl = {}, {}
    for name, (mcp, pip, _dip, tip) in FINGERS.items():
        # Out when the tip is further from the wrist than the middle joint is, by a margin.
        ext[name] = bool(np.linalg.norm(p[tip] - wrist) > np.linalg.norm(p[pip] - wrist) * 1.12)
        curl[name] = _angle(p[mcp] - wrist, p[tip] - p[mcp])
    # The thumb is out when its tip is well away from the index finger's base and from the palm's centre.
    palm = p[list(PALM)].mean(axis=0)
    thumb_out = bool(np.linalg.norm(p[4] - p[5]) > 0.55 * size and np.linalg.norm(p[4] - palm) > 0.75 * size)
    rise = (p[2][1] - p[4][1]) / size          # image y grows downwards
    # Across the palm, index knuckle and little knuckle seen from the wrist: for a right hand that points out of
    # the palm, for a left hand out of its back.
    normal = None
    if right is not None and abs(right - 0.5) >= 0.2:
        n = np.cross(p[5] - wrist, p[17] - wrist)
        length = float(np.linalg.norm(n))
        if length > 1e-9:
            normal = n / length * (1.0 if right > 0.5 else -1.0)
    flex = dict(curl)
    if normal is not None:
        fwd = (p[9] - wrist) - np.dot(p[9] - wrist, normal) * normal
        fwd /= float(np.linalg.norm(fwd)) or 1.0
        for name, (mcp, _pip, _dip, tip) in FINGERS.items():
            f = p[tip] - p[mcp]
            a = math.degrees(math.atan2(float(np.dot(f, normal)), float(np.dot(f, fwd))))
            flex[name] = a + 360.0 if a < -120.0 else a      # folded right back is past 180, not far the other way
    return Pose(size, ext, thumb_out, bool(thumb_out and rise > THUMB_RISE), bool(thumb_out and rise < -THUMB_RISE),
                float(np.linalg.norm(p[4] - p[8]) / size), float(np.linalg.norm(p[8] - p[5]) / size),
                curl, flex, normal)


@dataclass
class _Hand:
    """What the engine keeps about one hand while it's seen (and for a moment after)."""
    id: int
    right: float | None = None               # how sure it's a right hand, followed
    seen_at: float = 0.0
    t_prev: float = 0.0
    palm_px: np.ndarray | None = None        # palm centre, camera pixels
    palm: np.ndarray | None = None           # the same in hand sizes
    speed: float = 0.0                       # hand sizes a second
    filters: dict[int, OneEuro] = field(default_factory=dict)
    points: dict[int, np.ndarray] = field(default_factory=dict)    # index tip (or knuckles), smoothed, camera px
    order: float = 1.0                       # > 0: its index finger is right of its little finger in the picture
    dots: dict[int, np.ndarray] = field(default_factory=dict)      # their dots, screen pixels
    moved: float = 0.0                       # how far its first dot moved in the last picture (pixels)
    trail: deque = field(default_factory=lambda: deque(maxlen=16))  # (time, dots) of the last pictures
    pinched: bool = False
    votes: int = 0
    pinch_t0: float = 0.0
    pinch_from: np.ndarray | None = None
    frozen: np.ndarray | None = None         # the index dot, held still while a pinch forms
    glide_from: np.ndarray | None = None
    glide_t0: float = 0.0
    scrolling: bool = False
    scroll_acc: list[float] = field(default_factory=lambda: [0.0, 0.0])
    hold: str = ""
    hold_t0: float = 0.0
    hold_from: np.ndarray | None = None
    fired: str = ""                          # the held gesture already sent, until the hand changes
    open_side: str = ""                      # last seen open with the palm "up" or "down"...
    open_up_at: float = -1e9                 # ...then
    open_up_from: np.ndarray | None = None
    beckoned_at: float = -1e9
    armed: bool = True
    rest: dict[str, float] = field(default_factory=dict)            # each finger's bend at rest (typing)
    pressed: str = ""
    press_t0: float = 0.0


class Engine:
    """Turns hands into actions. update() takes the time (seconds) and the hands seen in that picture
    ({number: (joints, how sure it's a right hand or None)}, or one hand's joints, or None) and returns the
    actions, in order:
      ("move", x, y)             move the pointer to x, y (screen pixels)
      ("click", x, y)            a tap there
      ("scroll", dx, dy)         wheel clicks: dy > 0 scrolls down, dx > 0 right
      ("key", x, y)              a finger typed whatever key is there (with the keyboard up)
      ("gesture", name)          "palm" (stop, or wake), "yes", "no", "keyboard up", "keyboard down"
      ("hand", present)          hands came into view, or the last one left
      ("dots", [(x, y, kind)])   a dot per hand, for the screen to draw: kind "point", "pinch", "scroll",
                                 or with the keyboard up "key" and "press"
    """

    def __init__(self, screen_w: int, screen_h: int) -> None:
        self.screen_w, self.screen_h = screen_w, screen_h
        self.cursor = [screen_w / 2, screen_h / 2]
        self.hands: dict[int, _Hand] = {}
        self.keyboard: tuple[float, ...] | None = None    # x, y, width, height, columns, rows
        self.key_pitch = KEY_PITCH                          # hand sizes per key, with the keyboard up
        self.centre: np.ndarray | None = None              # the working space's centre, camera pixels
        self.unit = 0.0                                     # its hand size, pixels
        self.last_seen = -1e9
        self.present = False
        self.active: int | None = None                      # the hand driving the pointer
        self.calm = True

    # ---------------------------------------------------------------- settings
    def resize(self, w: int, h: int) -> None:
        self.screen_w, self.screen_h = w, h
        self.cursor = [min(max(self.cursor[0], 0), w - 1), min(max(self.cursor[1], 0), h - 1)]

    def set_keyboard(self, spec: tuple[float, ...] | None) -> None:
        """The keyboard is up (its keys' area: x, y, width, height in screen pixels, and how many keys across
        and rows down), or not (None). The dots start again either way, placed afresh."""
        self.keyboard = tuple(float(v) for v in spec) if spec else None
        self.centre, self.key_pitch = None, KEY_PITCH
        for h in self.hands.values():
            h.filters, h.points, h.dots, h.trail = {}, {}, {}, deque(maxlen=16)
            h.pinched = h.scrolling = False
            h.frozen = h.glide_from = h.pinch_from = None
            h.votes, h.rest, h.pressed = 0, {}, ""

    # ---------------------------------------------------------------- each picture
    def update(self, t: float, hands: Any = None) -> list[tuple]:
        if hands is None:
            hands = {}
        elif isinstance(hands, np.ndarray):
            hands = {0: (hands, None)}
        out: list[tuple] = []
        for hid in [k for k in self.hands if k not in hands]:
            if t - self.hands[hid].seen_at > LOST_S:
                self._drop(self.hands.pop(hid), out)
        for hid, (joints, _right) in hands.items():
            if hid not in self.hands:
                self.hands[hid] = self._adopt(hid, joints, hands) or _Hand(hid)
        if not self.hands:
            if self.present:
                self.present = False
                out += [("dots", []), ("hand", False)]
            if t - self.last_seen > FORGET_S:
                self.centre = None
            self.calm = True
            return out
        if hands:
            self.last_seen = t
        if not self.present:
            self.present = True
            out.append(("hand", True))

        poses: dict[int, Pose] = {}
        for hid, (joints, right) in hands.items():
            h = self.hands[hid]
            if right is not None:
                h.right = float(right) if h.right is None else 0.8 * h.right + 0.2 * float(right)
            poses[hid] = pose_of(joints, h.right)
        if poses:
            self._follow_unit(float(np.mean([p.size for p in poses.values()])), hands)

        for hid, (joints, _right) in hands.items():
            h, pose = self.hands[hid], poses[hid]
            j = np.asarray(joints, dtype=np.float64)
            palm_px = j[list(PALM), :2].mean(axis=0)
            palm = palm_px / self.unit
            delta = np.zeros(2) if h.palm is None else palm - h.palm
            if h.palm is not None:
                h.speed = 0.7 * h.speed + 0.3 * float(np.linalg.norm(delta)) / max(1e-3, t - h.t_prev)
            h.seen_at, h.t_prev, h.palm_px, h.palm = t, t, palm_px, palm
            if self.keyboard:
                # The knuckles: they stay put while a finger taps, so the keys under the dots do too.
                which, raw = HAND, j[list(KNUCKLES), :2].mean(axis=0)
                across = float(j[TIPS["index"], 0] - j[TIPS["little"], 0])
                if abs(across) > 0.05 * pose.size:
                    h.order = 0.7 * h.order + 0.3 * math.copysign(1.0, across)
            else:
                which, raw = TIPS["index"], j[TIPS["index"], :2]
            f = h.filters.setdefault(which, OneEuro(DOT_CUTOFF, DOT_BETA))
            h.points = {which: f(raw / self.unit, t) * self.unit}
            self._holds(t, h, pose, palm, out)
            self._beckon(t, h, pose, palm, out)
            if self.keyboard:
                self._type(t, h, pose, out)
            else:
                self._pinch(t, h, pose, palm, delta, out)

        if self.centre is None:
            self._place(hands)
        self._push(hands)
        for hid in hands:
            h = self.hands[hid]
            before = next(iter(h.dots.values()), None)
            if self.keyboard:
                self._finger_dots(h)
            else:
                h.dots[TIPS["index"]] = self._shown(t, h, self._to_screen(h.points[TIPS["index"]]))
            first = next(iter(h.dots.values()))
            h.moved = 0.0 if before is None else float(np.linalg.norm(first - before))
            h.trail.append((t, {k: v.copy() for k, v in h.dots.items()}))
        if not self.keyboard:
            self._drive_pointer(hands, out)
        self.calm = not self.keyboard and all(
            h.speed < CALM and not h.pinched and h.frozen is None for h in self.hands.values())
        out.append(("dots", self._dots()))
        return out

    def _adopt(self, hid: int, joints: np.ndarray, seen: Mapping) -> _Hand | None:
        """A hand the tracker lost for a picture and found again comes back with a new number: it's the one
        that was just there, if one was."""
        j = np.asarray(joints, dtype=np.float64)
        where = j[list(PALM), :2].mean(axis=0)
        reach = 1.5 * (self.unit or float(np.linalg.norm(j[9, :2] - j[0, :2])) or 1.0)
        for old in list(self.hands.values()):
            if old.id not in seen and old.palm_px is not None and float(np.linalg.norm(old.palm_px - where)) < reach:
                del self.hands[old.id]
                if self.active == old.id:
                    self.active = hid
                old.id = hid
                return old
        return None

    def _drop(self, h: _Hand, out: list[tuple]) -> None:
        if h.pinched:
            self._end_pinch(0.0, h, out, tap=False)
        if self.active == h.id:
            self.active = None

    def _follow_unit(self, size: float, hands: Mapping) -> None:
        """The hand size the working space is measured in, followed slowly (it changes with distance, and a
        little with the hand's shape), with everything kept in hand sizes put in the new unit and the space
        resized around the hands, so that a change of size isn't a movement."""
        if not self.unit:
            self.unit = size
            return
        new = 0.95 * self.unit + 0.05 * size
        k = self.unit / new
        for h in self.hands.values():
            for f in h.filters.values():
                f.rescale(k)
            for name in ("palm", "pinch_from", "hold_from", "open_up_from"):
                v = getattr(h, name)
                if v is not None:
                    setattr(h, name, v * k)
        if self.centre is not None:
            here = [self.hands[hid].palm_px for hid in hands if self.hands[hid].palm_px is not None]
            if here:
                a = np.mean(here, axis=0)
                self.centre = a - (a - self.centre) / k
        self.unit = new

    # ---------------------------------------------------------------- the working space
    def _space(self) -> tuple[tuple[float, float], tuple[float, float, float, float]]:
        """Half the working space's width and height (camera pixels), and the part of the screen it covers."""
        if self.keyboard:
            x, y, w, h, cols, rows = self.keyboard
            return (cols * self.key_pitch / 2 * self.unit, rows * ROW_PITCH / 2 * self.unit), (x, y, w, h)
        hw = SPAN / 2 * self.unit
        return (hw, hw * self.screen_h / self.screen_w), (0.0, 0.0, float(self.screen_w), float(self.screen_h))

    def _anchors(self, hands: Mapping) -> list[np.ndarray]:
        """The points that keep the working space under the hands: each index fingertip, or with the keyboard up
        each hand's knuckles."""
        return [p for hid in hands for p in self.hands[hid].points.values()]

    def _finger_dots(self, h: _Hand) -> None:
        """A hand's four dots on the keyboard: a key apart, along the row its knuckles are over, in the order its
        fingers are (index nearest the other hand)."""
        x, _y, w, _h, cols, _rows = self.keyboard
        a = self._to_screen(h.points[HAND])
        step = w / cols * (1.0 if h.order >= 0 else -1.0)    # the camera faces you: its right is your left
        for name, k in FINGER_KEYS.items():
            h.dots[TIPS[name]] = np.array([min(max(a[0] + k * step, 0.0), self.screen_w - 1.0), a[1]])

    def _place(self, hands: Mapping) -> None:
        """Put the working space where the hands are: the first index finger's dot where the pointer is, or the
        hands where they'd rest on a keyboard (one hand alone on its own side, if it's known which it is)."""
        anchors = self._anchors(hands)
        if not anchors:
            return
        if self.keyboard and len(anchors) == 2:
            # Both hands: the keys as far apart as these hands are, so each index finger arrives on its home key.
            apart = abs(float(anchors[0][0] - anchors[1][0])) / self.unit / (HOME[1] - HOME[0])
            self.key_pitch = min(max(apart, KEY_PITCH_RANGE[0]), KEY_PITCH_RANGE[1])
        (hw, hh), (x, y, w, h) = self._space()
        if self.keyboard:
            col = sum(HOME) / 2
            one = [self.hands[hid] for hid in hands]
            if len(one) == 1 and one[0].right is not None and abs(one[0].right - 0.5) >= 0.2:
                col = HOME[1] if one[0].right > 0.5 else HOME[0]
            a, (tx, ty) = np.mean(anchors, axis=0), (x + w * col / self.keyboard[4], y + h * HOME_ROW)
        else:
            a, (tx, ty) = anchors[0], self.cursor
        u, v = 0.5 - (tx - x) / w, (ty - y) / h - 0.5
        self.centre = np.array([a[0] - u * 2 * hw, a[1] - v * 2 * hh])

    def _push(self, hands: Mapping) -> None:
        """A hand past the working space's edge takes the space with it (unless hands are past both edges). With
        the keyboard up the edge is in from the keyboard's, so that all four of a hand's dots stay on keys."""
        if self.centre is None:
            return
        half = self._space()[0]
        if self.keyboard:
            cols, rows = self.keyboard[4], self.keyboard[5]
            reach = max(abs(k) for k in FINGER_KEYS.values()) + 0.5         # keys from a hand's middle to its edge
            half = (half[0] * (1 - 2 * reach / cols), half[1] * (1 - 1.0 / rows))
        anchors = self._anchors(hands)
        for axis in (0, 1):
            need = []
            for a in anchors:
                d = a[axis] - self.centre[axis]
                if abs(d) > half[axis]:
                    need.append(d - math.copysign(half[axis], d))
            if need and (all(n > 0 for n in need) or all(n < 0 for n in need)):
                self.centre[axis] += max(need, key=abs)

    def _to_screen(self, p: np.ndarray) -> np.ndarray:
        (hw, hh), (x, y, w, h) = self._space()
        u = (p[0] - self.centre[0]) / (2 * hw)
        v = (p[1] - self.centre[1]) / (2 * hh)
        sx = x + (0.5 - u) * w                  # the camera faces you: your right is its left
        sy = y + (0.5 + v) * h
        return np.array([min(max(sx, 0.0), self.screen_w - 1.0), min(max(sy, 0.0), self.screen_h - 1.0)])

    def _shown(self, t: float, h: _Hand, live: np.ndarray) -> np.ndarray:
        """Where the index dot shows: held still while a pinch forms (the finger moves to meet the thumb), then
        gliding back to the finger."""
        if h.frozen is not None:
            return h.frozen.copy()
        if h.glide_from is not None:
            k = (t - h.glide_t0) / GLIDE_S
            if k < 1.0:
                return h.glide_from + (live - h.glide_from) * k
            h.glide_from = None
        return live

    def _drive_pointer(self, hands: Mapping, out: list[tuple]) -> None:
        """The display's pointer goes with one hand's dot: the one pinching, or the one that last moved."""
        live = [self.hands[hid] for hid in hands]
        busy = [h for h in live if h.pinched or h.frozen is not None]
        if busy:
            self.active = busy[0].id
            return                                   # held where the pinch is
        mover = max(live, key=lambda h: h.moved, default=None)
        if mover is not None and (mover.moved > ACTIVE_MOVE or self.active not in self.hands):
            self.active = mover.id
        h = self.hands.get(self.active) if self.active is not None else None
        if h is None or TIPS["index"] not in h.dots:
            return
        d = h.dots[TIPS["index"]]
        if abs(d[0] - self.cursor[0]) >= 1.0 or abs(d[1] - self.cursor[1]) >= 1.0:
            self.cursor = [float(d[0]), float(d[1])]
            out.append(("move", self.cursor[0], self.cursor[1]))

    def _dots(self) -> list[tuple[float, float, str]]:
        """A dot per hand, on its index finger (with the keyboard up, the index finger's key: the other fingers'
        keys are the next ones out, not drawn)."""
        dots = []
        for h in self.hands.values():
            d = h.dots.get(TIPS["index"])
            if d is None:
                continue
            if self.keyboard:
                kind = "press" if h.pressed == "index" else "key"
            else:
                kind = "scroll" if h.scrolling else "pinch" if h.pinched else "point"
            dots.append((float(d[0]), float(d[1]), kind))
        return dots

    # ---------------------------------------------------------------- pinching
    def _pinch(self, t: float, h: _Hand, pose: Pose, palm: np.ndarray, delta: np.ndarray, out: list[tuple]) -> None:
        # A pinch needs the index finger reaching for the thumb: in a fist the thumb can rest on it too.
        forming = pose.pinch < PINCH_OFF and pose.reach > PINCH_REACH
        closed = (pose.pinch < PINCH_ON and pose.reach > PINCH_REACH) or (h.pinched and pose.pinch < PINCH_OFF)
        if (forming or h.pinched) and h.frozen is None and TIPS["index"] in h.dots:
            h.frozen, h.glide_from = h.dots[TIPS["index"]].copy(), None
        h.votes = h.votes + 1 if closed != h.pinched else 0
        if closed != h.pinched and h.votes >= 2:      # two pictures in a row, so one bad picture doesn't click
            h.votes = 0
            if closed:
                h.pinched, h.pinch_t0, h.pinch_from = True, t, palm.copy()
                h.scrolling, h.scroll_acc = False, [0.0, 0.0]
            else:
                self._end_pinch(t, h, out, tap=True)
        elif h.pinched and closed:
            if h.scrolling:
                self._scroll(h, delta, out)
            elif h.pinch_from is not None and float(np.linalg.norm(palm - h.pinch_from)) > DRAG_START:
                h.scrolling = True
                if h.frozen is not None:              # the wheel turns where the dot is
                    self.cursor = [float(h.frozen[0]), float(h.frozen[1])]
                    out.append(("move", self.cursor[0], self.cursor[1]))
        if not forming and not h.pinched and h.frozen is not None:
            h.glide_from, h.glide_t0, h.frozen = h.frozen, t, None

    def _end_pinch(self, t: float, h: _Hand, out: list[tuple], tap: bool) -> None:
        if tap and not h.scrolling and t - h.pinch_t0 <= TAP_MAX_S:
            at = h.frozen if h.frozen is not None else h.dots.get(TIPS["index"])
            if at is not None:
                self.cursor = [float(at[0]), float(at[1])]
                out.append(("click", self.cursor[0], self.cursor[1]))
        h.pinched, h.scrolling, h.pinch_from = False, False, None

    def _scroll(self, h: _Hand, delta: np.ndarray, out: list[tuple]) -> None:
        # Like dragging the page: the hand moves up, the page goes up (scrolls down).
        h.scroll_acc[0] += delta[0]
        h.scroll_acc[1] -= delta[1]
        steps = [int(a / SCROLL_STEP) for a in h.scroll_acc]
        if steps[0] or steps[1]:
            h.scroll_acc = [a - s * SCROLL_STEP for a, s in zip(h.scroll_acc, steps)]
            out.append(("scroll", steps[0], steps[1]))

    # ---------------------------------------------------------------- held gestures
    def _holds(self, t: float, h: _Hand, pose: Pose, palm: np.ndarray, out: list[tuple]) -> None:
        name = ("palm" if pose.open and pose.facing else "yes" if pose.thumb_up and pose.thumb_only
                else "no" if pose.thumb_down and pose.thumb_only else "")
        if name != h.hold:
            h.hold, h.hold_t0, h.hold_from = name, t, palm.copy()
            if name != h.fired:
                h.fired = ""
            return
        if not name or h.fired == name or h.hold_from is None:
            return
        if float(np.linalg.norm(palm - h.hold_from)) > HOLD_STILL:
            h.hold_t0, h.hold_from = t, palm.copy()          # moving: not held yet
            return
        if t - h.hold_t0 >= HOLD_S:
            h.fired = name
            out.append(("gesture", name))

    # ---------------------------------------------------------------- beckoning
    def _beckon(self, t: float, h: _Hand, pose: Pose, palm: np.ndarray, out: list[tuple]) -> None:
        """Fingers open, then all of them curled in, quickly and once: with the palm up (beckoning up) the
        keyboard comes up, with the palm down (beckoning down) it goes away."""
        if pose.normal is None:
            return                                     # not knowing which hand, "up" can't be told from "down"
        curls = list(pose.curl.values())
        side = "up" if pose.palm_up else "down" if pose.palm_down else ""
        if side and all(c < OPEN_DEG for c in curls):
            if side != h.open_side:
                h.armed = True                         # turned over: a beckon the other way can follow at once
            h.open_side, h.open_up_at, h.open_up_from = side, t, palm.copy()
            if not h.armed and t - h.beckoned_at > BECKON_AGAIN_S:
                h.armed = True
            return
        still_side = pose.normal[1] < UP / 2 if h.open_side == "up" else pose.normal[1] > -UP / 2
        if (h.armed and h.open_side and h.open_up_from is not None and t - h.open_up_at <= BECKON_S
                and sum(c > CURLED_DEG for c in curls) >= 3 and still_side
                and float(np.linalg.norm(palm - h.open_up_from)) < BECKON_STILL):
            h.armed, h.beckoned_at = False, t
            out.append(("gesture", "keyboard " + h.open_side))

    # ---------------------------------------------------------------- typing
    def _type(self, t: float, h: _Hand, pose: Pose, out: list[tuple]) -> None:
        """A finger bending down quickly, more than the others, is a key press: the key under its dot just
        before it moved."""
        bend = pose.flex
        if not h.rest:
            h.rest = dict(bend)
            return
        rise = {f: bend[f] - h.rest[f] for f in bend}
        if h.pressed:
            if rise[h.pressed] < TAP_OFF:
                h.pressed = ""
            elif t - h.press_t0 > TAP_HOLD_S:
                h.rest[h.pressed], h.pressed = bend[h.pressed], ""
        elif h.speed < TAP_STILL:
            order = sorted(rise, key=lambda f: rise[f], reverse=True)
            best, second = order[0], order[1]
            if rise[best] > TAP_ON and rise[best] - rise[second] > TAP_LEAD:
                h.pressed, h.press_t0 = best, t
                at = self._before(h, TIPS[best], t - TAP_HOVER_S)
                if at is not None:
                    out.append(("key", float(at[0]), float(at[1])))
        for f in bend:
            if f != h.pressed:     # where a finger rests follows it slowly up, and at once back down
                h.rest[f] += (REST_FOLLOW if bend[f] > h.rest[f] else 0.5) * (bend[f] - h.rest[f])

    @staticmethod
    def _before(h: _Hand, tip: int, when: float) -> np.ndarray | None:
        """Where a dot was at `when` (the latest picture then), or the earliest known."""
        found = None
        for t, dots in h.trail:
            if tip not in dots:
                continue
            if found is None or t <= when:
                found = dots[tip]
            if t > when:
                break
        return found
