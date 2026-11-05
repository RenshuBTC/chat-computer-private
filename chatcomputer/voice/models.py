"""Where the voice models live, and downloading them (checked against known SHA-256 hashes)."""
from __future__ import annotations

import hashlib
import logging
import os
import shutil
import urllib.request
from pathlib import Path
from typing import Any, Callable

from ..config import Paths, Settings

log = logging.getLogger("chatcomputer.models")

KOKORO_BASE = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/"
KOKORO_FILES = {
    "kokoro-v1.0.int8.onnx": "6e742170d309016e5891a994e1ce1559c702a2ccd0075e67ef7157974f6406cb",
    "voices-v1.0.bin": "bca610b8308e8d99f32e6fe4197e7ec01679264efed0cac9140fe9c29f1fbf7d",
}
WHISPER_MODELS = ("tiny.en", "base.en", "small.en", "medium.en", "tiny", "base", "small", "medium",
                  "large-v3", "distil-large-v3", "distil-small.en", "distil-medium.en")


def whisper_dir(paths: Paths, name: str) -> Path:
    return paths.models / "whisper" / f"faster-whisper-{name}"


def kokoro_files(paths: Paths) -> tuple[Path, Path]:
    d = paths.models / "kokoro"
    return d / "kokoro-v1.0.int8.onnx", d / "voices-v1.0.bin"


def vad_model() -> Path:
    import faster_whisper
    assets = Path(faster_whisper.__file__).resolve().parent / "assets"
    found = sorted(assets.glob("silero_vad*.onnx"))
    if not found:
        raise FileNotFoundError("Silero VAD model missing from faster-whisper")
    return found[-1]


def status(paths: Paths, settings: Settings) -> dict[str, Any]:
    onnx, voices = kokoro_files(paths)
    wdir = whisper_dir(paths, settings.stt_model)
    return {
        "speech_recognition": {"model": settings.stt_model, "ready": (wdir / "model.bin").exists(), "path": str(wdir)},
        "voice": {"ready": onnx.exists() and voices.exists(), "path": str(onnx.parent)},
        "fallback_voice": bool(shutil.which("espeak-ng")),
    }


def _sha256(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def _fetch(url: str, dest: Path, sha: str | None, progress: Callable[[str], None]) -> None:
    if dest.exists() and (sha is None or _sha256(dest) == sha):
        progress(f"already have {dest.name}")
        return
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".part")
    for attempt in range(3):
        try:
            progress(f"downloading {dest.name}…")
            with urllib.request.urlopen(url, timeout=60) as r, open(tmp, "wb") as f:
                shutil.copyfileobj(r, f, 1 << 20)
            if sha and _sha256(tmp) != sha:
                raise IOError(f"{dest.name} failed its checksum")
            os.replace(tmp, dest)
            progress(f"got {dest.name}")
            return
        except Exception as exc:
            log.warning("download %s failed (%s), attempt %d", url, exc, attempt + 1)
            if attempt == 2:
                raise
    raise IOError("download failed")


def download(paths: Paths, settings: Settings, progress: Callable[[str], None] = print) -> None:
    paths.ensure()
    onnx, voices = kokoro_files(paths)
    for name, sha in KOKORO_FILES.items():
        _fetch(KOKORO_BASE + name, onnx.parent / name, sha, progress)
    download_stt(paths, settings.stt_model, progress)
    progress("voice models ready")


def stt_repo(name: str) -> str:
    """The Hugging Face repository a speech recognition model comes from: the one faster-whisper itself downloads
    it from. The distil models are Systran/faster-distil-whisper-<size> (not Systran/faster-distil-<size>, which
    doesn't exist: choosing one of them used to leave the voice waiting for a download that could never work)."""
    try:
        from faster_whisper.utils import _MODELS
        if name in _MODELS:
            return _MODELS[name]
    except Exception:
        pass
    if name.startswith("distil-"):
        return f"Systran/faster-distil-whisper-{name[len('distil-'):]}"
    return f"Systran/faster-whisper-{name}"


def download_stt(paths: Paths, name: str, progress: Callable[[str], None] = print) -> None:
    """One speech recognition model (faster-whisper's, from Hugging Face), unless it's here already."""
    wdir = whisper_dir(paths, name)
    if not (wdir / "model.bin").exists():
        progress(f"downloading speech recognition ({name})…")
        from huggingface_hub import snapshot_download
        snapshot_download(stt_repo(name), local_dir=str(wdir))
