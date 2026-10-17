"""AirTouch's own process: camera, hand tracking, gestures and the pointer.

It runs apart from ClaudeOS (the networks run in numpy, which would otherwise hold up ClaudeOS's voice and
screen) and at lower priority. It moves the pointer and clicks itself (XTEST), and tells ClaudeOS over a pipe
what the screen should draw and what the hands did:
  ("status", {...})              state: "starting", "no camera", "running", "error"; camera; why
  ("dots", [(x, y, kind), ...])  the dots to draw (display pixels); kind as gestures.py says
  ("key", x, y)                  a finger typed the keyboard's key at x, y
  ("gesture", name)              "palm", "yes", "no", "keyboard up", "keyboard down"
  ("hand", present)
ClaudeOS sends ("bounds", (x, y, w, h)) when its screen moves or changes size, ("keyboard", (x, y, w, h,
columns, rows)) when its keyboard comes up (None when it goes), and ("stop",).

What it looks at, and how often (the networks cost most of a processor core at full speed):
- No hand: a 80x60 greyscale copy of each picture, compared with the last one; palm detection runs when
  something moved (at most 4 times a second), and every 2 seconds anyway (a hand held still).
- Hands: every picture, as fast as half a processor core allows (up to the camera's rate; 0.8 of a core with
  the keyboard up, for typing), and 8 a second while the hands rest; with one hand, palm detection looks for
  the other twice a second.
The pictures are never kept or sent anywhere: each is dropped once looked at.
"""
from __future__ import annotations

import logging
import os
import time
from typing import Any

import numpy as np
from PIL import Image

log = logging.getLogger("claudeos.airtouch")

BUDGET = 0.5                 # share of one processor core for following hands
TYPING_BUDGET = 0.8          # ...with the keyboard up
MAX_FPS = 30.0
REST_FPS = 8.0
SEARCH_EVERY = 0.25          # palm detection at most this often when something moves...
SEARCH_ANYWAY = 2.0          # ...and this often regardless
SECOND_HAND_EVERY = 0.5      # with one hand followed, looking for the other
MOTION = 0.01                # share of the small picture that changed, to count as movement


