"""Listening: turns the microphone stream into utterances.

Speech detection runs on every 32 ms frame. While you talk, ClaudeOS shows your words live;
when you pause, the utterance is transcribed and handed on. Talking while ClaudeOS speaks
interrupts it ("barge-in") when you wear headphones; with loudspeakers ClaudeOS ignores the mic
while it talks so it doesn't hear itself.
"""
from __future__ import annotations

import asyncio
import collections
import logging
import time
from concurrent.futures import Executor
from dataclasses import dataclass
from typing import Callable

import numpy as np

from .vad import FRAME, RATE

log = logging.getLogger("claudeos.listener")


@dataclass
class ListenerEvents:
    on_speech_start: Callable[[], None] = lambda: None
    on_partial: Callable[[str], None] = lambda text: None
    on_utterance: Callable[[str, float], None] = lambda text, seconds: None
    on_barge_in: Callable[[], None] = lambda: None
    on_discard: Callable[[], None] = lambda: None


class Listener:
    START_PROB = 0.5
    START_FRAMES = 3            # ~96 ms of speech to start
    BARGE_PROB = 0.7
    BARGE_FRAMES = 9            # ~290 ms of clear speech to talk over ClaudeOS
    END_PROB = 0.35
    PREROLL_FRAMES = 10         # keep ~320 ms before the start
    MAX_SECONDS = 30.0
    PARTIAL_EVERY = 0.9

    def __init__(self, vad, stt, executor: Executor, events: ListenerEvents,
                 is_speaking: Callable[[], bool] = lambda: False, barge_in: Callable[[], bool] = lambda: True) -> None:
        self.vad = vad
        self.stt = stt
        self.executor = executor
        self.ev = events
        self.is_speaking = is_speaking
        self.barge_in = barge_in
        self.mode = "normal"            # "confirm" ends utterances sooner
        self.enabled = True
        self.loop: asyncio.AbstractEventLoop | None = None
        self.pre: collections.deque[np.ndarray] = collections.deque(maxlen=self.PREROLL_FRAMES)
        self.buf: list[np.ndarray] = []
        self.in_speech = False
        self.hot = 0
        self.quiet = 0
        self.started_at = 0.0
        self.last_partial = 0.0
        self.partial_busy = False
        self.utt_id = 0
        self.speaking_until = 0.0
        self.leftover = np.zeros(0, dtype=np.float32)
        self._tasks: set[asyncio.Task] = set()

    # ------------------------------------------------------------------ input
    def attach(self, loop: asyncio.AbstractEventLoop) -> None:
        self.loop = loop

    def _keep(self, task: asyncio.Task) -> None:
        """Holds a transcription task until it's done (the event loop keeps only a weak reference)."""
        self._tasks.add(task)
        task.add_done_callback(self._tasks.discard)

    def push_threadsafe(self, frame: np.ndarray) -> None:
        """Called from the audio thread."""
        if self.loop is not None:
            self.loop.call_soon_threadsafe(self.push, frame)

    def push(self, samples: np.ndarray) -> None:
        if not self.enabled:
            return
        data = np.concatenate([self.leftover, samples]) if self.leftover.size else samples
        n = (data.size // FRAME) * FRAME
        self.leftover = data[n:].copy()
        for i in range(0, n, FRAME):
            self._frame(data[i:i + FRAME])

    @property
    def silence_frames(self) -> int:
        ms = 450 if self.mode == "confirm" else 700
        return int(ms / 1000 * RATE / FRAME)

    def _frame(self, f: np.ndarray) -> None:
        speaking = self.is_speaking()
        now = time.monotonic()
        if speaking:
            self.speaking_until = now + 0.35
        tts_active = speaking or now < self.speaking_until
        if tts_active and not self.barge_in():
            # Loudspeakers: don't listen to ourselves.
            if self.in_speech:
                self._reset()
            self.vad.reset()
            return
        p = self.vad(f)
        if not self.in_speech:
            self.pre.append(f)
            need_p, need_n = (self.BARGE_PROB, self.BARGE_FRAMES) if tts_active else (self.START_PROB, self.START_FRAMES)
            self.hot = self.hot + 1 if p >= need_p else 0
            if self.hot >= need_n:
                self.in_speech = True
                self.utt_id += 1
                self.buf = list(self.pre)
                self.pre.clear()
                self.quiet = 0
                self.started_at = now
                self.last_partial = now
                if tts_active:
                    self.ev.on_barge_in()
                self.ev.on_speech_start()
            return
        self.buf.append(f)
        self.quiet = self.quiet + 1 if p < self.END_PROB else 0
        dur = len(self.buf) * FRAME / RATE
        if self.quiet >= self.silence_frames or dur >= self.MAX_SECONDS:
            audio = np.concatenate(self.buf)
            if self.quiet:
                audio = audio[: max(FRAME, audio.size - (self.quiet - 3) * FRAME)]
            uid = self.utt_id
            self._reset()
            self._finish(uid, audio)
            return
        if now - self.last_partial >= self.PARTIAL_EVERY and not self.partial_busy:
            self.last_partial = now
            self._partial(self.utt_id, np.concatenate(self.buf))

    def _reset(self) -> None:
        self.in_speech = False
        self.buf = []
        self.hot = 0
        self.quiet = 0

    # ------------------------------------------------------------------ recognition
    def _partial(self, uid: int, audio: np.ndarray) -> None:
        if self.loop is None:
            return
        self.partial_busy = True

        async def run() -> None:
            try:
                text, _ = await self.loop.run_in_executor(self.executor, lambda: self.stt.transcribe(audio, True))
                if uid == self.utt_id and self.in_speech and text:
                    self.ev.on_partial(text)
            except Exception:
                log.exception("partial transcription failed")
            finally:
                self.partial_busy = False
        self._keep(self.loop.create_task(run()))

    def _finish(self, uid: int, audio: np.ndarray) -> None:
        if self.loop is None:
            return
        seconds = audio.size / RATE

        async def run() -> None:
            try:
                rms = float(np.sqrt(np.mean(audio ** 2))) if audio.size else 0.0
                if seconds < 0.3 or rms < 0.003:
                    self.ev.on_discard()
                    return
                text, meta = await self.loop.run_in_executor(self.executor, lambda: self.stt.transcribe(audio, False))
                if text:
                    self.ev.on_utterance(text, seconds)
                else:
                    self.ev.on_discard()
            except Exception:
                log.exception("transcription failed")
                self.ev.on_discard()
        self._keep(self.loop.create_task(run()))
