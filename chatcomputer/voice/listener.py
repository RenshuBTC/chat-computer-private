"""Listening: turns the microphone stream into utterances.

Speech detection runs on every 32 ms frame. While you talk, Chat Computer shows your words live;
when you pause, the utterance is transcribed and handed on. Talking while Chat Computer speaks
interrupts it ("barge-in") when you wear headphones; with loudspeakers Chat Computer ignores the mic
while it talks so it doesn't hear itself.

Speed: an utterance ends after 0.7 s of silence, but its transcription starts after 0.3 s
(EARLY_MS); if you don't go on talking, that result is the answer, ready sooner. Live captions
are only transcribed while you're talking, no more often than the computer can manage.
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

log = logging.getLogger("chatcomputer.listener")


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
    BARGE_FRAMES = 9            # ~290 ms of clear speech to talk over Chat Computer
    END_PROB = 0.35
    PREROLL_FRAMES = 10         # keep ~320 ms before the start
    MAX_SECONDS = 30.0
    PARTIAL_EVERY = 0.9         # at most this often (longer when transcribing is slow: partial_every)
    EARLY_MS = 300              # silence after which the final transcription starts
    TAIL_FRAMES = 3             # silence kept after the last speech, for the recognizer

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
        self.recognizing = 0            # transcriptions for partials (or early finals) under way
        self.utt_id = 0
        self.speaking_until = 0.0
        self.leftover = np.zeros(0, dtype=np.float32)
        self._tasks: set[asyncio.Task] = set()
        # The final transcription started early, during the pause: {"uid", "end" (frames of speech it
        # covers), "fut"}. Used when the utterance ends without more speech.
        self.early: dict | None = None
        self.stt_seconds = 0.0          # how long the last transcription took

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

    @property
    def early_frames(self) -> int:
        return int(self.EARLY_MS / 1000 * RATE / FRAME)

    @property
    def partial_every(self) -> float:
        # On a slow computer a transcription can take longer than the pause between partials: then the
        # recognizer would never be free when the utterance ends, and the answer would wait for a partial.
        return max(self.PARTIAL_EVERY, 1.5 * self.stt_seconds)

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
        if self.quiet == 0:
            self.early = None       # talking again: what was transcribed early only shows as a partial
        dur = len(self.buf) * FRAME / RATE
        if self.quiet >= self.silence_frames or dur >= self.MAX_SECONDS:
            uid = self.utt_id
            audio = self._speech()
            end = len(self.buf) - self.quiet
            early = self.early if self.early and self.early["uid"] == uid and self.early["end"] == end else None
            self._reset()
            self._finish(uid, audio, early)
            return
        if self.quiet == self.early_frames:
            self._early(self.utt_id, len(self.buf) - self.quiet, self._speech())
            return
        if self.quiet == 0 and now - self.last_partial >= self.partial_every and not self.recognizing:
            self.last_partial = now
            self._partial(self.utt_id, np.concatenate(self.buf))

    def _speech(self) -> np.ndarray:
        """The utterance so far, without the silence after its last speech (TAIL_FRAMES of it stay)."""
        audio = np.concatenate(self.buf)
        if self.quiet > self.TAIL_FRAMES:
            audio = audio[: max(FRAME, audio.size - (self.quiet - self.TAIL_FRAMES) * FRAME)]
        return audio

    def _reset(self) -> None:
        self.in_speech = False
        self.buf = []
        self.hot = 0
        self.quiet = 0
        self.early = None

    # ------------------------------------------------------------------ recognition
    def _transcribe(self, audio: np.ndarray, fast: bool) -> asyncio.Future:
        """Transcription on the recognizer's thread; how long it took is kept (partial_every)."""
        def run() -> tuple[str, dict]:
            t0 = time.monotonic()
            try:
                return self.stt.transcribe(audio, fast)
            finally:
                self.stt_seconds = time.monotonic() - t0
        assert self.loop is not None
        return self.loop.run_in_executor(self.executor, run)

    def _partial(self, uid: int, audio: np.ndarray) -> None:
        if self.loop is None:
            return
        self.recognizing += 1

        async def run() -> None:
            try:
                text, _ = await self._transcribe(audio, True)
                if uid == self.utt_id and self.in_speech and text:
                    self.ev.on_partial(text)
            except Exception:
                log.exception("partial transcription failed")
            finally:
                self.recognizing -= 1
        self._keep(self.loop.create_task(run()))

    def _early(self, uid: int, end: int, audio: np.ndarray) -> None:
        """The final transcription of what was said up to frame `end`, started during the pause."""
        if self.loop is None:
            return
        fut = self._transcribe(audio, False)
        self.early = {"uid": uid, "end": end, "fut": fut}
        self.recognizing += 1

        def done(f: asyncio.Future) -> None:
            self.recognizing -= 1
            if f.cancelled() or f.exception() is not None:
                return
            text, _ = f.result()
            if uid == self.utt_id and self.in_speech and text:
                self.ev.on_partial(text)        # they went on talking: the words so far
        fut.add_done_callback(done)

    def _finish(self, uid: int, audio: np.ndarray, early: dict | None = None) -> None:
        if self.loop is None:
            return
        seconds = audio.size / RATE

        async def run() -> None:
            try:
                rms = float(np.sqrt(np.mean(audio ** 2))) if audio.size else 0.0
                if seconds < 0.3 or rms < 0.003:
                    self.ev.on_discard()
                    return
                result = None
                if early is not None:
                    try:
                        result = await early["fut"]     # the same audio, transcribed during the pause
                    except Exception:
                        log.exception("early transcription failed")
                if result is None:
                    result = await self._transcribe(audio, False)
                text, meta = result
                if text:
                    self.ev.on_utterance(text, seconds)
                else:
                    self.ev.on_discard()
            except Exception:
                log.exception("transcription failed")
                self.ev.on_discard()
        self._keep(self.loop.create_task(run()))
