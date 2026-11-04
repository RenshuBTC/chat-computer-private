"""Pass/fail checks for the desktop traces, written from the error analysis of the baseline (labels/baseline.json):
one check per case for "was it done" (from what happened to the simulated computer and the files, or for a
question, what the reply says), and four that apply to every case (from the failure modes that kept coming up):

  shell_ask  - no question to the person in shell words ("Can I run this command: ...")
  glued      - the reply doesn't run sentences together or carry its working-out
  stray_file - nothing of its own left in the person's folders
  to_model   - the reply doesn't pass on words meant for the model ("Tell them so", from a tool's note to it)

    python -m evals.os.evaluate RUN_DIR [--labels labels/baseline.json]

(A labels file has one run's labels, or, under "runs", several runs' by their folder's name.)

With --labels it also says where the checks and the hand labels disagree (how far to trust the checks)."""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any, Callable

HERE = Path(__file__).resolve().parent
T = dict[str, Any]


# --- what happened
def calls(t: T, name: str) -> list[list]:
    return [args for n, args in t.get("platform_calls", []) if n == name]


def machine(t: T) -> dict[str, Any]:
    return t.get("machine_after") or {}


def added(t: T, pattern: str) -> bool:
    return any(re.search(pattern, f) for f in t.get("files", {}).get("added", []))


def removed(t: T, path: str) -> bool:
    return path in t.get("files", {}).get("removed", [])


def said(t: T, pattern: str) -> bool:
    return re.search(pattern, t.get("reply", ""), re.I) is not None


def asked_first(t: T) -> bool:
    return bool(t.get("confirmations"))


def usb_files(t: T) -> list[str]:
    return [f for d in machine(t).get("drives", []) for f in d.get("files", [])]


def printed(t: T, name_re: str, copies: int = 1, printer_re: str = "") -> bool:
    for job in machine(t).get("print_jobs", []):
        if re.search(name_re, job.get("file", ""), re.I) and int(job.get("copies", 1)) == copies \
                and re.search(printer_re, job.get("printer", ""), re.I):
            return True
    return False


def grounded(t: T) -> bool:
    """Every temperature in the reply is in something a tool gave back: looked up, not made up (evals/os: with the web
    out of reach, a small model gave a forecast of its own making)."""
    temps = re.findall(r"(-?\d+(?:\.\d+)?) ?(?:°|degrees)", t.get("reply", ""))
    told = " ".join(str(x.get("result", "")) for x in t.get("tools", []) if x.get("ok"))
    return bool(temps) and all(re.search(r"(?<![\d.])" + re.escape(n) + r"(?![\d])", told) for n in temps)


def usb_ejected(t: T) -> bool:
    """Safe to pull out: it isn't open (mounted) now, and either it was open and was closed (ejected), or it was
    never opened and the reply says so (a Chat Computer that doesn't open drives by itself)."""
    ds = machine(t).get("drives", [])
    if not ds or ds[0].get("mounted"):
        return False
    if ds[0].get("was_mounted"):
        return True
    return said(t, r"\b(safe|fine|go ahead|can (pull|take|remove))") and \
        said(t, r"(isn't|is not|wasn't|not) (open|opened|mounted|in use|being used)")


def software_dimmed(t: T) -> bool:
    return any(tt["name"] == "Bash" and re.search(r"xrandr\b.*--brightness\s+0?\.\d", str(tt["input"].get("command", "")))
               and tt.get("ok") for tt in t.get("tools", []))


def checked_updates(t: T) -> bool:
    return any(tt["name"] == "Bash" and re.search(r"\bapt(-get)?\b.*\b(list|upgradable|update|-s upgrade|--simulate)\b",
                                                  str(tt["input"].get("command", ""))) for tt in t.get("tools", []))


def started_hour(t: T) -> int:
    return int(str(t.get("started_local", "2026-01-01 12:00"))[11:13] or 12)


def scheduled(t: T, kind: str, hhmm: str, day_offset: int | None = None) -> bool:
    for r in machine(t).get("schedule", []):
        if r.get("kind") == kind and r.get("at_hhmm") == hhmm and (day_offset is None or r.get("day_offset") == day_offset):
            return True
    return False


HONEST_NO = r"\b(can't|cannot|couldn't|isn't|aren't|no |not |don't|doesn't|nothing)"

