"""Run the desktop eval cases: each in its own process (case.py) and its own sandbox (bubblewrap), a few at a time.

Inside a case's sandbox the computer looks like a Chat Computer's: the person is an ordinary user (uid 1000) with
their files in /home/sam, Chat Computer is in /opt/chatcomputer/app with its Python in /opt/chatcomputer/venv, the
commands are those a real one has (machine.py), and there's a display (Xvfb). Everything else of this machine is
read-only or hidden (/home, /root, /opt, /mnt), and the environment holds only what the web tools need (the network's
proxy): no other tools or connections of the session running the evals reach the Chat Computer under test.

    python -m evals.os.run OUT_DIR [--only id,id] [--area printing] [--jobs 2] [--app DIR]
                                   [--models DIR] [--model auto] [--effort medium]

Traces go to OUT_DIR/traces/<id>.json. Chat Computer thinks as it does on a real one: with its own model, run by its
own engine on this computer, from --models (a folder of models made ready, llm/NAME.ccm with its llm/NAME.refusal, as
Chat Computer leaves them in ~/ChatComputer/models; that folder unless told otherwise), the one it picks by this
computer's memory unless --model names one. --app runs another version of Chat Computer (a folder with its code, such
as an export of an earlier commit) with these same cases, harness and simulated machine, to compare before and
after."""
from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
VENV = Path(sys.prefix)
INSIDE = "/tmp/.ev"           # the case's own files, inside its sandbox
# What the web tools need (the network's proxy): nothing else goes in.
PASS_ENV = ("HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY", "https_proxy", "http_proxy", "no_proxy")
MODELS = Path.home() / "ChatComputer" / "models"
CA_ENV = ("SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "NODE_EXTRA_CA_CERTS", "CURL_CA_BUNDLE")


def cases(path: Path) -> tuple[dict, list[dict]]:
    data = json.loads(path.read_text())
    out = []
    for c in data["cases"]:
        state = dict(data["default_state"])
        state.update(c.get("state") or {})
        out.append({**c, "state": state})
    return data, out


def sandbox(work: Path, binfolder: Path, display: int = 120, app: Path = REPO, models: Path = MODELS) -> tuple[list[str], dict[str, str]]:
    env = {"EVAL_DISPLAY": str(display), "PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/home/sam", "USER": "sam", "LANG": "en_GB.UTF-8",
           "TZ": ":" + INSIDE + "/io/localtime", "CHATCOMPUTER_HOME": "/home/sam/ChatComputer",
           "PYTHONDONTWRITEBYTECODE": "1", "PYTHONPATH": "/opt/chatcomputer/app"}
    for k in PASS_ENV:
        if os.environ.get(k):
            env[k] = os.environ[k]
    args = ["bwrap", "--unshare-user", "--uid", "1000", "--gid", "1000", "--unshare-pid", "--die-with-parent",
            "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc",
            "--tmpfs", "/tmp", "--tmpfs", "/home", "--tmpfs", "/root", "--tmpfs", "/opt", "--tmpfs", "/mnt",
            "--tmpfs", "/media",       # where drives are mounted (opened): /media/sam/<label>
            "--bind", str(work / "home"), "/home/sam",
            "--ro-bind", str(app), "/opt/chatcomputer/app",
            "--ro-bind", str(HERE.parent), "/opt/chatcomputer/app/evals",     # this harness, whichever version runs
            "--ro-bind", str(VENV), "/opt/chatcomputer/venv",
            "--ro-bind", str(binfolder), "/opt/evals/bin",
            "--bind", str(work / "io"), INSIDE + "/io"]
    # The models made ready, where Chat Computer keeps them (read-only: each case uses them as they are), but for the
    # engine's reading of the system prompt (llm/cache), which a Chat Computer keeps from its first conversation on:
    # one for the run, written by its first case (and again whenever the prompt differs), so each case starts as a
    # Chat Computer in use does rather than reading the whole prompt first.
    if models.is_dir():
        args += ["--ro-bind", str(models), "/home/sam/ChatComputer/models"]
        if (models / "llm" / "cache").is_dir():
            args += ["--bind", str(work.parent.parent / "engine-cache"), "/home/sam/ChatComputer/models/llm/cache"]
    ca = next((os.environ[k] for k in CA_ENV if os.environ.get(k) and os.path.exists(os.environ[k])), None)
    if ca:
        args += ["--ro-bind", ca, INSIDE + "/ca.crt"]
        env.update({k: INSIDE + "/ca.crt" for k in CA_ENV})
    for name in ("passwd", "group"):      # the person is sam (user_files)
        mine = binfolder.parent / f"etc_{name}"
        if mine.exists():
            args += ["--ro-bind", str(mine), f"/etc/{name}"]
    args += ["--clearenv"]
    for k, v in env.items():
        args += ["--setenv", k, v]
    args += ["--chdir", "/home/sam"]
    return args, env


def user_files(out: Path) -> None:
    """The person is sam (uid 1000) in the sandbox, as on their Chat Computer, not this machine's own user of that
    number: /etc/passwd and /etc/group as this machine's with that one line changed."""
    for name in ("passwd", "group"):
        lines = [ln for ln in Path(f"/etc/{name}").read_text().splitlines() if ln.split(":")[2:3] != ["1000"]]
        lines.append("sam:x:1000:1000:Sam Lee:/home/sam:/bin/bash" if name == "passwd" else "sam:x:1000:")
        (out / f"etc_{name}").write_text("\n".join(lines) + "\n")


def run_case(case: dict, out: Path, model: str, effort: str, binfolder: Path, display: int,
             app: Path = REPO, models: Path = MODELS) -> tuple[str, float, str]:
    work = out / "work" / case["id"]
    shutil.rmtree(work, ignore_errors=True)
    (work / "home").mkdir(parents=True)
    (work / "io").mkdir()
    (work / "io" / "case.json").write_text(json.dumps({**case, "model": model, "effort": effort}))
    # The sandbox's time zone (TZ=:/tmp/.ev/io/localtime): Singapore, until Chat Computer changes it (world.py).
    shutil.copyfile("/usr/share/zoneinfo/" + case["state"].get("timezone", "Asia/Singapore"), work / "io" / "localtime")
    args, _ = sandbox(work, binfolder, display, app, models)
    cmd = args + ["/opt/chatcomputer/venv/bin/python", "-m", "evals.os.case",
                  INSIDE + "/io/case.json", INSIDE + "/io/trace.json"]
    t0 = time.time()
    with open(work / "log.txt", "w") as log:
        try:
            # (The turn may take 10 minutes; before it, the run's first case reads the prompt, which can take long.)
            p = subprocess.run(cmd, stdout=log, stderr=subprocess.STDOUT, timeout=4800)
            code = p.returncode
        except subprocess.TimeoutExpired:
            code = "timeout"
    trace = work / "io" / "trace.json"
    if trace.exists():
        shutil.copy(trace, out / "traces" / f"{case['id']}.json")
    return case["id"], round(time.time() - t0, 1), "ok" if trace.exists() else f"exit {code}"


def main() -> None:
    from .machine import build_bin

    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--cases", default=str(HERE / "cases.json"))
    ap.add_argument("--only", default="")
    ap.add_argument("--area", default="")
    ap.add_argument("--jobs", type=int, default=2, help="cases at a time (each runs a model: as many as the memory holds)")
    ap.add_argument("--models", default=str(MODELS), help="the folder of models made ready")
    ap.add_argument("--model", default="auto", help="the model's name (models.json), or auto: Chat Computer's own pick")
    ap.add_argument("--effort", default="medium")
    ap.add_argument("--app", default=str(REPO))
    a = ap.parse_args()
    out = Path(a.out).resolve()
    app = Path(a.app).resolve()
    if not (app / "evals").is_dir():
        sys.exit(f"{app} needs an evals folder (it can be empty): this harness is put there.")
    (out / "traces").mkdir(parents=True, exist_ok=True)
    (out / "engine-cache").mkdir(exist_ok=True)
    models = Path(a.models).expanduser().resolve()
    if not any(models.glob("llm/*.ccm")):
        sys.exit(f"No model made ready in {models}/llm: Chat Computer (or the engine's prepare command) makes one ready there.")
    (out / "run.json").write_text(json.dumps({"app": str(app), "model": a.model, "effort": a.effort, "models": str(models),
                                             "started": time.strftime("%Y-%m-%d %H:%M:%S")}))
    missing = build_bin(out / "bin", app)
    user_files(out)
    if missing:
        print("not on this machine (a real Chat Computer has them):", " ".join(missing), flush=True)

    _, todo = cases(Path(a.cases))
    if a.only:
        keep = set(a.only.split(","))
        todo = [c for c in todo if c["id"] in keep]
    if a.area:
        todo = [c for c in todo if c["area"] == a.area]
    print(f"{len(todo)} cases, {a.jobs} at a time, Chat Computer from {app} on {a.model} at {a.effort} effort", flush=True)
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        futs = [pool.submit(run_case, c, out, a.model, a.effort, out / "bin", 120 + i, app, models) for i, c in enumerate(todo)]
        for f in concurrent.futures.as_completed(futs):
            try:
                cid, secs, status = f.result()
            except Exception as exc:
                cid, secs, status = "?", 0, repr(exc)
            print(f"{cid}: {status} ({secs} s)", flush=True)


if __name__ == "__main__":
    main()
