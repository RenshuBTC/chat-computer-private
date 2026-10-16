"""ClaudeOS command line.

  python -m claudeos run [--windowed] [--no-voice] [--fake]   start ClaudeOS (the session runs this)
  python -m claudeos doctor                                   health check (JSON); exit 0 if usable
  python -m claudeos models [--download]                      voice model status / download
  python -m claudeos say "text"                               speak a sentence (sound check)
  python -m claudeos signin                                   save a new Claude sign-in token
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys

from . import __version__
from .config import Paths, Settings, read_token, save_token, token_kind


def _doctor(paths: Paths, settings: Settings) -> int:
    report: dict = {"version": __version__}
    ok = True
    mods = {}
    for mod in ("claude_agent_sdk", "sounddevice", "faster_whisper", "kokoro_onnx", "onnxruntime", "mss", "PIL", "numpy"):
        try:
            __import__(mod)
            mods[mod] = "ok"
        except Exception as exc:
            mods[mod] = f"missing ({exc.__class__.__name__})"
            ok = ok and mod not in ("claude_agent_sdk",)
    try:
        if sys.platform == "darwin":
            import AppKit  # noqa: F401
            import WebKit  # noqa: F401
        else:
            import gi
            gi.require_version("Gtk", "3.0")
            gi.require_version("WebKit2", "4.1")
            from gi.repository import WebKit2  # noqa: F401
        mods["webkit"] = "ok"
    except Exception as exc:
        mods["webkit"] = f"missing ({exc})"
        ok = False
    report["modules"] = mods
    try:
        import claude_agent_sdk
        cli = os.path.join(os.path.dirname(claude_agent_sdk.__file__), "_bundled", "claude")
        r = subprocess.run([cli, "--version"], capture_output=True, text=True, timeout=30)
        report["claude_engine"] = r.stdout.strip() or r.stderr.strip()[:200]
    except Exception as exc:
        report["claude_engine"] = f"unavailable ({exc})"
        ok = False
    kind = token_kind(read_token(paths))
    report["signed_in"] = {"subscription": "yes (Claude subscription)", "api_key": "yes (API key)"}.get(kind, "no")
    if kind == "invalid":
        ok = False
    from .voice import models as M
    report["voice"] = M.status(paths, settings)
    try:
        import sounddevice as sd
        di = sd.query_devices(kind="input")
        do = sd.query_devices(kind="output")
        report["audio"] = {"input": di.get("name"), "output": do.get("name")}
    except Exception as exc:
        report["audio"] = f"unavailable ({exc.__class__.__name__}: {exc})"
    try:
        with socket.create_connection(("api.anthropic.com", 443), timeout=4):
            report["internet"] = "ok"
    except OSError as exc:
        report["internet"] = f"no ({exc.__class__.__name__})"
    report["display"] = os.environ.get("DISPLAY") or "none"
    if os.path.exists("/usr/local/libexec/claudeos/claudeos-root"):
        from .platform.linux import LinuxPlatform
        try:
            report["windows"] = LinuxPlatform().windows_status()
        except Exception as exc:
            report["windows"] = f"unknown ({exc})"
    report["usable"] = ok
    print(json.dumps(report, indent=2))
    return 0 if ok else 1


def main(argv: list[str] | None = None) -> int:
    # The Mac app's launcher (mac/launcher/launcher.c) points Python at ClaudeOS's environment with
    # PYTHONEXECUTABLE. Programs ClaudeOS starts must not inherit it: another Python would take it as its own.
    os.environ.pop("PYTHONEXECUTABLE", None)
    ap = argparse.ArgumentParser(prog="claudeos")
    sub = ap.add_subparsers(dest="cmd")
    r = sub.add_parser("run")
    r.add_argument("--windowed", action="store_true")
    r.add_argument("--no-voice", action="store_true")
    r.add_argument("--fake", action="store_true", help="pretend machine (for previews and tests)")
    r.add_argument("-v", "--verbose", action="store_true")
    sub.add_parser("doctor")
    m = sub.add_parser("models")
    m.add_argument("--download", action="store_true")
    sy = sub.add_parser("say")
    sy.add_argument("text")
    sub.add_parser("signin")
    sub.add_parser("version")
    args = ap.parse_args(argv)
    paths = Paths().ensure()
    settings = Settings.load(paths)

    if args.cmd == "version":
        print(__version__)
        return 0
    if args.cmd == "doctor":
        return _doctor(paths, settings)
    if args.cmd == "models":
        from .voice import models as M
        if args.download:
            M.download(paths, settings)
        print(json.dumps(M.status(paths, settings), indent=2))
        return 0
    if args.cmd == "say":
        import time
        from .voice import models as M
        from .voice.audio import Player
        from .voice.tts import TTS
        onnx, voices = M.kokoro_files(paths)
        tts = TTS(str(onnx) if onnx.exists() else None, str(voices) if voices.exists() else None,
                  settings.voice, settings.speech_speed)
        p = Player()
        p.play(tts.synth(args.text))
        while p.busy:
            time.sleep(0.1)
        time.sleep(0.3)
        return 0
    if args.cmd == "signin":
        print("Paste your Claude sign-in token (from 'claude setup-token' on any computer), then press Enter:")
        tok = sys.stdin.readline().strip()
        if token_kind(tok) == "invalid":
            print("That doesn't look like a Claude token (it starts with sk-ant-).")
            return 1
        save_token(paths, tok)
        print("Saved. Restart ClaudeOS.")
        return 0
    if args.cmd in (None, "run"):
        scenario = os.environ.get("CLAUDEOS_TEST_SCENARIO")
        if scenario:
            # Tests only (tests/mac_ci/run.sh): a scripted session inside the real Mac app, in place of the
            # usual start, so it runs the way the app does (its name, its permissions, coming to the front).
            import runpy
            import shlex
            sys.argv = [scenario, *shlex.split(os.environ.get("CLAUDEOS_TEST_SCENARIO_ARGS", ""))]
            runpy.run_path(scenario, run_name="__main__")
            return 0
        from .journal import setup_logging
        setup_logging(paths, verbose=getattr(args, "verbose", False))
        os.environ.setdefault("CLAUDEOS_ENGINE", "mac" if sys.platform == "darwin" else "linux")
        from .platform import get_platform
        platform = get_platform("fake" if getattr(args, "fake", False) else None)
        from .controller import Controller
        if sys.platform == "darwin":
            from .ui.mac_app import run_app
        else:
            from .ui.gtk_app import run_app
        voice = not getattr(args, "no_voice", False)
        return run_app(paths, settings, lambda screen: Controller(paths, settings, platform, screen, voice=voice),
                       windowed=getattr(args, "windowed", False))
    ap.print_help()
    return 2


if __name__ == "__main__":
    sys.exit(main())