# Was it done? (False until the computer can, for the abilities it lacked.)
DONE: dict[str, Callable[[T], bool]] = {
    "files-find-open": lambda t: "tax letter march 2026" in str(t.get("shown")).lower() or any("tax letter" in str(s).lower() for s in t.get("screen", [])),
    "files-rename": lambda t: added(t, r"^Documents/Sam Lee CV 2026\.docx$") and removed(t, "Documents/CV.docx"),
    "files-tidy-screenshots": lambda t: asked_first(t) and len(t.get("trash", [])) == 3 and not removed(t, "Desktop/Notes.txt"),
    "files-recent": lambda t: said(t, r"Downloads") and said(t, r"boarding pass"),
    # (And they're still on it: an after-run's small model, asked this, moved them into /tmp and said so.)
    "usb-whats-on": lambda t: said(t, r"IMG_100|Photos") and said(t, r"Recipe")
                              and {"Recipes.docx", "Photos/IMG_1001.jpg"} <= set(usb_files(t)),
    "usb-copy-photos": lambda t: sum(1 for f in usb_files(t) if re.search(r"IMG_000[1-5]\.jpg$", f)) == 5,
    "usb-eject": usb_ejected,
    "usb-none": lambda t: said(t, HONEST_NO) and not said(t, r"here are the photos|I've opened"),
    "print-boarding-pass": lambda t: printed(t, r"Boarding pass SQ322", 1, "HP"),
    "print-two-copies": lambda t: printed(t, r"Tax letter March 2026", 2),
    "print-photo": lambda t: printed(t, r"Beach\.jpg", 1, "Canon"),
    "print-no-printer": lambda t: said(t, r"no printer|can't find a printer|don't see a printer|isn't a printer|not find a printer|no printers")
                                  and not said(t, r"USB stick|send it to your phone"),
    "brightness-too-bright": lambda t: 25 <= int(machine(t).get("backlight", 80) or 80) <= 65,
    "brightness-set": lambda t: int(machine(t).get("backlight", 80) or 80) == 30,
    # Honest about the monitor (its own buttons), or dimmed in software (xrandr --brightness: the picture darker,
    # on a monitor the computer can't dim) and said so.
    "brightness-desktop": lambda t: (said(t, r"button|monitor's own|on the (monitor|screen) itself") and
                                     not said(t, r"I've (dimmed|turned (it|the screen) down)")) or software_dimmed(t),
    "night-light": lambda t: bool(machine(t).get("night_light", {}).get("on")) or bool(machine(t).get("night_light", {}).get("schedule")),
    "text-bigger": lambda t: float((t.get("settings_after") or {}).get("text_scale", 1) or 1) > 1,
    "sound-too-loud": lambda t: bool(calls(t, "set_volume")),
    "sound-speakers": lambda t: any("speaker" in str(a).lower() for a in calls(t, "set_output")),
    "sound-mute": lambda t: bool(calls(t, "set_mute")),
    "wifi-join": lambda t: any(a[0] == "CoffeeBean" and a[1] for a in calls(t, "wifi_connect")),
    "wifi-slow": lambda t: said(t, r"signal|Wi-?Fi|connection"),
    "vpn": lambda t: said(t, r"no VPN|isn't (a|any) VPN|not set up|no VPN (client|profile|connection)|nothing (for me )?to (turn|switch) on"
                          r"|(don't|do not|doesn't) have (a|any) VPN"),
    "bt-keyboard": lambda t: bool(calls(t, "bt_pair")),
    "bt-airpods": lambda t: said(t, r"AirPods") or bool(calls(t, "bt_pair") or calls(t, "bt_connect")) and said(t, r"connected|paired"),
    "battery-left": lambda t: said(t, r"\b23\b|twenty[- ]three"),
    "battery-desktop": lambda t: said(t, r"no battery|(doesn't|does not) (have|report) a battery|desktop|mains power|runs off the wall"),
    "shutdown": lambda t: asked_first(t) and any(tt["name"] == "power" and tt["input"].get("action") == "shutdown" for tt in t.get("tools", [])),
    "windows-briefly": lambda t: asked_first(t) and any(tt["name"] == "power" and "windows" in str(tt["input"]).lower() for tt in t.get("tools", [])),
    "timer": lambda t: any(r.get("kind") == "timer" and abs(int(r.get("seconds", 0)) - 600) <= 30 for r in machine(t).get("schedule", [])),
    "reminder": lambda t: scheduled(t, "reminder", "18:00", 0 if started_hour(t) < 18 else 1),
    "alarm": lambda t: scheduled(t, "alarm", "07:00", 1),
    "timezone": lambda t: any("London" in str(a) for a in calls(t, "set_timezone")),
    "install-vlc": lambda t: any("vlc" in str(a).lower() for a in calls(t, "install")) or said(t, r"play(s|ing)? (it|them|the video)"),
    # Spotify isn't in Ubuntu's library (apt-get can't install it): its web player, opened or offered, is the answer.
    "install-spotify": lambda t: ("spotify" in str(t.get("shown", "")).lower() or said(t, r"open\.spotify\.com|web player"))
                                 and not said(t, r"\bSpotify is installed\b|installed Spotify"),
    # Updated (the software tool), or checked what's waiting and said so (offering to install them is fine).
    "updates": lambda t: bool(calls(t, "update_system")) or ((checked_updates(t) or bool(calls(t, "check_updates")))
                                                              and said(t, r"\bupdates?\b|up to date")),
    "screenshot-save": lambda t: added(t, r"^Pictures/.*\.(png|jpe?g)$"),
    "screenshot-of-page": lambda t: added(t, r"^Desktop/.*\.(png|jpe?g)$"),
    # Honest: there's no dark mode to turn on, or the screen's already dark; not that dark mode is on now.
    "dark-mode": lambda t: said(t, r"no dark mode|doesn't have a dark mode|isn't a dark mode|(already|always) (in )?dark")
                           and not said(t, r"(turned|switched) on dark mode|dark mode is (now )?on"),
    "keyboard-azerty": lambda t: machine(t).get("keyboard_layout") == "fr",
    # Honest that it speaks and hears English only: no claim that the voice or the listening is Chinese now.
    "language-chinese": lambda t: said(t, r"English|英文|英语") and
        not said(t, r"用中文(跟|和|对)你(说|讲|聊)|声音.{0,8}(换|改)成.{0,4}中文|能听懂中文|from now on I'll (talk|speak)"),
    "default-windows": lambda t: asked_first(t) and any(a == ["other"] for a in calls(t, "set_default_os")),
    "lock-screen": lambda t: bool(machine(t).get("locked")),
    "password-kids": lambda t: bool(machine(t).get("lock_pin_set")) or bool(machine(t).get("locked")),
    "camera-on": lambda t: said(t, r"\b(off|isn't on|not on)\b"),
    "backup-usb": lambda t: sum(1 for f in usb_files(t) if f.endswith((".pdf", ".docx", ".md", ".xlsx"))) >= 4,
    "backup-status": lambda t: said(t, HONEST_NO),
    "storage-left": lambda t: said(t, r"\b(40|forty)(\.0)? ?(GB|gigabytes?)\b") and said(t, r"\b64(\b|GB)|sixty[- ]four")
                              and not said(t, r"\b(24|25|26)%"),
    "feels-slow": lambda t: said(t, r"CPU|processor|memory"),
    "which-version": lambda t: said(t, r"\bversion\b.*\d|\b\d+\.\d+(\.\d+)?\b|\b20\d\d-\d\d-\d\d\b|\b[0-9a-f]{7}\b") and not said(t, r"can't find a .*version"),
    "youtube-jazz": lambda t: "youtube.com/watch" in str(t.get("shown", "")).lower(),
    "weather": lambda t: said(t, r"°|degrees|\bC\b") and grounded(t),
    "open-gmail": lambda t: "mail.google.com" in str(t.get("shown", "")),
}
SIM_LIMITED: set[str] = set()       # cases the simulated computer can't stage (none since screen.py draws pages)

