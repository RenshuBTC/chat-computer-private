"""Chat Computer command line.

  python -m chatcomputer run [--windowed] [--no-voice] [--fake]   start Chat Computer (the session runs this)
  python -m chatcomputer doctor                                   health check (JSON); exit 0 if usable
  python -m chatcomputer models [--download]                      the voice's and the thinking model's status /
                                                                  download (and making the thinking model ready)
  python -m chatcomputer say "text"                               speak a sentence (sound check)
"""
from __future__ import annotations

import argparse
import json
import os
import socket
import sys

from . import __version__
from .config import Paths, Settings


def _doctor(paths: Paths, settings: Settings) -> int:
    report: dict = {"version": __version__}
    ok = True
    mods = {}
    for mod in ("sounddevice", "faster_whisper", "kokoro_onnx", "onnxruntime", "mss", "PIL", "numpy"):
        try:
            __import__(mod)
            mods[mod] = "ok"
        except Exception as exc:
            mods[mod] = f"missing ({exc.__class__.__name__})"
    try:
        import gi
        gi.require_version("Gtk", "3.0")
        gi.require_version("WebKit2", "4.1")
        from gi.repository import WebKit2  # noqa: F401
        mods["webkit"] = "ok"
    except Exception as exc:
        mods["webkit"] = f"missing ({exc})"
        ok = False
    try:
        import gi
        gi.require_foreign("cairo")       # the onscreen keyboard is drawn with it (python3-gi-cairo)
        mods["cairo"] = "ok"
    except Exception as exc:
        mods["cairo"] = f"missing ({exc.__class__.__name__}): the onscreen keyboard can't be drawn"
    report["modules"] = mods
    from .brain import models as llm
    program = llm.engine_program()
    m = llm.chosen(settings.model)
    ready = llm.Store(paths.models).ready(m)
    report["model"] = {"engine": program or "missing", "model": m.label,
                       "ready": bool(ready), "file": str(ready) if ready else None,
                       "memory_gb": round(llm.memory_bytes() / 2**30, 1)}
    if not program:
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
        with socket.create_connection(("huggingface.co", 443), timeout=4):
            report["internet"] = "ok"
    except OSError as exc:
        report["internet"] = f"no ({exc.__class__.__name__}): only needed to download the model, and for the web"
    report["display"] = os.environ.get("DISPLAY") or "none"
    if os.path.exists("/usr/local/libexec/chatcomputer/chatcomputer-root"):
        from .platform import get_platform
        p = get_platform()
        try:
            report[p.other_os.lower()] = p.windows_status()
        except Exception as exc:
            report[p.other_os.lower()] = f"unknown ({exc})"
    report["usable"] = ok
    print(json.dumps(report, indent=2))
    return 0 if ok else 1


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="chatcomputer")
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
        from .brain import models as llm
        from .voice import models as M
        if args.download:
            M.download(paths, settings)
            m = llm.chosen(settings.model)
            progress = llm.Progress()
            llm.Store(paths.models).prepare(m, progress, on_change=lambda p: print(m.label + ": " + p.words(),
                                                                                    file=sys.stderr))
        status = M.status(paths, settings)
        m = llm.chosen(settings.model)
        status["thinking_model"] = {"model": m.label, "ready": bool(llm.Store(paths.models).ready(m))}
        print(json.dumps(status, indent=2))
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
    if args.cmd in (None, "run"):
        from .journal import setup_logging
        setup_logging(paths, verbose=getattr(args, "verbose", False))
        from .platform import get_platform
        platform = get_platform("fake" if getattr(args, "fake", False) else None)
        from .ui.gtk_app import run_app
        voice = not getattr(args, "no_voice", False)

        def make_controller(screen):
            # Loaded once the window shows (run_app), so the window comes up first.
            from .controller import Controller
            return Controller(paths, settings, platform, screen, voice=voice,
                              offer_headphones=not getattr(args, "fake", False),
                              offer_updates=not getattr(args, "fake", False),
                              # The weather, for the greeting under the word logo (chat.js: GREETING).
                              weather=True)
        return run_app(paths, settings, make_controller, windowed=getattr(args, "windowed", False))
    ap.print_help()
    return 2


if __name__ == "__main__":
    sys.exit(main())
