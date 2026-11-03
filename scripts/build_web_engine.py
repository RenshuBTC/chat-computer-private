"""Builds Chat Computer's own model engine for the website's chat (docs/demo/engine/): os/model, through
os/webengine, as one WebAssembly module (engine.wasm), and the model it gets ready there (model.json: Qwen3.5 0.8B,
pinned as chatcomputer/brain/models.json pins it, with its refusal direction once that's put beside the engine).
worker.js and engine.js beside them are written by hand.

    python3 scripts/build_web_engine.py            builds them into docs/demo/engine/
    python3 scripts/build_web_engine.py --check    builds them elsewhere and checks they're the files there
    python3 scripts/build_web_engine.py --bench OUT.wasm
                                                   a module that can also make a model of Qwen3.5 0.8B's shape with
                                                   random weights, for timing (tests/web_engine_check.js --bench)

Built with the compiler os/rust-toolchain.toml pins (rustup adds WebAssembly to it if it hasn't it), for
wasm32-unknown-unknown with WebAssembly's 128-bit SIMD, one thread (threads need headers GitHub Pages doesn't send).
Reproducible: the paths it was built in are left out (as os/model/release.sh does), so the same source builds the
same file anywhere. Rust's output goes to os/target/wasm.
"""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OS = ROOT / "os"
OUT = ROOT / "docs" / "demo" / "engine"
TARGET = "wasm32-unknown-unknown"
MODEL = "qwen3.5-0.8b"                     # the website's model: the one a browser runs well


def manifest() -> bytes:
    """model.json: the website's model as chatcomputer/brain/models.json pins it (each file's size and SHA-256), and
    where its files come from (Qwen's repository on Hugging Face, at that revision). With its refusal direction, if
    it's beside the engine (docs/demo/engine/qwen3.5-0.8b.refusal, as `chat-computer-model find-refusal` finds it for
    the model's file, which the page makes byte for byte the same): pinned too, and taken out as the model runs."""
    models = json.loads((ROOT / "chatcomputer" / "brain" / "models.json").read_text())["models"]
    m = next(x for x in models if x["name"] == MODEL)
    out = {"name": m["name"], "label": m["label"], "repo": m["repo"], "revision": m["revision"], "bits": m["bits"],
           "base": "https://huggingface.co", "files": m["files"]}
    refusal = OUT / f"{MODEL}.refusal"
    if refusal.exists():
        data = refusal.read_bytes()
        out["refusal"] = {"name": refusal.name, "size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    return (json.dumps(out, indent=1) + "\n").encode()


def build(target_dir: Path, features: str = "") -> bytes:
    """engine.wasm, built by cargo with the pinned compiler."""
    installed = subprocess.run(["rustup", "target", "list", "--installed"], cwd=OS, capture_output=True, text=True).stdout
    if TARGET not in installed.split():
        subprocess.run(["rustup", "target", "add", TARGET], cwd=OS, check=True)
    cargo_home = os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))
    flags = f"-C target-feature=+simd128 -C strip=symbols --remap-path-prefix={OS}=/src --remap-path-prefix={cargo_home}=/cargo"
    command = ["cargo", "build", "--release", "-p", "webengine", "--target", TARGET, "--target-dir", str(target_dir)]
    if features:
        command += ["--features", features]
    subprocess.run(command, cwd=OS, check=True, env={**os.environ, "RUSTFLAGS": flags})
    return (target_dir / TARGET / "release" / "webengine.wasm").read_bytes()


def main() -> None:
    args = sys.argv[1:]
    if args[:1] == ["--bench"] and len(args) == 2:
        Path(args[1]).write_bytes(build(OS / "target" / "wasm-bench", "bench"))
        print("wrote", args[1])
        return
    if args == ["--check"]:
        with tempfile.TemporaryDirectory(dir=OS / "target") as tmp:
            made = {"engine.wasm": build(Path(tmp)), "model.json": manifest()}
        stale = [name for name, data in made.items() if not (OUT / name).exists() or (OUT / name).read_bytes() != data]
        if stale:
            sys.exit(f"docs/demo/engine/{', '.join(stale)} isn't what this source builds: run python3 scripts/build_web_engine.py")
        print("docs/demo/engine is what this source builds.")
        return
    if args:
        sys.exit(__doc__)
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in {"engine.wasm": build(OS / "target" / "wasm"), "model.json": manifest()}.items():
        if not (OUT / name).exists() or (OUT / name).read_bytes() != data:
            (OUT / name).write_bytes(data)
            print("wrote", (OUT / name).relative_to(ROOT), f"({len(data)} bytes)")
    for name in ("worker.js", "engine.js"):
        if not (OUT / name).exists():
            sys.exit(f"docs/demo/engine/{name} is missing")


if __name__ == "__main__":
    main()