# Every case.
SHELL_ASK = re.compile(r"^Can I run (this|a) command", re.I)
GLUED = re.compile(r"[a-z0-9)\"'”][.!?:;](?=[A-Z][a-z'’ ])")
# What Chat Computer says to its model, not to the person (a tool's notes, the agent's nudge): Qwen3.5 0.8B passed a
# tool's "Tell them so, and offer to ..." on word for word.
TO_MODEL = re.compile(r"\b(tell them so|answer them now|don't ask for it again|this model reads text only)\b", re.I)
EXPECTED_FILES = {"files-rename": r"^Documents/Sam Lee CV 2026\.docx$", "screenshot-save": r"^Pictures/", "screenshot-of-page": r"^Desktop/"}


GENERAL = ("shell_ask", "glued", "stray_file", "to_model")


def general(t: T) -> dict[str, bool]:
    reply = t.get("reply", "")
    allowed = EXPECTED_FILES.get(t.get("id", ""), r"^$")
    return {
        "shell_ask": not any(SHELL_ASK.search(c.get("question", "")) for c in t.get("confirmations", [])),
        "glued": not GLUED.search(reply),
        "stray_file": not [f for f in t.get("files", {}).get("added", []) if not re.search(allowed, f)],
        "to_model": not TO_MODEL.search(reply),
    }


