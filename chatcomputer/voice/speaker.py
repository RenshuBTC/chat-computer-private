"""Speaking Chat Computer's words as they stream in, sentence by sentence; stops instantly."""
from __future__ import annotations

import asyncio
import logging
from concurrent.futures import Executor
from typing import Callable

from .tts import SentenceSplitter

log = logging.getLogger("chatcomputer.speaker")


class Speaker:
    def __init__(self, tts, player, executor: Executor, on_change: Callable[[bool], None] = lambda s: None) -> None:
        self.tts = tts
        self.player = player
        self.executor = executor
        self.on_change = on_change
        self.queue: asyncio.Queue[tuple[int, str]] = asyncio.Queue()
        self.gen = 0
        self.splitter = SentenceSplitter()
        self.pending = 0
        self._worker: asyncio.Task | None = None
        self._watcher: asyncio.Task | None = None
        self._was = False
        self.muted = False

    def start(self) -> None:
        if self._worker is None:
            self._worker = asyncio.get_running_loop().create_task(self._run())
            self._watcher = asyncio.get_running_loop().create_task(self._watch())

    # ------------------------------------------------------------------ streaming
    def begin(self) -> None:
        self.splitter = SentenceSplitter()

    def feed(self, delta: str) -> None:
        for s in self.splitter.feed(delta):
            self._enqueue(s)

    def end(self) -> None:
        for s in self.splitter.flush():
            self._enqueue(s)

    def say(self, text: str) -> None:
        sp = SentenceSplitter()
        for s in sp.feed(text) + sp.flush():
            self._enqueue(s)

    def _enqueue(self, sentence: str) -> None:
        if self.muted or not sentence.strip() or self.tts is None:
            return
        self.pending += 1
        self.queue.put_nowait((self.gen, sentence))
        self._notify()

    def stop(self) -> None:
        """Silence now: drop queued sentences and cut the current one."""
        self.gen += 1
        self.splitter = SentenceSplitter()
        while not self.queue.empty():
            try:
                self.queue.get_nowait()
            except asyncio.QueueEmpty:
                break
        self.pending = 0
        self.player.stop()
        self._notify()

    @property
    def speaking(self) -> bool:
        return self.pending > 0 or self.player.busy

    async def until_quiet(self, timeout: float = 60.0) -> None:
        t = 0.0
        while self.speaking and t < timeout:
            await asyncio.sleep(0.05)
            t += 0.05

    # ------------------------------------------------------------------ workers
    async def _run(self) -> None:
        loop = asyncio.get_running_loop()
        while True:
            gen, sentence = await self.queue.get()
            if gen != self.gen:
                continue
            try:
                audio = await loop.run_in_executor(self.executor, self.tts.synth, sentence)
                if gen == self.gen and audio.size:
                    self.player.play(audio)
            except Exception:
                log.exception("speech failed")
            finally:
                if gen == self.gen:
                    self.pending = max(0, self.pending - 1)
                self._notify()

    async def _watch(self) -> None:
        while True:
            await asyncio.sleep(0.1)
            self._notify()
            try:
                self.player.close_if_idle()
            except Exception:
                pass

    def _notify(self) -> None:
        now = self.speaking
        if now != self._was:
            self._was = now
            try:
                self.on_change(now)
            except Exception:
                log.exception("on_change failed")
