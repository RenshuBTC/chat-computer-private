"""Microphone and speaker streams (PipeWire on a PC, Core Audio on a Mac, through PortAudio), plus stand-ins
for tests."""
from __future__ import annotations

import collections
import ctypes
import logging
import sys
import threading
import time
from typing import Callable

import numpy as np

log = logging.getLogger("claudeos.audio")
# Held while any stream is opened or closed, and while PortAudio restarts (it isn't safe to use from several
# threads at once, and a restart closes every stream it has open).
PORTAUDIO_LOCK = threading.RLock()
MIC_RATE = 16000
MIC_BLOCK = 512
OUT_RATE = 24000


class Mic:
    """The default input (AirPods mic when connected). Calls on_frame(np.float32[512]) from
    PortAudio's thread."""

    def __init__(self, on_frame: Callable[[np.ndarray], None]) -> None:
        self.on_frame = on_frame
        self.stream = None
        self.lock = threading.Lock()
        self.error: str | None = None

    @property
    def is_open(self) -> bool:
        return self.stream is not None

    def open(self) -> bool:
        with PORTAUDIO_LOCK, self.lock:
            if self.stream is not None:
                return True
            try:
                import sounddevice as sd

                def cb(indata, frames, t, status):  # noqa: ANN001
                    self.on_frame(indata[:, 0].copy())

                self.stream = sd.InputStream(samplerate=MIC_RATE, channels=1, dtype="float32", blocksize=MIC_BLOCK,
                                             callback=cb, latency="low")
                self.stream.start()
                self.error = None
                return True
            except Exception as exc:
                self.stream = None
                self.error = str(exc)
                log.warning("microphone unavailable: %s", exc)
                return False

    def close(self) -> None:
        with PORTAUDIO_LOCK:
            with self.lock:
                s, self.stream = self.stream, None
            if s is not None:
                try:
                    s.stop()
                    s.close()
                except Exception:
                    pass


class Player:
    """Plays queued 24 kHz mono audio on the default output. stop() is instant."""

    def __init__(self) -> None:
        self.queue: collections.deque[np.ndarray] = collections.deque()
        self.cur = np.zeros(0, dtype=np.float32)
        self.pos = 0
        self.lock = threading.Lock()
        self.stream_lock = threading.Lock()
        self.stream = None
        self.last_sound = 0.0
        self.gain = 1.0

    def _ensure(self) -> None:
        with PORTAUDIO_LOCK, self.stream_lock:
            if self.stream is not None:
                return
            import sounddevice as sd
            self.stream = sd.OutputStream(samplerate=OUT_RATE, channels=1, dtype="float32", blocksize=480,
                                          callback=self._cb, latency="low")
            self.stream.start()

    def reset(self) -> None:
        """Closes the output stream; the next sound opens one on whatever the default output is then."""
        with PORTAUDIO_LOCK:
            with self.stream_lock:
                s, self.stream = self.stream, None
            if s is not None:
                try:
                    s.stop()
                    s.close()
                except Exception:
                    pass

    def _cb(self, outdata, frames, t, status):  # noqa: ANN001
        out = outdata[:, 0]
        n = 0
        with self.lock:
            while n < frames:
                if self.pos >= self.cur.size:
                    if not self.queue:
                        break
                    self.cur, self.pos = self.queue.popleft(), 0
                take = min(frames - n, self.cur.size - self.pos)
                out[n:n + take] = self.cur[self.pos:self.pos + take] * self.gain
                self.pos += take
                n += take
            if n:
                self.last_sound = time.monotonic()
        if n < frames:
            out[n:] = 0

    def play(self, audio: np.ndarray) -> None:
        if audio.size == 0:
            return
        with self.lock:
            self.queue.append(audio.astype(np.float32, copy=False))
        try:
            self._ensure()
        except Exception as exc:
            log.warning("speaker unavailable: %s", exc)
            with self.lock:
                self.queue.clear()

    def stop(self) -> None:
        with self.lock:
            self.queue.clear()
            self.cur = np.zeros(0, dtype=np.float32)
            self.pos = 0

    @property
    def busy(self) -> bool:
        with self.lock:
            return bool(self.queue) or self.pos < self.cur.size

    def close_if_idle(self, idle_s: float = 4.0) -> None:
        if self.stream is not None and not self.busy and time.monotonic() - self.last_sound > idle_s:
            self.reset()


def default_devices() -> tuple[int, int] | None:
    """On a Mac: Core Audio's default input and output device ids, to notice AirPods connecting or leaving.
    PortAudio reads the Mac's devices only when it starts, so ClaudeOS restarts it when these change.
    None elsewhere, or if Core Audio can't be asked."""
    if sys.platform != "darwin":
        return None
    try:
        ca = ctypes.CDLL("/System/Library/Frameworks/CoreAudio.framework/CoreAudio")
    except OSError:
        return None

    class Address(ctypes.Structure):          # AudioObjectPropertyAddress
        _fields_ = [("selector", ctypes.c_uint32), ("scope", ctypes.c_uint32), ("element", ctypes.c_uint32)]

    def code(s: str) -> int:
        return int.from_bytes(s.encode("ascii"), "big")
    ids = []
    for selector in ("dIn ", "dOut"):        # kAudioHardwarePropertyDefaultInputDevice / ...OutputDevice
        addr = Address(code(selector), code("glob"), 0)
        dev, size = ctypes.c_uint32(0), ctypes.c_uint32(4)
        status = ca.AudioObjectGetPropertyData(ctypes.c_uint32(1), ctypes.byref(addr), ctypes.c_uint32(0), None,
                                               ctypes.byref(size), ctypes.byref(dev))   # 1 = the system object
        if status != 0:
            return None
        ids.append(dev.value)
    return ids[0], ids[1]


def restart_portaudio() -> None:
    """Makes PortAudio read the list of microphones and speakers again (all its streams must be closed)."""
    import sounddevice as sd
    with PORTAUDIO_LOCK:
        sd._terminate()
        sd._initialize()


class FakeMic:
    """Feeds prepared audio as if spoken into the mic (optionally in real time)."""

    def __init__(self, on_frame: Callable[[np.ndarray], None]) -> None:
        self.on_frame = on_frame
        self.is_open = False
        self.error = None

    def open(self) -> bool:
        self.is_open = True
        return True

    def close(self) -> None:
        self.is_open = False

    def feed(self, audio: np.ndarray, realtime: bool = False, speed: float = 1.0) -> None:
        for i in range(0, audio.size - MIC_BLOCK + 1, MIC_BLOCK):
            if not self.is_open:
                return
            self.on_frame(audio[i:i + MIC_BLOCK].astype(np.float32))
            if realtime:
                time.sleep(MIC_BLOCK / MIC_RATE / speed)


class FakePlayer:
    """Records what would be played; 'plays' in simulated real time."""

    def __init__(self) -> None:
        self.played: list[np.ndarray] = []
        self.until = 0.0
        self.stops = 0

    def play(self, audio: np.ndarray) -> None:
        self.played.append(audio)
        now = time.monotonic()
        self.until = max(self.until, now) + audio.size / OUT_RATE

    def stop(self) -> None:
        self.stops += 1
        self.until = 0.0

    @property
    def busy(self) -> bool:
        return time.monotonic() < self.until

    def close_if_idle(self, idle_s: float = 4.0) -> None:
        pass


def silence(seconds: float, rate: int = MIC_RATE) -> np.ndarray:
    return np.zeros(int(seconds * rate), dtype=np.float32)