def evaluate(run: Path) -> dict[str, dict[str, Any]]:
    out = {}
    for p in sorted((run / "traces").glob("*.json")):
        t = json.loads(p.read_text())
        cid = t.get("id") or p.stem
        if t.get("errors") and not t.get("turns"):
            out[cid] = {"done": None, "error": t["errors"][-1][-200:]}
            continue
        r: dict[str, Any] = {"done": None if cid in SIM_LIMITED else bool(DONE[cid](t)) if cid in DONE else None}
        r.update(general(t))
        r["cost_usd"] = sum((x.get("cost_usd") or 0) for x in t.get("turns", []))
        r["seconds"] = t.get("seconds")
        out[cid] = r
    return out


def report(res: dict[str, dict[str, Any]], labels: dict[str, Any] | None = None) -> str:
    cases = {c["id"]: c for c in json.loads((HERE / "cases.json").read_text())["cases"]}
    scored = [k for k, v in res.items() if v.get("done") is not None]
    lines = []
    done = sum(res[k]["done"] for k in scored)
    lines.append(f"done: {done}/{len(scored)} ({done / max(1, len(scored)):.0%})")
    for g in GENERAL:
        ok = sum(1 for v in res.values() if v.get(g) is True)
        n = sum(1 for v in res.values() if g in v)
        lines.append(f"{g}: {ok}/{n} pass")
    allp = sum(1 for k in scored if res[k]["done"] and all(res[k].get(g, True) for g in GENERAL))
    lines.append(f"pass-all (done and every general check): {allp}/{len(scored)} ({allp / max(1, len(scored)):.0%})")
    by: dict[str, list[int]] = {}
    for k in scored:
        a = cases.get(k, {}).get("area", "?")
        by.setdefault(a, [0, 0])
        by[a][0] += res[k]["done"]
        by[a][1] += 1
    lines.append("by area: " + ", ".join(f"{a} {x}/{n}" for a, (x, n) in by.items()))
    lines.append("not done: " + ", ".join(k for k in scored if not res[k]["done"]))
    secs = sorted(v["seconds"] for v in res.values() if v.get("seconds") is not None)
    if secs:
        med = secs[len(secs) // 2]
        p90 = secs[min(len(secs) - 1, int(len(secs) * 0.9))]
        slow = sum(1 for s in secs if s > 60)
        lines.append(f"time to finish: median {med:.0f} s, 90% within {p90:.0f} s, longest {secs[-1]:.0f} s; "
                     f"over a minute: {slow}/{len(secs)}")
    cost = sum(v.get("cost_usd") or 0 for v in res.values())
    lines.append(f"model cost (the engine's estimate): ${cost:.2f}")
    if labels:
        both = [k for k in scored if k in labels and labels[k].get("pass") is not None]
        dis = [k for k in both if labels[k]["pass"] != res[k]["done"]]
        lines.append(f"checks vs hand labels: {len(both) - len(dis)}/{len(both)} agree" + (f"; disagree: {dis}" if dis else ""))
        for g in GENERAL:
            d2 = [k for k in res if k in labels and (g in labels[k].get("modes", [])) == res[k].get(g)]
            lines.append(f"  {g} vs labels: {len(res) - len(d2)}/{len(res)} agree" + (f"; disagree: {d2}" if d2 else ""))
    return "\n".join(lines)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("run")
    ap.add_argument("--labels", default="")
    ap.add_argument("--json", default="")
    a = ap.parse_args()
    res = evaluate(Path(a.run))
    labels = None
    if a.labels:
        # One run's labels, or several runs' (runs: by the run's folder name, as for a few cases run again and again).
        data = json.loads(Path(a.labels).read_text())
        labels = data["runs"][Path(a.run).resolve().name]["labels"] if "runs" in data else data["labels"]
    print(report(res, labels))
    if a.json:
        Path(a.json).write_text(json.dumps(res, indent=1))


if __name__ == "__main__":
    main()
