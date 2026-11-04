"""The model's server on this computer: Chat Computer's own engine (chat-computer-model serve) with the model's file,
listening on 127.0.0.1, started and kept running from here; and the way to ask it, a stream of what it writes
(OpenAI's chat completions API, which it speaks).
"""
from __future__ import annotations

import asyncio
import contextlib
import json
import logging
import os
import subprocess
import threading
import time
from typing import Any, AsyncIterator, Callable

from . import models

log = logging.getLogger("chatcomputer.model")

PORT = 8765            # the port Chat Computer's pages and apps look for the model on
START_TIMEOUT_S = 120.0


class ServerError(RuntimeError):
    pass


class NotReady(RuntimeError):
    """The model isn't on this computer yet (it's being got ready: the message says how far along)."""


class ModelServer:
    def __init__(self, program: str, model: str, port: int = PORT, threads: int | None = None,
                 log_file: str | None = None, cache: str | None = None, uncensored: bool = False) -> None:
        self.program, self.model, self.port, self.threads = program, model, port, threads
        self.log_file, self.cache = log_file, cache
        # (Whether the model's refusal direction was there to take out when it started: the engine reads it then.)
        self.uncensored = uncensored
        self.proc: subprocess.Popen | None = None
        self._lock = asyncio.Lock()

    @property
    def running(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    async def start(self) -> None:
        """Starts it (unless it's running) and waits until it answers."""
        async with self._lock:
            if self.running and await self.healthy():
                return
            await self._stop()
            args = [self.program, "serve", self.model, "--port", str(self.port)]
            if self.threads:
                args += ["--threads", str(self.threads)]
            if self.cache:
                args += ["--cache", self.cache]     # the system prompt's reading kept across restarts
            out = open(self.log_file, "ab") if self.log_file else subprocess.DEVNULL
            try:
                # Its own process group: a signal for Chat Computer doesn't stop the model, and it can be stopped alone.
                self.proc = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=out, stderr=out,
                                             start_new_session=True)
            finally:
                if out is not subprocess.DEVNULL:
                    out.close()
            deadline = time.monotonic() + START_TIMEOUT_S
            while time.monotonic() < deadline:
                if self.proc.poll() is not None:
                    raise ServerError(f"the model's engine stopped as it started (code {self.proc.returncode})")
                if await self.healthy():
                    log.info("the model is ready: %s on port %d", os.path.basename(self.model), self.port)
                    return
                await asyncio.sleep(0.25)
            await self._stop()
            raise ServerError("the model's engine didn't start in time")

    async def healthy(self) -> bool:
        try:
            status, body = await request(self.port, "GET", "/health", timeout=2.0)
            return status == 200 and b'"ok":true' in body
        except Exception:
            return False

    async def stop(self) -> None:
        async with self._lock:
            await self._stop()

    async def _stop(self) -> None:
        p, self.proc = self.proc, None
        if p is None or p.poll() is not None:
            return
        p.terminate()
        for _ in range(40):
            if p.poll() is not None:
                return
            await asyncio.sleep(0.05)
        p.kill()


async def request(port: int, method: str, path: str, body: bytes = b"", timeout: float = 10.0) -> tuple[int, bytes]:
    """One whole request to the server: its status and body."""
    async def go() -> tuple[int, bytes]:
        reader, writer = await asyncio.open_connection("127.0.0.1", port)
        try:
            writer.write(_head(port, method, path, len(body)) + body)
            await writer.drain()
            status, _ = await _status_and_headers(reader)
            return status, await reader.read()
        finally:
            writer.close()
    return await asyncio.wait_for(go(), timeout)


def _head(port: int, method: str, path: str, length: int) -> bytes:
    return (f"{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n"
            f"Content-Length: {length}\r\nConnection: close\r\n\r\n").encode()


async def _status_and_headers(reader: asyncio.StreamReader) -> tuple[int, dict[str, str]]:
    line = await reader.readline()
    parts = line.decode("latin-1").split(" ", 2)
    if len(parts) < 2 or not parts[1].isdigit():
        raise ServerError("the model's engine gave no answer")
    headers = {}
    while True:
        h = await reader.readline()
        if h in (b"\r\n", b"\n", b""):
            break
        k, _, v = h.decode("latin-1").partition(":")
        headers[k.strip().lower()] = v.strip()
    return int(parts[1]), headers


async def stream(port: int, body: dict[str, Any]) -> AsyncIterator[dict[str, Any]]:
    """Asks for a completion, streamed: each piece as it comes (OpenAI's chunks). Leaving the loop early (or the task
    being cancelled) closes the connection, and the engine stops writing."""
    data = json.dumps({**body, "stream": True}, ensure_ascii=False).encode()
    reader, writer = await asyncio.open_connection("127.0.0.1", port, limit=1 << 24)
    try:
        writer.write(_head(port, "POST", "/v1/chat/completions", len(data)) + data)
        await writer.drain()
        status, _ = await _status_and_headers(reader)
        if status != 200:
            raw = await reader.read()
            with contextlib.suppress(Exception):
                raw = json.loads(raw)["error"]["message"].encode()
            raise ServerError(f"the model's engine refused the request ({status}): {raw.decode(errors='replace')[:300]}")
        while True:
            line = await reader.readline()
            if not line:
                break
            line = line.strip()
            if not line.startswith(b"data: "):
                continue
            payload = line[6:]
            if payload == b"[DONE]":
                break
            piece = json.loads(payload)
            if "error" in piece:
                raise ServerError(str(piece["error"].get("message") if isinstance(piece["error"], dict) else piece["error"]))
            yield piece
    finally:
        writer.close()
        with contextlib.suppress(Exception):
            await writer.wait_closed()


