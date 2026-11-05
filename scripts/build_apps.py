"""Builds what chatcomputer.ai/get offers from what's in apps/ (docs/get/):

- mac.sh: Chat Computer Setup for Mac, in one line of Terminal (curl -fsSL https://chatcomputer.ai/get/mac.sh | bash):
  apps/desktop/get/mac.sh, with the SHA-256 of the disk image beside it (Chat-Computer-Mac.dmg, which GitHub builds:
  run this again when it's replaced).

The same bytes each time, so tests/test_apps.py can check docs/get is what this makes.
Run it after changing apps/:  python scripts/build_apps.py

And what the Windows and Mac app bundles, before building it (.github/workflows/apps.yml does it on each):
python scripts/build_apps.py desktop [--for windows|mac];  python scripts/build_apps.py engine [--target TRIPLE]
- apps/desktop/frontend (not kept in git): the website's page (docs/index.html, docs/demo/), with Heart's code beside
  it (demo/lm/: kokoro-js and the ONNX Runtime WebAssembly its Transformers.js loads, the versions the website loads
  from jsDelivr, fetched with npm, which checks them against the registry's checksums), so nothing that runs in the
  app's own page comes from elsewhere; and the app's first page, which asks how you want Chat Computer
  (apps/desktop/start/, as start/).
- apps/desktop/src-tauri/setup (not kept in git either): Chat Computer Setup, for the app to run when its own system
  is chosen: the repository's files as git has them, but the website, the apps, the evals, GitHub's workflows and the
  other computer's setup (Windows' for a Mac, the Mac's for Windows). For --for (Windows or Mac; by default, this
  computer's), and for neither on Linux, where the app is built only for its tests.
- apps/desktop/src-tauri/binaries (not kept in git): the model's engine, beside the app's program. The model that
  answers isn't in the page: it's the engine's.
"""
from __future__ import annotations

import hashlib
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
APPS = ROOT / "apps"
GET = ROOT / "docs" / "get"
DOCS = ROOT / "docs"
FRONTEND = APPS / "desktop" / "frontend"
START = APPS / "desktop" / "start"
SETUP = APPS / "desktop" / "src-tauri" / "setup"
# What Chat Computer Setup doesn't need of the repository: the website, the apps, the evals, GitHub's workflows.
SETUP_SKIP = ("docs/", "apps/", "evals/", ".github/")
MAC_SH = APPS / "desktop" / "get" / "mac.sh"
MAC_DMG = "Chat-Computer-Mac.dmg"
# Heart's code (the warmer voice: kokoro-js, and the ONNX Runtime its Transformers.js loads), as docs/demo/host.js
# loads it on the website: npm package, and the files taken from it (to demo/lm/).
LM = {
    "kokoro-js@1.2.1": ["dist/kokoro.web.js"],
    "@huggingface/transformers@3.5.1": ["dist/ort-wasm-simd-threaded.jsep.mjs", "dist/ort-wasm-simd-threaded.jsep.wasm"],
}


def mac_sh() -> bytes:
    """Chat Computer Setup for Mac in one line of Terminal: the disk image beside it, pinned by its SHA-256."""
    sha = hashlib.sha256((GET / MAC_DMG).read_bytes()).hexdigest()
    return MAC_SH.read_text().replace("@DMG@", MAC_DMG).replace("@SHA256@", sha).encode()


def files() -> dict[Path, bytes]:
    return {GET / "mac.sh": mac_sh()}


def desktop_frontend(lm: bool = True) -> Path:
    """apps/desktop/frontend: the website's page, and (unless lm is False) Heart's code beside it."""
    shutil.rmtree(FRONTEND, ignore_errors=True)
    FRONTEND.mkdir(parents=True)
    shutil.copy2(DOCS / "index.html", FRONTEND / "index.html")
    shutil.copytree(DOCS / "demo", FRONTEND / "demo")
    shutil.copytree(START, FRONTEND / "start")
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


def this_system() -> str:
    return {"win32": "windows", "darwin": "mac"}.get(sys.platform, "linux")


def setup_files(system: str) -> list[str]:
    """What Chat Computer Setup is, for Windows or a Mac: the files git keeps (as they're checked out: Windows' .cmd
    with its CRLF), less SETUP_SKIP and the other computer's setup."""
    out = subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True).stdout.decode()
    other = ("mac/", "Chat Computer Setup.command") if system == "windows" else ("windows/",)
    return sorted(f for f in out.split("\0") if f and not f.startswith(SETUP_SKIP + other) and (ROOT / f).is_file())


def desktop_setup(system: str) -> Path:
    """apps/desktop/src-tauri/setup: Chat Computer Setup, as the app brings it (Tauri's resources, setup/)."""
    shutil.rmtree(SETUP, ignore_errors=True)
    SETUP.mkdir(parents=True)
    if system not in ("windows", "mac"):
        (SETUP / "NOT-HERE.txt").write_text("Chat Computer Setup comes with the app for Windows and for Mac "
                                            "(scripts/build_apps.py desktop --for windows|mac).\n")
        return SETUP
    for f in setup_files(system):
        (SETUP / f).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / f, SETUP / f)
    return SETUP


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
        system = sys.argv[sys.argv.index("--for") + 1] if "--for" in sys.argv else this_system()
        print("wrote", desktop_frontend(lm="--no-lm" not in sys.argv).relative_to(ROOT))
        print("wrote", desktop_setup(system).relative_to(ROOT), f"(Chat Computer Setup for {system})")
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
