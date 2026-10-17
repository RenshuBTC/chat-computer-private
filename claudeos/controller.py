"""ClaudeOS's conductor: what you say goes to Claude, Claude's words go to the screen and your
AirPods, and questions that need your yes or no are asked out loud.

States: listening (awake), sleeping (only the wake word or an AirPods press wakes it),
thinking (Claude is working), speaking. Saying "stop" or talking over ClaudeOS interrupts it.
"""
from __future__ import annotations

import asyncio
import contextlib
import json
import logging
import os
import time
import uuid
from concurrent.futures import ThreadPoolExecutor
from typing import Any, Callable

from . import answers
from .brain.agent import Brain, BrainEvents, TurnResult, friendly_error
from .brain.permissions import PermissionGate
from .brain.prompt import build_system_prompt, turn_prefix
from .config import Paths, Settings, State, has_login, login_valid, read_token, save_login, save_token, token_kind
from .files_index import FileIndex
from .journal import Journal
from .memory import Memory
from .platform import Platform
from .screen import Screen
from . import tools as T

HEADPHONES_RECHECK_S = 5.0
AIRTOUCH_DELAY_S = 3.0              # AirTouch starts this long after the rest (the voice and the engine first)
AIRTOUCH_HINT = ("AirTouch is on: a dot follows each index finger. Pinch thumb and finger to tap, pinch and move to "
                 "scroll, hold up an open hand to stop me. Beckon with your palm up for the keyboard, and with your "
                 "palm down to put it away.")
DICTATE_S = 15.0                    # after the keyboard's microphone key, what's said within this long is typed
HISTORY_TAIL_BYTES = 256 * 1024     # the end of the transcript, where the last messages are

log = logging.getLogger("claudeos.controller")