def _identity(path: str) -> tuple[int, int] | None:
    """Which file is at `path`, and when it was written: a file made again in its place is another."""
    try:
        st = os.stat(path)
    except OSError:
        return None
    return st.st_ino, st.st_mtime_ns


class ModelService:
    """The model that thinks: which one (the settings: models.chosen), getting it onto this computer (models.Store,
    in the background) and its server. on_progress is told how getting it ready goes (from another thread).

    While the chosen model is being got ready, one already on this computer that the memory runs answers instead
    (the biggest), if there is one: a bigger model chosen later doesn't leave Chat Computer without one meanwhile.

    Every model answers with its refusals taken out (models.Store.uncensor): one got ready before that was done
    has them found in the background, while it answers, and its server starts again with them taken out."""

    def __init__(self, paths: Any, settings: Any, on_progress: Callable[[models.Progress], None] | None = None,
                 port: int = PORT, threads: int | None = None) -> None:
        self.paths, self.settings, self.port, self.threads = paths, settings, port, threads
        self.store = models.Store(paths.models)
        self.model = models.chosen(settings.model, self.store)
        self.progress = models.Progress()
        self.on_progress = on_progress
        self.server: ModelServer | None = None
        self._thread: threading.Thread | None = None
        self._stop = threading.Event()
        self._uncensoring: threading.Thread | None = None
        self._served: tuple[int, int] | None = None   # (the file served: which it is, and when it was written)
        self._uncensor_tried: set[str] = set()     # (once a run: one that didn't work isn't tried again until the next)

    def serving(self) -> models.Model | None:
        """The model that answers now: the chosen one once it's here, else the biggest one here that the memory
        runs (or none)."""
        if self.store.ready(self.model):
            return self.model
        here = [m for m in models.catalogue() if self.store.ready(m)]
        return next((m for m in here if models.runs_in(m)), here[-1] if here else None)

    def file(self) -> str | None:
        m = self.serving()
        f = self.store.ready(m) if m else None
        return str(f) if f else None

    @property
    def ready(self) -> bool:
        return self.file() is not None

    @property
    def label(self) -> str:
        """The name of the model that answers now (the chosen one's while none is here yet)."""
        return (self.serving() or self.model).label

    def prepare_in_background(self) -> None:
        """Gets the chosen model ready in the background (another here answering meanwhile, if one is): one that
        isn't here, or one made before the engine kept the part that sees (made again)."""
        here = self.store.ready(self.model) and not self.store.outdated(self.model)
        if here or (self._thread and self._thread.is_alive()):
            return
        self._stop.clear()
        model, progress = self.model, self.progress

        def work() -> None:
            try:
                self.store.prepare(model, progress, self._stop, self.on_progress)
            except Exception as exc:
                log.warning("%s couldn't be got ready: %s", model.label, exc)
        self._thread = threading.Thread(target=work, name="model-prepare", daemon=True)
        self._thread.start()

    def uncensor_in_background(self, m: models.Model) -> None:
        """Has a model that's here, whose refusals haven't been looked for, have them found (in the background)."""
        busy = any(t and t.is_alive() for t in (self._thread, self._uncensoring))
        f = self.store.ready(m)
        if busy or not f or m.name in self._uncensor_tried or self.store.uncensored(m):
            return
        self._uncensor_tried.add(m.name)
        self._stop.clear()

        def work() -> None:
            try:
                self.store.uncensor(m, f, models.Progress(), self._stop)
            except Exception as exc:
                log.warning("%s's refusals couldn't be looked for: %s", m.label, exc)
        self._uncensoring = threading.Thread(target=work, name="model-uncensor", daemon=True)
        self._uncensoring.start()

    async def ensure(self) -> int:
        """The server's port, once it's running with the model; NotReady while the model is being got ready (and no
        other is here to answer meanwhile)."""
        if not self.store.ready(self.model) or self.store.outdated(self.model):
            self.prepare_in_background()
        f = self.file()
        m = self.serving()
        if not f or m is None:
            raise NotReady(self.progress.words())
        program = models.engine_program(self.paths.models)
        if not program:
            raise NotReady("Chat Computer's model engine isn't installed")
        uncensored = self.store.uncensored(m)
        if not uncensored:
            self.uncensor_in_background(m)
        # (A file made again has the same name: it's told apart by what the system says of it.)
        made = _identity(f)
        if self.server is None or self.server.model != f or self.server.uncensored != uncensored or \
                self._served != made:
            if self.server:
                await self.server.stop()
            self.server = ModelServer(program, f, self.port, self.threads, log_file=str(self.paths.logs / "model.log"),
                                      cache=str(self.store.root / "cache"), uncensored=uncensored)
            self._served = made
        await self.server.start()
        return self.server.port

    async def switch(self, setting: str | None) -> None:
        """Another model (the settings changed): the server stops; it starts with the new one when next needed."""
        new = models.chosen(setting, self.store)
        if new.name == self.model.name:
            return
        self._stop.set()
        self.model, self.progress = new, models.Progress()
        if self.server:
            await self.server.stop()
            self.server = None

    async def stop(self) -> None:
        self._stop.set()
        if self.server:
            await self.server.stop()
