"""The brain: one long-running Claude session (the official Claude Code engine via the Agent
SDK), signed in with the user's own Claude subscription.

It streams Claude's words as they're written, reports what Claude is doing, can be
interrupted when the user speaks over it, and resumes the same conversation after a restart.

The same session can also be reached from the Claude app on the user's phone (Claude Code's
Remote Control). Everything the engine says is read by one reader task, so a turn that starts on
the phone, with no question asked here, is shown on the screen like any other; ClaudeOS just
doesn't speak it.
"""
from __future__ import annotations

import asyncio
import collections
import contextlib
import itertools
import json
import logging
import os
import re
import socket
import time
from dataclasses import dataclass, field
from typing import Any, Callable
from urllib.parse import urlparse

from claude_agent_sdk import (AssistantMessage, ClaudeAgentOptions, ClaudeSDKClient, ResultMessage, ServerToolResultBlock,
                              ServerToolUseBlock, SystemMessage, ToolResultBlock, ToolUseBlock, UserMessage)
from claude_agent_sdk.types import RateLimitEvent, StreamEvent

from ..config import EFFORTS, MODEL_RE, Paths, Settings, State, has_login, token_kind
from ..done import result_text

log = logging.getLogger("claudeos.brain")

BUILTIN_TOOLS = ["Bash", "Read", "Write", "Edit", "Glob", "Grep", "WebSearch", "WebFetch"]
HERE, PHONE = "here", "phone"
# Turns injected by the engine itself (a finished background task and the like) belong to the person here.
ENGINE_ORIGINS = {"task-notification", "auto-continuation", "peer", "coordinator", "unclassified", "channel"}
PHONE_TURN_IDLE_S = 15 * 60   # a phone turn that has gone silent this long is considered over
# When Remote Control doesn't start, it's tried again after these many seconds. On the engine's first start it
# looks up the account in the background, and refuses Remote Control until it has ("Unable to determine your
# organization"); a moment later the same request works.
REMOTE_RETRY_S = (20.0, 60.0)


@dataclass
class BrainEvents:
    on_turn_start: Callable[[str], None] = lambda tid: None
    on_text: Callable[[str, str], None] = lambda tid, delta: None
    on_activity: Callable[[str, str], None] = lambda tid, text: None
    on_turn_end: Callable[[str, "TurnResult"], None] = lambda tid, res: None
    on_rate_limit: Callable[[dict[str, Any]], None] = lambda info: None
    on_user_text: Callable[[str, str], None] = lambda tid, text: None        # what they typed on the phone
    on_remote: Callable[[str, Any], None] = lambda state, info: None         # the Claude app link: on | failed
    # A tool call finished: (turn, tool name, its input, whether it worked, the words of its result).
    on_tool: Callable[[str, str, dict[str, Any], bool, str], None] = lambda tid, name, inp, ok, result: None


@dataclass
class TurnResult:
    ok: bool = True
    interrupted: bool = False
    error_kind: str | None = None      # auth | billing | rate_limit | overloaded | network | engine | other
    error_text: str | None = None
    text: str = ""
    cost_usd: float | None = None
    tools: list[str] = field(default_factory=list)


