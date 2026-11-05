"""Chat Computer's conductor: what you say goes to the model (on this computer: brain/), its words go to the screen
and your AirPods, and questions that need your yes or no are asked out loud.

States: listening (awake), sleeping (only the wake word or an AirPods press wakes it),
thinking (the model is working), speaking. Saying "stop" or talking over Chat Computer interrupts it.
"""
from __future__ import annotations

import asyncio
import contextlib
import json
import logging
import os
import re
import time
import uuid
from concurrent.futures import ThreadPoolExecutor
from typing import Any, Callable

from . import __version__, answers, logcopy, schedule as sch, slash_search, update
from .brain.agent import Brain, BrainEvents, TurnResult, friendly_error
from .brain.models import Progress
from .brain.permissions import PermissionGate, is_private
from .brain.prompt import build_system_prompt, turn_prefix
from .brain.server import ModelService
from .config import Paths, Settings, State, model_label
from .chats import Chats
from .done import FAILED, History, describe as describe_done, for_screen, result_text
from .files_index import FileIndex
from .here import Here
from .messages import Inbox
from .pins import Pins
from . import journal as jn
from .journal import CantUndo, Journal
from .lock import PIN_RE, PinLock
from .memory import Memory
from .platform import Platform
from .screen import Screen
from . import tools as T

HEADPHONES_RECHECK_S = 5.0
DICTATE_S = 15.0                    # after the keyboard's microphone key, what's said within this long is typed
HISTORY_TAIL_BYTES = 256 * 1024     # the end of the transcript, where the last messages are
START_QUESTIONS_DELAY_S = 4.0       # questions at the start (an update, AirPods) come this long after it
UPDATE_DRIVE_WAIT_S = 60.0          # just after the computer started: how long to wait for Windows' drive, for updates
AIRPODS_PAIR_S = 60.0               # how long Chat Computer looks for AirPods in pairing mode
AIRPODS_WAIT_S = 120.0              # at the start: how long AirPods known here are waited for (out of their case)
AIRPODS_RETRY_S = 3.0               # ...trying to connect them this often
AIRPODS_ASKED_S = 20.0              # asked for ("connect my AirPods"): how long to keep trying
LOG_COPY_FIRST_S = 60.0             # the logs are copied to Windows' drive this long after the start...
LOG_COPY_EVERY_S = 600.0            # ...and then this often (see logcopy.py)
DRIVE_CHECK_S = 3.0                 # how often to look for a drive (or keyboard) just plugged in
RING_FOR_S = {"reminder": 0.0, "timer": 60.0, "alarm": 600.0}   # how long each rings unless stopped first
RING_EVERY_S = {"reminder": 5.0, "timer": 4.0, "alarm": 6.0}
SNOOZE_S = 9 * 60
# The tools that show something in the middle or work on what's there: a typed turn that used one leaves what's
# open on the screen; one answered only in words brings the conversation back to show them (_show_words).
SCREEN_TOOLS = {"show_website", "read_screen", "click", "type_text", "press_keys", "on_screen_keyboard", "scroll",
                "look_at_screen", "browser", "media", "show_document", "show_page", "saved_pages", "show_file",
                "show_conversation", "confirm_with_user", "messages", "sound", "screenshot", "display", "printer",
                "lock_screen"}

log = logging.getLogger("chatcomputer.controller")


