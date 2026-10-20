"""Run the desktop eval cases: each in its own process (case.py) and its own sandbox (bubblewrap), a few at a time.

Inside a case's sandbox the computer looks like a Chat Computer's: the person is an ordinary user (uid 1000) with
their files in /home/ren, Chat Computer is in /opt/chatcomputer/app with its Python in /opt/chatcomputer/venv, the
commands are those a real one has (machine.py), and there's a display (Xvfb). Everything else of this machine is
read-only or hidden (/home, /root, /opt, /mnt), and the environment holds only what reaching Claude needs: no other
tools or connections of the session running the evals reach the Chat Computer under test.

    python -m evals.os.run OUT_DIR [--only id,id] [--area printing] [--jobs 6] [--app DIR]
                                   [--model claude-opus-5-5] [--effort max]

Traces go to OUT_DIR/traces/<id>.json. Claude signs in with the access of the session running this (an Anthropic
model only, as on a real Chat Computer): Opus 5.5 at max effort unless told otherwise. --app runs another version of
Chat Computer (a folder with its code, such as an export of an earlier commit) with these same cases, harness and
simulated machine, to compare before and after."""
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
# Format-valid stand-in so Chat Computer starts its brain; case.py keeps it away from the engine.
PLACEHOLDER = "sk-ant-oat01-" + "evals" * 8
INSIDE = "/tmp/.ev"           # the case's own files, inside its sandbox
# What reaching Claude through this session needs (and the network's proxy): nothing else goes in.
PASS_ENV = ("ANTHROPIC_BASE_URL", "HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY", "https_proxy", "http_proxy", "no_proxy")
CA_ENV = ("SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "NODE_EXTRA_CA_CERTS", "CURL_CA_BUNDLE")


def cases(path: Path) -> tuple[dict, list[dict]]:
    data = json.loads(path.read_text())
    out = []
    for c in data["cases"]:
        state = dict(data["default_state"])
        state.update(c.get("state") or {})
        out.append({**c, "state": state})
    return data, out


def sandbox(work: Path, binfolder: Path, display: int = 120, app: Path = REPO) -> tuple[list[str], dict[str, str]]:
    env = {"EVAL_DISPLAY": str(display), "PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/home/ren", "USER": "ren", "LANG": "en_GB.UTF-8",
           "TZ": ":" + INSIDE + "/io/localtime", "CHATCOMPUTER_HOME": "/home/ren/ChatComputer", "CHATCOMPUTER_TOKEN": PLACEHOLDER,
           "PYTHONDONTWRITEBYTECODE": "1", "PYTHONPATH": "/opt/chatcomputer/app"}
    for k in PASS_ENV:
        if os.environ.get(k):
            env[k] = os.environ[k]
    args = ["bwrap", "--unshare-user", "--uid", "1000", "--gid", "1000", "--unshare-pid", "--die-with-parent",
            "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc",
            "--tmpfs", "/tmp", "--tmpfs", "/home", "--tmpfs", "/root", "--tmpfs", "/opt", "--tmpfs", "/mnt",
            "--tmpfs", "/media",       # where drives are mounted (opened): /media/ren/<label>
            "--bind", str(work / "home"), "/home/ren",
            "--ro-bind", str(app), "/opt/chatcomputer/app",
            "--ro-bind", str(HERE.parent), "/opt/chatcomputer/app/evals",     # this harness, whichever version runs
            "--ro-bind", str(VENV), "/opt/chatcomputer/venv",
            "--ro-bind", str(binfolder), "/opt/evals/bin",
            "--bind", str(work / "io"), INSIDE + "/io"]
    # The session's sign-in, where the engine looks for it (beside its ingress file), read-only; nothing else of
    # that folder.
    token = os.environ.get("CLAUDE_SESSION_INGRESS_TOKEN_FILE")
    if token and os.path.exists(token):
        for f in sorted(Path(token).parent.iterdir()):
            if f.is_file() and f.name.startswith(".") and f.name.endswith("_token"):
                args += ["--ro-bind", str(f), str(f)]
        env["CLAUDE_SESSION_INGRESS_TOKEN_FILE"] = token
    ca = next((os.environ[k] for k in CA_ENV if os.environ.get(k) and os.path.exists(os.environ[k])), None)
    if ca:
        args += ["--ro-bind", ca, INSIDE + "/ca.crt"]
        env.update({k: INSIDE + "/ca.crt" for k in CA_ENV})
    for name in ("passwd", "group"):      # the person is ren (user_files)
        mine = binfolder.parent / f"etc_{name}"
        if mine.exists():
            args += ["--ro-bind", str(mine), f"/etc/{name}"]
    args += ["--clearenv"]
    for k, v in env.items():
        args += ["--setenv", k, v]
    args += ["--chdir", "/home/ren"]
    return args, env


def user_files(out: Path) -> None:
    """The person is ren (uid 1000) in the sandbox, as on their Chat Computer, not this machine's own user of that
    number: /etc/passwd and /etc/group as this machine's with that one line changed."""
    for name in ("passwd", "group"):
        lines = [ln for ln in Path(f"/etc/{name}").read_text().splitlines() if ln.split(":")[2:3] != ["1000"]]
        lines.append("ren:x:1000:1000:Ren Shu:/home/ren:/bin/bash" if name == "passwd" else "ren:x:1000:")
        (out / f"etc_{name}").write_text("\n".join(lines) + "\n")


def run_case(case: dict, out: Path, model: str, effort: str, binfolder: Path, display: int,
             app: Path = REPO) -> tuple[str, float, str]:
    work = out / "work" / case["id"]
    shutil.rmtree(work, ignore_errors=True)
    (work / "home").mkdir(parents=True)
    (work / "io").mkdir()
    (work / "io" / "case.json").write_text(json.dumps({**case, "model": model, "effort": effort}))
    # The sandbox's time zone (TZ=:/tmp/.ev/io/localtime): Singapore, until Chat Computer changes it (world.py).
    shutil.copyfile("/usr/share/zoneinfo/" + case["state"].get("timezone", "Asia/Singapore"), work / "io" / "localtime")
    args, _ = sandbox(work, binfolder, display, app)
    cmd = args + ["/opt/chatcomputer/venv/bin/python", "-m", "evals.os.case",
                  INSIDE + "/io/case.json", INSIDE + "/io/trace.json"]
    t0 = time.time()
    with open(work / "log.txt", "w") as log:
        try:
            p = subprocess.run(cmd, stdout=log, stderr=subprocess.STDOUT, timeout=900)
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
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--model", default="claude-opus-5-5")
    ap.add_argument("--effort", default="max")
    ap.add_argument("--app", default=str(REPO))
    a = ap.parse_args()
    out = Path(a.out).resolve()
    app = Path(a.app).resolve()
    if not (app / "evals").is_dir():
        sys.exit(f"{app} needs an evals folder (it can be empty): this harness is put there.")
    (out / "traces").mkdir(parents=True, exist_ok=True)
    (out / "run.json").write_text(json.dumps({"app": str(app), "model": a.model, "effort": a.effort,
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
        futs = [pool.submit(run_case, c, out, a.model, a.effort, out / "bin", 120 + i, app) for i, c in enumerate(todo)]
        for f in concurrent.futures.as_completed(futs):
            try:
                cid, secs, status = f.result()
            except Exception as exc:
                cid, secs, status = "?", 0, repr(exc)
            print(f"{cid}: {status} ({secs} s)", flush=True)


if __name__ == "__main__":
    main()
