"""Paths and settings.

Everything Chat Computer keeps lives in one folder the user owns: ~/ChatComputer (override with
CHATCOMPUTER_HOME for tests). Settings are a small JSON file; unknown keys are kept so newer
versions can add settings without breaking older files.
"""
from __future__ import annotations

import json
import os
import tempfile
from dataclasses import asdict, dataclass, field, fields
from pathlib import Path
from typing import Any

# How hard the model thinks before it answers: "off" answers at once (quickest, for a quick question by voice); the
# rest think first, Qwen 3.8 as hard as said (its reasoning effort), the others as they do.
EFFORTS = ("off", "low", "medium", "high", "max")
OLD_CLAUDE_WAKE_WORDS = ["claude", "hey claude", "ok claude", "okay claude"]


def model_label(model: str | None) -> str:
    """The model's name as people read it ("Qwen3.6 35B-A3B"); for "auto", the one this computer gets."""
    from .brain.models import label
    return label(model)


class Paths:
    """All on-disk locations, derived from one root folder."""

    def __init__(self, root: Path | None = None) -> None:
        root = root or Path(os.environ.get("CHATCOMPUTER_HOME") or (Path.home() / "ChatComputer"))
        self.root = Path(root)
        self.secrets = self.root / "secrets"
        self.settings_file = self.root / "settings.json"
        self.state_file = self.root / "state.json"
        self.logs = self.root / "logs"
        self.models = self.root / "models"
        self.memory = self.root / "memory"
        self.memory_file = self.memory / "memory.md"
        self.pages = self.root / "pages"          # pages and apps made for you
        self.undo = self.root / "undo"            # backups that make file changes undoable
        self.journal = self.root / "journal.jsonl"  # what Chat Computer did, newest last
        self.transcript = self.root / "transcript.jsonl"
        self.done = self.root / "done.jsonl"      # Actions: what Chat Computer did for the person (done.py)
        self.index = self.root / "index"
        self.web = self.root / "web"              # the built-in browser's logins and cache
        self.sources = self.root / "sources"      # WhatsApp's and the social sites' sign-ins, for Messages
        self.engine = self.root / ".engine"       # the conversations with the model (brain/agent.py)

    def ensure(self) -> "Paths":
        for p in (self.root, self.logs, self.models, self.memory, self.pages, self.undo,
                  self.index, self.web, self.engine):
            p.mkdir(parents=True, exist_ok=True)
        self.secrets.mkdir(parents=True, exist_ok=True)
        try:
            os.chmod(self.secrets, 0o700)
        except OSError:
            pass
        return self

    def is_secret(self, path: str | os.PathLike[str]) -> bool:
        """True for anything inside the secrets folder (never shown to the model's tools)."""
        try:
            p = Path(os.path.realpath(os.path.expanduser(str(path))))
            s = Path(os.path.realpath(self.secrets))
            return p == s or s in p.parents
        except Exception:
            return False