class Controller:
    def __init__(self, paths: Paths, settings: Settings, platform: Platform, screen: Screen, *,
                 voice: bool = True, mic_factory: Callable | None = None, player: Any = None,
                 stt: Any = None, tts: Any = None, index: bool = True, extra_env: dict[str, str] | None = None,
                 greet: bool = True, offer_headphones: bool = False,
                 offer_updates: bool = False, weather: bool = False) -> None:
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
        self.offer_headphones = offer_headphones   # ask about AirPods at the start (off in tests)
        self.offer_updates = offer_updates         # offer a newer Chat Computer at the start (update.py; off in tests)
        # Where they are and its weather, for the greeting (here.py: Open-Meteo; looked up only when `weather`, off in
        # tests: the place alone otherwise).
        self.here = Here(self.paths.root / "here.json")
        self.weather = weather
        self._answer_typed = False                 # the last answer to a question was typed, not said
        self.keyboard_up = False              # the onscreen keyboard is up
        self._typing: asyncio.Lock | None = None
        self.dictate_until = 0.0              # the keyboard's microphone key: words said until then are typed

        self.state = State(self.paths)
        self.journal = Journal(self.paths)
        self.memory = Memory(self.paths)
        self.history = History(self.paths)          # Actions (chatcomputer/done.py)
        self.chats = Chats(self.paths)              # each task's chat (the right sidebar's lower half)
        self.schedule = sch.Schedule(self.paths.root / "schedule.json")   # timers, reminders, alarms
        self.ringing: dict[str, Any] | None = None   # the timer, reminder or alarm ringing now
        self._time_wake: asyncio.Event | None = None  # what rings next changed: look again
        self._night_applied: tuple[bool, int] | None = None   # the night light as last set here (on, warmth)
        self.pinlock = PinLock(self.paths.secrets / "lock_pin")   # the lock screen's PIN (chatcomputer/lock.py)
        self.locked = False
        self._pin_form: dict[str, Any] | None = None   # the PIN form on the screen: {"mode", "then_lock"}
        self.pins = Pins(self.paths.root / "pins.json")     # pinned in Actions and in Messages
        self.inbox: Inbox | None = None                 # Messages (chatcomputer/messages)
        # The message that asked for each turn, for Actions: turn id -> (its id on the screen, words, time).
        self.prompt_of: dict[str, tuple[str, str, float]] = {}
        self._last_prompt: tuple[str, str, float] | None = None
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
        self.current_turn: str | None = None
        self.turn_text: dict[str, str] = {}
        self.ready: asyncio.Event | None = None  # created in start()
        self.brain: Brain | None = None
        self.ctx: T.ToolContext | None = None      # the tools' (start())
        self.listener = None
        self.speaker = None
        self.mic = None
        self.index: FileIndex | None = None
        self.model: ModelService | None = None   # the model that thinks: getting it ready, and its server
        self._model_told = 0.0                     # when getting the model ready was last shown
        self.exit_code: int | None = None
        self._sleep_after_turn = False
        self._headphones: tuple[float, bool] | None = None   # (when checked, sound plays on headphones?)
        self._headphones_checking = False
        self._files_settled: asyncio.Event | None = None   # the other system's drive is there, or won't come
        self._engine_had_files = False                      # the engine started knowing where those files are
        self.last_asked = ""                  # the last thing said here
        self._airpods_start: asyncio.Task | None = None      # the AirPods at the start (_airpods_quietly)

    # ================================================================ start / stop
    async def start(self) -> None:
        self.loop = asyncio.get_running_loop()
        self.confirm_lock = asyncio.Lock()
        self.ready = asyncio.Event()
        s = self.settings
        self.screen.apply_settings(s.text_size, (s.user_name or "").split(" ")[0] if s.user_name else "",
                                   model_label(s.model))
        self._say_here()
        if self.weather:
            self._spawn(self._weather_now_and_then())
        self.screen.panels(self._panels())
        if self.pinlock.has_pin():
            self.lock_now()
        self._show_history()
        self.screen.on_goto = lambda target, ts, before: self._spawn(self.go_back_to(target, ts, before))
        self.screen.on_page_event = self.page_event
        # The onscreen keyboard (for a finger on the touchscreen): where it is, and its keys typed.
        self.screen.on_keyboard = self._keyboard_moved
        self.screen.on_keyboard_action = lambda action: self._spawn(self._keyboard_action(action))
        self._spawn(self._show_done())
        self.inbox = Inbox(self.paths, s, self.screen, self.pins, lambda text, kind="info": self.screen.notice(text, kind))
        self.screen.on_source = self.inbox.web_snapshot
        self._spawn(self.inbox.start())

        # Files: index Linux home + the other system's files (Windows' user folder, or on a Mac the drive
        # with the files sent from macOS) in the background.
        other_files = self._other_os_files()
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
            own = (self.paths.secrets, self.paths.engine, self.paths.web, self.paths.sources, self.paths.index,
                   self.paths.models, self.paths.logs, self.paths.undo)
            self.index = FileIndex(self.paths.index / "files.db", roots, windows_root=s.windows_mount, skip=own)
            self.index.start(first_delay_s=10)      # after the voice and the engine have started

        # Tools + permission gate + brain.
        self.ctx = T.ToolContext(self.paths, s, self.platform, self.journal, self.screen, self.memory, self.index,
                                 confirm=self.confirm, on_sleep=self._request_sleep,
                                 on_settings_changed=self._settings_changed,
                                 on_mic_changed=lambda: self.loop.call_soon_threadsafe(self._spawn, self._reopen_mic()),
                                 keyboard=self.set_keyboard, before_power=self._before_power,
                                 airpods=self.connect_airpods, history=self.history, inbox=self.inbox,
                                 announce=lambda text: self.loop.call_soon_threadsafe(self._announce, text),
                                 schedule=self.schedule, on_schedule_changed=self._schedule_changed,
                                 stop_ringing=self._stop_ringing_from_tool, on_night_light=self.apply_night_light,
                                 lock=self.lock_action, asked=lambda: self.last_asked)
        specs, policies, self.tool_names = T.build(self.ctx)
        self.gate = PermissionGate(self.paths, self.journal, self.confirm, policies)
        self.model = ModelService(self.paths, s, on_progress=self._model_progress)
        self.ctx.model_status = self._model_status
        self.brain = Brain(self.paths, s, self.state, self.model, specs, self.ctx.tool_fns, self.gate,
                           self._system_prompt,
                           BrainEvents(on_turn_start=self._on_turn_start, on_text=self._on_text,
                                       on_activity=self._on_activity, on_turn_end=self._on_turn_end,
                                       on_tool=self._on_tool),
                           env=self.extra_env, resolve=self.ctx.resolve)
        self._spawn(self._warm_up(wait_for_files=True))

        if self.offer_headphones:
            self._airpods_start = self._spawn(self._airpods_quietly())     # first: no question, and quick
        if self.voice_enabled:
            if self._voice_models_missing():
                # First start: the voice is downloaded once, in the background.
                self._spawn(self._fetch_voice_then_start())
            else:
                await self._start_voice()
        self._spawn(self._housekeeping())
        self._time_wake = asyncio.Event()
        self._spawn(self._keep_time())
        self._spawn(self._watch_hardware())
        self._spawn(self._keep_night_light())
        self.screen.state("listening")
        self.ready.set()

        if not self.model.ready:
            m = self.model.model
            msg = (f"Getting my model ready ({m.label}, which runs on this computer): a one-time download of about "
                   f"{m.download_bytes / 1e9:.0f} GB from its makers, then a few minutes to set it up. Until then I can "
                   f"only do simple things like Wi-Fi, volume and switching to {self.platform.other_os}.")
            self.screen.notice(msg)
            self._spawn(self._say_on_headphones(msg))
        elif self.model.serving() != self.model.model or self.model.store.outdated(self.model.model):
            m = self.model.model
            if self.model.serving() != m:
                # A bigger model than the one here suits this computer: it's got ready in the background, and the
                # one here answers meanwhile.
                self.screen.notice(f"Getting a better model ready in the background ({m.label}, a download of about "
                                   f"{m.download_bytes / 1e9:.0f} GB). {self.model.label} answers until then.")
            else:
                # Its file was made before the engine kept the part that sees: it's made again in the background,
                # and it answers meanwhile.
                self.screen.notice(f"Making {m.label} again so it can see pictures (its file was made before it "
                                   f"could): a download of about {m.download_bytes / 1e9:.0f} GB, in the background. "
                                   f"It answers meanwhile.")
            self.model.prepare_in_background()
            if self.greet and self.voice_enabled and not self.locked:
                first = (s.user_name or "").split(" ")[0]
                self._spawn(self._say_on_headphones(f"Hi{' ' + first if first else ''}. I'm listening."))
        elif self.greet and self.voice_enabled and not self.locked:
            first = (s.user_name or "").split(" ")[0]
            self._spawn(self._say_on_headphones(f"Hi{' ' + first if first else ''}. I'm listening."))
        if self.offer_headphones or self.offer_updates:
            self._spawn(self._start_questions())

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
            # The system prompt says where the other system's files are: give the drive a few seconds if the
            # computer is still mounting it (if someone talks first, the conversation starts without waiting).
            with contextlib.suppress(asyncio.TimeoutError):
                await asyncio.wait_for(self._files_settled.wait(), 10)
        self._engine_had_files = self._other_os_files() is not None
        try:
            await self.brain.connect()
        except Exception as exc:
            log.info("the model isn't ready yet: %s", exc)

    def _model_progress(self, p: Progress) -> None:
        """How getting the model ready goes (from its own thread): shown now and then, and when it's done."""
        if self.loop is None:
            return

        def show() -> None:
            now = time.monotonic()
            if p.stage == "ready":
                self.screen.notice(f"My model is ready: {self.model.model.label}, on this computer. Ask me anything.")
                self._spawn(self._warm_up())
            elif p.stage == "failed":
                self.screen.notice("I couldn't get my model ready: " + p.detail + ". I'll try again when you next "
                                   "ask me something.", "warn")
            elif now - self._model_told > 60:
                self._model_told = now
                self.screen.notice(f"Getting my model ready: {p.words()}.")
        self.loop.call_soon_threadsafe(show)

    def _model_status(self) -> dict[str, Any]:
        m = self.model
        if m is None:
            return {}
        out = {"model": m.label, "ready": m.ready, "runs_on": "this computer (Chat Computer's own engine)"}
        serving = m.serving()
        if serving is not None:
            out["refusals_taken_out"] = m.store.uncensored(serving)
        if m.serving() != m.model:
            out["getting_ready"] = f"{m.model.label}: {m.progress.words()}"
        return out

    def _other_files_pending(self) -> bool:
        """Whether the other system's drive may still be on its way: the computer mounts it (Windows' drive, or
        the one shared with macOS) alongside Chat Computer's start, so for the first minutes Chat Computer can be first."""
        if not self.platform.just_started():
            return False
        if self.platform.name == "mac":
            from .platform.linux import MAC_FILES
            p = os.path.expanduser(MAC_FILES)
            return os.path.isdir(p) and not os.path.ismount(p)
        return bool(self.settings.windows_user) and not os.path.ismount(self.settings.windows_mount)

    async def _take_in_other_files(self) -> None:
        """The other system's drive arrived after Chat Computer started: add its files to the search (the next
        conversation's system prompt says where they are)."""
        while self._other_files_pending():
            await asyncio.sleep(0.5)
        self._files_settled.set()
        other = self._other_os_files()
        if not other:
            return
        log.info("the other system's files arrived after the start: %s", other)
        if self.index is not None:
            self.index.add_root("mac" if self.platform.name == "mac" else "windows", other)

    async def stop(self) -> None:
        with contextlib.suppress(Exception):
            await self._copy_logs()
        if self.index:
            self.index.stop()
        if self.mic:
            self.mic.close()
        if self.inbox is not None:
            with contextlib.suppress(Exception):
                await self.inbox.stop()
        if self.brain:
            await self.brain.interrupt()
            await self.brain.close()
        if self.model:
            await self.model.stop()

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
        msg = f"Hi{' ' + first if first else ''}. My voice is ready. Just talk to me."
        self.screen.notice(msg)
        await self._say_on_headphones(msg)

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
            wake = (self.settings.wake_words or ["computer"])[0].capitalize()
            hint = ", ".join(x for x in (wake, self.settings.user_name, "YouTube") if x)
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
                               "Run: chatcomputer models --download", "warn")
            return
        self.listener = Listener(vad, stt, self.stt_exec,
                                 ListenerEvents(self._on_speech_start, self._on_partial, self._on_utterance,
                                                self._on_barge_in, self._on_discard),
                                 is_speaking=lambda: bool(self.speaker and self.speaker.speaking),
                                 barge_in=self._barge_in_allowed)
        self.listener.attach(self.loop)
        if self.settings.barge_in == "auto":
            self._on_headphones()           # checked now, so it's known before Chat Computer first talks
        if self._stt is None:
            self._spawn(self._warm_up_hearing(stt))
        if self.mic_factory is None:
            from .voice.audio import Mic
            self.mic = Mic(self.listener.push_threadsafe)
        else:
            self.mic = self.mic_factory(self.listener.push_threadsafe)
        if not self._mic_open():
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
            if not self.mic_released and self._mic_open():
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
        """Whether sound plays on headphones (then talking over Chat Computer stops it). The listener asks on every
        32 ms frame while Chat Computer talks, and the check (wpctl) takes about a quarter of a frame, on the thread
        that also hears and shows everything: so the answer is the last check's, made again in the background
        when it's a few seconds old. Until the first, loudspeakers (Chat Computer doesn't hear itself)."""
        known = self._headphones
        if (known is None or time.monotonic() - known[0] > HEADPHONES_RECHECK_S) and not self._headphones_checking:
            self._headphones_checking = True
            self.loop.run_in_executor(None, self._check_headphones)
        return known[1] if known else False

    def _check_headphones(self) -> None:
        try:
            on = bool(self.platform.output_is_headset())
        except Exception:
            on = True       # can't tell: talking over Chat Computer still works
        self._headphones = (time.monotonic(), on)
        self._headphones_checking = False

    # ================================================================ helpers
    def _windows_home(self) -> str | None:
        wu = self.settings.windows_user
        base = os.path.join(self.settings.windows_mount, "Users")
        if wu and os.path.isdir(os.path.join(base, wu)):
            return os.path.join(base, wu)
        return None

    def _other_os_files(self) -> str | None:
        """The other system's files: Windows' home folder on a PC; on a Mac, the drive with what Chat Computer Setup
        sent over from macOS (macOS's own files can't be opened from here: Apple silicon keeps them encrypted with
        keys in the Secure Enclave, FileVault or not, which Linux can't use yet)."""
        if self.platform.name == "mac":
            from .platform.linux import MAC_FILES
            p = os.path.expanduser(MAC_FILES)
            return p if os.path.isdir(p) else None
        return self._windows_home()

    def _system_prompt(self) -> str:
        return build_system_prompt(self.settings, self.memory.for_prompt(), os.path.expanduser("~"),
                                   self._other_os_files(), platform=self.platform.name,
                                   model=self.model.label if self.model else "Qwen")

    def _say(self, text: str) -> None:
        if self.speaker:
            self.speaker.say(text)

    def _announce(self, text: str) -> None:
        """Shown, and said unless the request was typed, right away in the middle of a tool: the code to type on a
        keyboard being paired."""
        self.screen.notice(text)
        if not self._typing_now():
            self._say(text)

    async def _say_on_headphones(self, text: str) -> None:
        """Something Chat Computer says without being asked (a greeting): out loud only on headphones, like AirPods.
        Through the PC's or the monitor's speakers it is only shown."""
        if not self.speaker:
            return
        try:
            private = await self.loop.run_in_executor(None, self.platform.output_is_headset)
        except Exception:
            private = False
        if private:
            self._say(text)

    def _transcript(self, role: str, text: str, via: str | None = None, msg_id: str | None = None,
                    ts: float | None = None) -> None:
        """`msg_id`: the message's id on the screen, so Actions can find it again (an earlier day's too)."""
        if not text.strip():
            return
        item: dict[str, Any] = {"ts": ts or time.time(), "role": role, "text": text}
        if msg_id:
            item["id"] = msg_id
        if via:
            item["via"] = via
        item["chat"] = self.chats.current
        if role == "user" and text.strip():
            self.chats.said(text)
            self.screen.chats(self.chats.shown(), self.chats.current)
        try:
            with open(self.paths.transcript, "a", encoding="utf-8") as f:
                f.write(json.dumps(item, ensure_ascii=False) + "\n")
        except OSError:
            pass

    def _show_history(self) -> None:
        # Only the end of the transcript is read: it holds every conversation, and grows by the day.
        self.screen.chats(self.chats.shown(), self.chats.current)
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
        # Only the chat on the screen's (and from before chats, what was said, while it has nothing of its own), and
        # only recent history (same day).
        here = self.chats.current
        untitled = not self.chats.on_screen().get("title")
        items = [i for i in items if i.get("chat", here if untitled else None) == here]
        items = [i for i in items if time.time() - i["ts"] < 12 * 3600]
        if items:
            self.screen.history(items)

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

    async def handle(self, text: str, typed: bool = False) -> None:
        """Everything the user says arrives here, and everything typed on this PC's keyboard (`typed`): that is
        answered in writing only, never out loud."""
        text = text.strip()
        if not text:
            return
        log.debug("heard %r (asleep=%s, busy=%s)", text, self.asleep, bool(self.brain and self.brain.busy))
        self.last_heard = time.time()
        self.screen.partial("")

        if self.ringing is not None:
            # Whatever they say or type stops a timer or alarm that's ringing; "snooze", "stop", "okay thanks"
            # are about it and go no further, anything else is a request as usual.
            item = self._stop_ringing()
            if item and (answers.is_snooze(text) or answers.stops_ringing_only(text)):
                uid = "u" + uuid.uuid4().hex[:8]
                self.screen.user_text(uid, text, True)
                self._transcript("user", text, msg_id=uid)
                if answers.is_snooze(text) and item["kind"] == "alarm":
                    again = self.schedule.add("alarm", time.time() + SNOOZE_S, item.get("label", ""))
                    self._schedule_changed()
                    self._reply(f"Snoozed. I'll ring again at {sch.clock(again['at'])}.", typed)
                return

        if self.locked:
            # Locked: whoever is talking or typing here isn't listened to until the PIN is typed on the screen.
            log.info("locked: not listening to that")
            return

        if self.asleep:
            if typed:
                self.wake()          # typing wakes Chat Computer by itself: no wake word to type
            else:
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
            self._transcript("user", text, msg_id=uid)
            self._answer_typed = typed
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
            wake = (self.settings.wake_words or ["computer"])[0]
            if typed:
                self.screen.notice(f"Okay. Type, or say “{wake.capitalize()}”, when you need me.")
            else:
                self._say(f"Okay. Say {wake} when you need me.")
            await self._sleep_when_quiet()
            return

        address = answers.typed_address(text)
        if address:
            await self._open_address(uid, text, address)
            return

        if self.brain is None or not (self.model and self.model.ready):
            # (The model works without the internet: only while it's still being got ready is this needed.)
            self.screen.user_text(uid, text, True)
            await self._offline_command(text, signed_out=True, typed=typed)
            return

        if self.speaker:
            self.speaker.stop()
        prompt = self._prompt(uid, text)
        self.last_asked = text
        info = await self._screen_info()
        tid = await self.brain.ask(turn_prefix(info, self.brain.last_activity, typed=typed) + text, typed=typed)
        self._asked(tid, prompt)

    async def _open_address(self, uid: str, text: str, url: str) -> None:
        """A web address typed or said alone ("youtube.com", "open bbc.co.uk"): its site in the middle at once, as a
        browser's address bar would open it, without a turn of the model's (the next turn's note of the screen says
        what's open)."""
        prompt = self._prompt(uid, text)
        if self.speaker:
            self.speaker.stop()
        r = await self.screen.show_url(url)
        if not r.get("ok"):
            self.screen.notice(f"Couldn't open {url}: {r.get('error') or 'it didn’t load'}", "warn")
            return
        self.journal.record("screen", f"opened {r.get('url') or url}")
        d = describe_done("mcp__chatcomputer__show_website", {"url": url}, "")
        if d is not None:
            entry = self.history.add(d[0], tool="show_website", inp={"url": url}, minor=d[1], prompt=prompt)
            self.screen.done(for_screen(entry))

    def _prompt(self, uid: str, text: str, via: str | None = None) -> tuple[str, str, float]:
        """A message that asks the model for something: on the screen and in the transcript, with the same id and
        time, so what's done for it can be linked back to it (Actions)."""
        ts = time.time()
        self.screen.user_text(uid, text, True)
        self._transcript("user", text, via=via, msg_id=uid, ts=ts)
        self._last_prompt = (uid, text, ts)
        return self._last_prompt

    def _asked(self, tid: str, prompt: tuple[str, str, float] | None) -> None:
        if prompt is None:
            return
        self.prompt_of[tid] = prompt
        while len(self.prompt_of) > 500:
            self.prompt_of.pop(next(iter(self.prompt_of)))

    def _on_tool(self, tid: str, name: str, inp: dict[str, Any], ok: bool, result: str, use_id: str = "") -> None:
        """A tool call finished: what it did goes into Actions (if it worked and did something), with what it changed
        that Revert can take back (the journal's records of that call)."""
        if use_id and self.journal.call == use_id:
            self.journal.call = None        # (it's over: nothing recorded from now on is this call's)
        if not ok:
            return
        try:
            d = describe_done(name, inp, result)
        except Exception:
            log.exception("couldn't describe %s for Actions", name)
            return
        if d is None:
            return
        text, minor = d
        changes = self.journal.changes_of(use_id)
        entry = self.history.add(text, tool=name, inp=inp, minor=minor,
                                 prompt=self.prompt_of.get(tid) or self._last_prompt, changes=changes)
        if not minor:
            self.screen.done(for_screen(entry, (set(changes), set()) if changes else None))
        if name.endswith("undo_last_change"):
            self._spawn(self._show_done(states_only=True))     # what it undid shows as reverted

    def _change_state(self) -> tuple[set[str], set[str]]:
        """The journal's records still kept, and the changes among them taken back (for Revert, in Actions)."""
        records = self.journal.everything()
        return {e["id"] for e in records}, jn.undone_ids(records)

    async def _show_done(self, states_only: bool = False) -> None:
        """Actions, all of them (at the start); or, `states_only`, which can be reverted and which have been, on
        those already shown (after a revert, or "undo that")."""
        pinned = self.pins.of("done")
        entries = await self.loop.run_in_executor(None, lambda: self.history.recent(150, pinned, self._change_state()))
        if states_only:
            self.screen.done_states(entries)
        else:
            self.screen.done_list(entries)

    async def revert(self, done_id: str) -> None:
        """Revert, on an item in Actions: what that action changed is taken back, its last change first (files as
        they were, settings as they were: journal.py), and that is listed as an action of its own, which can be
        reverted in turn (putting the action back). The screen says what couldn't be taken back, and why."""
        entry = await asyncio.to_thread(self.history.get, done_id)
        records = await asyncio.to_thread(self.journal.everything)
        by_id, undone = {e["id"]: e for e in records}, jn.undone_ids(records)
        ids = (entry or {}).get("changes") or []
        todo = [by_id[c] for c in ids if c in by_id and c not in undone]
        if entry is None or not todo:
            why = ("It was already reverted." if ids and all(c in undone for c in ids) else
                   "That was too long ago to revert." if ids else "That can't be reverted.")
            self.screen.notice(f"Nothing reverted: {why}", "warn")
            await self._show_done(states_only=True)
            return
        call = "revert-" + uuid.uuid4().hex[:10]
        said: list[str] = []
        failed: list[str] = []
        token = jn.CALL.set(call)
        try:
            for e in reversed(todo):
                back = jn.TAKING_BACK.set(e["id"])
                try:
                    said.append(await self._take_back(e))
                    self.journal.note_undone(e, said[-1])
                except Exception as exc:          # CantUndo, or the file system's own error
                    log.info("couldn't take back %s: %s", e.get("summary"), exc)
                    failed.append(str(exc) or type(exc).__name__)
                finally:
                    jn.TAKING_BACK.reset(back)
        finally:
            jn.CALL.reset(token)
        if said:
            new = self.history.add(f"Reverted “{entry['text']}”", tool="revert", inp={"id": done_id},
                                   changes=self.journal.changes_of(call), reverts=done_id)
            self.screen.done(for_screen(new, (set(self.journal.changes_of(call)), set())))
        if failed:
            what = "Couldn't revert all of it" if said else f"Couldn't revert “{entry['text']}”"
            self.screen.notice(f"{what}: " + "; ".join(dict.fromkeys(failed))[:300], "warn")
        await self._show_done(states_only=True)

    async def _take_back(self, e: dict[str, Any]) -> str:
        """Takes back one change from the journal: a file's by the journal itself; a setting's by the tool call that
        puts it back; the memory's here. Says what it did; raises CantUndo (or OSError) when it can't."""
        u = e.get("undo") or {}
        kind = u.get("type")
        if kind == "tool":
            fn = self.ctx.tool_fns.get(str(u.get("tool") or "")) if self.ctx is not None else None
            if fn is None:
                raise CantUndo("that can't be changed from here")
            res = await fn(dict(u.get("input") or {}))
            text = result_text(res.get("content")).strip()
            if res.get("is_error") or FAILED.match(text):
                raise CantUndo(text or "it didn't work")
            return text
        if kind == "memory_forget":
            gone = self.memory.drop_lines(list(u.get("lines") or []))
            if not gone:
                raise CantUndo("that isn't remembered any more")
            self.journal.record_change(f"forgot {len(gone)} fact(s)", {"type": "memory_restore", "lines": gone})
            return "forgot " + "; ".join(ln[2:] for ln in gone)
        if kind == "memory_restore":
            back = self.memory.add_lines(list(u.get("lines") or []))
            if not back:
                raise CantUndo("that's remembered already")
            self.journal.record_change(f"remembered {len(back)} fact(s) again", {"type": "memory_forget", "lines": back})
            return "remembered again " + "; ".join(ln[2:] for ln in back)
        return await asyncio.to_thread(self.journal.undo_file, e)

    async def _new_chat(self) -> None:
        """+ in Chats: a new chat on the screen, and a fresh conversation with the engine."""
        if self.brain:
            await self.brain.new_conversation()
        self.chats.new()
        self.screen.clear()
        self.screen.chats(self.chats.shown(), self.chats.current)

    async def _open_chat(self, chat_id: str) -> None:
        """A chat in Chats clicked: its words back on the screen, and the engine carrying on its conversation."""
        chat = self.chats.open(chat_id)
        if chat is None:
            return
        if self.brain:
            await self.brain.switch_conversation(chat.get("session"))
        items = await asyncio.to_thread(self._chat_items, chat_id)
        if items:
            self.screen.history(items)
        else:
            self.screen.clear()
        self.screen.chats(self.chats.shown(), self.chats.current)

    def _chat_items(self, chat_id: str, most: int = 200) -> list[dict[str, Any]]:
        """What was said in a chat, from the transcript (its last `most` lines)."""
        out: list[dict[str, Any]] = []
        try:
            with open(self.paths.transcript, encoding="utf-8", errors="replace") as f:
                for line in f:
                    if chat_id not in line:
                        continue
                    try:
                        item = json.loads(line)
                    except ValueError:
                        continue
                    if isinstance(item, dict) and item.get("chat") == chat_id and isinstance(item.get("text"), str):
                        out.append(item)
        except OSError:
            pass
        return out[-most:]

    def page_event(self, msg: dict[str, Any]) -> None:
        """What the conversation page says was done on it (GtkScreen.PAGE_EVENTS): a pin, a message opened, an
        account to connect, a sign-in form filled in or cancelled."""
        kind = msg.get("type")
        if kind == "unlock":
            self._unlock(str(msg.get("pin") or ""))
            return
        if kind in ("account", "account_cancel") and msg.get("form") == "lock_pin":
            self._pin_form_done(msg.get("values") if kind == "account" and isinstance(msg.get("values"), dict) else None)
            return
        if self.locked:
            return
        if kind == "speak":
            # Read aloud, under a message: said now, in place of whatever was being said (even with speech off).
            text = str(msg.get("text") or "").strip()
            if text and self.speaker is not None and self.speaker.tts is not None:
                self.speaker.stop()
                was, self.speaker.muted = self.speaker.muted, False
                self.speaker.say(text)
                self.speaker.muted = was
            elif text:
                self.screen.notice("Reading aloud needs Chat Computer's voice, which isn't ready yet.")
            return
        if kind == "feedback":
            # Good or bad, under an answer: kept with the question and the answer (~/ChatComputer/feedback.jsonl),
            # for finding what to make better.
            entry = {"ts": round(time.time(), 1), "good": bool(msg.get("good")), "on": bool(msg.get("on")),
                     "id": str(msg.get("id") or ""), "prompt": str(msg.get("prompt") or "")[:4000],
                     "answer": str(msg.get("text") or "")[:8000]}
            try:
                with open(self.paths.root / "feedback.jsonl", "a", encoding="utf-8") as f:
                    f.write(json.dumps(entry, ensure_ascii=False) + "\n")
            except OSError:
                log.exception("couldn't keep the feedback")
            return
        if self._search_event(msg):
            return
        if kind == "text_scale":
            # Two fingers pinched on the conversation: its text that much bigger or smaller (as zooming does on an
            # iPad), and kept.
            try:
                factor = float(msg.get("factor") or 1)
            except (TypeError, ValueError):
                return
            new = min(3.0, max(0.5, round(self.settings.text_scale * min(4.0, max(0.25, factor)), 2)))
            if new != self.settings.text_scale:
                self.settings.text_scale = new
                self.settings.save(self.paths)
                self._settings_changed(["text_scale"])
            return
        if kind == "new_chat":
            self._spawn(self._new_chat())
            return
        if kind == "open_chat":
            self._spawn(self._open_chat(str(msg.get("id") or "")))
            return
        if kind == "mic_toggle":
            # The microphone button under the message box: listening (awake) or not (asleep: only the wake word, an
            # AirPod's press or typing wakes it), as asked.
            if self.asleep:
                self.wake()
            else:
                self._spawn(self._sleep_when_quiet())
            return
        if kind == "panels":
            # A sidebar's switch pressed: kept for the next start.
            changed = [f"{k}_panel" for k in ("left", "right")
                       if isinstance(msg.get(k), bool) and msg[k] != getattr(self.settings, f"{k}_panel")]
            for key in changed:
                setattr(self.settings, key, not getattr(self.settings, key))
            if changed:
                self.settings.save(self.paths)
            return
        if kind == "pin":
            which, item = str(msg.get("list") or ""), str(msg.get("id") or "")
            self.screen.set_pinned(which, item, self.pins.set(which, item, bool(msg.get("on"))))
        elif kind == "revert":
            self._spawn(self.revert(str(msg.get("id") or "")))
        elif self.inbox is None:
            return
        elif kind == "open_message":
            self._spawn(self.inbox.open(str(msg.get("id") or "")))
        elif kind == "add_account":
            self._spawn(self.inbox.connect(msg.get("provider"), msg.get("group")))
        elif kind == "account":
            values = msg.get("values") if isinstance(msg.get("values"), dict) else {}
            self._spawn(self.inbox.form_submit(str(msg.get("form") or ""), values))
        elif kind == "account_cancel":
            self.inbox.form_cancel(str(msg.get("form") or ""))

    def _search_event(self, msg: dict[str, Any]) -> bool:
        """ "/" in the message box (page_event): searching this computer, or opening what was found."""
        kind = msg.get("type")
        if kind == "search":
            try:
                n = int(msg.get("n") or 0)
            except (TypeError, ValueError):
                n = 0
            self._spawn(self._search(str(msg.get("q") or "")[:200], n))
        elif kind == "open_file":
            self._spawn(self._open_file(str(msg.get("path") or "")))
        else:
            return False
        return True

    # ================================================================ "/" in the message box
    async def _search(self, query: str, n: int) -> None:
        """What on this computer matches what's typed after "/"."""
        items, note = await self.loop.run_in_executor(None, slash_search.search, self.index, query, self.paths)
        self.screen.search_results(n, query, items, note)

    async def _open_file(self, path: str) -> None:
        """A file chosen from "/": shown between the sidebars, as show_file would."""
        from .ui import render
        if not path:
            return
        if is_private(self.paths, path):
            self.screen.notice("That file holds Chat Computer's sign-ins, so it stays private.", "warn")
            return
        if not os.path.exists(path):
            self.screen.notice("That file isn't there any more.", "warn")
            return
        kind, payload = await self.loop.run_in_executor(None, render.file_view, path, self.settings, self.paths)
        r = await (self.screen.show_url(payload) if kind == "url" else
                   self.screen.show_html(payload, os.path.basename(path)))
        if not r.get("ok"):
            self.screen.notice(f"Couldn't show {os.path.basename(path)}: {r.get('error') or 'unknown error'}", "warn")
            return
        self.journal.record("screen", f"showed {path}")

    async def go_back_to(self, target_id: str | None, target_ts: float, before_ts: float | None) -> None:
        """Actions: an item was tapped whose message isn't on the screen any more. The conversation from that
        message on is put back, and the screen scrolls to it."""
        items = await self.loop.run_in_executor(None, self.history.earlier, target_id, target_ts, before_ts)
        self.screen.earlier(items, target_id, target_ts)

    async def _screen_info(self) -> dict[str, Any]:
        try:
            info = await asyncio.wait_for(self.screen.content_info(), 2)
        except Exception:
            info = {"mode": "chat"}
        info["media_playing"] = self.media_playing
        return info

    # ================================================================ brain events
    def _spoken(self, tid: str) -> bool:
        """Replies to what was typed on this PC's keyboard are shown, not spoken. Only what was said out loud gets a
        spoken answer."""
        if not self.speaker or not self.brain:
            return bool(self.speaker)
        return not self.brain.typed_here(tid)

    def _typing_now(self) -> bool:
        """The turn under way was typed on this PC's keyboard."""
        return bool(self.brain and self.current_turn and self.brain.typed_here(self.current_turn))

    def _on_turn_start(self, tid: str) -> None:
        self.current_turn = tid
        self.turn_text[tid] = ""
        self.screen.ai_delta(tid, "")
        if self._spoken(tid):
            self.speaker.begin()
        self._set_mode("thinking")

    def _on_text(self, tid: str, delta: str) -> None:
        self.turn_text[tid] = self.turn_text.get(tid, "") + delta
        self.screen.ai_delta(tid, delta)
        if self._spoken(tid):
            self.speaker.feed(delta)

    def _show_words(self, tid: str, said: str, res: Any) -> None:
        """A question typed here, answered only in words, while something is open: the answer isn't on the screen
        (what's open has the middle, and the conversation is put away) and isn't heard (a typed question gets a
        written answer), so the conversation comes back to show it, leaving music playing, as show_conversation
        does. A turn that worked on what's open (scrolled it, played it, opened something) leaves it there."""
        if not said.strip() or not self.brain or not self.brain.typed_here(tid):
            return
        if getattr(self.screen, "mode", "chat") != "content":
            return
        if any(str(name).split("__")[-1] in SCREEN_TOOLS for name in getattr(res, "tools", None) or []):
            return
        self._spawn(self.screen.hide_content())

    def _on_activity(self, tid: str, text: str) -> None:
        if self.settings.show_activity:
            self.screen.activity(tid, text)
        else:
            # What the model says after using a tool starts a new paragraph.
            said = self.turn_text.get(tid, "")
            if said and not said.endswith("\n"):
                self.turn_text[tid] = said + "\n\n"
                self.screen.ai_delta(tid, "\n\n")
        # Separate what the model said before and after a tool, for speech.
        if self._spoken(tid):
            self.speaker.end()
            self.speaker.begin()

    def _on_turn_end(self, tid: str, res: TurnResult) -> None:
        self.screen.ai_done(tid)
        spoken = self._spoken(tid)
        if spoken:
            self.speaker.end()
        said = self.turn_text.pop(tid, "") or ""
        self._transcript("assistant", said, msg_id=tid)
        if self.brain:
            self.chats.session(getattr(self.brain, "session_id", None))   # (what opening this chat again carries on)
        self._show_words(tid, said, res)
        if not res.ok and not res.interrupted:
            msg = friendly_error(res, self.model.progress.words() if self.model else None)
            self.screen.notice(msg, "warn")
            if spoken:
                self._say(msg)
        if tid == self.current_turn:
            self.current_turn = None
        if self._sleep_after_turn:
            self._sleep_after_turn = False
            self._spawn(self._sleep_when_quiet())
        elif not (self.speaker and self.speaker.speaking):
            self._set_mode("listening")

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

    # ================================================================ the onscreen keyboard
    async def set_keyboard(self, show: bool) -> str:
        """The onscreen keyboard up or down: asked for, or its own hide key. Its keys are tapped on the
        touchscreen."""
        spec = await self.screen.show_keyboard(show)
        self._keyboard_moved(spec if show else None)
        if self.keyboard_up:
            return "The keyboard is up: tap its keys on the screen."
        return "The keyboard is away." if not show else "The keyboard couldn't be shown on this screen."

    def _keyboard_moved(self, spec: tuple | None) -> None:
        """The onscreen keyboard came up, moved or went (from the window's thread too)."""
        self.keyboard_up = spec is not None

    async def _keyboard_action(self, action: tuple) -> None:
        """A key of the onscreen keyboard (tapped on the touchscreen): typed where keys go (the conversation's box,
        or the page's box being typed in), in the order they were typed."""
        if self._typing is None:
            self._typing = asyncio.Lock()
        async with self._typing:
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
                log.warning("onscreen keyboard: %s couldn't be typed: %s", action, exc)

    # ================================================================ yes / no
    async def confirm(self, question: str, typed: bool | None = None) -> tuple[str, str]:
        """Ask; returns ('yes'|'no'|'unclear'|'timeout', the answer). The question is shown, and said out loud
        unless it belongs to something typed (`typed`, or by default the turn under way). The answer can be said
        or typed on this PC's keyboard."""
        if typed is None:
            typed = self._typing_now()
        hearing = bool(self.listener and self.mic and self.mic.is_open)
        speak = not typed and hearing
        assert self.confirm_lock is not None
        async with self.confirm_lock:
            said_all = []
            for attempt in range(2):
                q = question if attempt == 0 else "Sorry, was that a yes or a no?"
                if not hearing or typed:
                    q += " (Type yes or no.)"
                if self.speaker and speak:
                    await self.speaker.until_quiet(15)
                qid = "q" + uuid.uuid4().hex[:8]
                self.screen.ai_delta(qid, q)
                self.screen.ai_done(qid)
                if speak:
                    self._say(q)
                self.pending_confirm = self.loop.create_future()
                if self.listener:
                    self.listener.mode = "confirm"
                if self.asleep:
                    self.wake()
                try:
                    said = await asyncio.wait_for(self.pending_confirm, 40 if speak else 120)
                except asyncio.TimeoutError:
                    self.screen.notice("No answer, so I didn't do it.")
                    return "timeout", ""
                finally:
                    self.pending_confirm = None
                    if self.listener:
                        self.listener.mode = "normal"
                said_all.append(said)
                verdict = answers.classify_answer(said)
                if verdict != "unclear":
                    return verdict, said
                if len(said.split()) > 3:  # a real instruction, not a mumble: hand it to the model
                    return "unclear", said
            return "unclear", " / ".join(said_all)

    # ================================================================ questions at the start
    async def _start_questions(self) -> None:
        """A newer Chat Computer to update to, and AirPods, one question at a time. The update comes first when Windows'
        drive is already there: yes restarts Chat Computer, which would otherwise ask about the AirPods twice. While
        the drive is still on its way (the computer has just started), the AirPods come first."""
        await asyncio.sleep(START_QUESTIONS_DELAY_S)
        there = lambda: self.loop.run_in_executor(None, update.source_there)  # noqa: E731
        looked = False
        if self.offer_updates and await there():
            looked = True
            if await self._offer_update():
                return
        if self.offer_headphones:
            await self._offer_airpods()
        if self.offer_updates and not looked:
            end = time.monotonic() + UPDATE_DRIVE_WAIT_S
            while self.platform.just_started() and time.monotonic() < end and not await there():
                await asyncio.sleep(2)
            await self._offer_update()

    async def _offer_update(self) -> bool:
        """Offers the other version of Chat Computer in the folder Chat Computer Setup ran from, if there is one (update.py),
        on the screen. True when Chat Computer is restarting into it."""
        offer = await self.loop.run_in_executor(None, update.check)
        if offer is None:
            return False
        log.info("another version of Chat Computer is in %s (%s; %d files differ%s)", offer.source,
                 offer.version or "no version number", offer.changed,
                 "; it needs other Python packages" if offer.needs_setup else "")
        if offer.needs_setup:
            self.screen.notice("There's a new version of Chat Computer in the folder you ran Chat Computer Setup from. It needs "
                               f"Chat Computer Setup to install it: start {self.platform.other_os} and run Chat Computer Setup "
                               "again.")
            return False
        named = f" ({offer.version})" if offer.version and offer.version != __version__ else ""
        verdict, said = await self.confirm(f"There's a new version of Chat Computer{named} in the folder you ran Chat Computer "
                                           "Setup from. Update now? Chat Computer restarts, which takes a few seconds.",
                                           typed=True)
        if verdict != "yes":
            if verdict == "unclear" and len(said.split()) > 3:
                self._spawn(self.handle(said, typed=self._answer_typed))   # a request, not an answer
            elif verdict != "timeout":
                self.screen.notice("Okay. I'll ask again the next time Chat Computer starts.")
            return False
        self.screen.notice("Updating Chat Computer…")
        res = await self.loop.run_in_executor(None, self.platform.update_app)
        if not res.get("ok"):
            log.warning("the update didn't work: %s", res)
            self.screen.notice("The update didn't work: " + str(res.get("error") or "no reason given"), "warn")
            return False
        if not res.get("changed"):
            self.screen.notice("This Chat Computer is already that version.")
            return False
        log.info("Chat Computer updated (%s files changed); starting again", res.get("changed"))
        self.screen.notice("Updated. Chat Computer is starting again…")
        await asyncio.sleep(1.5)
        self.restart()
        return True

    def restart(self) -> None:
        """Ends Chat Computer so that the session starts it again at once (exit code 3)."""
        self.exit_code = 3
        self.screen.quit()

    # ================================================================ AirPods
    @staticmethod
    def _is_airpods(d: dict[str, Any]) -> bool:
        return "airpods" in str(d.get("name", "")).lower()

    @staticmethod
    def _unnamed(d: dict[str, Any]) -> bool:
        """A device BlueZ knows only by its address (no name yet): it lists those by the address itself."""
        name = str(d.get("name", "")).strip().replace("-", ":").upper()
        return not name or name == str(d.get("address", "")).upper()

    def _bt(self, fn: Callable, *a: Any) -> Any:
        return self.loop.run_in_executor(None, fn, *a)

    async def _airpods_candidates(self, copy_from_other: bool = True) -> list[dict[str, Any]]:
        """AirPods paired here, else the ones paired in Windows, copied over now: Chat Computer Setup told the person it
        would take over that pairing, so it isn't asked about. Devices BlueZ has no name for yet (copied over
        before Chat Computer brought names along) come last: one of them may be the AirPods."""
        p = self.platform
        st = await self._bt(p.bt_status)
        paired = st.get("paired", [])
        found = [d for d in paired if self._is_airpods(d)]
        if not found and copy_from_other and p.name != "mac":
            try:
                log.info("AirPods: %s", await self._bt(p.bt_import_windows_keys))
            except Exception:
                log.info("couldn't copy the pairings from Windows", exc_info=True)
            paired = (await self._bt(p.bt_status)).get("paired", [])
            found = [d for d in paired if self._is_airpods(d)]
        return found + [d for d in paired if self._unnamed(d) and d not in found]

    async def _connect_one_of(self, devices: list[dict[str, Any]]) -> str | None:
        """Connects the first of these AirPods that answers (out of their case). Their name, or None."""
        p = self.platform
        for d in devices:
            try:
                await self._bt(p.bt_connect, d["address"])
                st = await self._bt(p.bt_status)
            except Exception:
                log.info("connecting %s failed", d.get("address"), exc_info=True)
                continue
            for c in st.get("connected", []):
                if c.get("address") == d["address"] and (self._is_airpods(c) or self._is_airpods(d)):
                    return str(c.get("name") if self._is_airpods(c) else d.get("name")) or "AirPods"
        return None

    async def _airpods_connected(self) -> str | None:
        try:
            st = await self._bt(self.platform.bt_status)
        except Exception:
            return None
        for c in st.get("connected", []):
            if self._is_airpods(c):
                return str(c.get("name") or "AirPods")
        return None

    async def _airpods_quietly(self) -> str:
        """At the start: AirPods Chat Computer knows (paired here or in Windows) are connected without a question, as
        soon as they're out of their case: right away if they are, otherwise one line says so and Chat Computer keeps
        trying for two minutes. Returns 'connected', 'waiting', 'unknown' (AirPods known nowhere: asked about by
        _start_questions) or 'none' (sound already on headphones, or no Bluetooth)."""
        p = self.platform
        try:
            if await self._bt(p.output_is_headset) or await self._airpods_connected():
                return "none"
            if not (await self._bt(p.bt_status)).get("adapter", True):
                return "none"
            devices = await self._airpods_candidates()
        except Exception:
            log.info("no Bluetooth to connect AirPods with", exc_info=True)
            return "none"
        if not devices:
            return "unknown"
        name = await self._connect_one_of(devices)
        if name:
            self.screen.notice(f"Your {name} are connected.")
            return "connected"
        self.screen.notice("Take your AirPods out of their case: they'll connect by themselves.")
        self._spawn(self._keep_connecting(devices, AIRPODS_WAIT_S))
        return "waiting"

    async def _keep_connecting(self, devices: list[dict[str, Any]], wait_s: float) -> str | None:
        """Tries to connect these AirPods every few seconds until they answer or `wait_s` has passed (only then:
        trying all the time would take them from the person's phone)."""
        end = time.monotonic() + wait_s
        while time.monotonic() < end:
            await asyncio.sleep(AIRPODS_RETRY_S)
            try:
                if await self._bt(self.platform.output_is_headset):
                    name = await self._airpods_connected()
                    if name:
                        self.screen.notice(f"Your {name} are connected.")
                    return name           # connected some other way (or other headphones): done
            except Exception:
                pass
            name = await self._connect_one_of(devices)
            if name:
                self.screen.notice(f"Your {name} are connected.")
                return name
        self.screen.notice("Your AirPods didn't connect. Say or type “connect my AirPods” when you want them.")
        return None

    async def _offer_airpods(self) -> None:
        """The AirPods question at the start, only when Chat Computer knows no AirPods (paired neither here nor in
        Windows): connect them? Yes: they're paired here, with the button on their case."""
        state = await self._airpods_start if self._airpods_start is not None else await self._airpods_quietly()
        if state != "unknown":
            return
        verdict, said = await self.confirm("Connect your AirPods?", typed=True)
        if verdict != "yes":
            if verdict == "unclear" and len(said.split()) > 3:
                self._spawn(self.handle(said, typed=self._answer_typed))   # a request, not an answer
            elif verdict != "timeout":
                self.screen.notice("Okay. Say or type “connect my AirPods” whenever you want them.")
            return
        name = await self._pair_new_airpods()
        if name:
            self.screen.notice(f"Your {name} are connected.")
        else:
            self.screen.notice("I couldn't find the AirPods. Say or type “connect my AirPods” to try again.", "warn")

    async def _pair_new_airpods(self) -> str | None:
        self.screen.notice("Put the AirPods in their case with the lid open, then hold the button on the back of the "
                           "case until the light flashes white.")
        try:
            return await self._pair_airpods()
        except Exception:
            log.warning("pairing the AirPods failed", exc_info=True)
            return None

    async def _pair_airpods(self) -> str | None:
        """Looks for AirPods in pairing mode for up to a minute and pairs the first found. Their name, or None."""
        p = self.platform
        end = time.monotonic() + AIRPODS_PAIR_S
        while time.monotonic() < end:
            for d in await self._bt(p.bt_scan, 8):
                if self._is_airpods(d):
                    result = str(await self._bt(p.bt_pair, d["address"]))
                    log.info("AirPods pairing: %s", result)
                    st = await self._bt(p.bt_status)
                    if any(c["address"] == d["address"] for c in st.get("connected", [])):
                        return str(d.get("name") or "AirPods")
        return None

    async def connect_airpods(self) -> str:
        """"Connect my AirPods" (the bluetooth tool's airpods action): in one step, however they can be. Known here
        or in Windows: connected as they come out of their case (tried for a little while). Known nowhere: paired,
        with the button on their case."""
        p = self.platform
        if not (await self._bt(p.bt_status)).get("adapter", True):
            return "This computer has no Bluetooth."
        name = await self._airpods_connected()
        if name:
            return f"Your {name} are already connected."
        devices = await self._airpods_candidates()
        if devices:
            name = await self._connect_one_of(devices)
            if not name:
                self._announce("Take your AirPods out of their case.")
                end = time.monotonic() + AIRPODS_ASKED_S
                while not name and time.monotonic() < end:
                    await asyncio.sleep(AIRPODS_RETRY_S)
                    name = await self._connect_one_of(devices)
            if name:
                return f"Your {name} are connected."
            return ("Your AirPods are known here but didn't answer. Take them out of their case near the computer "
                    "(or put them in it with the lid open) and ask again.")
        self._announce("Put the AirPods in their case with the lid open, then hold the button on the back of the case "
                       "until the light flashes white.")
        name = await self._pair_airpods()
        return f"Your {name} are connected." if name else "I couldn't find AirPods in pairing mode."

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
        if self.pinlock.has_pin():
            self.lock_now()         # with a PIN, going to sleep locks the screen too

    def wake(self) -> None:
        if self.asleep:
            self.asleep = False
            self.last_heard = time.time()
            self.screen.state("listening")

    async def _copy_logs(self) -> None:
        """A copy of the logs on Windows' drive, for reading from Windows when something goes wrong (logcopy.py)."""
        if self.platform.name == "mac" or self.loop is None:
            return
        info = {
            "model": self._model_status(),
            "voice": bool(self.speaker), "microphone": bool(self.mic and self.mic.is_open),
        }
        where = await self.loop.run_in_executor(None, logcopy.copy_logs, self.paths.logs, self._windows_home(), info)
        if where:
            log.info("logs copied to Windows' drive: %s", where)

    async def _keep_update(self) -> None:
        """An update that has run this long is kept: the session no longer goes back to the version before."""
        if await self.loop.run_in_executor(None, update.pending):
            await self.loop.run_in_executor(None, self.platform.update_done)
            log.info("the updated Chat Computer started well; keeping it")

    async def _before_power(self) -> None:
        """Just before the computer restarts or turns off: the logs to Windows' drive, as they are now (Chat Computer is
        ended without time to do it then)."""
        try:
            await asyncio.wait_for(self._copy_logs(), 10)
        except Exception:
            log.info("couldn't copy the logs before the restart", exc_info=True)

    async def _housekeeping(self) -> None:
        started = time.monotonic()
        next_copy = started + LOG_COPY_FIRST_S
        kept = not self.offer_updates
        while True:
            await asyncio.sleep(15)
            if not kept and time.monotonic() - started >= update.HEALTHY_AFTER_S:
                kept = True
                try:
                    await self._keep_update()
                except Exception:
                    log.exception("keeping the update")
            if time.monotonic() >= next_copy:
                next_copy = time.monotonic() + LOG_COPY_EVERY_S
                try:
                    await self._copy_logs()
                except Exception:
                    log.exception("copying the logs to Windows' drive")
            mins = self.settings.auto_sleep_minutes
            idle = time.time() - self.last_heard
            busy = (self.brain and self.brain.busy) or (self.speaker and self.speaker.speaking) or self.pending_confirm
            if mins and mins > 0 and not self.asleep and not busy and not self.media_playing and idle > mins * 60:
                log.info("auto-sleep after %.0f min", idle / 60)
                await self._sleep_when_quiet()

    # ================================================================ the lock screen
    def has_lock_pin(self) -> bool:
        return self.pinlock.has_pin()

    def lock_now(self) -> bool:
        """Lock the screen (only with a PIN: without one, anyone could unlock it). Timers and alarms still ring; what
        the model opens stays off the screen until it's unlocked."""
        if not self.pinlock.has_pin():
            return False
        if not self.locked:
            log.info("locked")
            self.locked = True
            if self.pending_confirm is not None and not self.pending_confirm.done():
                self.pending_confirm.set_result("no")         # a question asked here goes unanswered: no
        self.screen.lock_screen({"note": "Type your PIN to unlock."})
        return True

    def _unlock(self, pin: str) -> None:
        if not self.locked:
            return
        ok, why = self.pinlock.check(pin)
        if ok:
            log.info("unlocked")
            self.locked = False
            self.last_heard = time.time()
            self.screen.lock_screen(None)
            if self.asleep:
                self.wake()
        else:
            log.info("wrong PIN on the lock screen")      # (never the PIN itself)
            self.screen.lock_screen({"error": why})

    def _show_pin_form(self, error: str = "") -> None:
        mode = (self._pin_form or {}).get("mode", "set")
        current = [{"name": "current", "label": "Your PIN now", "type": "password"}] if mode in ("change", "remove") else []
        if mode == "remove":
            form = {"id": "lock_pin", "title": "Turn off the PIN", "fields": current, "ok": "Turn off",
                    "text": ["The screen won't lock any more."]}
        else:
            form = {"id": "lock_pin", "title": "Choose a PIN" if mode == "set" else "Change the PIN", "ok": "Set PIN",
                    "fields": current + [{"name": "pin", "label": "New PIN", "type": "password"},
                                         {"name": "again", "label": "The same PIN again", "type": "password"}],
                    "text": ["4 to 12 digits, typed here: the screen asks for it to unlock. The model never sees it.",
                             "With a PIN, the screen locks when Chat Computer starts and when it goes to sleep."]}
        if error:
            form["error"] = error
        self.screen.account_form(form)

    def _pin_form_done(self, values: dict[str, Any] | None) -> None:
        """The PIN form filled in (values) or cancelled (None). The PINs in it go no further than here."""
        f = self._pin_form
        if f is None:
            self.screen.account_form(None)
            return
        if values is None:
            self._pin_form = None
            self.screen.account_form(None)
            return
        if f["mode"] in ("change", "remove"):
            ok, why = self.pinlock.check(str(values.get("current") or ""))
            if not ok:
                self._show_pin_form(why)
                return
        if f["mode"] == "remove":
            self.pinlock.remove_pin()
            self._pin_form = None
            self.screen.account_form(None)
            self.screen.notice("The PIN is off: the screen won't lock any more.")
            return
        pin, again = str(values.get("pin") or ""), str(values.get("again") or "")
        if not PIN_RE.match(pin):
            self._show_pin_form("A PIN is 4 to 12 digits.")
            return
        if pin != again:
            self._show_pin_form("The two PINs aren't the same. Type it twice.")
            return
        self.pinlock.set_pin(pin)
        self._pin_form = None
        self.screen.account_form(None)
        if f.get("then_lock"):
            self.lock_now()
        else:
            self.screen.notice("PIN set. The screen locks when Chat Computer starts and when it goes to sleep.")

    async def lock_action(self, action: str) -> str:
        """The lock_screen tool. The PIN is only ever typed on the screen, never through the model."""
        has = self.pinlock.has_pin()
        if action == "status":
            return (f"The screen is {'locked' if self.locked else 'unlocked'}. " +
                    ("A PIN is set: the screen locks when Chat Computer starts and when it goes to sleep, and they "
                     "unlock it by typing the PIN on it." if has else "No PIN is set, so the screen doesn't lock."))
        if action == "lock":
            if has:
                self.lock_now()
                return "Locked. Nothing can be used until the PIN is typed on the screen."
            self._pin_form = {"mode": "set", "then_lock": True}
            self._show_pin_form()
            return ("There's no PIN yet: a form to choose one is on the screen. They type it there (never aloud); the "
                    "screen locks as soon as it's set.")
        if action == "set_pin":
            self._pin_form = {"mode": "change" if has else "set", "then_lock": False}
            self._show_pin_form()
            return "A form to choose the PIN is on the screen: they type it there, twice (never aloud or to you)."
        if action == "remove_pin":
            if not has:
                return "No PIN is set."
            self._pin_form = {"mode": "remove", "then_lock": False}
            self._show_pin_form()
            return "A form is on the screen: they type their PIN there to turn it off."
        return f"Unknown action {action}."

    # ================================================================ timers, reminders, alarms
    def _schedule_changed(self) -> None:
        """(Any thread.) What rings next may have changed: the timekeeper looks again."""
        if self.loop is not None and self._time_wake is not None:
            self.loop.call_soon_threadsafe(self._time_wake.set)

    async def _keep_time(self) -> None:
        """Rings each timer, reminder and alarm when it's due. One that came due while the computer was off or
        asleep is shown as missed instead (ringing an alarm hours late would only confuse)."""
        while True:
            now = time.time()
            for item in self.schedule.due(now):
                self.schedule.rang(item["id"], now)
                if now - item["at"] > sch.LATE_S:
                    self._missed(item)
                else:
                    self._spawn(self._ring(item))
            nxt = self.schedule.next_at()
            wait = 30.0 if nxt is None else max(0.2, min(30.0, nxt - time.time()))
            self._time_wake.clear()
            with contextlib.suppress(asyncio.TimeoutError):
                await asyncio.wait_for(self._time_wake.wait(), wait)

    @staticmethod
    def _ring_text(item: dict[str, Any]) -> str:
        label = (item.get("label") or "").strip().rstrip(".")
        if item["kind"] == "timer":
            what = sch.say_duration(item.get("duration") or item["at"] - item["created"])
            return f"Your {what} timer is done" + (f": {label}." if label else ".")
        if item["kind"] == "reminder":
            return f"Reminder: {label}." if label else "Here's your reminder."
        return f"It's {sch.clock(item['at'])}" + (f": {label}." if label else ".")

    def _chatcomputer_says(self, text: str) -> None:
        """Something Chat Computer says by itself (a timer ringing): in the conversation, as its own message."""
        mid = "s" + uuid.uuid4().hex[:8]
        self.screen.ai_delta(mid, text)
        self.screen.ai_done(mid)
        self._transcript("assistant", text, msg_id=mid)

    def _reply(self, text: str, typed: bool) -> None:
        self._chatcomputer_says(text)
        if not typed:
            self._say(text)

    async def _ring(self, item: dict[str, Any]) -> None:
        """Ring: the chime and the words, again and again (a timer for a minute, an alarm for ten) until they say
        or type anything, or press an AirPod. A reminder rings once."""
        if self.ringing is not None:
            self._stop_ringing()
        self.ringing = item
        text = self._ring_text(item)
        if self.asleep:
            self.wake()
        self.screen.notice(text)
        self._chatcomputer_says(text)
        until = time.time() + RING_FOR_S[item["kind"]]
        rounds = 0
        try:
            from .voice.chime import chime
            sound = chime(item["kind"])
        except Exception:
            sound = None
        while self.ringing is item:
            if self.player is not None and sound is not None:
                self.player.play(sound)
            if rounds % 5 == 0:
                self._say(text)
            rounds += 1
            if time.time() >= until:
                break
            await asyncio.sleep(RING_EVERY_S[item["kind"]])
        if self.ringing is item:
            self.ringing = None

    def _stop_ringing(self) -> dict[str, Any] | None:
        item, self.ringing = self.ringing, None
        if item is not None:
            if self.speaker:
                self.speaker.stop()
            elif self.player is not None:
                self.player.stop()
        return item

    def _stop_ringing_from_tool(self) -> str:
        """(Any thread: a tool.)"""
        item = self.ringing
        if item is None:
            return "Nothing is ringing."
        self.loop.call_soon_threadsafe(self._stop_ringing)
        return f"Stopped the {item['kind']}."

    def _missed(self, item: dict[str, Any]) -> None:
        when = sch.clock(item["at"])
        if item["kind"] == "timer":
            text = f"Your timer finished at {when}, while the computer was asleep or off."
        elif item["kind"] == "reminder":
            text = f"A reminder from {when}, while the computer was asleep or off: {item.get('label') or ''}".rstrip(": ")
        else:
            text = f"Your alarm for {when} couldn't ring: the computer was asleep or off."
        self.screen.notice(text, "warn")
        self._chatcomputer_says(text)

    # ================================================================ drives, the keyboard, the night light
    async def _watch_hardware(self) -> None:
        """A drive plugged in opens by itself, as on Windows and macOS (one ejected here stays closed until it's
        unplugged), and a keyboard plugged in gets the keyboard layout they chose."""
        seen: set[str] | None = None
        devices: list[str] | None = None
        warned = False
        while True:
            try:
                ds = await self.loop.run_in_executor(None, self.platform.drives)
            except Exception as exc:
                log.debug("drives: %s", exc)
                ds = None
            if ds is not None:
                for d in ds:
                    if (seen is not None and d["id"] in seen) or d.get("mounted"):
                        continue
                    try:
                        await self.loop.run_in_executor(None, self.platform.open_drive, d["id"])
                        if seen is not None:
                            self.screen.notice(f"{d['name']} is plugged in: its files are ready.")
                    except Exception as exc:
                        log.info("couldn't open the drive %s: %s", d.get("name"), exc)
                        if "udisks2" in str(exc) and not warned:
                            warned = True
                            self.screen.notice(f"{d['name']} is plugged in, but this computer can't open drives "
                                               "yet: ask me to set that up.", "warn")
                seen = {d["id"] for d in ds}
            if self.settings.keyboard_layout:
                try:
                    now = await self.loop.run_in_executor(None, self.platform.input_devices)
                    if devices is not None and set(now) - set(devices):
                        await self.loop.run_in_executor(None, self.platform.set_keyboard_layout,
                                                        self.settings.keyboard_layout)
                    devices = now
                except Exception as exc:
                    log.debug("keyboards: %s", exc)
            await asyncio.sleep(DRIVE_CHECK_S)

    def _night_light_wanted(self, now: float | None = None) -> bool:
        s = self.settings
        if s.night_light == "on":
            return True
        if s.night_light != "evenings":
            return False
        import datetime
        try:
            a, b = sch.clock_time(s.night_light_from), sch.clock_time(s.night_light_until)
        except ValueError:
            a, b = sch.clock_time("21:00"), sch.clock_time("07:00")
        t = datetime.datetime.fromtimestamp(now if now is not None else time.time()).time()
        return (a <= t < b) if a < b else (t >= a or t < b)

    def apply_night_light(self) -> None:
        """(Any thread.) The night light as its setting says now: on, off, or by the clock in the evenings; set again
        if the screen lost it (after a sleep, or another monitor)."""
        want = (self._night_light_wanted(), int(self.settings.night_light_warmth or 50))
        lost = want[0] and bool(getattr(self.platform, "night_light_lost", lambda: False)())
        if want == self._night_applied and not lost:
            return
        if self._night_applied is None and not want[0]:
            self._night_applied = want         # it's off, as the screen starts: nothing to undo
            return
        self.platform.set_night_light(want[0], want[1])
        self._night_applied = want

    async def _keep_night_light(self) -> None:
        if self.settings.keyboard_layout:
            try:
                await self.loop.run_in_executor(None, self.platform.set_keyboard_layout, self.settings.keyboard_layout)
            except Exception as exc:
                log.info("couldn't set the keyboard layout %s: %s", self.settings.keyboard_layout, exc)
        while True:
            try:
                await self.loop.run_in_executor(None, self.apply_night_light)
            except Exception as exc:
                log.info("night light: %s", exc)
            await asyncio.sleep(60)

    # ================================================================ hardware events (from the UI)
    def media_key(self, key: str) -> None:
        """An AirPods stem press or a keyboard media key."""
        self.loop.call_soon_threadsafe(lambda: self._spawn(self._media_key(key)))

    async def _media_key(self, key: str) -> None:
        log.info("media key %s", key)
        if self.ringing is not None:
            self._stop_ringing()
            return
        if self.locked:
            return
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
        if playing and self.mic.is_open and release in ("on", "auto"):
            if release == "on":
                self._release_mic()
            else:
                self._spawn(self._release_mic_if_bluetooth())
        elif not playing and self.mic_released:
            self.mic_released = False
            self._mic_open()

    async def _release_mic_if_bluetooth(self) -> None:
        # Asking the sound system takes a moment: not on the loop, where it would hold up everything else.
        if await self.loop.run_in_executor(None, self._input_is_bluetooth) and self.media_playing and self.mic.is_open:
            self._release_mic()

    def _release_mic(self) -> None:
        # AirPods sound much better when nothing is using their microphone.
        self.mic.close()
        self.mic_released = True
        self.screen.mic(False)
        self.screen.notice("Enjoy. Press an AirPod to talk to me.")

    async def _reopen_mic(self) -> None:
        """Another microphone was chosen (the sound system's default input): the stream follows the default when
        it opens, so it's opened again."""
        if self.mic and self.mic.is_open:
            await self.loop.run_in_executor(None, self.mic.close)
            self._mic_open()

    def _mic_open(self) -> bool:
        """Open the microphone, and say on the screen whether Chat Computer can hear."""
        ok = bool(self.mic and self.mic.open())
        self.screen.mic(ok)
        return ok

    def _input_is_bluetooth(self) -> bool:
        try:
            return bool(self.platform.input_is_bluetooth())
        except Exception:
            return False

    # ================================================================ where they are, for the greeting
    def _say_here(self, look: bool = False) -> None:
        """Tell the screen where they are and the weather there (what's known now); looked up again, when `look` or
        it's old, where the weather is looked up at all."""
        s = self.settings
        place = self.here.place_for(s.home_place, s.timezone)
        known = self.here.known(place)
        if not (self.weather and (look or self.here.stale(place)) and self.loop):
            self.screen.here(known)
            return
        self.screen.here({**known, "pending": True})       # (a greeting about to be chosen waits a moment for it)

        async def fetch() -> None:
            info = await self.loop.run_in_executor(None, self.here.look_up, place)
            if place == self.here.place_for(self.settings.home_place, self.settings.timezone):
                self.screen.here(info or known)
        self._spawn(fetch())

    async def _weather_now_and_then(self) -> None:
        """Every half hour, so the next conversation's greeting has the weather as it is."""
        while True:
            await asyncio.sleep(30 * 60)
            self._say_here()

    def _settings_changed(self, keys: list[str], restart: bool = True) -> None:
        """Settings changed (by the model's chatcomputer_settings, or here). A new model is got ready (if it isn't
        yet) and used from the next question; effort applies from the next question by itself."""
        s = self.settings
        if {"text_size", "text_scale", "viewing_distance_cm", "screen_inches", "web_zoom",
            "user_name", "model"} & set(keys):
            self.screen.apply_settings(s.text_size, (s.user_name or "").split(" ")[0], model_label(s.model))
        if {"left_panel", "right_panel"} & set(keys):
            self.screen.panels(self._panels())
        if {"home_place", "timezone"} & set(keys):
            self._say_here(look=True)
        if "model" in keys and self.model and restart:
            self._spawn(self._switch_model_when_idle())
        if self.speaker and self.speaker.tts is not None:
            if "voice" in keys:
                self.speaker.tts.voice = s.voice
            if "speech_speed" in keys:
                self.speaker.tts.speed = s.speech_speed

    def _panels(self) -> dict[str, bool]:
        s = self.settings
        return {"left": s.left_panel, "right": s.right_panel}

    async def _switch_model_when_idle(self) -> None:
        """Another model was chosen: once this turn is over, its server stops and the new one is used (got ready
        first, if it isn't here yet). The conversation carries on."""
        assert self.brain is not None and self.model is not None
        await self.brain.wait_idle()
        await self.model.switch(self.settings.model)
        if self.model.serving() != self.model.model:
            self.model.prepare_in_background()
        await self._warm_up()

    # ================================================================ before the model is ready
    async def _offline_command(self, text: str, signed_out: bool, typed: bool = False) -> None:
        """A few things that work without the model (while it's still being got ready): Wi-Fi, volume, power. Typed
        requests get written answers."""
        t = answers.normalize(text)
        p = self.platform
        say = (lambda _text: None) if typed else self._say
        run = lambda fn, *a: self.loop.run_in_executor(None, fn, *a)
        reply = None
        # "connect to <network>" is a Wi-Fi request even without a Wi-Fi word in it (the offline reply tells
        # them to say exactly this), and the name keeps its original spelling (SSIDs are case-sensitive).
        joining = re.match(r"\s*connect to\s+(.+)", text.strip(), re.I)
        if joining or any(w in t for w in ("wifi", "wi fi", "internet", "network", "connected")):
            if joining:
                name = joining.group(1).strip(" .?!")
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
            verdict, _ = await self.confirm(f"Restart into {other}?", typed=typed)
            if verdict == "yes":
                say(f"Restarting into {other}.")
                await asyncio.gather(asyncio.sleep(3), self._before_power())
                reply = await run(p.power, "switch_to_macos" if p.name == "mac" else "switch_to_windows")
            else:
                reply = "Okay, staying here."
        elif any(w in t for w in ("shut down", "shutdown", "turn off", "power off")):
            verdict, _ = await self.confirm("Shut down the computer?", typed=typed)
            if verdict == "yes":
                say("Shutting down.")
                await asyncio.gather(asyncio.sleep(3), self._before_power())
                reply = await run(p.power, "shutdown")
        elif "restart" in t or "reboot" in t:
            verdict, _ = await self.confirm("Restart the computer?", typed=typed)
            if verdict == "yes":
                say("Restarting.")
                await asyncio.gather(asyncio.sleep(3), self._before_power())
                reply = await run(p.power, "restart")
        if reply is None:
            other = self.platform.other_os
            how = self.model.progress.words() if self.model else ""
            reply = (f"I'm still getting my model ready{': ' + how if how and how != 'waiting' else ''}, so for now I "
                     f"can only help with Wi-Fi, volume, restarting, or switching to {other}.")
            if self.model and not self.model.ready:
                self.model.prepare_in_background()      # (again, if it had stopped)
        qid = "o" + uuid.uuid4().hex[:8]
        self.screen.ai_delta(qid, reply)
        self.screen.ai_done(qid)
        say(reply)
