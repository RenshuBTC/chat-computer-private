"""A small mock of the Claude Messages API for end-to-end tests.

The real Claude Code CLI (bundled with the Agent SDK) is pointed at this server with
ANTHROPIC_BASE_URL, so ClaudeOS can be tested with the real agent loop, the real tool
plumbing and real streaming, without a Claude account.

Behaviour is driven by "rules": each rule inspects the request and returns a scripted
assistant turn (a list of content blocks). Tests add rules; a default rule echoes text.
Every request is recorded in `server.requests` for assertions.
"""
from __future__ import annotations

import asyncio
import json
import threading
import uuid
from dataclasses import dataclass, field
from typing import Any, Callable

from aiohttp import web

Rule = Callable[[dict[str, Any]], "list[dict[str, Any]] | None"]


def last_user_text(req: dict[str, Any]) -> str:
    """Text of the last user message that contains text (ignores tool_result-only turns)."""
    for msg in reversed(req.get("messages", [])):
        if msg.get("role") != "user":
            continue
        content = msg.get("content")
        if isinstance(content, str):
            return content
        texts = [b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text"]
        if texts:
            return "\n".join(texts)
    return ""


def conversation(req: dict[str, Any]) -> list[dict[str, Any]]:
    """User/assistant messages only. Claude Code also sends mid-conversation `system` role
    messages (environment updates, reminders); those are not turns."""
    return [m for m in req.get("messages", []) if m.get("role") in ("user", "assistant")]


def last_message_is_tool_result(req: dict[str, Any]) -> bool:
    msgs = conversation(req)
    if not msgs or msgs[-1].get("role") != "user":
        return False
    content = msgs[-1].get("content")
    return isinstance(content, list) and any(
        isinstance(b, dict) and b.get("type") == "tool_result" for b in content
    )


def tool_results(req: dict[str, Any]) -> list[dict[str, Any]]:
    msgs = conversation(req)
    if not msgs:
        return []
    content = msgs[-1].get("content")
    if not isinstance(content, list):
        return []
    return [b for b in content if isinstance(b, dict) and b.get("type") == "tool_result"]


def text_block(text: str) -> dict[str, Any]:
    return {"type": "text", "text": text}


def tool_use_block(name: str, input: dict[str, Any]) -> dict[str, Any]:
    return {"type": "tool_use", "id": "toolu_" + uuid.uuid4().hex[:20], "name": name, "input": input}


@dataclass
class MockAnthropic:
    host: str = "127.0.0.1"
    port: int = 0
    rules: list[Rule] = field(default_factory=list)
    requests: list[dict[str, Any]] = field(default_factory=list)
    paths: list[str] = field(default_factory=list)
    chunk_delay: float = 0.0
    unknown_models: set[str] = field(default_factory=set)   # answered like the real API: 404, "model: ..."
    _runner: web.AppRunner | None = None
    _loop: asyncio.AbstractEventLoop | None = None
    _thread: threading.Thread | None = None

    # ---------------------------------------------------------------- responses
    def respond(self, req: dict[str, Any]) -> list[dict[str, Any]]:
        for rule in reversed(self.rules):  # newest rules win
            out = rule(req)
            if out is not None:
                return out
        if last_message_is_tool_result(req):
            return [text_block("Done.")]
        return [text_block("You said: " + last_user_text(req)[-200:])]

    async def _messages(self, request: web.Request) -> web.StreamResponse:
        req = await request.json()
        self.requests.append(req)
        self.paths.append(request.path)
        if req.get("model") in self.unknown_models:
            return web.json_response({"type": "error", "error": {"type": "not_found_error",
                                                                 "message": f"model: {req.get('model')}"}}, status=404)
        blocks = self.respond(req)
        stop_reason = "tool_use" if any(b["type"] == "tool_use" for b in blocks) else "end_turn"
        msg_id = "msg_" + uuid.uuid4().hex[:24]
        model = req.get("model", "claude-sonnet-5-5")
        usage = {"input_tokens": 100, "output_tokens": 20, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0}
        if not req.get("stream"):
            return web.json_response({
                "id": msg_id, "type": "message", "role": "assistant", "model": model,
                "content": blocks, "stop_reason": stop_reason, "stop_sequence": None, "usage": usage,
            })
        resp = web.StreamResponse(headers={"Content-Type": "text/event-stream", "Cache-Control": "no-cache"})
        await resp.prepare(request)

        async def send(event: str, data: dict[str, Any]) -> None:
            await resp.write(f"event: {event}\ndata: {json.dumps(data)}\n\n".encode())
            if self.chunk_delay:
                await asyncio.sleep(self.chunk_delay)

        await send("message_start", {"type": "message_start", "message": {
            "id": msg_id, "type": "message", "role": "assistant", "model": model, "content": [],
            "stop_reason": None, "stop_sequence": None, "usage": usage}})
        for i, b in enumerate(blocks):
            if b["type"] == "text":
                await send("content_block_start", {"type": "content_block_start", "index": i,
                                                   "content_block": {"type": "text", "text": ""}})
                words = b["text"].split(" ")
                for j, w in enumerate(words):
                    piece = w if j == 0 else " " + w
                    await send("content_block_delta", {"type": "content_block_delta", "index": i,
                                                       "delta": {"type": "text_delta", "text": piece}})
            elif b["type"] == "tool_use":
                await send("content_block_start", {"type": "content_block_start", "index": i,
                                                   "content_block": {"type": "tool_use", "id": b["id"], "name": b["name"], "input": {}}})
                await send("content_block_delta", {"type": "content_block_delta", "index": i,
                                                   "delta": {"type": "input_json_delta", "partial_json": json.dumps(b["input"])}})
            await send("content_block_stop", {"type": "content_block_stop", "index": i})
        await send("message_delta", {"type": "message_delta", "delta": {"stop_reason": stop_reason, "stop_sequence": None},
                                     "usage": {"output_tokens": 20}})
        await send("message_stop", {"type": "message_stop"})
        await resp.write_eof()
        return resp

    async def _count_tokens(self, request: web.Request) -> web.Response:
        req = await request.json()
        self.paths.append(request.path)
        return web.json_response({"input_tokens": max(1, len(json.dumps(req)) // 4)})

    async def _other(self, request: web.Request) -> web.Response:
        self.paths.append(f"{request.method} {request.path}")
        return web.json_response({})

    # ---------------------------------------------------------------- lifecycle
    def _make_app(self) -> web.Application:
        app = web.Application(client_max_size=64 * 1024 * 1024)
        app.router.add_post("/v1/messages", self._messages)
        app.router.add_post("/v1/messages/count_tokens", self._count_tokens)
        app.router.add_route("*", "/{tail:.*}", self._other)
        return app

    def start(self) -> str:
        ready = threading.Event()

        def run() -> None:
            self._loop = asyncio.new_event_loop()
            asyncio.set_event_loop(self._loop)
            self._runner = web.AppRunner(self._make_app())
            self._loop.run_until_complete(self._runner.setup())
            site = web.TCPSite(self._runner, self.host, self.port)
            self._loop.run_until_complete(site.start())
            sock = site._server.sockets[0]  # type: ignore[union-attr]
            self.port = sock.getsockname()[1]
            ready.set()
            self._loop.run_forever()

        self._thread = threading.Thread(target=run, daemon=True)
        self._thread.start()
        ready.wait(10)
        return self.url

    @property
    def url(self) -> str:
        return f"http://{self.host}:{self.port}"

    def stop(self) -> None:
        if self._loop and self._runner:
            fut = asyncio.run_coroutine_threadsafe(self._runner.cleanup(), self._loop)
            try:
                fut.result(5)
            except Exception:
                pass
            self._loop.call_soon_threadsafe(self._loop.stop)


def env_for(mock: MockAnthropic) -> dict[str, str]:
    """Environment that points the bundled Claude Code CLI at the mock."""
    return {
        "ANTHROPIC_BASE_URL": mock.url,
        "ANTHROPIC_API_KEY": "sk-ant-test-mock",
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
        "DISABLE_TELEMETRY": "1",
        "DISABLE_AUTOUPDATER": "1",
    }
