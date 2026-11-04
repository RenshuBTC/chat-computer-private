"""A stand-in for Chat Computer's model server (os/model's chat-computer-model serve), for end-to-end tests: it speaks
the same API (OpenAI's chat completions, streamed as server-sent events), so the brain's own agent loop, the real
tools and the permission gate are tested without a model.

What it answers is driven by "rules": each looks at the request and returns a scripted answer (a list of words and
tool calls), or None; a default rule says "OK.". Every request is kept in `requests`.
"""
from __future__ import annotations

import asyncio
import json
import threading
from typing import Any, Callable

from aiohttp import web

Rule = Callable[[dict[str, Any]], "list[Any] | None"]


def call(name: str, **args: Any) -> dict[str, Any]:
    """A tool call in a scripted answer."""
    return {"call": name, "args": args}


def last_user_text(req: dict[str, Any]) -> str:
    for m in reversed(req.get("messages") or []):
        if m.get("role") == "user":
            return str(m.get("content") or "")
    return ""


def last_is_tool_result(req: dict[str, Any]) -> bool:
    msgs = req.get("messages") or []
    return bool(msgs) and msgs[-1].get("role") == "tool"


def tool_results(req: dict[str, Any]) -> list[str]:
    """The tools' results since the model last wrote."""
    out = []
    for m in reversed(req.get("messages") or []):
        if m.get("role") != "tool":
            break
        out.append(str(m.get("content") or ""))
    return list(reversed(out))


class MockModel:
    def __init__(self) -> None:
        self.rules: list[Rule] = []
        self.requests: list[dict[str, Any]] = []
        self.delay_s = 0.0            # between the pieces of an answer
        self.port = 0
        self._loop: asyncio.AbstractEventLoop | None = None
        self._runner: web.AppRunner | None = None
        self._thread: threading.Thread | None = None
        self._ready = threading.Event()
        self._calls = 0

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def answer(self, req: dict[str, Any]) -> list[Any]:
        for rule in self.rules:
            out = rule(req)
            if out is not None:
                return out
        return ["OK."]

    async def _health(self, request: web.Request) -> web.Response:
        return web.json_response({"ok": True, "model": "mock", "style": "qwen3.5"})

    async def _complete(self, request: web.Request) -> web.StreamResponse:
        req = await request.json()
        self.requests.append(req)
        parts = self.answer(req)
        if not req.get("stream"):
            text = "".join(p for p in parts if isinstance(p, str))
            return web.json_response({"choices": [{"message": {"role": "assistant", "content": text},
                                                   "finish_reason": "stop"}]})
        resp = web.StreamResponse(headers={"Content-Type": "text/event-stream"})
        await resp.prepare(request)

        async def send(delta: dict[str, Any], finish: str | None = None) -> None:
            chunk = {"object": "chat.completion.chunk", "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
            await resp.write(b"data: " + json.dumps(chunk).encode() + b"\n\n")
            if self.delay_s:
                await asyncio.sleep(self.delay_s)

        await send({"role": "assistant"})
        calls = 0
        try:
            for p in parts:
                if isinstance(p, str):
                    for word in p.split(" "):
                        await send({"content": word + " " if word != p.split(" ")[-1] else word})
                elif isinstance(p, dict) and "think" in p:
                    await send({"reasoning_content": p["think"]})
                else:
                    self._calls += 1
                    await send({"tool_calls": [{"index": calls, "id": f"call_{self._calls}", "type": "function",
                                                "function": {"name": p["call"], "arguments": json.dumps(p["args"])}}]})
                    calls += 1
            await send({"content": ""}, "tool_calls" if calls else "stop")
            await resp.write(b"data: [DONE]\n\n")
        except (ConnectionResetError, asyncio.CancelledError):
            pass
        return resp

    def start(self) -> None:
        def run() -> None:
            self._loop = asyncio.new_event_loop()
            app = web.Application()
            app.router.add_get("/health", self._health)
            app.router.add_post("/v1/chat/completions", self._complete)
            self._runner = web.AppRunner(app)
            self._loop.run_until_complete(self._runner.setup())
            site = web.TCPSite(self._runner, "127.0.0.1", 0)
            self._loop.run_until_complete(site.start())
            self.port = site._server.sockets[0].getsockname()[1]
            self._ready.set()
            self._loop.run_forever()
        self._thread = threading.Thread(target=run, daemon=True)
        self._thread.start()
        self._ready.wait(10)

    def stop(self) -> None:
        if self._loop:
            async def shut() -> None:
                if self._runner:
                    await self._runner.cleanup()
            asyncio.run_coroutine_threadsafe(shut(), self._loop).result(10)
            self._loop.call_soon_threadsafe(self._loop.stop)


class FakeModelService:
    """What the brain needs of brain.server.ModelService, pointed at the stand-in (or not ready yet)."""

    def __init__(self, mock: MockModel | None, ready: bool = True) -> None:
        from types import SimpleNamespace
        from chatcomputer.brain.models import Progress
        self.mock = mock
        self.ready = ready
        self.model = SimpleNamespace(label="Qwen (test)", name="test", download_bytes=10**9)
        self.progress = Progress(stage="downloading", done_bytes=4 * 10**8, total_bytes=10**9)
        self.prepared = 0
        self.outdated = False          # (its file made before the engine kept the part that sees)
        self.store = SimpleNamespace(outdated=lambda m: self.outdated)

    def serving(self):
        return self.model if self.ready else None

    @property
    def label(self) -> str:
        return self.model.label

    async def ensure(self) -> int:
        from chatcomputer.brain.server import NotReady
        if not self.ready or self.mock is None:
            self.prepared += 1
            raise NotReady(self.progress.words())
        return self.mock.port

    def prepare_in_background(self) -> None:
        self.prepared += 1

    async def switch(self, setting: str | None) -> None:
        pass

    async def stop(self) -> None:
        pass
