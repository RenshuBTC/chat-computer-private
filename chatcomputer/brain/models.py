"""Which model Chat Computer thinks with, and getting it onto this computer.

The models are Qwen's open ones (Apache 2.0), run by Chat Computer's own engine (os/model, the program
chat-computer-model). models.json pins each to one revision of Qwen's own repository on Hugging Face, with every
file's size and SHA-256 (scripts/model_manifest.py writes it). Getting one ready: its files are downloaded from
there (a download that stops carries on where it stopped), each checked against its pinned SHA-256, then the
engine converts them into its own file (the weights in 4 or 8 bits) and the downloads are removed.

Which one: as big as this computer's memory runs well and its disk has room to get ready (pick).
"""
from __future__ import annotations

import contextlib
import hashlib
import json
import logging
import os
import shutil
import subprocess
import threading
import time
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

log = logging.getLogger("chatcomputer.models")

MANIFEST = Path(__file__).with_name("models.json")
BIN = Path(__file__).resolve().parent / "bin"       # the engine's program, one for each kind of processor
BUILT = Path(__file__).resolve().parents[2] / "os" / "target" / "release" / "chat-computer-model"  # built here
HUB = "https://huggingface.co"
CHUNK = 1 << 20
ROOM_LEFT = 4 << 30       # the disk space left for everything else once a model is ready (when choosing one)


@dataclass(frozen=True)
class Model:
    name: str
    label: str
    repo: str
    revision: str
    memory_gb: int
    bits: int
    files: tuple[dict[str, Any], ...] = field(default=())

    @property
    def download_bytes(self) -> int:
        return sum(int(f["size"]) for f in self.files)

    @property
    def file_bytes(self) -> int:
        """About how big its own file is: its weights at `bits` a weight (less the vision part, which isn't kept)."""
        weights = sum(int(f["size"]) for f in self.files if f["name"].endswith(".safetensors"))
        return int(weights / 2 * (self.bits + 0.5) / 8 * 0.95)


def catalogue() -> list[Model]:
    data = json.loads(MANIFEST.read_text(encoding="utf-8"))
    return [Model(m["name"], m["label"], m["repo"], m["revision"], m["memory_gb"], m["bits"], tuple(m["files"]))
            for m in data["models"]]


def by_name(name: str) -> Model | None:
    return next((m for m in catalogue() if m.name == name), None)


def names() -> list[str]:
    return [m.name for m in catalogue()]


def memory_bytes() -> int:
    try:
        import psutil
        return int(psutil.virtual_memory().total)
    except Exception:
        with contextlib.suppress(Exception):
            return os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES")
    return 0


def runs_in(m: Model, memory: int | None = None) -> bool:
    """Whether this computer's memory runs the model well (with room for everything else)."""
    gb = (memory if memory is not None else memory_bytes()) / 2**30
    # (A computer's memory is said in GB, and a little of it is the graphics' or the firmware's: 32 GB shows ~31.)
    return gb + 1.5 >= m.memory_gb


def pick(memory: int | None = None, store: "Store | None" = None) -> Model:
    """The biggest model this computer's memory runs well (the list is biggest first); given where the models go
    (store), the biggest of those that's there already or that the disk has room to get ready."""
    models = catalogue()
    fit = [m for m in models if runs_in(m, memory)] or [models[-1]]
    if store is None:
        return fit[0]
    try:
        free = store.free_bytes()
        for m in fit:
            if store.ready(m) or store.needed_bytes(m) + ROOM_LEFT <= free:
                return m
    except OSError:
        return fit[0]
    return fit[-1]


def chosen(setting: str | None, store: "Store | None" = None) -> Model:
    """The model the settings say ("auto": pick)."""
    return by_name(setting or "") or pick(store=store)


def label(setting: str | None) -> str:
    """How a model setting reads: its model's name (for "auto", the one picked)."""
    m = by_name(setting or "")
    if m:
        return m.label
    if not setting or setting == "auto":
        try:
            return pick().label
        except Exception:
            return "Qwen"
    return setting


def engine_program(models_dir: Path | None = None) -> str | None:
    """The engine's program: the one that comes with Chat Computer (brain/bin, one for each kind of processor, built
    reproducibly from os/model by os/model/release.sh), or one given by CHATCOMPUTER_MODEL_BIN or built from this
    source (os/target). One that lost its permission to run on the way (an update makes the files it brings
    read-only, scripts aside) is copied where it can run, models/llm/chat-computer-model, and checked against the
    one that came each time, so the copy is always that one."""
    import platform as pf
    env = os.environ.get("CHATCOMPUTER_MODEL_BIN")
    shipped = BIN / f"chat-computer-model-{pf.machine()}"
    for p in (env, str(shipped), shutil.which("chat-computer-model"), str(BUILT)):
        if p and os.path.isfile(p) and os.access(p, os.X_OK):
            return p
    if shipped.is_file() and models_dir is not None:
        mine = Path(models_dir) / "llm" / "chat-computer-model"
        with contextlib.suppress(OSError):
            if not mine.is_file() or mine.stat().st_size != shipped.stat().st_size or \
                    mine.read_bytes() != shipped.read_bytes():
                mine.parent.mkdir(parents=True, exist_ok=True)
                tmp = mine.with_name(mine.name + ".part")
                shutil.copyfile(shipped, tmp)
                os.replace(tmp, mine)
            os.chmod(mine, 0o755)
            return str(mine)
    return None


