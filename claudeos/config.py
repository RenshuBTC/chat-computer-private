"""Paths, settings and the Claude sign-in token.

Everything ClaudeOS keeps lives in one folder the user owns: ~/ClaudeOS (override with
CLAUDEOS_HOME for tests). Settings are a small JSON file; unknown keys are kept so newer
versions can add settings without breaking older files.
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
import time
from dataclasses import asdict, dataclass, field, fields
from pathlib import Path
from typing import Any

TOKEN_RE = re.compile(r"^sk-ant-(oat01|api03)-[A-Za-z0-9_\-]{20,}$")


class Paths:
    """All on-disk locations, derived from one root folder."""

    def __init__(self, root: Path | None = None) -> None:
        root = root or Path(os.environ.get("CLAUDEOS_HOME") or (Path.home() / "ClaudeOS"))
        self.root = Path(root)
        self.secrets = self.root / "secrets"
        self.token_file = self.secrets / "claude_token"
        self.settings_file = self.root / "settings.json"
        self.state_file = self.root / "state.json"
        self.logs = self.root / "logs"
        self.models = self.root / "models"
        self.memory = self.root / "memory"
        self.memory_file = self.memory / "memory.md"
        self.pages = self.root / "pages"          # pages and apps Claude made for you
        self.undo = self.root / "undo"            # backups that make file changes undoable
        self.journal = self.root / "journal.jsonl"  # what ClaudeOS did, newest last
        self.transcript = self.root / "transcript.jsonl"
        self.index = self.root / "index"
        self.web = self.root / "web"              # the built-in browser's logins and cache
        self.engine = self.root / ".engine"       # Claude Code's own config for this session
        # A full Claude sign-in (from `claude auth login`), kept where Claude Code looks for it. Needed for
        # the Claude app on the phone (Remote Control); the one-year token can't do that.
        self.login_file = self.engine / ".credentials.json"

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
        """True for anything inside the secrets folder (never shown to Claude's tools)."""
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
    # Claude
    model: str = "claude-sonnet-5-5"
    effort: str = "medium"
    fresh_session_after_idle_hours: float = 8.0
    # Voice
    voice: str = "af_heart"
    speech_speed: float = 1.05
    stt_model: str = "small.en"
    stt_language: str = "en"
    wake_words: list[str] = field(default_factory=lambda: ["claude", "hey claude", "ok claude", "okay claude"])
    auto_sleep_minutes: int = 10
    barge_in: str = "auto"                  # auto | on | off
    release_mic_during_media: str = "auto"  # auto | on | off
    # Screen
    show_activity: bool = False
    theme: str = "light"            # light = white background, black text
    # Text size and line width are worked out from the screen's real size (ui/layout.py).
    text_size: int = 0              # 0 = automatic; otherwise a fixed size in pixels
    text_scale: float = 1.0         # "bigger"/"smaller" on top of the automatic size
    viewing_distance_cm: int = 0    # 0 = estimated from the screen size
    screen_inches: float = 0.0      # 0 = what the monitor reports (set it if the monitor reports nothing)
    web_zoom: float = 0.0           # 0 = automatic zoom for websites
    # Typing: the Claude app on the phone is the keyboard (Remote Control); needs the full sign-in
    phone_access: bool = True
    # Files
    windows_mount: str = "/mnt/windows"
    extra_search_roots: list[str] = field(default_factory=list)
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
        known = {f.name: f for f in fields(cls) if f.name != "extra"}
        if data.get("text_size") == 28 and "text_scale" not in data:
            data["text_size"] = 0   # 28 px was the fixed default before text was sized from the screen
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


def read_token(paths: Paths) -> str | None:
    """The Claude sign-in token from `claude setup-token` (or an API key), if present."""
    env = os.environ.get("CLAUDEOS_TOKEN")
    if env:
        return env.strip()
    try:
        tok = paths.token_file.read_text(encoding="utf-8").strip()
    except OSError:
        return None
    return tok or None


def token_kind(token: str | None) -> str:
    """'subscription', 'api_key' or 'invalid'."""
    if not token or not TOKEN_RE.match(token):
        return "invalid"
    return "subscription" if token.startswith("sk-ant-oat01-") else "api_key"


def save_token(paths: Paths, token: str) -> None:
    paths.ensure()
    atomic_write(paths.token_file, token.strip() + "\n", mode=0o600)


def login_valid(text: str | None) -> bool:
    """True for the contents of a Claude Code `.credentials.json` holding a claude.ai sign-in."""
    try:
        data = json.loads(text or "")
    except ValueError:
        return False
    oauth = data.get("claudeAiOauth") if isinstance(data, dict) else None
    return isinstance(oauth, dict) and all(isinstance(oauth.get(k), str) and oauth.get(k)
                                           for k in ("accessToken", "refreshToken"))


def has_login(paths: Paths) -> bool:
    try:
        if login_valid(paths.login_file.read_text(encoding="utf-8")):
            return True
    except OSError:
        pass
    return sys.platform == "darwin" and keychain_login(paths)


_KEYCHAIN_LOGIN: dict[str, tuple[float, bool]] = {}


def engine_cli() -> str | None:
    """The Claude Code engine that comes with the Agent SDK."""
    try:
        import claude_agent_sdk
        p = Path(claude_agent_sdk.__file__).parent / "_bundled" / ("claude.exe" if os.name == "nt" else "claude")
        return str(p) if p.exists() else None
    except Exception:
        return None


def keychain_login(paths: Paths, max_age: float = 300.0, runner: Any = subprocess.run) -> bool:
    """On a Mac, Claude Code keeps the full sign-in in the Keychain rather than in .credentials.json, so ask
    the engine whether it has one for ClaudeOS's settings folder (a claude.ai sign-in, not a token)."""
    key = str(paths.engine)
    hit = _KEYCHAIN_LOGIN.get(key)
    if hit and time.time() - hit[0] < max_age:
        return hit[1]
    cli = engine_cli()
    ok = False
    if cli:
        env = {k: v for k, v in os.environ.items()
               if k not in ("CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN")}
        env["CLAUDE_CONFIG_DIR"] = str(paths.engine)
        try:
            r = runner([cli, "auth", "status", "--json"], capture_output=True, text=True, timeout=30, env=env)
            data = json.loads(r.stdout or "{}")
            ok = bool(data.get("loggedIn")) and data.get("authMethod") == "claude.ai"
        except Exception:
            ok = False
    _KEYCHAIN_LOGIN[key] = (time.time(), ok)
    return ok


def save_login(paths: Paths, text: str) -> None:
    """Store a full sign-in where Claude Code reads (and refreshes) it."""
    if not login_valid(text):
        raise ValueError("not a Claude sign-in")
    paths.ensure()
    atomic_write(paths.login_file, text.strip() + "\n", mode=0o600)


class State:
    """Small persistent state: last session id, last activity time."""

    def __init__(self, paths: Paths) -> None:
        self.paths = paths
        self.data: dict[str, Any] = {}
        try:
            self.data = json.loads(paths.state_file.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            self.data = {}

    def get(self, key: str, default: Any = None) -> Any:
        return self.data.get(key, default)

    def set(self, **kv: Any) -> None:
        self.data.update(kv)
        try:
            atomic_write(self.paths.state_file, json.dumps(self.data, indent=2))
        except OSError:
            pass
