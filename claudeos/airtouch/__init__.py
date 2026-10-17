"""AirTouch: using ClaudeOS with your hand in the air, through the screen's camera (the Apple Studio Display's,
a laptop's, or any webcam).

All of it is ClaudeOS's own code: the camera (camera.py, Video4Linux2 directly), the engine that runs the
networks (tflite.py reads the model files, nn.py runs them in numpy), hand tracking (hands.py, MediaPipe's
published method, up to two hands), gestures (gestures.py: a dot per index finger, pinches, stop, yes/no, the
beckons that bring the keyboard up and take it away, and typing on it), the keyboard (keyboard.py: the iPad's
layout, measured; the screen draws it) and the pointer (xinput.py, XTEST). The two networks that find a palm and a hand's joints are Google's,
openly licensed (Apache 2.0), downloaded with the voice models.

It runs in a process of its own (worker.py); this side starts and stops it and hands its news to ClaudeOS.
"""
from __future__ import annotations

import logging
import threading
from typing import Any, Callable

log = logging.getLogger("claudeos.airtouch")


class AirTouch:
    def __init__(self, palm_model: str, landmark_model: str, on_event: Callable[[tuple], None],
                 camera: str = "", bounds: tuple[int, int, int, int] | None = None) -> None:
        self.options = {"palm_model": palm_model, "landmark_model": landmark_model, "camera": camera, "bounds": bounds}
        self.on_event = on_event
        self.keyboard: tuple | None = None
        self.status: dict[str, Any] = {"state": "off"}
        self.proc: Any = None
        self.conn: Any = None
        self._reader: threading.Thread | None = None

    @property
    def running(self) -> bool:
        return self.proc is not None and self.proc.is_alive()

    def start(self) -> None:
        if self.running:
            return
        import multiprocessing
        from . import worker
        ctx = multiprocessing.get_context("spawn")        # never fork a process with threads (GTK, the voice)
        mine, theirs = ctx.Pipe()
        self.proc = ctx.Process(target=worker.main, args=(theirs, dict(self.options)), name="claudeos-airtouch",
                                daemon=True)
        self.proc.start()
        theirs.close()
        self.conn = mine
        self.status = {"state": "starting"}
        self._reader = threading.Thread(target=self._read, args=(mine,), name="airtouch-events", daemon=True)
        self._reader.start()
        if self.keyboard:
            self._send(("keyboard", self.keyboard))

    def _read(self, conn: Any) -> None:
        while True:
            try:
                msg = conn.recv()
            except (EOFError, OSError):
                break
            if msg and msg[0] == "status":
                self.status = dict(msg[1])
            try:
                self.on_event(tuple(msg))
            except Exception:
                log.exception("AirTouch event")
        if self.status.get("state") not in ("off",):
            self.status = {"state": "stopped"}
            try:
                self.on_event(("status", dict(self.status)))
            except Exception:
                log.exception("AirTouch event")

    def set_bounds(self, bounds: tuple[int, int, int, int]) -> None:
        self.options["bounds"] = bounds
        self._send(("bounds", bounds))

    def set_keyboard(self, spec: tuple | None) -> None:
        """The on-screen keyboard is up (where its keys are, display pixels, and how many across and down), or
        not: the fingers type on it instead of pointing."""
        self.keyboard = tuple(spec) if spec else None
        self._send(("keyboard", self.keyboard))

    def _send(self, msg: tuple) -> None:
        if self.conn is not None:
            try:
                self.conn.send(msg)
            except (OSError, ValueError):
                pass

    def stop(self) -> None:
        self.status = {"state": "off"}
        proc, self.proc = self.proc, None
        self._send(("stop",))
        if proc is not None:
            proc.join(3)
            if proc.is_alive():
                proc.kill()
                proc.join(3)
        if self.conn is not None:
            try:
                self.conn.close()
            except OSError:
                pass
            self.conn = None