def describe_tool(name: str, inp: dict[str, Any]) -> str:
    """A few plain words about what Claude is doing, shown faintly on screen."""
    short = name.split("__")[-1]
    g = lambda k: str(inp.get(k, "") or "")
    base = lambda p: os.path.basename(str(p).rstrip("/")) or str(p)
    host = lambda u: (urlparse(u if "://" in u else "https://" + u).netloc or u)[:60]
    table = {
        "show_website": lambda: "Opening " + host(g("url")),
        "read_screen": lambda: "Reading the page",
        "click": lambda: f"Clicking “{g('text')[:40]}”" if g("text") else "Clicking",
        "type_text": lambda: "Typing",
        "press_keys": lambda: "Pressing " + g("keys"),
        "scroll": lambda: "Scrolling",
        "look_at_screen": lambda: "Looking at the screen",
        "browser": lambda: {"back": "Going back", "forward": "Going forward", "reload": "Reloading"}.get(g("action"), "Browser"),
        "media": lambda: {"play": "Playing", "pause": "Pausing", "seek": "Skipping", "volume": "Changing the volume",
                          "fullscreen": "Going full screen"}.get(g("action"), "Controlling the video"),
        "show_document": lambda: f"Showing “{g('title')[:50]}”",
        "show_page": lambda: f"Building “{g('title')[:50]}”",
        "saved_pages": lambda: "Opening a saved page" if g("action") == "open" else "Looking at saved pages",
        "show_file": lambda: "Opening " + base(g("path")),
        "show_conversation": lambda: "Back to the conversation",
        "confirm_with_user": lambda: "Asking you",
        "search_files": lambda: f"Searching your files for “{g('query')[:40]}”" if g("query") else "Looking at recent files",
        "move_files": lambda: "Moving files",
        "copy_files": lambda: "Copying files",
        "trash_files": lambda: "Moving to the trash",
        "make_folder": lambda: "Making a folder",
        "undo_last_change": lambda: "Undoing",
        "recent_actions": lambda: "Checking what I did",
        "system_status": lambda: "Checking the computer",
        "wifi": lambda: "Wi-Fi: " + g("action"),
        "bluetooth": lambda: "Connecting your AirPods" if g("action") == "airpods" else "Bluetooth: " + g("action"),
        "sound": lambda: "Sound: " + g("action").replace("_", " "),
        "power": lambda: g("action").replace("_", " ").capitalize(),
        "start_by_default": lambda: "Changing what starts first",
        "software": lambda: (g("action").capitalize() + " " + ", ".join(inp.get("packages") or [])).strip(),
        "set_timezone": lambda: "Setting the time zone",
        "memory": lambda: {"remember": "Remembering", "forget": "Forgetting"}.get(g("action"), "Checking my memory"),
        "claudeos_settings": lambda: "Changing a setting" if g("action") == "set" else "Checking settings",
        "stop_listening": lambda: "Going to sleep",
        "Bash": lambda: (g("description") or "Running a command")[:70],
        "Read": lambda: "Reading " + base(g("file_path")),
        "Write": lambda: "Writing " + base(g("file_path")),
        "Edit": lambda: "Editing " + base(g("file_path")),
        "Glob": lambda: "Looking through files",
        "Grep": lambda: "Searching inside files",
        "WebSearch": lambda: f"Searching the web for “{g('query')[:50]}”",
        "WebFetch": lambda: "Reading " + host(g("url")),
    }
    try:
        return table.get(short, lambda: short.replace("_", " ").capitalize())()
    except Exception:
        return short


@dataclass
class _Turn:
    tid: str
    origin: str                      # HERE (voice or keyboard on this PC) or PHONE (the Claude app)
    res: TurnResult = field(default_factory=TurnResult)
    done: asyncio.Event = field(default_factory=asyncio.Event)
    last: float = field(default_factory=time.time)


def _user_text(m: UserMessage) -> str:
    """The words of a user message; '' for tool results."""
    if m.parent_tool_use_id or m.tool_use_result is not None:
        return ""
    if isinstance(m.content, str):
        return m.content.strip()
    parts = []
    for b in m.content:
        t = getattr(b, "text", None)
        if isinstance(t, str):
            parts.append(t)
        elif isinstance(b, dict) and b.get("type") == "text":
            parts.append(str(b.get("text", "")))
    return "\n".join(p for p in parts if p).strip()