class Controller:
    def __init__(self, paths: Paths, settings: Settings, platform: Platform, screen: Screen, *,
                 voice: bool = True, mic_factory: Callable | None = None, player: Any = None,
                 stt: Any = None, tts: Any = None, index: bool = True, extra_env: dict[str, str] | None = None,
                 greet: bool = True, airtouch: bool = False) -> None:
        self.paths = paths.ensure()
        self.settings = settings
        self.platform = platform
        self.screen = screen
        self.voice_enabled = voice
        self.mic_factory = mic_factory
        self.player = player
        self._stt, self._tts = stt, tts
        self.use_index = index
        self.extra_env = extra_env or {}
        self.greet = greet
        self.airtouch_enabled = airtouch      # the hand as pointer (claudeos/airtouch); off in tests
        self.airtouch: Any = None
        self.airtouch_status: dict[str, Any] = {}
        self.keyboard_up = False              # AirTouch's on-screen keyboard
        self._typing: asyncio.Lock | None = None
        self.dictate_until = 0.0              # the keyboard's microphone key: words said until then are typed

        self.state = State(self.paths)
        self.journal = Journal(self.paths)
        self.memory = Memory(self.paths)
        self.loop: asyncio.AbstractEventLoop | None = None
        self._tasks: set[asyncio.Task] = set()
        self.stt_exec = ThreadPoolExecutor(1, thread_name_prefix="stt")
        self.tts_exec = ThreadPoolExecutor(1, thread_name_prefix="tts")

        self.asleep = False
        self.mode = "listening"
        self.pending_confirm: asyncio.Future | None = None
        self.confirm_lock: asyncio.Lock | None = None
        self.last_heard = time.time()
        self.media_playing = False
        self.mic_released = False
        self.rate_warned: str | None = None
        self.current_turn: str | None = None
        self.turn_text: dict[str, str] = {}
        self.ready: asyncio.Event | None = None  # created in start()
        self.brain: Brain | None = None
        self.listener = None
        self.speaker = None
        self.mic = None
        self.index: FileIndex | None = None
        self.token: str | None = None
        self.exit_code: int | None = None
        self._sleep_after_turn = False
        self._online_cache: tuple[float, bool] = (0.0, True)
        self._online_checking = False
        self._headphones: tuple[float, bool] | None = None   # (when checked, sound plays on headphones?)
        self._headphones_checking = False
        self._files_settled: asyncio.Event | None = None   # the other system's drive is there, or won't come
        self._engine_had_files = False                      # the engine started knowing where those files are
        self.last_asked = ""                  # the last thing said here, to ask again after a sign-in switch
        self.remote_told = False
        self._phone_retry_at = 0.0

    # ================================================================ start / stop
    async def start(self) -> None:
        self.loop = asyncio.get_running_loop()
        self.confirm_lock = asyncio.Lock()
        self.ready = asyncio.Event()
        s = self.settings
        self.screen.apply_settings(s.text_size, (s.user_name or "").split(" ")[0] if s.user_name else "")
        self._show_history()

        # Files: index Linux home + the other system's files (Windows' user folder, or on a Mac the drive
        # with the files sent from macOS) in the background.
        other_files = self._other_os_files()
        self._import_signin(self._signin_drop())   # before the file index can see it
        self._files_settled = asyncio.Event()
        if self._other_files_pending():
            self._spawn(self._take_in_other_files())
        else:
            self._files_settled.set()
        if self.use_index:
            roots = [("home", os.path.expanduser("~"))]
            if other_files:
                roots.append(("mac" if self.platform.name == "mac" else "windows", other_files))
            roots += [(f"extra{i}", os.path.expanduser(p)) for i, p in enumerate(s.extra_search_roots)]
            own = (self.paths.secrets, self.paths.engine, self.paths.web, self.paths.index, self.paths.models,
                   self.paths.logs, self.paths.undo)
            self.index = FileIndex(self.paths.index / "files.db", roots, windows_root=s.windows_mount, skip=own)
            self.index.start(first_delay_s=10)      # after the voice and the engine have started

        # Tools + permission gate + brain.
        self.ctx = T.ToolContext(self.paths, s, self.platform, self.journal, self.screen, self.memory, self.index,
                                 confirm=self.confirm, on_sleep=self._request_sleep,
                                 on_settings_changed=self._settings_changed, airtouch_status=self._airtouch_info,
                                 keyboard=self.set_keyboard,
                                 announce=lambda text: self.loop.call_soon_threadsafe(self._announce, text))
        server, policies, self.tool_names = T.build(self.ctx)
        self.gate = PermissionGate(self.paths, self.journal, self.confirm, policies)
        self.token = read_token(self.paths)
        if token_kind(self.token) != "invalid" or has_login(self.paths):
            self.brain = Brain(self.paths, s, self.state, self.token, server, self.gate, self._system_prompt,
                               BrainEvents(self._on_turn_start, self._on_text, self._on_activity, self._on_turn_end,
                                           self._on_rate_limit, self._on_user_text, self._on_remote),
                               extra_env=self.extra_env, add_dirs=[d for d in (other_files,) if d])
            self._spawn(self._warm_up(wait_for_files=True))

        if self.voice_enabled:
            if self._voice_models_missing():
                # First start: the voice is downloaded once, in the background.
                self._spawn(self._fetch_voice_then_start())
            else:
                await self._start_voice()
        self._spawn(self._housekeeping())
        if self.airtouch_enabled and s.airtouch:
            self._spawn(self._start_airtouch(delay=AIRTOUCH_DELAY_S))
        self.screen.state("listening")
        self.ready.set()

        if self.brain is None:
            msg = (f"I'm not signed in to Claude yet. Sign in with {self.platform.setup_place}, then start me again. "
                   f"Until then I can only do simple things like Wi-Fi, volume and switching to {self.platform.other_os}.")
            self.screen.notice(msg, "warn")
            self._say(msg)
        elif self.greet and self.voice_enabled:
            first = (s.user_name or "").split(" ")[0]
            self._say(f"Hi{' ' + first if first else ''}. I'm listening.")

    def _spawn(self, coro: Any) -> asyncio.Task:
        """Runs `coro` in the background. The event loop keeps only a weak reference to a task, so one that
        nothing else holds can be destroyed before it finishes: this keeps each until it's done, and logs it if
        it fails (otherwise that would only show when it's destroyed, if ever)."""
        task = self.loop.create_task(coro)
        self._tasks.add(task)
        task.add_done_callback(self._task_done)
        return task

    def _task_done(self, task: asyncio.Task) -> None:
        self._tasks.discard(task)
        if not task.cancelled() and task.exception() is not None:
            log.error("a background task failed", exc_info=task.exception())

    async def _warm_up(self, wait_for_files: bool = False) -> None:
        if wait_for_files:
            # Claude is told where the other system's files are when its engine starts: give the drive a few
            # seconds if the computer is still mounting it (if someone talks first, ask() starts the engine).
            with contextlib.suppress(asyncio.TimeoutError):
                await asyncio.wait_for(self._files_settled.wait(), 10)
        self._engine_had_files = self._other_os_files() is not None
        try:
            await self.brain.connect()
        except Exception as exc:
            log.warning("Claude engine not ready yet: %s", exc)

    def _other_files_pending(self) -> bool:
        """Whether the other system's drive may still be on its way: the computer mounts it (Windows' drive, or
        the one shared with macOS) alongside ClaudeOS's start, so for the first minutes ClaudeOS can be first."""
        if not self.platform.just_started():
            return False
        if self.platform.name == "mac":
            from .platform.linux import MAC_FILES
            p = os.path.expanduser(MAC_FILES)
            return os.path.isdir(p) and not os.path.ismount(p)
        return bool(self.settings.windows_user) and not os.path.ismount(self.settings.windows_mount)

    async def _take_in_other_files(self) -> None:
        """The other system's drive arrived after ClaudeOS started: take in a sign-in left there, add its files
        to the search, and tell Claude where they are (its engine starts again for that once it's idle)."""
        while self._other_files_pending():
            await asyncio.sleep(0.5)
        self._files_settled.set()
        other = self._other_os_files()
        if not other:
            return
        log.info("the other system's files arrived after the start: %s", other)
        self._import_signin(self._signin_drop())
        if self.index is not None:
            self.index.add_root("mac" if self.platform.name == "mac" else "windows", other)
        b = self.brain
        if b is not None:
            if other not in b.add_dirs:
                b.add_dirs.append(other)
            if b.client is not None and not self._engine_had_files:
                self._spawn(self._restart_brain_when_idle())

    async def stop(self) -> None:
        if self.airtouch is not None:
            at, self.airtouch = self.airtouch, None
            await self.loop.run_in_executor(None, at.stop)
        if self.index:
            self.index.stop()
        if self.mic:
            self.mic.close()
        if self.brain:
            await self.brain.interrupt()
            await self.brain.close()

    def _voice_models_missing(self) -> bool:
        from .voice import models as M
        st = M.status(self.paths, self.settings)
        need_stt = self._stt is None and not st["speech_recognition"]["ready"]
        need_tts = self._tts is None and not st["voice"]["ready"]
        return need_stt or need_tts

    async def _fetch_voice_then_start(self) -> None:
        """Downloads speech recognition and the speaking voice (about 600 MB, once), then starts listening.
        Without the internet it keeps trying, waiting longer each time."""
        from .voice import models as M
        self.screen.notice("Getting my voice ready: a one-time download of about 600 MB. "
                           "It takes a few minutes; I'll speak as soon as it's done.")
        delay = 30.0
        while True:
            try:
                await self.loop.run_in_executor(
                    None, lambda: M.download(self.paths, self.settings, progress=lambda m: log.info("voice models: %s", m)))
                break
            except Exception as exc:
                log.warning("voice model download failed: %s", exc)
                self.screen.notice("I need the internet to finish setting up my voice. I'll keep trying.", "warn")
                await asyncio.sleep(delay)
                delay = min(delay * 2, 600.0)
        await self._start_voice()
        first = (self.settings.user_name or "").split(" ")[0]
        self._say(f"Hi{' ' + first if first else ''}. My voice is ready. Just talk to me.")

    async def _start_voice(self) -> None:
        from .voice.listener import Listener, ListenerEvents
        from .voice.speaker import Speaker
        from .voice import models as M
        from .voice.vad import StreamingVAD

        def load_tts() -> Any:
            if self._tts is not None:
                return self._tts
            onnx, voices = M.kokoro_files(self.paths)
            from .voice.tts import TTS
            return TTS(str(onnx) if onnx.exists() else None, str(voices) if voices.exists() else None,
                       self.settings.voice, self.settings.speech_speed)

        def load_stt() -> Any:
            if self._stt is not None:
                return self._stt
            wdir = M.whisper_dir(self.paths, self.settings.stt_model)
            if not (wdir / "model.bin").exists():
                return None
            from .voice.stt import STT
            hint = ", ".join(x for x in ("Claude", self.settings.user_name, "YouTube") if x)
            return STT(str(wdir), self.settings.stt_language, hint=hint)

        def imports() -> None:
            # Loaded once here, before the models load side by side below (two threads importing the same
            # module at once is safe, but slower and pointless).
            for mod in ("onnxruntime", "kokoro_onnx", "faster_whisper"):
                try:
                    __import__(mod)
                except ImportError:
                    pass

        await self.loop.run_in_executor(None, imports)
        # Side by side: most of the time goes to reading and preparing each model (a second each, or more).
        stt, tts, vad = await asyncio.gather(
            self.loop.run_in_executor(None, load_stt), self.loop.run_in_executor(None, load_tts),
            self.loop.run_in_executor(None, lambda: StreamingVAD(str(M.vad_model()))))
        if self.player is None:
            from .voice.audio import Player
            self.player = Player()
        self.speaker = Speaker(tts, self.player, self.tts_exec, self._on_speaking)
        self.speaker.start()
        if stt is None:
            self.screen.notice("Speech recognition isn't installed yet, so I can't hear you. "
                               "Run: claudeos models --download", "warn")
            return
        self.listener = Listener(vad, stt, self.stt_exec,
                                 ListenerEvents(self._on_speech_start, self._on_partial, self._on_utterance,
                                                self._on_barge_in, self._on_discard),
                                 is_speaking=lambda: bool(self.speaker and self.speaker.speaking),
                                 barge_in=self._barge_in_allowed)
        self.listener.attach(self.loop)
        if self.settings.barge_in == "auto":
            self._on_headphones()           # checked now, so it's known before ClaudeOS first talks
        if self._stt is None:
            self._spawn(self._warm_up_hearing(stt))
        if self.mic_factory is None:
            from .voice.audio import Mic
            self.mic = Mic(self.listener.push_threadsafe)
        else:
            self.mic = self.mic_factory(self.listener.push_threadsafe)
        if not self.mic.open():
            self.screen.notice("I can't hear a microphone. Connect your AirPods, or say nothing and check Bluetooth.", "warn")
            self._spawn(self._retry_mic())

    async def _warm_up_hearing(self, stt: Any) -> None:
        """A first transcription, of a moment of silence, right away: the recognizer's first run is the slowest,
        and the first thing said shouldn't wait for it."""
        import numpy as np
        try:
            await self.loop.run_in_executor(self.stt_exec, lambda: stt.transcribe(np.zeros(16000, dtype=np.float32), True))
        except Exception:
            log.debug("warming up speech recognition failed", exc_info=True)

    async def _retry_mic(self) -> None:
        while self.mic and not self.mic.is_open:
            await asyncio.sleep(5)
            if not self.mic_released and self.mic.open():
                self.screen.notice("Microphone connected.")
                return

    def _barge_in_allowed(self) -> bool:
        b = self.settings.barge_in
        if b == "on":
            return True
        if b == "off":
            return False
        return self._on_headphones()

    def _on_headphones(self) -> bool:
        """Whether sound plays on headphones (then talking over ClaudeOS stops it). The listener asks on every
        32 ms frame while ClaudeOS talks, and the check (wpctl) takes about a quarter of a frame, on the thread
        that also hears and shows everything: so the answer is the last check's, made again in the background
        when it's a few seconds old. Until the first, loudspeakers (ClaudeOS doesn't hear itself)."""
        known = self._headphones
        if (known is None or time.monotonic() - known[0] > HEADPHONES_RECHECK_S) and not self._headphones_checking:
            self._headphones_checking = True
            self.loop.run_in_executor(None, self._check_headphones)
        return known[1] if known else False

    def _check_headphones(self) -> None:
        try:
            on = bool(self.platform.output_is_headset())
        except Exception:
            on = True       # can't tell: talking over ClaudeOS still works
        self._headphones = (time.monotonic(), on)
        self._headphones_checking = False

    # ================================================================ helpers
    def _signin_drop(self) -> str | None:
        """Where ClaudeOS Setup on the other system leaves a new Claude sign-in ("Sign in to Claude again"):
        C:\\Users\\<you>\\ClaudeOS on Windows; on a Mac, a hidden folder on the drive both systems open."""
        other = self._other_os_files()
        if not other:
            return None
        return os.path.join(other, ".claudeos" if self.platform.name == "mac" else "ClaudeOS")

    def _import_signin(self, drop: str | None) -> None:
        """Take a new sign-in left by ClaudeOS Setup on the other system: claude_token (the one-year token) and
        claude_login.json (the full sign-in the Claude app on the phone needs). Keep them private, and remove
        the copies left there."""
        if not drop:
            return
        other = self.platform.other_os
        self._import_login(os.path.join(drop, "claude_login.json"))
        src = os.path.join(drop, "claude_token")
        try:
            with open(src, encoding="utf-8-sig") as f:
                tok = f.read().strip()
        except OSError:
            return
        if token_kind(tok) == "invalid":
            log.warning("ignoring an invalid sign-in left on %s", other)
            return
        if tok != (read_token(self.paths) or ""):
            save_token(self.paths, tok)
            self.screen.notice(f"Signed in to Claude with the new sign-in from {other}.")
        try:
            os.remove(src)
        except OSError:
            log.info("couldn't remove the %s copy of the sign-in (its drive is read-only)", other)

    def _import_login(self, src: str) -> None:
        other = self.platform.other_os
        try:
            with open(src, encoding="utf-8-sig") as f:
                text = f.read()
        except OSError:
            return
        if not login_valid(text):
            log.warning("ignoring an invalid full sign-in left on %s", other)
        else:
            save_login(self.paths, text)
            self.screen.notice(f"Signed in again from {other}. You can type to me in the Claude app on your phone.")
        try:
            os.remove(src)
        except OSError:
            log.info("couldn't remove the %s copy of the full sign-in (its drive is read-only)", other)

    def _windows_home(self) -> str | None:
        wu = self.settings.windows_user
        base = os.path.join(self.settings.windows_mount, "Users")
        if wu and os.path.isdir(os.path.join(base, wu)):
            return os.path.join(base, wu)
        return None

    def _other_os_files(self) -> str | None:
        """The other system's files: Windows' home folder on a PC; on a Mac, the drive with what ClaudeOS Setup
        sent over from macOS (macOS's own files can't be opened from here: Apple silicon keeps them encrypted with
        keys in the Secure Enclave, FileVault or not, which Linux can't use yet)."""
        if self.platform.name == "mac":
            from .platform.linux import MAC_FILES
            p = os.path.expanduser(MAC_FILES)
            return p if os.path.isdir(p) else None
        return self._windows_home()

    def _system_prompt(self) -> str:
        return build_system_prompt(self.settings, self.memory.for_prompt(), os.path.expanduser("~"),
                                   self._other_os_files(), platform=self.platform.name)

    def _say(self, text: str) -> None:
        if self.speaker:
            self.speaker.say(text)

    def _announce(self, text: str) -> None:
        """Shown and said right away, in the middle of a tool: the code to type on a keyboard being paired."""
        self.screen.notice(text)
        self._say(text)

    def _transcript(self, role: str, text: str, via: str | None = None) -> None:
        if not text.strip():
            return
        item: dict[str, Any] = {"ts": time.time(), "role": role, "text": text}
        if via:
            item["via"] = via
        try:
            with open(self.paths.transcript, "a", encoding="utf-8") as f:
                f.write(json.dumps(item, ensure_ascii=False) + "\n")
        except OSError:
            pass

    def _show_history(self) -> None:
        # Only the end of the transcript is read: it holds every conversation, and grows by the day.
        try:
            with open(self.paths.transcript, "rb") as f:
                size = f.seek(0, os.SEEK_END)
                f.seek(max(0, size - HISTORY_TAIL_BYTES))
                lines = f.read().decode("utf-8", errors="replace").splitlines()
            if size > HISTORY_TAIL_BYTES:
                lines = lines[1:]           # the first line read may start partway through
            lines = lines[-12:]
        except OSError:
            lines = []
        items = []
        for line in lines:
            try:
                item = json.loads(line)
            except ValueError:
                continue         # a line cut short (the power went off mid-write): the others still show
            if isinstance(item, dict) and isinstance(item.get("ts"), (int, float)) and isinstance(item.get("text"), str):
                items.append(item)
        # Only show recent history (same day).
        items = [i for i in items if time.time() - i["ts"] < 12 * 3600]
        if items:
            self.screen.history(items)

    def _online(self) -> bool:
        t, ok = self._online_cache
        if time.time() - t < 20:
            return ok
        try:
            ok = bool(self.platform.internet_ok())
        except Exception:
            ok = True
        self._online_cache = (time.time(), ok)
        return ok

    async def _online_now(self) -> bool:
        """Whether the internet is there, without making each request wait for a check (a connection to Claude's
        servers, up to 3 s): a recent answer is used as it is, and an older "yes" too, while it's checked again in
        the background. Only an older "no" is checked first: they may be back online."""
        t, ok = self._online_cache
        if ok and time.time() - t >= 20 and not self._online_checking:
            self._online_checking = True

            async def refresh() -> None:
                try:
                    await self.loop.run_in_executor(None, self._online)
                finally:
                    self._online_checking = False
            self._spawn(refresh())
        if ok:
            return True
        return await self.loop.run_in_executor(None, self._online)

    def _set_mode(self, mode: str) -> None:
        self.mode = mode
        if self.asleep:
            self.screen.state("sleeping")
        else:
            self.screen.state(mode)

    # ================================================================ listening
    def _on_speech_start(self) -> None:
        self.last_heard = time.time()
        if not self.asleep:
            self.screen.partial("")

    def _on_partial(self, text: str) -> None:
        if not self.asleep:
            self.screen.partial(text)

    def _on_discard(self) -> None:
        self.screen.partial("")

    def _on_barge_in(self) -> None:
        if self.asleep:
            return
        log.info("barge-in: stopping speech")
        if self.speaker:
            self.speaker.stop()

    def _on_utterance(self, text: str, seconds: float) -> None:
        if time.time() < self.dictate_until and text.strip():
            self.dictate_until = 0.0
            self._spawn(self._dictate(text.strip()))
            return
        self._spawn(self.handle(text))

    async def _dictate(self, text: str) -> None:
        """The keyboard's microphone key was pressed: what was said is typed where the keyboard types."""
        self.screen.partial("")
        try:
            await asyncio.to_thread(self.platform.type_text, text)
        except Exception as exc:
            log.warning("dictation couldn't be typed: %s", exc)

    async def handle(self, text: str) -> None:
        """Everything the user says arrives here (also used by tests and the keyboard fallback)."""
        text = text.strip()
        if not text:
            return
        log.debug("heard %r (asleep=%s, busy=%s)", text, self.asleep, bool(self.brain and self.brain.busy))
        self.last_heard = time.time()
        self.screen.partial("")

        if self.asleep:
            woke, rest = answers.strip_wake_word(text, self.settings.wake_words)
            if not woke:
                return
            self.wake()
            if not rest:
                self._say("Yes?")
                return
            text = rest

        if self.pending_confirm is not None and not self.pending_confirm.done():
            uid = "u" + uuid.uuid4().hex[:8]
            self.screen.user_text(uid, text, True)
            self._transcript("user", text)
            self.pending_confirm.set_result(text)
            return

        uid = "u" + uuid.uuid4().hex[:8]
        if answers.is_stop(text):
            self.screen.user_text(uid, text, True)
            if self.speaker:
                self.speaker.stop()
            if self.brain and self.brain.busy:
                await self.brain.interrupt()
            return

        if answers.is_sleep(text) and not (self.brain and self.brain.busy):
            self.screen.user_text(uid, text, True)
            if self.speaker:
                self.speaker.stop()
            self._say("Okay. Say Claude when you need me.")
            await self._sleep_when_quiet()
            return

        if self.brain is None:
            self.screen.user_text(uid, text, True)
            await self._offline_command(text, signed_out=True)
            return
        if not await self._online_now():
            self.screen.user_text(uid, text, True)
            await self._offline_command(text, signed_out=False)
            return

        if self.speaker:
            self.speaker.stop()
        self.screen.user_text(uid, text, True)
        self._transcript("user", text)
        self.last_asked = text
        info = await self._screen_info()
        await self.brain.ask(turn_prefix(info, self.brain.last_activity) + text)

    async def _screen_info(self) -> dict[str, Any]:
        try:
            info = await asyncio.wait_for(self.screen.content_info(), 2)
        except Exception:
            info = {"mode": "chat"}
        info["media_playing"] = self.media_playing
        return info

    # ================================================================ brain events
    def _spoken(self, tid: str) -> bool:
        """Replies to the phone (the Claude app) are shown here but not spoken."""
        return bool(self.speaker) and not (self.brain and self.brain.origin_of(tid) == "phone")

    def _typed(self, tid: str) -> bool:
        """Typed in the Claude app on the phone (the phone is the keyboard), rather than spoken here."""
        return bool(self.brain and self.brain.origin_of(tid) == "phone")

    def _on_turn_start(self, tid: str) -> None:
        self.current_turn = tid
        self.turn_text[tid] = ""
        self.screen.ai_delta(tid, "")
        if self._spoken(tid):
            self.speaker.begin()
        if self.asleep and self._typed(tid):
            # Show the typed conversation on the screen; the microphone stays asleep until "Claude".
            self.mode = "thinking"
            self.screen.state("thinking")
        else:
            self._set_mode("thinking")

    def _on_text(self, tid: str, delta: str) -> None:
        self.turn_text[tid] = self.turn_text.get(tid, "") + delta
        self.screen.ai_delta(tid, delta)
        if self._spoken(tid):
            self.speaker.feed(delta)

    def _on_user_text(self, tid: str, text: str) -> None:
        """Something typed in the Claude app on the phone: shown like anything said here."""
        uid = "u" + uuid.uuid4().hex[:8]
        self.last_heard = time.time()
        self.screen.user_text(uid, text, True)
        self._transcript("user", text, via="phone")
        try:
            self.gate.phone_reply(text)
        except Exception:
            log.exception("phone reply")

    def _on_remote(self, state: str, info: Any) -> None:
        """The link to the Claude app on the phone (Remote Control) came up or failed."""
        if state == "on" and not self.state.get("phone_told"):
            self.state.set(phone_told=True)
            self.screen.notice("To type to me, use the Claude app on your phone: tap Code, then ClaudeOS.")
        elif state == "failed" and not self.remote_told:
            self.remote_told = True
            reason = str(info or "")
            if "full-scope" in reason or "setup-token" in reason or "subscription" in reason:
                reason = f"it needs a new sign-in. In {self.platform.setup_place}, choose to sign in again."
            self.screen.notice("Typing from the Claude app on your phone can't reach me right now: " + reason[:200],
                               "warn")

    def _on_activity(self, tid: str, text: str) -> None:
        if self.settings.show_activity:
            self.screen.activity(tid, text)
        else:
            # What Claude says after using a tool starts a new paragraph.
            said = self.turn_text.get(tid, "")
            if said and not said.endswith("\n"):
                self.turn_text[tid] = said + "\n\n"
                self.screen.ai_delta(tid, "\n\n")
        # Separate what Claude said before and after a tool, for speech.
        if self._spoken(tid):
            self.speaker.end()
            self.speaker.begin()

    def _on_turn_end(self, tid: str, res: TurnResult) -> None:
        self.screen.ai_done(tid)
        spoken = self._spoken(tid)
        if spoken:
            self.speaker.end()
        self._transcript("assistant", self.turn_text.pop(tid, "") or "", via="phone" if self._typed(tid) else None)
        if (not res.ok and res.error_kind == "auth" and self.brain and self.brain.use_token_instead()):
            # The full sign-in (for the phone) stopped working; the one-year sign-in still does.
            self.screen.notice("My sign-in for the Claude app on your phone has expired, so the phone can't reach "
                               f"me until you sign in again ({self.platform.setup_place}). Everything else works.", "warn")
            self._spawn(self._ask_again_after_switch(tid))
            if tid == self.current_turn:
                self.current_turn = None
            return
        default_model = Settings().model
        if not res.ok and res.error_kind == "model" and self.brain and self.settings.model != default_model:
            # The model chosen in the settings isn't available: every request would fail, and asking Claude to
            # change it back would fail too. Go back to the usual model and ask again.
            bad, self.settings.model = self.settings.model, default_model
            self.settings.save(self.paths)
            self.screen.notice(f"The Claude model “{bad}” isn't available, so I went back to the usual one.", "warn")
            self._spawn(self._ask_again_after_switch(tid))
            if tid == self.current_turn:
                self.current_turn = None
            return
        if not res.ok and not res.interrupted:
            msg = friendly_error(res, self.ctx.rate_limit, self.platform.setup_place)
            self.screen.notice(msg, "warn")
            if spoken:
                self._say(msg)
        if tid == self.current_turn:
            self.current_turn = None
        if self._sleep_after_turn:
            self._sleep_after_turn = False
            self._spawn(self._sleep_when_quiet())
        elif self.asleep and self._typed(tid):
            self.mode = "resting"
            self.screen.state("resting")     # the typed conversation stays visible; the microphone sleeps
        elif not (self.speaker and self.speaker.speaking):
            self._set_mode("listening")

    async def _ask_again_after_switch(self, tid: str) -> None:
        assert self.brain is not None
        await self.brain.close()
        if self.brain.origin_of(tid) == "here" and self.last_asked:
            info = await self._screen_info()
            await self.brain.ask(turn_prefix(info, self.brain.last_activity) + self.last_asked)
        else:
            await self._warm_up()

    def _on_rate_limit(self, info: dict[str, Any]) -> None:
        self.ctx.rate_limit = info
        if info.get("status") == "allowed_warning":
            key = f"{info.get('type')}:{info.get('resets_at')}"
            if key != self.rate_warned:
                self.rate_warned = key
                pct = info.get("utilization")
                when = ""
                if info.get("resets_at"):
                    when = " until " + time.strftime("%-I:%M %p", time.localtime(info["resets_at"]))
                self.screen.notice(f"You've used {round(pct * 100) if pct else 'most'}% of your Claude limit{when}.", "warn")

    def _on_speaking(self, speaking: bool) -> None:
        if speaking:
            if self.mode != "thinking":
                self._set_mode("speaking")
            if self.media_playing:
                self._spawn(self._duck(True))
        else:
            if self.media_playing:
                self._spawn(self._duck(False))
            if not (self.brain and self.brain.busy):
                self._set_mode("listening")

    async def _duck(self, on: bool) -> None:
        try:
            await self.screen.eval_js(f"(window.__CO && __CO.duck({'true' if on else 'false'}))")
        except Exception:
            pass

    # ================================================================ AirTouch
    async def _start_airtouch(self, delay: float = 0.0) -> None:
        """AirTouch's process, after the voice and the engine (`delay`); its two small models are downloaded
        first if they aren't there yet (a ClaudeOS updated from before AirTouch)."""
        if delay:
            await asyncio.sleep(delay)
        if self.airtouch is not None or not self.settings.airtouch:
            return
        from .voice import models as M
        palm, landmarks = M.airtouch_files(self.paths)
        if not (palm.exists() and landmarks.exists()):
            try:
                await self.loop.run_in_executor(None, lambda: M.download(self.paths, self.settings,
                                                                         progress=lambda m: log.info("models: %s", m)))
            except Exception as exc:
                log.warning("AirTouch's models couldn't be downloaded: %s", exc)
                self.airtouch_status = {"state": "error", "error": f"its models couldn't be downloaded: {exc}"}
                return
            if self.airtouch is not None or not self.settings.airtouch:
                return
        from .airtouch import AirTouch
        bounds = self.screen.screen_bounds() if hasattr(self.screen, "screen_bounds") else None
        self.airtouch = AirTouch(str(palm), str(landmarks),
                                 lambda ev: self.loop.call_soon_threadsafe(self._on_airtouch, ev),
                                 camera=self.settings.airtouch_camera, bounds=bounds)
        self.airtouch.start()
        if hasattr(self.screen, "on_bounds"):       # the screen moved or changed size: the pointer follows
            self.screen.on_bounds = lambda b: self.airtouch.set_bounds(b) if self.airtouch else None
        if hasattr(self.screen, "on_keyboard"):     # ...and the keyboard
            self.screen.on_keyboard = lambda spec: self.airtouch.set_keyboard(spec) if self.airtouch else None

    async def _stop_airtouch(self) -> None:
        at, self.airtouch = self.airtouch, None
        if at is not None:
            await self.loop.run_in_executor(None, at.stop)
        self.airtouch_status = {"state": "off"}
        self.screen.dots([])
        if self.keyboard_up:
            self.keyboard_up = False
            await self.screen.show_keyboard(False)

    async def set_keyboard(self, show: bool) -> str:
        """AirTouch's keyboard up or down: a beckon, its own hide key, or asked for out loud."""
        if show and self.airtouch is None:
            return "AirTouch is off, so there are no fingers to type with: turn it on first."
        spec = await self.screen.show_keyboard(show)
        self.keyboard_up = bool(show and spec)
        if self.airtouch is not None:
            self.airtouch.set_keyboard(spec if self.keyboard_up else None)
        if self.keyboard_up:
            return "The keyboard is up: type with your fingers in the air, a dot on each."
        return "The keyboard is away." if not show else "The keyboard couldn't be shown on this screen."

    async def _air_key(self, x: float, y: float) -> None:
        """A finger typed on AirTouch's keyboard: the key goes where the keyboard's keys go (the conversation, or
        the page's box being typed in), in the order they were typed."""
        if self._typing is None:
            self._typing = asyncio.Lock()
        async with self._typing:
            action = await self.screen.keyboard_press(x, y)
            if not action:
                return
            if action[0] == "hide":
                await self.set_keyboard(False)
                return
            if action[0] == "dictate":                  # the microphone key: say it, and it's typed
                if self.asleep:
                    self.wake()
                self.dictate_until = time.time() + DICTATE_S
                self.screen.notice("Say what to type.")
                return
            try:
                if action[0] == "text":
                    await asyncio.to_thread(self.platform.type_text, action[1])
                elif action[0] == "key":
                    await asyncio.to_thread(self.platform.key, action[1])
                elif action[0] == "replace":            # Space twice: the first space becomes a full stop
                    await asyncio.to_thread(self.platform.key, "BackSpace")
                    await asyncio.to_thread(self.platform.type_text, action[1])
            except Exception as exc:
                log.warning("AirTouch keyboard: %s couldn't be typed: %s", action, exc)

    def _on_airtouch(self, ev: tuple) -> None:
        kind = ev[0]
        if kind == "dots":
            self.screen.dots(ev[1])
        elif kind == "key":
            self.last_heard = time.time()
            self._spawn(self._air_key(ev[1], ev[2]))
        elif kind == "hand":
            if ev[1]:
                self.last_heard = time.time()        # a hand in use is someone here: no going to sleep
                if not self.state.get("airtouch_told"):
                    self.state.set(airtouch_told=True)
                    self.screen.notice(AIRTOUCH_HINT)
        elif kind == "gesture":
            self.last_heard = time.time()
            self._spawn(self._gesture(ev[1]))
        elif kind == "status":
            if ev[1].get("state") != self.airtouch_status.get("state"):
                log.info("AirTouch: %s", ev[1])
            self.airtouch_status = dict(ev[1])

    async def _gesture(self, name: str) -> None:
        """A hand gesture: thumbs up or down answers a question; an open hand stops (or wakes) ClaudeOS; a
        beckon up brings the keyboard up, a beckon down takes it away."""
        log.info("AirTouch gesture: %s", name)
        if name in ("keyboard up", "keyboard down"):
            if (name == "keyboard up") != self.keyboard_up:
                await self.set_keyboard(name == "keyboard up")
            return
        if name in ("yes", "no"):
            if self.pending_confirm is not None and not self.pending_confirm.done():
                answer = "Yes" if name == "yes" else "No"
                self.screen.user_text("u" + uuid.uuid4().hex[:8], f"{answer} (thumbs {'up' if name == 'yes' else 'down'})", True)
                self._transcript("user", answer)
                self.pending_confirm.set_result(answer.lower())
            return
        if self.asleep:
            self.wake()
            self._say("Yes?")
            return
        if self.speaker and self.speaker.speaking:
            self.speaker.stop()
        if self.media_playing:
            try:
                await self.screen.eval_js("(window.__CO && __CO.media('pause'))")
            except Exception:
                pass
            self._set_media(False)
        if self.brain and self.brain.busy:
            await self.brain.interrupt()

    def _airtouch_info(self) -> dict[str, Any]:
        if not self.settings.airtouch:
            return {"state": "off (turned off in the settings)"}
        return dict(self.airtouch_status) or {"state": "starting"}

    # ================================================================ yes / no
    async def confirm(self, question: str) -> tuple[str, str]:
        """Ask out loud; returns ('yes'|'no'|'unclear'|'timeout', what they said). During a typed turn (the
        Claude app on the phone) the answer is typed too: returns ('phone', '') so the question goes in the chat."""
        if self.brain and self.brain.current_origin == "phone":
            self.screen.notice("Waiting for your answer in the Claude app: " + question)
            return "phone", ""
        if not self.listener or not self.mic or not self.mic.is_open:
            self.screen.notice(question + " (I can't hear you, so I didn't do it.)", "warn")
            return "timeout", ""
        assert self.confirm_lock is not None
        async with self.confirm_lock:
            said_all = []
            for attempt in range(2):
                q = question if attempt == 0 else "Sorry, was that a yes or a no?"
                if self.speaker:
                    await self.speaker.until_quiet(15)
                qid = "q" + uuid.uuid4().hex[:8]
                self.screen.ai_delta(qid, q)
                self.screen.ai_done(qid)
                self._say(q)
                self.pending_confirm = self.loop.create_future()
                self.listener.mode = "confirm"
                if self.asleep:
                    self.wake()
                try:
                    said = await asyncio.wait_for(self.pending_confirm, 40)
                except asyncio.TimeoutError:
                    self.screen.notice("No answer, so I didn't do it.")
                    return "timeout", ""
                finally:
                    self.pending_confirm = None
                    self.listener.mode = "normal"
                said_all.append(said)
                verdict = answers.classify_answer(said)
                if verdict != "unclear":
                    return verdict, said
                if len(said.split()) > 3:  # a real instruction, not a mumble: hand it to Claude
                    return "unclear", said
            return "unclear", " / ".join(said_all)

    # ================================================================ sleep / wake
    def _request_sleep(self) -> None:
        self._sleep_after_turn = True

    async def _sleep_when_quiet(self) -> None:
        if self.speaker:
            await self.speaker.until_quiet(20)
        log.debug("going to sleep")
        self.asleep = True
        self.screen.partial("")
        self.screen.state("sleeping")

    def wake(self) -> None:
        if self.asleep:
            self.asleep = False
            self.last_heard = time.time()
            self.screen.state("listening")

    async def _keep_phone_reachable(self) -> None:
        """With the phone set up, the engine must be running for the Claude app to reach it: start it again
        if it stopped (a crash, a lost connection), at most once a minute."""
        b = self.brain
        if not b or b.client is not None or b.busy or b.auth_mode != "login" or not self.settings.phone_access:
            return
        if time.time() - self._phone_retry_at < 60:
            return
        self._phone_retry_at = time.time()
        await self._warm_up()

    async def _housekeeping(self) -> None:
        while True:
            await asyncio.sleep(15)
            try:
                await self._keep_phone_reachable()
            except Exception:
                log.exception("keeping the phone link up")
            mins = self.settings.auto_sleep_minutes
            idle = time.time() - self.last_heard
            busy = (self.brain and self.brain.busy) or (self.speaker and self.speaker.speaking) or self.pending_confirm
            if mins and mins > 0 and not self.asleep and not busy and not self.media_playing and idle > mins * 60:
                log.info("auto-sleep after %.0f min", idle / 60)
                await self._sleep_when_quiet()

    # ================================================================ hardware events (from the UI)
    def media_key(self, key: str) -> None:
        """An AirPods stem press or a keyboard media key."""
        self.loop.call_soon_threadsafe(lambda: self._spawn(self._media_key(key)))

    async def _media_key(self, key: str) -> None:
        log.info("media key %s", key)
        if self.speaker and self.speaker.speaking:
            self.speaker.stop()
            return
        if self.media_playing:
            try:
                await self.screen.eval_js("(window.__CO && __CO.media('pause'))")
            except Exception:
                pass
            self._set_media(False)
        if self.asleep:
            self.wake()
        self._say("Yes?")

    def media_state(self, playing: bool) -> None:
        self.loop.call_soon_threadsafe(self._set_media, playing)

    def _set_media(self, playing: bool) -> None:
        self.media_playing = playing
        if not self.mic:
            return
        release = self.settings.release_mic_during_media
        should = release == "on" or (release == "auto" and self._input_is_bluetooth())
        if playing and should and self.mic.is_open:
            # AirPods sound much better when nothing is using their microphone.
            self.mic.close()
            self.mic_released = True
            self.screen.notice("Enjoy. Press an AirPod to talk to me.")
        elif not playing and self.mic_released:
            self.mic_released = False
            self.mic.open()

    def _input_is_bluetooth(self) -> bool:
        try:
            return bool(self.platform.input_is_bluetooth())
        except Exception:
            return False

    def _settings_changed(self, keys: list[str]) -> None:
        s = self.settings
        if {"text_size", "text_scale", "viewing_distance_cm", "screen_inches", "web_zoom",
            "user_name"} & set(keys):
            self.screen.apply_settings(s.text_size, (s.user_name or "").split(" ")[0])
        if "airtouch" in keys and self.airtouch_enabled:
            self._spawn(self._start_airtouch() if s.airtouch else self._stop_airtouch())
        if "phone_access" in keys and self.brain:
            self.remote_told = False
            self._spawn(self._restart_brain_when_idle())
        if self.speaker and self.speaker.tts is not None:
            if "voice" in keys:
                self.speaker.tts.voice = s.voice
            if "speech_speed" in keys:
                self.speaker.tts.speed = s.speech_speed

    async def _restart_brain_when_idle(self) -> None:
        """Apply a sign-in change (phone access on or off): start the engine again once this turn is over."""
        assert self.brain is not None
        await self.brain.wait_idle()
        await self.brain.close()
        await self._warm_up()

    # ================================================================ without Claude
    async def _offline_command(self, text: str, signed_out: bool) -> None:
        """A few things that must work without Claude: Wi-Fi, volume, power."""
        t = answers.normalize(text)
        p = self.platform
        run = lambda fn, *a: self.loop.run_in_executor(None, fn, *a)
        reply = None
        if any(w in t for w in ("wifi", "wi fi", "internet", "network", "connected")):
            if t.startswith("connect to "):
                name = text.strip()[len("connect to "):].strip(" .?!")
                reply = await run(p.wifi_connect, name, None)
                if "Failed" in reply:
                    reply = (f"I couldn't join {name} without its password. Plug in a keyboard and say "
                             "'type the Wi-Fi password', or use a network you've joined before.")
            else:
                nets = await run(p.wifi_scan)
                st = await run(p.wifi_status)
                names = ", ".join(n["name"] for n in nets[:6]) or "none"
                now = f"Connected to {st.get('connected_to')}." if st.get("connected_to") else "Not connected."
                reply = f"{now} Networks I can see: {names}. Say 'connect to' and the name."
        elif "volume" in t or "louder" in t or "quieter" in t or "mute" in t:
            if "mute" in t and "unmute" not in t:
                reply = await run(p.set_mute, True)
            elif "unmute" in t:
                reply = await run(p.set_mute, False)
            else:
                up = any(w in t for w in ("up", "louder", "increase", "higher"))
                reply = await run(p.set_volume, None, 15 if up else -15)
        elif (any(w in t for w in (("macos", "mac os") if p.name == "mac" else ("windows",)))
              and any(w in t for w in ("switch", "go", "restart", "open", "boot", "back"))):
            other = p.other_os
            verdict, _ = await self.confirm(f"Restart into {other}?")
            if verdict == "yes":
                self._say(f"Restarting into {other}.")
                await asyncio.sleep(3)
                reply = await run(p.power, "switch_to_macos" if p.name == "mac" else "switch_to_windows")
            else:
                reply = "Okay, staying here."
        elif any(w in t for w in ("shut down", "shutdown", "turn off", "power off")):
            verdict, _ = await self.confirm("Shut down the computer?")
            if verdict == "yes":
                self._say("Shutting down.")
                await asyncio.sleep(3)
                reply = await run(p.power, "shutdown")
        elif "restart" in t or "reboot" in t:
            verdict, _ = await self.confirm("Restart the computer?")
            if verdict == "yes":
                self._say("Restarting.")
                await asyncio.sleep(3)
                reply = await run(p.power, "restart")
        if reply is None:
            other = self.platform.other_os
            reply = (f"I'm not signed in to Claude, so I can only help with Wi-Fi, volume, restarting, or switching to {other}."
                     if signed_out else
                     "I can't reach the internet right now, so I can only help with Wi-Fi, volume, restarting, "
                     f"or switching to {other}. Say 'Wi-Fi' to see the networks.")
        qid = "o" + uuid.uuid4().hex[:8]
        self.screen.ai_delta(qid, reply)
        self.screen.ai_done(qid)
        self._say(reply)
        self._online_cache = (0.0, False)     # checked again before the next request: they may be back online