@dataclass
class Settings:
    user_name: str = ""
    windows_user: str = ""
    timezone: str = ""
    locale: str = "en_US.UTF-8"
    home_place: str = ""            # the town or city they live in, for the greeting's weather (else the time zone's)
    # The model that thinks: Qwen's open models on this computer (brain/models.py); "auto" picks by its memory.
    model: str = "auto"
    effort: str = "medium"
    defaults_version: int = 3       # 3: settings from after Chat Computer's own model replaced Claude
    fresh_session_after_idle_hours: float = 8.0
    # Voice
    voice: str = "af_heart"
    speech_speed: float = 1.05
    stt_model: str = "small.en"
    stt_language: str = "en"
    wake_words: list[str] = field(default_factory=lambda: ["computer", "hey computer", "ok computer", "okay computer"])
    auto_sleep_minutes: int = 10
    barge_in: str = "auto"                  # auto | on | off
    release_mic_during_media: str = "auto"  # auto | on | off
    # Screen
    show_activity: bool = False
    # The screen is always light text on Claude's dark (ui/assets/theme.css); there is no theme to choose.
    # Text size and line width are worked out from the screen's real size (ui/layout.py).
    text_size: int = 0              # 0 = automatic; otherwise a fixed size in pixels
    text_scale: float = 1.0         # "bigger"/"smaller" on top of the automatic size
    viewing_distance_cm: int = 0    # 0 = estimated from the screen size
    screen_inches: float = 0.0      # 0 = what the monitor reports (set it if the monitor reports nothing)
    web_zoom: float = 0.0           # 0 = automatic zoom for websites
    # The sidebars, each with a switch on the screen: Messages (left) and Actions (right).
    left_panel: bool = True
    right_panel: bool = True
    # The screen itself: the night light (warmer colours): off, on, or evenings (by itself from night_light_from to
    # night_light_until), and how warm (1-100). The brightness is the screen's own (it keeps it).
    night_light: str = "off"
    night_light_from: str = "21:00"
    night_light_until: str = "07:00"
    night_light_warmth: int = 50
    # The keyboard's layout (XKB: us, gb, fr, de, fr(azerty)...); "" = as the computer was set up.
    keyboard_layout: str = ""
    # Files
    windows_mount: str = "/mnt/windows"
    extra_search_roots: list[str] = field(default_factory=list)
    # Messages (chatcomputer/messages): how often email is checked, and the Microsoft sign-in app for Outlook.com
    # (its application id, from registering an app in Microsoft Entra; see README). Social sites are read from their
    # own pages in Chat Computer's browser, read-only.
    mail_check_minutes: float = 2.0
    outlook_client_id: str = ""
    # Anything else found in the file is preserved here.
    extra: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def load(cls, paths: Paths) -> "Settings":
        data: dict[str, Any] = {}
        if paths.settings_file.exists():
            try:
                data = json.loads(paths.settings_file.read_text(encoding="utf-8")) or {}
            except (OSError, ValueError):
                data = {}
            if not isinstance(data, dict):
                data = {}        # a file that isn't a JSON object (hand-edited): the defaults, not a crash at start
        known = {f.name: f for f in fields(cls) if f.name != "extra"}
        if data.get("text_size") == 28 and "text_scale" not in data:
            data["text_size"] = 0   # 28 px was the fixed default before text was sized from the screen
        data.pop("theme", None)     # the old light/dark choice: the screen is black on off-white now
        data.pop("bottom_panel", None)  # the old switch for the conversation under what's open: nothing is under it now
        if int(data.get("defaults_version") or 0) < 3:
            # Settings from when Claude thought: its model and effort mean nothing to the own model, and its name
            # was the wake word.
            from .brain.models import names
            if data.get("model") not in names():
                data.pop("model", None)
            if data.get("effort") not in EFFORTS:
                data.pop("effort", None)
            if [str(w).lower() for w in data.get("wake_words") or []] == OLD_CLAUDE_WAKE_WORDS:
                data.pop("wake_words")
            data.pop("defaults_version", None)
        data.pop("phone_access", None)  # the Claude app on the phone, which only Claude's sign-in could reach
        kwargs: dict[str, Any] = {}
        extra: dict[str, Any] = {}
        defaults = cls()
        for k, v in data.items():
            if k in known:
                kwargs[k] = _coerce(v, getattr(defaults, k))
            else:
                extra[k] = v
        s = cls(**kwargs)
        s.extra = extra
        return s

    def save(self, paths: Paths) -> None:
        data = {k: v for k, v in asdict(self).items() if k != "extra"}
        data.update(self.extra)
        atomic_write(paths.settings_file, json.dumps(data, indent=2, ensure_ascii=False))

    def public(self) -> dict[str, Any]:
        return {k: v for k, v in asdict(self).items() if k != "extra"}

    def set(self, key: str, value: Any) -> None:
        if key not in {f.name for f in fields(self)} or key == "extra":
            raise KeyError(key)
        setattr(self, key, _coerce(value, getattr(self, key)))


def _coerce(value: Any, default: Any) -> Any:
    """Make a settings value the same type as its default; keep the default if impossible."""
    try:
        if isinstance(default, bool):
            if isinstance(value, str):
                return value.strip().lower() in ("1", "true", "yes", "on")
            return bool(value)
        if isinstance(default, int):
            return int(value)
        if isinstance(default, float):
            return float(value)
        if isinstance(default, list):
            return list(value) if isinstance(value, (list, tuple)) else [str(value)]
        if isinstance(default, str):
            return str(value)
    except (TypeError, ValueError):
        return default
    return value


def atomic_write(path: Path, text: str, mode: int | None = None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=str(path.parent), prefix="." + path.name + ".")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(text)
        if mode is not None:
            os.chmod(tmp, mode)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


class State:
    """Small persistent state: last session id, last activity time."""

    def __init__(self, paths: Paths) -> None:
        self.paths = paths
        self.data: dict[str, Any] = {}
        try:
            self.data = json.loads(paths.state_file.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            self.data = {}
        if not isinstance(self.data, dict):
            self.data = {}

    def get(self, key: str, default: Any = None) -> Any:
        return self.data.get(key, default)

    def set(self, **kv: Any) -> None:
        self.data.update(kv)
        try:
            atomic_write(self.paths.state_file, json.dumps(self.data, indent=2))
        except OSError:
            pass
