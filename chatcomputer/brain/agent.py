"""The brain: a conversation with the model running on this computer (Chat Computer's own engine with one of Qwen's
open models: server.py, models.py), and the loop that lets it act: it writes, calls tools (Chat Computer's own, and
the everyday ones in builtin.py), each call passing the permission gate first, reads what they gave back, and goes
on until it has answered.

It streams the model's words as they're written, reports what it's doing, can be interrupted when the person speaks
over it, and keeps each conversation (its messages, in .engine/conversations) to carry on after a restart. Nothing
leaves the computer: the model, its tools and the conversations are all here.
"""
from __future__ import annotations

import asyncio
import contextlib
import itertools
import json
import logging
import os
import re
import secrets
import time
from dataclasses import dataclass, field
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Awaitable, Callable
from urllib.parse import urlparse

from ..config import Paths, Settings, State, atomic_write
from ..done import result_text
from . import builtin
from .server import ServerError, request, stream

log = logging.getLogger("chatcomputer.brain")

HERE = "here"
OURS = "mcp__chatcomputer__"       # Chat Computer's own tools, as the gate and Actions name them
MOST_STEPS = 60                    # tool calls in one turn before it's stopped
MOST_RESULT = 40_000               # characters of a tool's result kept in the conversation
MOST_MESSAGES = 400                # messages kept in a conversation (older ones go, the first question stays)
SPOKEN_TOKENS = 4096               # the most a turn's answer may write (its thinking included)
TYPED_TOKENS = 16384


@dataclass
class BrainEvents:
    on_turn_start: Callable[[str], None] = lambda tid: None
    on_text: Callable[[str, str], None] = lambda tid, delta: None
    on_activity: Callable[[str, str], None] = lambda tid, text: None
    on_turn_end: Callable[[str, "TurnResult"], None] = lambda tid, res: None
    # A tool call finished: (turn id, tool, input, worked, result, the call's id).
    on_tool: Callable[[str, str, dict[str, Any], bool, str, str], None] = lambda tid, name, inp, ok, result, use_id: None


@dataclass
class TurnResult:
    ok: bool = True
    interrupted: bool = False
    error_kind: str | None = None      # model_not_ready | engine | circles | other
    error_text: str | None = None
    text: str = ""
    tools: list[str] = field(default_factory=list)


def describe_tool(name: str, inp: dict[str, Any]) -> str:
    """A few plain words about what the model is doing, shown faintly on screen."""
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
        "chatcomputer_settings": lambda: "Changing a setting" if g("action") == "set" else "Checking settings",
        "stop_listening": lambda: "Going to sleep",
        "messages": lambda: {"recent": "Checking your messages", "search": "Searching your messages",
                             "read": "Reading a message", "open": "Opening a message", "accounts": "Checking accounts",
                             "connect": "Connecting an account", "disconnect": "Disconnecting an account"}.get(
            g("action"), "Messages"),
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
    origin: str = HERE
    res: TurnResult = field(default_factory=TurnResult)
    done: asyncio.Event = field(default_factory=asyncio.Event)
    say: str = ""          # the last tool's own answer, if it gave one: said if the model says nothing


def tool_text(result: Any) -> str:
    """A tool's result as the model reads it: its words (a picture, which this model can't see, said to be one)."""
    content = result.get("content") if isinstance(result, dict) else result
    if isinstance(content, list):
        parts = []
        for p in content:
            if isinstance(p, dict) and p.get("type") == "image":
                parts.append("[A picture: this model reads text only, so it can't see it. read_screen gives a page's "
                             "words and what's on it to click.]")
            elif isinstance(p, dict):
                parts.append(str(p.get("text", "")))
        text = "\n".join(p for p in parts if p)
    else:
        text = result_text(content)
    return builtin._cut(text, MOST_RESULT) if text else "(nothing)"