class Brain:
    def __init__(self, paths: Paths, settings: Settings, state: State, token: str | None, mcp_server: dict[str, Any],
                 gate: Any, system_prompt_fn: Callable[[], str], events: BrainEvents,
                 extra_env: dict[str, str] | None = None, cwd: str | None = None,
                 add_dirs: list[str] | None = None, client_factory: Callable[..., Any] | None = None) -> None:
        self.paths = paths
        self.settings = settings
        self.state = state
        self.token = token
        self.mcp_server = mcp_server
        self.gate = gate
        self.system_prompt_fn = system_prompt_fn
        self.ev = events
        self.extra_env = extra_env or {}
        self.cwd = cwd or os.path.expanduser("~")
        self.add_dirs = [d for d in (add_dirs or []) if os.path.isdir(d)]
        self.client_factory = client_factory or (lambda options: ClaudeSDKClient(options=options))
        self.client: Any | None = None
        self.session_id: str | None = None
        self._reader: asyncio.Task | None = None
        self._background: set[asyncio.Task] = set()
        self._cur: _Turn | None = None
        self.origins: dict[str, str] = {}
        self.typed: set[str] = set()      # turns asked by typing on this PC's keyboard (answered in text only)
        # Tool calls waiting for their result: tool use id -> (turn, name, input). See _tool_done.
        self._tool_calls: collections.OrderedDict[str, tuple[str, str, dict[str, Any]]] = collections.OrderedDict()
        # What the engine last wrote to its error output: logged when something fails (the reason the phone
        # can't connect, say, is often only there).
        self.engine_said: collections.deque[str] = collections.deque(maxlen=200)
        self._ids = itertools.count(1)
        self._connect_lock = asyncio.Lock()
        self.last_activity = float(state.get("last_activity", 0) or 0)
        self.login_failed = False         # the full sign-in stopped working: use the one-year token
        self.remote = "off"               # the Claude app link: off | connecting | on | failed
        self.remote_detail = ""

    # ------------------------------------------------------------------ sign-in
    @property
    def auth_mode(self) -> str:
        """'login' (full sign-in, the phone can connect), 'token' (one-year token) or 'none'."""
        token_ok = token_kind(self.token) != "invalid"
        login_ok = has_login(self.paths) and not self.login_failed
        if login_ok and (self.settings.phone_access or not token_ok):
            return "login"
        return "token" if token_ok else "none"

    @property
    def can_sign_in(self) -> bool:
        return self.auth_mode != "none"

    def use_token_instead(self) -> bool:
        """After the full sign-in failed: switch to the one-year token if there is one. True if switched."""
        if self.auth_mode != "login" or token_kind(self.token) == "invalid":
            return False
        self.login_failed = True
        self.remote = "off"
        return True

    # ------------------------------------------------------------------ session
    @property
    def busy(self) -> bool:
        return self._cur is not None and not self._cur.done.is_set()

    @property
    def current_origin(self) -> str:
        return self._cur.origin if self.busy and self._cur else HERE

    def origin_of(self, tid: str) -> str:
        return self.origins.get(tid, HERE)

    def typed_here(self, tid: str) -> bool:
        """Asked by typing on this PC's keyboard (not said, and not from the phone)."""
        return tid in self.typed

    def engine_tail(self, n: int = 40) -> str:
        """The engine's last lines of error output, without anything that looks like a sign-in."""
        lines = list(self.engine_said)[-n:]
        return re.sub(r"sk-ant-[A-Za-z0-9_\-]+", "sk-ant-(hidden)", "\n".join(lines))

    def _env(self) -> dict[str, str]:
        env = {
            "CLAUDE_CONFIG_DIR": str(self.paths.engine),
            "DISABLE_AUTOUPDATER": "1",
            "DISABLE_TELEMETRY": "1",
            "DISABLE_ERROR_REPORTING": "1",
            "ENABLE_TOOL_SEARCH": "false",          # load ClaudeOS tools up front: faster replies by voice
            "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",  # ClaudeOS keeps its own memory file
            "CLAUDE_AGENT_SDK_CLIENT_APP": "claudeos/0.1",
            "BASH_DEFAULT_TIMEOUT_MS": "180000",
            "MAX_MCP_OUTPUT_TOKENS": "40000",
        }
        if self.auth_mode == "login":
            # Claude Code reads the full sign-in from CLAUDE_CONFIG_DIR. A token in the environment would win
            # over it (the engine inherits ClaudeOS's environment), so make sure there is none. Remote Control
            # needs Claude Code's feature flags, so the "non-essential traffic" switch stays off in this mode.
            for k in ("CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"):
                os.environ.pop(k, None)
        else:
            env["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"] = "1"
            if token_kind(self.token) == "api_key":
                env["ANTHROPIC_API_KEY"] = self.token or ""
            else:
                env["CLAUDE_CODE_OAUTH_TOKEN"] = self.token or ""
        env.update(self.extra_env)
        return env

    def _should_resume(self) -> str | None:
        sid = self.state.get("session_id")
        if not sid:
            return None
        idle_h = (time.time() - self.last_activity) / 3600 if self.last_activity else 1e9
        try:
            limit = float(self.settings.fresh_session_after_idle_hours or 8)
        except (TypeError, ValueError):
            limit = 8.0
        if not limit > 0:
            limit = 8.0          # 0 or less would forget the conversation every time the engine starts
        if idle_h > limit:
            log.info("starting a fresh conversation (idle %.1f h)", idle_h)
            return None
        return sid

    def _options(self, resume: str | None) -> ClaudeAgentOptions:
        return ClaudeAgentOptions(
            system_prompt=self.system_prompt_fn(),
            tools=list(BUILTIN_TOOLS),
            allowed_tools=[],                    # everything goes through the permission gate
            mcp_servers={"claudeos": self.mcp_server},
            can_use_tool=self.gate,
            permission_mode="default",
            include_partial_messages=True,
            setting_sources=[],
            # A hand-edited settings file can hold anything: a value the engine can't take is left out.
            model=self.settings.model if MODEL_RE.match(self.settings.model or "") else None,
            effort=self.settings.effort if self.settings.effort in EFFORTS else None,
            cwd=self.cwd,
            add_dirs=self.add_dirs,
            env=self._env(),
            resume=resume,
            max_buffer_size=32 * 1024 * 1024,
            stderr=self._engine_stderr,
        )

    async def connect(self) -> None:
        async with self._connect_lock:
            if self.client is not None:
                return
            resume = self.session_id or self._should_resume()
            for attempt in (resume, None) if resume else (None,):
                client = self.client_factory(self._options(attempt))
                try:
                    await client.connect()
                except Exception as exc:
                    log.warning("connect failed (resume=%s): %s", attempt, exc)
                    with contextlib.suppress(Exception):
                        await client.disconnect()
                    continue
                self.client = client
                self.session_id = attempt
                self._reader = asyncio.create_task(self._read_loop(client))
                log.info("Claude session %s (%s sign-in)", "resumed " + attempt if attempt else "started", self.auth_mode)
                if self.auth_mode == "login" and self.settings.phone_access:
                    self._spawn(self._enable_remote(client))
                return
            raise RuntimeError("couldn't start the Claude engine")

    def _spawn(self, coro: Any) -> None:
        """Runs `coro` in the background. The event loop keeps only a weak reference to a task, so one
        nothing else holds can be destroyed before it finishes; this set holds each until it's done."""
        task = asyncio.create_task(coro)
        self._background.add(task)
        task.add_done_callback(self._background.discard)

    async def _enable_remote(self, client: Any) -> None:
        """Make this session reachable from the Claude app on the phone (Remote Control). Claude Code's
        own hosts (the desktop app, VS Code) do this with the same control request; the Python SDK has no
        public call for it yet, so it goes through the SDK's control channel directly."""
        name = "ClaudeOS"
        host = socket.gethostname()
        if host and host.lower() not in ("claudeos", "localhost"):
            name = f"ClaudeOS ({host})"
        req: dict[str, Any] = {"subtype": "remote_control", "enabled": True, "name": name}
        previous = self.state.get("remote_session_id")
        if previous:
            req["reattach_session_id"] = previous
        self.remote = "connecting"
        tries = len(REMOTE_RETRY_S) + 1
        for attempt in range(tries):
            try:
                resp = await self._remote_request(client, req)
                break
            except Exception as exc:
                if client is not self.client:
                    return
                detail = str(exc)[:300] or type(exc).__name__
                log.warning("Remote Control (the Claude app on the phone) didn't start (try %d of %d): %s (%s)",
                            attempt + 1, tries, detail, type(exc).__name__)
                log.warning("the engine's last words before that:\n%s", self.engine_tail())
                if attempt + 1 < tries:
                    await asyncio.sleep(REMOTE_RETRY_S[attempt])
                    if client is not self.client:
                        return
                    continue
                self.remote = "failed"
                self.remote_detail = detail
                self.ev.on_remote("failed", self.remote_detail)
                return
        if client is not self.client:
            return
        self.remote = "on"
        self.remote_detail = json.dumps(resp)[:500] if resp else ""
        log.info("Remote Control is on: %s", self.remote_detail)
        sid = None
        if isinstance(resp, dict):
            for k in ("session_id", "sessionId", "bridge_session_id", "bridgeSessionId", "remote_session_id"):
                if isinstance(resp.get(k), str) and resp.get(k):
                    sid = resp[k]
                    break
        if sid:
            self.state.set(remote_session_id=sid)
        self.ev.on_remote("on", resp or {})

    @staticmethod
    async def _remote_request(client: Any, req: dict[str, Any]) -> Any:
        query = getattr(client, "_query", None)
        if query is None:
            raise RuntimeError("the engine connection isn't ready")
        try:
            return await query._send_control_request(req, timeout=90)
        except Exception:
            if "reattach_session_id" not in req:
                raise
            req.pop("reattach_session_id")          # that session is gone: start a new one
            return await query._send_control_request(req, timeout=90)

    async def new_conversation(self) -> None:
        """Forget the current conversation (memory facts stay)."""
        await self.interrupt()
        await self.close()
        self.session_id = None
        self.state.set(session_id=None)

    async def close(self) -> None:
        client, self.client = self.client, None
        reader, self._reader = self._reader, None
        if reader and reader is not asyncio.current_task():
            reader.cancel()
            with contextlib.suppress(BaseException):
                await reader
        if client:
            with contextlib.suppress(Exception):
                await client.disconnect()
        if self._cur and not self._cur.done.is_set():
            self._cur.res.interrupted = True
            self._finish(self._cur)

    # ------------------------------------------------------------------ turns
    async def interrupt(self) -> None:
        t = self._cur
        if t is None or t.done.is_set():
            return
        try:
            if self.client:
                await asyncio.wait_for(self.client.interrupt(), timeout=5)
        except Exception as exc:
            log.info("interrupt failed: %s", exc)
        try:
            await asyncio.wait_for(asyncio.shield(t.done.wait()), timeout=10)
        except asyncio.TimeoutError:
            log.warning("the engine didn't stop; starting clean next time")
            await self.close()   # the engine is stuck

    def _engine_stderr(self, line: str) -> None:
        line = line.rstrip()
        log.debug("engine: %s", line)
        if line:
            self.engine_said.append(line[:500])

    async def ask(self, text: str, typed: bool = False) -> str:
        """Send what the user said here (or typed on this PC's keyboard: `typed`). Interrupts anything still
        running. Returns the turn id."""
        if self.busy:
            await self.interrupt()
        t = self._new_turn(HERE)
        if typed:
            self.typed.add(t.tid)
        self._cur = t
        self.ev.on_turn_start(t.tid)
        try:
            if self.client is None:
                await self.connect()
            assert self.client is not None
            await self.client.query(text)
        except Exception as exc:
            log.exception("couldn't send to Claude")
            msg = str(exc)
            t.res.ok = False
            t.res.error_kind = "network" if any(w in msg.lower() for w in ("network", "connect", "timed out", "dns")) else "engine"
            t.res.error_text = msg[:300]
            self._finish(t)
            await self.close()
        return t.tid

    async def wait_idle(self) -> None:
        while self._cur is not None and not self._cur.done.is_set():
            with contextlib.suppress(Exception):
                await asyncio.shield(self._cur.done.wait())

    def _new_turn(self, origin: str) -> _Turn:
        prefix = "p" if origin == PHONE else "t"
        t = _Turn(f"{prefix}{next(self._ids)}-{int(time.time())}", origin)
        self.origins[t.tid] = origin
        if len(self.origins) > 500:
            for k in list(self.origins)[:250]:
                self.origins.pop(k, None)
                self.typed.discard(k)
        return t

    def _start_unasked(self, origin: str) -> _Turn:
        """A turn this PC didn't ask for: typed on the phone, or the engine continuing on its own."""
        t = self._new_turn(origin)
        self._cur = t
        log.info("turn %s started from %s", t.tid, origin)
        self.ev.on_turn_start(t.tid)
        return t

    def _finish(self, t: _Turn) -> None:
        if t.done.is_set():
            return
        t.done.set()
        if self._cur is t:
            self._cur = None
        self.last_activity = time.time()
        self.state.set(session_id=self.session_id, last_activity=self.last_activity)
        try:
            self.ev.on_turn_end(t.tid, t.res)
        except Exception:
            log.exception("on_turn_end failed")

    # ------------------------------------------------------------------ everything the engine says
    async def _read_loop(self, client: Any) -> None:
        error: Exception | None = None
        try:
            async for m in client.receive_messages():
                try:
                    self._route(m)
                except Exception:
                    log.exception("couldn't handle a message from the engine")
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            error = exc
            log.warning("the engine connection ended: %s", exc)
        # The engine stopped (or its output ended). Start clean next time.
        if self.client is client:
            self.client = None
            self._reader = None
            self._spawn(self._dispose(client))
        t = self._cur
        if t is not None and not t.done.is_set():
            if error is not None:
                msg = str(error)
                t.res.ok = False
                t.res.error_kind = ("network" if any(w in msg.lower() for w in ("network", "connect", "timed out", "dns"))
                                    else "engine")
                t.res.error_text = msg[:300]
            else:
                t.res.interrupted = True
            self._finish(t)

    @staticmethod
    async def _dispose(client: Any) -> None:
        with contextlib.suppress(Exception):
            await client.disconnect()

    def _tool_done(self, use_id: str, ok: bool, result: str) -> None:
        """A tool call's result came back (What's done lists what worked: claudeos/done.py)."""
        call = self._tool_calls.pop(use_id, None)
        if call is None:
            return
        tid, name, inp = call
        try:
            self.ev.on_tool(tid, name, inp, ok, result)
        except Exception:
            log.exception("on_tool failed")

    def _route(self, m: Any) -> None:
        now = time.time()
        t = self._cur if self._cur is not None and not self._cur.done.is_set() else None
        if t is not None and t.origin == PHONE and now - t.last > PHONE_TURN_IDLE_S:
            t.res.interrupted = True     # the phone turn went quiet long ago; whatever comes now is new
            self._finish(t)
            t = None

        if isinstance(m, SystemMessage):
            sid = m.data.get("session_id") if isinstance(m.data, dict) else None
            if sid:
                self.session_id = sid
            return
        if isinstance(m, RateLimitEvent):
            info = m.rate_limit_info
            self.ev.on_rate_limit({"status": info.status, "type": info.rate_limit_type,
                                   "utilization": info.utilization, "resets_at": info.resets_at})
            return
        if isinstance(m, UserMessage):
            if isinstance(m.content, list):
                for b in m.content:
                    if isinstance(b, ToolResultBlock):
                        self._tool_done(b.tool_use_id, not b.is_error, result_text(b.content))
            text = _user_text(m)
            kind = (m.origin or {}).get("kind") if isinstance(m.origin, dict) else None
            if t is None and text:
                # Nobody here asked: typed in the Claude app, or injected by the engine itself.
                t = self._start_unasked(HERE if kind in ENGINE_ORIGINS else PHONE)
                if t.origin == PHONE:
                    self.ev.on_user_text(t.tid, text)
            elif t is not None and t.origin == PHONE and text and kind not in ENGINE_ORIGINS:
                self.ev.on_user_text(t.tid, text)   # another message from the phone in the same turn
            if t is not None:
                t.last = now
            return

        if t is None:
            if not isinstance(m, (StreamEvent, AssistantMessage)):
                return          # a stray result or status with no turn
            t = self._start_unasked(PHONE)
        t.last = now
        res = t.res
        if isinstance(m, StreamEvent):
            e = m.event
            if e.get("type") == "content_block_delta":
                d = e.get("delta", {})
                if d.get("type") == "text_delta" and d.get("text"):
                    res.text += d["text"]
                    self.ev.on_text(t.tid, d["text"])
        elif isinstance(m, AssistantMessage):
            if m.error:
                res.error_kind = {"authentication_failed": "auth", "billing_error": "billing",
                                  "rate_limit": "rate_limit", "server_error": "overloaded"}.get(m.error, "other")
            for b in m.content:
                if isinstance(b, (ToolUseBlock, ServerToolUseBlock)):
                    self._tool_calls[b.id] = (t.tid, b.name, b.input or {})
                    while len(self._tool_calls) > 200:
                        self._tool_calls.popitem(last=False)
                if isinstance(b, ToolUseBlock):
                    res.tools.append(b.name)
                    self.ev.on_activity(t.tid, describe_tool(b.name, b.input or {}))
                elif isinstance(b, ServerToolResultBlock):
                    # Run by Claude's servers (a web search): an error comes back as the result's content.
                    c = b.content if isinstance(b.content, dict) else {}
                    self._tool_done(b.tool_use_id, "error_code" not in c and not str(c.get("type", "")).endswith("_error"),
                                    "")
        elif isinstance(m, ResultMessage):
            self.session_id = m.session_id or self.session_id
            res.cost_usd = m.total_cost_usd
            if m.terminal_reason in ("aborted_streaming", "aborted_tools"):
                res.interrupted = True
            if m.is_error and not res.interrupted:
                res.ok = False
                status = m.api_error_status
                if status == 401 or res.error_kind == "auth":
                    res.error_kind = "auth"
                elif status == 429 or res.error_kind == "rate_limit":
                    res.error_kind = "rate_limit"
                elif status == 404 and "model" in "; ".join(m.errors or [m.result or ""]).lower():
                    res.error_kind = "model"
                elif status in (500, 502, 503, 529) or res.error_kind == "overloaded":
                    res.error_kind = "overloaded"
                elif m.subtype == "error_during_execution" and not status:
                    res.error_kind = res.error_kind or "engine"
                else:
                    res.error_kind = res.error_kind or "other"
                res.error_text = "; ".join(m.errors or []) or (m.result or m.subtype)
            self._finish(t)


def friendly_error(res: TurnResult, rate: dict[str, Any] | None = None,
                   setup_place: str = "ClaudeOS Setup on Windows") -> str:
    k = res.error_kind
    if k == "auth":
        # setup_place: where this computer's people sign in again (on a Mac, ClaudeOS Setup in macOS).
        return (f"I can't sign in to Claude: your sign-in has expired or was removed. Run {setup_place} "
                "again to sign in, then come back.")
    if k == "billing":
        return "Your Claude plan needs attention before I can continue. Check your account at claude.ai."
    if k == "rate_limit":
        when = ""
        if rate and rate.get("resets_at"):
            when = " It resets at " + time.strftime("%-I:%M %p", time.localtime(rate["resets_at"])) + "."
        return "You've reached your Claude usage limit for now." + when
    if k == "overloaded":
        return "Claude is very busy right now. Try again in a minute."
    if k == "model":
        return "The Claude model in my settings isn't available."
    if k == "network":
        return "I can't reach Claude. Check the internet connection."
    return "Something went wrong on my side. Please say that again."