def thumbnail(cam: Any, data: bytes) -> np.ndarray:
    """An 80x60-ish greyscale picture, cheaply: straight from YUYV bytes, or decoded at an eighth of its size."""
    from . import camera
    if cam.fmt == camera.YUYV:
        return camera.yuyv_luma(data, cam.width, cam.height, step=max(1, cam.width // 80), stride=cam.stride)
    import io
    img = Image.open(io.BytesIO(camera.with_huffman_tables(data)))
    img.draft("L", (max(1, img.width // 8), max(1, img.height // 8)))
    return np.asarray(img.convert("L").resize((80, 60)), dtype=np.float32)


def moved(a: np.ndarray | None, b: np.ndarray) -> bool:
    if a is None or a.shape != b.shape:
        return True
    return float(np.mean(np.abs(a - b) > 20)) > MOTION


class Pacer:
    """Which pictures to look at, and which to search for hands in: see the module's notes."""

    def __init__(self) -> None:
        self.last_search = -1e9
        self.last_frame = -1e9
        self.cost = 0.02                 # seconds per picture, measured
        self.budget = BUDGET
        self.rest_since: float | None = None
        self.prev_thumb: np.ndarray | None = None

    def want(self, t: float, tracking: bool, thumb: np.ndarray | None) -> bool:
        if tracking:
            fps = REST_FPS if self.resting(t) else min(MAX_FPS, self.budget / max(self.cost, 1e-3))
            return t - self.last_frame >= 1.0 / fps - 0.004
        if t - self.last_search >= SEARCH_ANYWAY:
            return True
        return t - self.last_search >= SEARCH_EVERY and thumb is not None and moved(self.prev_thumb, thumb)

    def search(self, t: float, hands: int, most: int) -> bool:
        """Whether this picture should look for new hands too (one that want() chose)."""
        return hands == 0 or (hands < most and t - self.last_search >= SECOND_HAND_EVERY)

    def resting(self, t: float) -> bool:
        return self.rest_since is not None and t - self.rest_since >= 1.5

    def done(self, t: float, cost: float, tracking: bool, searched: bool, calm: bool, thumb: np.ndarray | None) -> None:
        self.last_frame = t
        if searched:
            self.last_search = t
        if not tracking:
            self.prev_thumb = thumb
        self.cost = 0.8 * self.cost + 0.2 * cost
        if not calm:
            self.rest_since = None
        elif self.rest_since is None:
            self.rest_since = t


def main(conn: Any, options: dict[str, Any]) -> None:
    """The process's whole life."""
    try:
        os.nice(5)
    except OSError:
        pass
    from . import camera
    from .gestures import Engine
    from .hands import HandTracker
    from .xinput import SharedPointer, XInput

    def send(*msg: Any) -> None:
        try:
            conn.send(msg)
        except (OSError, EOFError):
            raise SystemExit(0)

    send("status", {"state": "starting"})
    with open(options["palm_model"], "rb") as f:
        palm = f.read()
    with open(options["landmark_model"], "rb") as f:
        landmarks = f.read()
    tracker = HandTracker(palm, landmarks)
    try:
        x = XInput()
    except OSError as exc:
        log.warning("AirTouch can't move the pointer: %s", exc)
        x = None
    pointer = SharedPointer(x) if x else None
    bounds = options.get("bounds") or (0, 0) + (x.screen_size() if x else (1920, 1080))
    engine = Engine(int(bounds[2]), int(bounds[3]))
    if x:
        px, py = x.position()
        engine.cursor = [min(max(px - bounds[0], 0), bounds[2] - 1), min(max(py - bounds[1], 0), bounds[3] - 1)]
    stopping = False
    last_dots: list | None = None

    def commands() -> None:
        nonlocal bounds, stopping
        while conn.poll(0):
            try:
                msg = conn.recv()
            except (OSError, EOFError):
                stopping = True
                return
            if msg[0] == "stop":
                stopping = True
                return
            if msg[0] == "bounds":
                bounds = msg[1]
                engine.resize(int(bounds[2]), int(bounds[3]))
            elif msg[0] == "keyboard":
                spec = msg[1]
                engine.set_keyboard((spec[0] - bounds[0], spec[1] - bounds[1]) + tuple(spec[2:]) if spec else None)

    def dispatch(actions: list[tuple], t: float) -> None:
        nonlocal last_dots
        bx, by = bounds[0], bounds[1]
        for a in actions:
            kind = a[0]
            if kind == "move" and pointer:
                pointer.move(t, bx + a[1], by + a[2])
            elif kind == "click" and pointer:
                pointer.click(t, bx + a[1], by + a[2])
            elif kind == "scroll" and pointer:
                pointer.scroll(t, a[1], a[2])
            elif kind == "dots":
                dots = [(round(bx + d[0], 1), round(by + d[1], 1), d[2]) for d in a[1]]
                if dots != last_dots:                   # a hand at rest needn't be drawn again
                    last_dots = dots
                    send("dots", dots)
            elif kind == "key":
                send("key", bx + a[1], by + a[2])
            elif kind in ("gesture", "hand"):
                send(*a)

    hint = (options.get("camera") or "").lower()
    while True:
        commands()
        if stopping:
            break
        devs = camera.devices()
        if hint:
            devs = [d for d in devs if hint in d.name.lower() or hint == d.path] or devs
        if not devs:
            send("status", {"state": "no camera"})
            for _ in range(50):                         # look again in 5 s (a display plugged in later)
                time.sleep(0.1)
                commands()
                if stopping:
                    break
            continue
        try:
            cam = camera.Camera(devs[0])
        except OSError as exc:
            send("status", {"state": "error", "camera": devs[0].name, "error": f"couldn't open it: {exc}"})
            time.sleep(5)
            commands()
            continue
        send("status", {"state": "running", "camera": devs[0].name, "size": [cam.width, cam.height],
                        "format": "YUYV" if cam.fmt == camera.YUYV else "MJPEG"})
        pacer = Pacer()
        try:
            for data in cam.frames():
                commands()
                if stopping:
                    break
                t = time.monotonic()
                if pointer:
                    pointer.check(t)                    # a mouse moving the pointer gets it back
                tracking = bool(tracker.tracks)
                thumb = None if tracking else thumbnail(cam, data)
                pacer.budget = TYPING_BUDGET if engine.keyboard else BUDGET
                if not pacer.want(t, tracking, thumb):
                    if not tracking:
                        dispatch(engine.update(t, {}), t)     # the time passes: a hand that left is let go
                    continue
                start = time.perf_counter()
                search = pacer.search(t, len(tracker.tracks), tracker.max_hands)
                found = tracker.track(cam.decode(data), search=search)
                dispatch(engine.update(t, {hid: (h.joints, h.right) for hid, h in found}), t)
                pacer.done(t, time.perf_counter() - start, bool(found), search, engine.calm, thumb)
        except Exception as exc:              # report, and start the camera again
            log.exception("AirTouch stopped")
            send("status", {"state": "error", "camera": devs[0].name, "error": str(exc)[:300]})
            time.sleep(2)
        finally:
            cam.close()
            if pointer:
                pointer.release()
        if not stopping:
            send("status", {"state": "no camera", "camera": devs[0].name})       # unplugged: look again
    if x:
        x.close()