class Conversation:
    """One conversation: its system prompt (fixed when it starts, so what the model read of it is read once) and its
    messages, as the model's API has them."""

    def __init__(self, folder: Path, cid: str | None = None) -> None:
        self.folder = folder
        self.id = cid or time.strftime("c%Y%m%d-%H%M%S-") + secrets.token_hex(3)
        self.system = ""
        self.messages: list[dict[str, Any]] = []

    @property
    def path(self) -> Path:
        return self.folder / f"{self.id}.json"

    @classmethod
    def load(cls, folder: Path, cid: str) -> "Conversation | None":
        if not re.fullmatch(r"[A-Za-z0-9_\-]{1,80}", cid or ""):
            return None
        c = cls(folder, cid)
        try:
            data = json.loads(c.path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return None
        c.system = str(data.get("system") or "")
        c.messages = [m for m in data.get("messages") or [] if isinstance(m, dict) and m.get("role")]
        return c

    def save(self) -> None:
        if len(self.messages) > MOST_MESSAGES:
            # The oldest go (from a question on, so a tool's result never loses its call), the first question stays.
            cut = len(self.messages) - MOST_MESSAGES
            while cut < len(self.messages) and self.messages[cut].get("role") != "user":
                cut += 1
            self.messages = self.messages[:1] + self.messages[cut:]
        atomic_write(self.path, json.dumps({"id": self.id, "system": self.system, "messages": self.messages,
                                            "updated": time.time()}, ensure_ascii=False))

    def for_model(self) -> list[dict[str, Any]]:
        return [{"role": "system", "content": self.system}] + self.messages


class Brain:
    """`model` gets the model's server ready (server.ModelService); `tools` are Chat Computer's own tools' names,
    descriptions and input schemas, and `handlers` what each does; `gate` is the permission gate."""

    def __init__(self, paths: Paths, settings: Settings, state: State, model: Any, tools: list[dict[str, Any]],
                 handlers: dict[str, Callable[[dict[str, Any]], Awaitable[dict[str, Any]]]], gate: Any,
                 system_prompt_fn: Callable[[], str], events: BrainEvents, cwd: str | None = None,
                 env: dict[str, str] | None = None, resolve: Callable[[str], str] | None = None) -> None:
        self.paths = paths
        self.settings = settings
        self.state = state
        self.model = model
        self.handlers = handlers
        self.gate = gate
        self.system_prompt_fn = system_prompt_fn
        self.ev = events
        self.cwd = cwd or os.path.expanduser("~")
        self.builtins = builtin.Builtins(self.cwd, env, resolve)
        # Chat Computer's own tools first: the prompt says to use them before a command.
        self.tool_specs = [{"type": "function", "function": {"name": t["name"], "description": t["description"],
                                                             "parameters": t["input_schema"]}}
                           for t in [*tools, *builtin.SPECS]]
        self.folder = paths.engine / "conversations"
        self.conversation: Conversation | None = None
        self.session_id: str | None = None
        self._cur: _Turn | None = None
        self._task: asyncio.Task | None = None
        self.origins: dict[str, str] = {}
        self.typed: set[str] = set()      # turns asked by typing on this PC's keyboard (answered in text only)
        self._ids = itertools.count(1)
        self.last_activity = float(state.get("last_activity", 0) or 0)

    # ------------------------------------------------------------------ the conversation
    @property
    def busy(self) -> bool:
        return self._cur is not None and not self._cur.done.is_set()

    @property
    def current_origin(self) -> str:
        return HERE

    def origin_of(self, tid: str) -> str:
        return self.origins.get(tid, HERE)

    def typed_here(self, tid: str) -> bool:
        """Asked by typing on this PC's keyboard (not said)."""
        return tid in self.typed

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
            limit = 8.0          # 0 or less would forget the conversation every time Chat Computer starts
        if idle_h > limit:
            log.info("starting a fresh conversation (idle %.1f h)", idle_h)
            return None
        return sid

    def _conversation(self) -> Conversation:
        if self.conversation is None:
            sid = self.session_id or self._should_resume()
            c = Conversation.load(self.folder, sid) if sid else None
            if c is None:
                c = Conversation(self.folder)
                c.system = self.system_prompt_fn()
            self.conversation = c
            self.session_id = c.id
        return self.conversation

    async def connect(self) -> None:
        """Gets the model's server going, and has it read the system prompt and the tools' descriptions (thousands of
        words: on a processor, a minute or more the first time; the engine keeps that reading, on disk too), so the
        first question doesn't wait for that."""
        conv = self._conversation()
        port = await self.model.ensure()
        if self.busy:
            return
        body = {"messages": [{"role": "system", "content": conv.system}, {"role": "user", "content": "."}],
                "tools": self.tool_specs, "think": False, "max_tokens": 1}
        with contextlib.suppress(Exception):
            await request(port, "POST", "/v1/chat/completions", json.dumps(body, ensure_ascii=False).encode(),
                          timeout=3600)

    async def switch_conversation(self, session_id: str | None) -> None:
        """Carry on another conversation (a chat opened from Chats)."""
        await self.interrupt()
        self.conversation = None
        self.session_id = session_id or None
        self.last_activity = time.time()
        self.state.set(session_id=self.session_id, last_activity=self.last_activity)

    async def new_conversation(self) -> None:
        """Forget the current conversation (memory facts stay)."""
        await self.interrupt()
        self.conversation = None
        self.session_id = None
        self.state.set(session_id=None)

    async def close(self) -> None:
        await self.interrupt()

    # ------------------------------------------------------------------ turns
    async def interrupt(self) -> None:
        t, task = self._cur, self._task
        if t is None or t.done.is_set() or task is None:
            return
        task.cancel()
        with contextlib.suppress(BaseException):
            await asyncio.wait_for(asyncio.shield(t.done.wait()), timeout=10)

    async def ask(self, text: str, typed: bool = False) -> str:
        """Send what the person said here (or typed on this PC's keyboard: `typed`). Interrupts anything still
        running. Returns the turn id."""
        if self.busy:
            await self.interrupt()
        t = self._new_turn()
        if typed:
            self.typed.add(t.tid)
        self._cur = t
        self.ev.on_turn_start(t.tid)
        self._task = asyncio.create_task(self._turn(t, text))
        return t.tid

    async def wait_idle(self) -> None:
        while self._cur is not None and not self._cur.done.is_set():
            with contextlib.suppress(Exception):
                await asyncio.shield(self._cur.done.wait())

    def _new_turn(self) -> _Turn:
        t = _Turn(f"t{next(self._ids)}-{int(time.time())}")
        self.origins[t.tid] = HERE
        if len(self.origins) > 500:
            for k in list(self.origins)[:250]:
                self.origins.pop(k, None)
                self.typed.discard(k)
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

    async def _turn(self, t: _Turn, text: str) -> None:
        conv: Conversation | None = None
        try:
            conv = self._conversation()
            conv.messages.append({"role": "user", "content": text})
            port = await self.model.ensure()
            # What each tool gave back this turn, by its name and input: asked for again with the same input, it isn't
            # run again (its answer is given back, with a word to try something else or answer); a second step in a
            # row that asks for nothing new is going round in circles, and the turn stops there. (evals/os: Qwen3.5
            # 0.8B showed the same folder thirty times until the turn ran out of time.)
            given: dict[str, str] = {}
            again = 0            # steps in a row that asked only for what was done already
            for _ in range(MOST_STEPS):
                answer = await self._write(t, port, conv)
                conv.messages.append(answer)
                calls = answer.get("tool_calls") or []
                if not calls:
                    break
                keys = [str((c.get("function") or {}).get("name") or "") + " " + str((c.get("function") or {}).get("arguments") or "")
                        for c in calls]
                again = again + 1 if all(k in given for k in keys) else 0
                if again == 2:
                    conv.messages.pop()
                    if not t.res.text.strip() and not t.say:      # (with a tool's own answer: that's said, below)
                        t.res.ok, t.res.error_kind = False, "circles"
                        t.res.error_text = "it asked for the same thing again and again, so it was stopped"
                    break
                for call, key in zip(calls, keys):
                    if key in given:
                        content = ("Already done, with the same input, earlier in this turn; it said: " + given[key][:400] +
                                   " Don't ask for it again: try something else, or answer them now.")
                    else:
                        content = given[key] = await self._call(t, call)
                    conv.messages.append({"role": "tool", "tool_call_id": call.get("id", ""), "content": content})
                conv.save()
            else:
                t.res.ok, t.res.error_kind = False, "other"
                t.res.error_text = f"it was still going after {MOST_STEPS} steps, so it was stopped"
            # (Done, with no word said: what was shown is there, but they'd hear nothing, or see an empty reply.
            # evals/os: Qwen3.5 0.8B found the tax letter and showed it, and said nothing. A tool that knows the
            # answer itself gives it as `say`, said here: evals/os found the model told there's no dark mode, then
            # asking the same again until stopped, with nothing said.)
            if t.res.ok and t.res.tools and not t.res.text.strip():
                shown = any(n.split("__")[-1].startswith("show_") for n in t.res.tools[-2:])
                words = t.say or ("It's on the screen." if shown else "Done.")
                t.res.text = words
                self.ev.on_text(t.tid, words)
                conv.messages.append({"role": "assistant", "content": words})
        except asyncio.CancelledError:
            t.res.interrupted = True
            if conv is not None and conv.messages and conv.messages[-1].get("role") != "user":
                conv.messages.append({"role": "user", "content": "[Request interrupted by user]"})
        except Exception as exc:
            from .server import NotReady
            t.res.ok = False
            if isinstance(exc, NotReady):
                t.res.error_kind = "model_not_ready"
            elif isinstance(exc, (ServerError, OSError)):
                t.res.error_kind = "engine"
                log.warning("the model's engine failed: %s", exc)
            else:
                t.res.error_kind = "other"
                log.exception("the turn failed")
            t.res.error_text = str(exc)[:300]
            if conv is not None and conv.messages and conv.messages[-1].get("role") == "user" and t.res.error_kind == "model_not_ready":
                conv.messages.pop()        # (asked again once the model is ready)
        finally:
            if conv is not None:
                with contextlib.suppress(Exception):
                    conv.save()
            self._finish(t)

    def _effort(self) -> tuple[bool, str]:
        e = (self.settings.effort or "medium").lower()
        think = e != "off"
        return think, {"max": "xhigh", "high": "xhigh", "medium": "medium", "low": "low"}.get(e, "medium")

    async def _write(self, t: _Turn, port: int, conv: Conversation) -> dict[str, Any]:
        """The model's next message, its words streamed to the screen as they come; returned as the API's assistant
        message (with its thinking and tool calls)."""
        think, effort = self._effort()
        body = {"messages": conv.for_model(), "tools": self.tool_specs, "think": think, "reasoning_effort": effort,
                "max_tokens": TYPED_TOKENS if t.tid in self.typed else SPOKEN_TOKENS}
        text, thinking = [], []
        calls: list[dict[str, Any]] = []
        started_thinking = False
        gap = bool(t.res.text) and not t.res.text[-1].isspace()
        async for piece in stream(port, body):
            for choice in piece.get("choices") or []:
                d = choice.get("delta") or {}
                if d.get("reasoning_content"):
                    thinking.append(d["reasoning_content"])
                    if not started_thinking:
                        started_thinking = True
                        self.ev.on_activity(t.tid, "Thinking")
                if d.get("content"):
                    words = d["content"]
                    if gap:
                        gap = False
                        if not words[0].isspace():
                            words = " " + words      # carries on the same reply after a tool call
                    text.append(d["content"])
                    t.res.text += words
                    self.ev.on_text(t.tid, words)
                for c in d.get("tool_calls") or []:
                    calls.append({"id": c.get("id") or f"call_{len(calls)}", "type": "function",
                                  "function": c.get("function") or {}})
        message: dict[str, Any] = {"role": "assistant", "content": "".join(text)}
        if thinking:
            message["reasoning_content"] = "".join(thinking)
        if calls:
            message["tool_calls"] = calls
        return message

    async def _call(self, t: _Turn, call: dict[str, Any]) -> str:
        """Runs one tool call, past the gate first: what it gave back, as words."""
        f = call.get("function") or {}
        name = str(f.get("name") or "")
        use_id = str(call.get("id") or "")
        try:
            args = json.loads(f.get("arguments") or "{}")
        except ValueError:
            args = None
        if not isinstance(args, dict):
            return "That tool call's arguments weren't a JSON object."
        handler = self.builtins.handler(name) or self.handlers.get(name)
        if handler is None:
            return f"There's no tool called {name}."
        full = name if name in builtin.NAMES else OURS + name
        t.res.tools.append(full)
        self.ev.on_activity(t.tid, describe_tool(name, args))
        decision = await self.gate(full, args, SimpleNamespace(tool_use_id=use_id))
        if getattr(decision, "behavior", "allow") == "deny":
            result: dict[str, Any] = {"content": [{"type": "text", "text": decision.message}], "is_error": True}
        else:
            inp = getattr(decision, "updated_input", None) or args
            try:
                result = await handler(inp)
            except Exception as exc:
                log.exception("tool %s failed", name)
                result = {"content": [{"type": "text", "text": f"{name} failed: {exc}"}], "is_error": True}
        # (A tool's own answer counts only while it's the last thing done: anything done after it may change it.)
        has_say = isinstance(result, dict) and isinstance(result.get("say"), str) and not result.get("is_error")
        t.say = result["say"] if has_say else ""
        words = tool_text(result)
        try:
            self.ev.on_tool(t.tid, full, args, not result.get("is_error"), words, use_id)
        except Exception:
            log.exception("on_tool failed")
        return words


def friendly_error(res: TurnResult, progress: str | None = None) -> str:
    k = res.error_kind
    if k == "model_not_ready":
        return ("I'm still getting my model ready" + (f": {progress}" if progress else "") +
                ". Until then I can only help with Wi-Fi, volume and restarting.")
    if k == "engine":
        return "My model stopped working for a moment. Please say that again."
    if k == "circles":
        return "I went round in circles there and stopped. Could you say it another way?"
    return "Something went wrong on my side. Please say that again."
