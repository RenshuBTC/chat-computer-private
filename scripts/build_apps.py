"""Builds what chatcomputer.ai/get offers from what's in apps/ (docs/get/):

- Chat-Computer-Chrome.zip: the Chrome extension (apps/chrome), in a folder of its own, for "Load unpacked" (as
  Annotated's "Add to Chrome" is). It's small: its side panel shows the website itself (chatcomputer.ai, so it's
  always the newest), and opens the sites asked for in real Chrome tabs.

The same bytes each time (fixed dates in the zip), so tests/test_apps.py can check docs/get is what this makes.
Run it after changing apps/:  python scripts/build_apps.py

And the page the Windows and Mac app bundles (apps/desktop/frontend, not kept in git), and the model's engine it
brings (apps/desktop/src-tauri/binaries, not kept in git either), before building the app (.github/workflows/apps.yml
does both):  python scripts/build_apps.py desktop;  python scripts/build_apps.py engine [--target TRIPLE]
It's the website's page (docs/index.html, docs/demo/), with Heart's code beside it (demo/lm/: kokoro-js and the ONNX
Runtime WebAssembly its Transformers.js loads, the versions the website loads from jsDelivr, fetched with npm, which
checks them against the registry's checksums), so nothing that runs in the app's own page comes from elsewhere. The
model that answers isn't in the page: it's the engine's, beside the app's program.
"""
from __future__ import annotations

import io
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
APPS = ROOT / "apps"
GET = ROOT / "docs" / "get"
CHROME_FILES = ["manifest.json", "background.js", "panel.html", "panel.css", "panel.js"]
CHROME_FOLDER = "Chat Computer"
DOCS = ROOT / "docs"
FRONTEND = APPS / "desktop" / "frontend"
# Heart's code (the warmer voice: kokoro-js, and the ONNX Runtime its Transformers.js loads), as docs/demo/host.js
# loads it on the website: npm package, and the files taken from it (to demo/lm/).
LM = {
    "kokoro-js@1.2.1": ["dist/kokoro.web.js"],
    "@huggingface/transformers@3.5.1": ["dist/ort-wasm-simd-threaded.jsep.mjs", "dist/ort-wasm-simd-threaded.jsep.wasm"],
}


def chrome_zip() -> bytes:
    """The extension's files, under one folder, with fixed dates: the same zip for the same files."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        for name in CHROME_FILES:
            info = zipfile.ZipInfo(f"{CHROME_FOLDER}/{name}", date_time=(2026, 1, 1, 0, 0, 0))
            info.external_attr = 0o644 << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, (APPS / "chrome" / name).read_bytes())
    return buf.getvalue()


def files() -> dict[Path, bytes]:
    return {GET / "Chat-Computer-Chrome.zip": chrome_zip()}


def desktop_frontend(lm: bool = True) -> Path:
    """apps/desktop/frontend: the website's page, and (unless lm is False) Heart's code beside it."""
    shutil.rmtree(FRONTEND, ignore_errors=True)
    FRONTEND.mkdir(parents=True)
    shutil.copy2(DOCS / "index.html", FRONTEND / "index.html")
    shutil.copytree(DOCS / "demo", FRONTEND / "demo")
    if lm:
        (FRONTEND / "demo" / "lm").mkdir()
        npm = shutil.which("npm")
        if not npm:
            raise SystemExit("npm is needed to fetch Heart's code (or: build_apps.py desktop --no-lm)")
        with tempfile.TemporaryDirectory() as tmp:
            for spec, wanted in LM.items():
                out = subprocess.run([npm, "pack", spec, "--silent"], cwd=tmp, capture_output=True, text=True, check=True)
                with tarfile.open(Path(tmp) / out.stdout.strip().splitlines()[-1]) as tar:
                    for name in wanted:
                        data = tar.extractfile("package/" + name)
                        (FRONTEND / "demo" / "lm" / Path(name).name).write_bytes(data.read())
    return FRONTEND


BINARIES = APPS / "desktop" / "src-tauri" / "binaries"


def host_triple() -> str:
    out = subprocess.run(["rustc", "-vV"], cwd=ROOT / "os", capture_output=True, text=True, check=True).stdout
    return next(line.split(": ", 1)[1] for line in out.splitlines() if line.startswith("host: "))


def desktop_engine(target: str | None = None) -> Path:
    """The model's engine (os/model's chat-computer-model), built for the app's target, where Tauri takes what the
    app brings with it (src-tauri/binaries/chat-computer-model-<target>, its externalBin). A Mac's app for Apple
    silicon and Intel at once (universal-apple-darwin) gets one program for both; Windows', built elsewhere than on
    Windows, is built with cargo-xwin."""
    target = target or host_triple()
    BINARIES.mkdir(parents=True, exist_ok=True)
    out = BINARIES / f"chat-computer-model-{target}{'.exe' if 'windows' in target else ''}"
    parts = ["aarch64-apple-darwin", "x86_64-apple-darwin"] if target == "universal-apple-darwin" else [target]
    built = []
    for t in parts:
        cmd = ["cargo", "build", "--release", "-p", "model", "--features", "std", "--bin", "chat-computer-model", "--target", t]
        if "windows-msvc" in t and sys.platform != "win32":
            cmd[1:2] = ["xwin", "build"]
        subprocess.run(cmd, cwd=ROOT / "os", check=True)
        built.append(ROOT / "os" / "target" / t / "release" / f"chat-computer-model{'.exe' if 'windows' in t else ''}")
    if len(built) > 1:
        subprocess.run(["lipo", "-create", "-output", str(out), *map(str, built)], check=True)
        for t, b in zip(parts, built):              # (and each on its own, for a build for one of them: its tests)
            shutil.copy2(b, BINARIES / f"chat-computer-model-{t}")
    else:
        shutil.copy2(built[0], out)
    return out


def main() -> None:
    if sys.argv[1:2] == ["desktop"]:
        print("wrote", desktop_frontend(lm="--no-lm" not in sys.argv).relative_to(ROOT))
        return
    if sys.argv[1:2] == ["engine"]:
        target = sys.argv[sys.argv.index("--target") + 1] if "--target" in sys.argv else None
        print("wrote", desktop_engine(target).relative_to(ROOT))
        return
    for path, data in files().items():
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists() or path.read_bytes() != data:
            path.write_bytes(data)
            print("wrote", path.relative_to(ROOT))


if __name__ == "__main__":
    main()
