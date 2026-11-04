"""One desktop eval case, end to end: the real Chat Computer (controller, tools, permission gate, its own model run by
its own engine) on the simulated computer (platform.py) with a fresh home folder (home.py). It runs inside the case's
sandbox (run.py), as the person: HOME is /home/sam, PATH the commands a real Chat Computer has.

    python -m evals.os.case CASE.json TRACE.json

Writes the trace: what was asked, every tool call with its input and result, the questions Chat Computer asked
(answered yes: the person asked for it), what it did to the machine, the files before and after, and the reply."""
from __future__ import annotations

import asyncio
import contextlib
import json
import os
import subprocess
import sys
import time
import traceback
from pathlib import Path
from typing import Any

TURN_TIMEOUT_S = 600          # a model on a processor can take a while; how long it took is reported
INSTALLED = "/tmp/.ev/installed"      # programs "installed" during the case (platform.py) go on the PATH here
PATH = INSTALLED + ":/opt/evals/bin"


def _clip(s: Any, n: int = 4000) -> str:
    s = s if isinstance(s, str) else json.dumps(s, ensure_ascii=False, default=str)
    return s if len(s) <= n else s[:n] + f"... ({len(s) - n} more characters)"


def _display() -> subprocess.Popen:
    """The screen: a virtual one (Xvfb), under the name a real Chat Computer's has (Xorg), on its own number."""
    n = os.environ.get("EVAL_DISPLAY", "120")
    x = subprocess.Popen(["Xorg", ":" + n, "-screen", "0", "1920x1080x24", "-nolisten", "tcp", "-ac", "-noreset"],
                         executable="/usr/bin/Xvfb", stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(50):
        if os.path.exists(f"/tmp/.X11-unix/X{n}"):
            break
        time.sleep(0.1)
    os.environ["DISPLAY"] = ":" + n
    return x


async def run(case: dict[str, Any]) -> dict[str, Any]:
    from chatcomputer.config import Paths, Settings
    from chatcomputer.controller import Controller
    from . import world
    from .home import build_home, listing, usb_files
    from .platform import EvalPlatform
    from .screen import SimScreen

    home = Path.home()
    build_home(home)
    world.init(case["state"], usb_files())
    time.tzset()
    os.makedirs(INSTALLED, exist_ok=True)
    os.environ["PATH"] = PATH

    state = case["state"]
    settings = Settings(user_name="Sam Lee")
    if case.get("model"):
        settings.model = case["model"]
    if case.get("effort"):
        settings.effort = case["effort"]
    platform = EvalPlatform(state, home, INSTALLED)
    screen = SimScreen(platform, settings)
    if state.get("open_page"):
        screen.open(state["open_page"])
    else:
        screen.paint()
    c = Controller(Paths().ensure(), settings, platform, screen, voice=False, index=True, greet=False)

    trace: dict[str, Any] = {"id": case["id"], "query": case["query"], "modality": case["modality"],
                             "state": state, "model": settings.model, "effort": settings.effort, "tools": [],
                             "confirmations": [], "turns": [], "errors": [], "started": time.time(),
                             "started_local": time.strftime("%Y-%m-%d %H:%M")}
    t0 = time.time()

    async def confirm(question: str, typed: bool | None = None) -> tuple[str, str]:
        trace["confirmations"].append({"t": round(time.time() - t0, 1), "question": question, "answer": "yes"})
        return "yes", "yes"
    c.confirm = confirm           # the person says yes: they asked for it

    await c.start()
    if c.index is not None:     # the file index, built now rather than in a while
        await asyncio.to_thread(c.index.crawl_once)
        await asyncio.to_thread(c.index.extract_contents)
    if c.brain is None:
        trace["errors"].append("no brain (no model ready in ~/ChatComputer/models: see run.py --models)")
        return trace
    before = listing(home)

    # The model's reading of the system prompt and the tools (thousands of words: minutes on a processor, the first
    # time), before the person says anything, as on a Chat Computer that's been on a while (it reads them when it
    # starts, and keeps that reading: run.py's engine-cache, from the run's first case on). How long it took is kept.
    w0 = time.time()
    with contextlib.suppress(Exception):
        await asyncio.wait_for(c.brain.connect(), 3600)
    trace["warm_up_s"] = round(time.time() - w0, 1)

    done = asyncio.Event()
    ev = c.brain.ev
    on_tool, on_end = ev.on_tool, ev.on_turn_end

    def tool(tid: str, name: str, inp: dict[str, Any], ok: bool, result: str, *rest: Any) -> None:
        # (The brain gives the tool call's id too, now: passed on as it came.)
        trace["tools"].append({"t": round(time.time() - t0, 1), "name": name.split("__")[-1], "full": name,
                               "input": inp, "ok": ok, "result": _clip(result)})
        on_tool(tid, name, inp, ok, result, *rest)

    def end(tid: str, res: Any) -> None:
        trace["turns"].append({"ok": res.ok, "error_kind": res.error_kind, "error_text": res.error_text,
                               "text": res.text, "tools": res.tools,
                               "t": round(time.time() - t0, 1)})
        on_end(tid, res)
        done.set()
    ev.on_tool, ev.on_turn_end = tool, end

    t0 = time.time()
    try:
        await c.handle(case["query"], typed=case["modality"] == "typed")
        await asyncio.wait_for(done.wait(), TURN_TIMEOUT_S)
        await asyncio.sleep(1.0)           # Actions, and anything the turn left running, settle
    except asyncio.TimeoutError:
        trace["errors"].append(f"no answer within {TURN_TIMEOUT_S} s")
    except Exception:
        trace["errors"].append(traceback.format_exc()[-3000:])
    trace["seconds"] = round(time.time() - t0, 1)

    trace["reply"] = " ".join(t["text"] for t in trace["turns"]).strip()
    # What the screen showed as the reply, as it was streamed (the reply's own words, joined as on screen).
    trace["shown_reply"] = "".join(v[1] for k, v in screen.events if k == "ai" and isinstance(v, tuple))
    trace["platform_calls"] = [[n, [a if isinstance(a, (str, int, float, bool, type(None))) else str(a) for a in args]]
                               for n, args in platform.calls]
    trace["screen"] = [[k, _clip(v, 600)] for k, v in screen.events
                       if k in ("show_url", "show_html", "hide", "navigate", "notice", "done", "keyboard", "form",
                                "settings", "search", "message", "source_show")]
    trace["shown"] = {"mode": screen.mode, "url": screen.url, "title": screen.title}
    after = listing(home)
    trace["files"] = {"added": sorted(set(after) - set(before)), "removed": sorted(set(before) - set(after))}
    trash = home / ".local" / "share" / "Trash" / "files"
    trace["trash"] = sorted(os.listdir(trash)) if trash.is_dir() else []
    paths = Paths()
    for key, p in (("settings_after", paths.settings_file), ("memory_after", paths.memory_file)):
        try:
            trace[key] = json.loads(p.read_text()) if p.suffix == ".json" else p.read_text()[-3000:]
        except Exception:
            pass
    trace["machine_after"] = machine_after(c, world.snapshot(), trace.get("settings_after") or {}, trace["started"])
    try:
        await asyncio.wait_for(c.stop(), 30)
    except Exception:
        pass
    return trace


def machine_after(c: Any, snap: dict[str, Any], settings: dict[str, Any], started: float) -> dict[str, Any]:
    """The machine after the turn (world.snapshot()), with Chat Computer's own part of it: what's scheduled (timers,
    reminders, alarms: when, as a clock time and in days from the case's start), the night light's schedule, and
    whether the screen is locked. A Chat Computer from before those abilities has none of them."""
    import datetime

    def days_from_start(ts: float) -> int:
        return (datetime.date.fromtimestamp(ts) - datetime.date.fromtimestamp(started)).days

    sched = []
    schedule = getattr(c, "schedule", None)
    for it in (schedule.items() if schedule is not None else []):
        at = float(it["at"])
        sched.append({"kind": it.get("kind"), "at_hhmm": time.strftime("%H:%M", time.localtime(at)),
                      "day_offset": days_from_start(at), "seconds": round(at - float(it.get("created") or started)),
                      "label": it.get("label", ""), "repeat": it.get("repeat", "")})
    snap["schedule"] = sched
    nl = dict(snap.get("night_light") or {})
    if settings.get("night_light") == "evenings":
        nl["schedule"] = f"{settings.get('night_light_from', '')}-{settings.get('night_light_until', '')}"
    snap["night_light"] = nl
    snap["locked"] = bool(getattr(c, "locked", False))
    has_pin = getattr(c, "has_lock_pin", None)
    snap["lock_pin_set"] = bool(has_pin()) if callable(has_pin) else False
    return snap


def main() -> None:
    case_path, out_path = sys.argv[1], sys.argv[2]
    case = json.loads(Path(case_path).read_text())
    x = _display()
    try:
        trace = asyncio.run(run(case))
    except Exception:
        trace = {"id": case.get("id"), "query": case.get("query"), "errors": [traceback.format_exc()[-3000:]]}
    Path(out_path).write_text(json.dumps(trace, indent=1, ensure_ascii=False, default=str))
    x.terminate()
    os._exit(0)          # the engine's threads don't hold the run up


if __name__ == "__main__":
    main()