@dataclass
class Progress:
    """How getting a model ready is going (for the screen)."""
    stage: str = "waiting"        # waiting | downloading | converting | ready | failed
    done_bytes: int = 0
    total_bytes: int = 0
    detail: str = ""

    def words(self) -> str:
        if self.stage == "downloading" and self.total_bytes:
            return (f"downloading it: {self.done_bytes / 1e9:.1f} of {self.total_bytes / 1e9:.1f} GB "
                    f"({100 * self.done_bytes // self.total_bytes}%)")
        if self.stage == "converting":
            return "making it ready to run (a few minutes)"
        if self.stage == "failed":
            return "it couldn't be got ready: " + self.detail
        return self.stage


class Store:
    """Where the models live: ~/ChatComputer/models/llm/<name>.ccm, and their downloads while they come."""

    def __init__(self, models_dir: Path) -> None:
        self.root = Path(models_dir) / "llm"

    def path(self, m: Model) -> Path:
        return self.root / f"{m.name}.ccm"

    def ready(self, m: Model) -> Path | None:
        """Its file, if it's there and finished (the converter writes to a temporary name, renamed at the end)."""
        p = self.path(m)
        return p if p.is_file() and p.stat().st_size > 0 else None

    def downloads(self, m: Model) -> Path:
        return self.root / f"download-{m.name}"

    def free_bytes(self) -> int:
        self.root.mkdir(parents=True, exist_ok=True)
        return shutil.disk_usage(self.root).free

    def needed_bytes(self, m: Model) -> int:
        """Room needed to get it ready: what's still to download, its own file, and a little more."""
        have = 0
        d = self.downloads(m)
        for f in m.files:
            with contextlib.suppress(OSError):
                have += (d / f["name"]).stat().st_size
            with contextlib.suppress(OSError):
                have += (d / (f["name"] + ".part")).stat().st_size
        return max(m.download_bytes - have, 0) + m.file_bytes + (1 << 30)

    def prepare(self, m: Model, progress: Progress, stop: threading.Event | None = None,
                on_change: Callable[[Progress], None] | None = None, program: str | None = None) -> Path:
        """Downloads (or carries on downloading) and converts the model; its file. Raises on failure (progress says
        why too)."""
        stop = stop or threading.Event()
        tell = on_change or (lambda p: None)
        try:
            done = self.ready(m)
            if done:
                progress.stage = "ready"
                tell(progress)
                return done
            program = program or engine_program(self.root.parent)
            if not program:
                raise RuntimeError("Chat Computer's model engine isn't installed")
            need = self.needed_bytes(m)
            if self.free_bytes() < need:
                raise RuntimeError(f"it needs {need / 1e9:.1f} GB of free space and there's "
                                   f"{self.free_bytes() / 1e9:.1f} GB")
            d = self.downloads(m)
            d.mkdir(parents=True, exist_ok=True)
            progress.stage, progress.total_bytes = "downloading", m.download_bytes
            progress.done_bytes = 0
            for f in m.files:
                self._download(m, f, d, progress, stop, tell)
            progress.stage = "converting"
            tell(progress)
            out = self.path(m)
            tmp = out.with_suffix(".ccm.part")
            r = subprocess.run([program, "convert", str(d), str(tmp), "--bits", str(m.bits)],
                               capture_output=True, text=True)
            if r.returncode != 0:
                raise RuntimeError("converting it failed: " + (r.stderr.strip().splitlines() or ["?"])[-1])
            os.replace(tmp, out)
            shutil.rmtree(d, ignore_errors=True)
            progress.stage = "ready"
            tell(progress)
            return out
        except Exception as exc:
            progress.stage, progress.detail = "failed", str(exc)
            tell(progress)
            raise

    def _download(self, m: Model, f: dict[str, Any], d: Path, progress: Progress, stop: threading.Event,
                  tell: Callable[[Progress], None]) -> None:
        dest = d / f["name"]
        size = int(f["size"])
        if dest.is_file() and dest.stat().st_size == size:
            progress.done_bytes += size        # (checked when it finished downloading)
            tell(progress)
            return
        part = dest.with_name(dest.name + ".part")
        have = part.stat().st_size if part.is_file() else 0
        if have > size:
            part.unlink()
            have = 0
        url = f"{HUB}/{m.repo}/resolve/{m.revision}/{f['name']}"
        h = hashlib.sha256()
        if have:
            with open(part, "rb") as p:
                while chunk := p.read(CHUNK):
                    h.update(chunk)
        progress.done_bytes += have
        last = time.monotonic()
        tries = 0
        while have < size:
            if stop.is_set():
                raise RuntimeError("stopped")
            req = urllib.request.Request(url, headers={"User-Agent": "ChatComputer/1.0", "Range": f"bytes={have}-"})
            try:
                with urllib.request.urlopen(req, timeout=60) as r, open(part, "ab") as out:
                    if have and r.status != 206:
                        raise RuntimeError("the server didn't carry on where the download stopped")
                    while chunk := r.read(CHUNK):
                        if stop.is_set():
                            raise RuntimeError("stopped")
                        out.write(chunk)
                        h.update(chunk)
                        have += len(chunk)
                        progress.done_bytes += len(chunk)
                        if time.monotonic() - last > 1:
                            last = time.monotonic()
                            tell(progress)
                tries = 0
            except RuntimeError:
                raise
            except Exception as exc:
                tries += 1
                if tries > 8:
                    raise RuntimeError(f"downloading {f['name']} failed: {exc}") from exc
                log.info("download of %s broke off (%s): again in %d s", f["name"], exc, 2 * tries)
                time.sleep(2 * tries)
        if h.hexdigest() != f["sha256"]:
            part.unlink()
            raise RuntimeError(f"{f['name']} came down different from what Chat Computer pinned (its SHA-256): "
                               "not used")
        os.replace(part, dest)
        tell(progress)
