"""The code checks: the failures that a rule can find in a trace, each pass or fail, no model needed.

  narration        Words said together with tool calls. The kernel shows a round's words as they come, so they are on
                   the screen in the middle of the answer, against the prompt's "don't say you're about to use one".
  rounds-used-up   It used all of its rounds of tools for one message (the kernel then makes it stop).
  argument-error   A tool said its arguments were wrong (bad JSON, a missing or out-of-range value), or there's no such tool.
  power-unasked    It restarted, turned off or ended the conversation when nothing the person said asked for it.
  link-markup      Its answer has a Markdown link ([words](address)), which the screen doesn't show.
  language         Its answer isn't in the person's language (a message in Chinese answered in English, or the reverse).
  claimed-action   Its answer says it did something (shutting down, restarting, a timer set, the text made bigger, the
                   chimes turned down, a note made, a new conversation) that no tool call of that turn did.

    python3 evals/meta/checks.py TRACES.jsonl [--json OUT]"""
from __future__ import annotations

import argparse
import json
import re

ARG_ERRORS = ("must be", "Say which words", "Say when:", "isn't a time of day", "isn't one of", "There's no tool called",
              "say which drive", "A timer can be from")
POWER = {"restart_computer": r"restart|reboot|windows|turn (it |this |the computer |the pc |everything )?off|shut ?down|switch off|power off|重启|关机|關機|回到|窗口|windows",
         "turn_off_computer": r"turn (it |this |the computer |the pc |the laptop |everything )?off|shut ?(it |this )?down|switch (it )?off|power off|关机|關機|关掉|關掉|bed|leaving|going out|goodnight",
         "start_new_conversation": r"new (chat|conversation)|start (over|again|fresh)|clear (the|this) (chat|conversation)|forget (this|everything)|delete everything"}
CJK = re.compile(r"[\u3400-\u9fff]")
# What an answer may say it did, and the tool that does it (English; and the Chinese for power and alarms).
DID = {"turn_off_computer": r"\b(shutting (it |this |the computer )?down|turning (it |this |the computer |the pc )?off)\b|正在关机|马上关机",
       "restart_computer": r"\brestarting (now|the computer|this (pc|computer)|it|to|into|back|in a)\b|^restarting\W*$|正在重启",
       "reminders": r"\b(timer|reminder|alarm)s? (is |are )?(now )?set\b|\bset (a |an |the |your |both |two )?(\d+[- ]minute )?(timer|reminder|alarm)|\bi'?ll (remind|wake) you\b|已设好|叫你起床",
       "set_text_size": r"\b(made|making) (the |your )?text (bigger|smaller|larger)|\btext is now\b",
       "set_volume": r"\b(turned|turning) (it|the sound|the chimes|that|them) (down|up)\b|^muted\b",
       "remember": r"^noted\b|\bi'?ll remember\b",
       "start_new_conversation": r"\bstarting (fresh|a new conversation|over)\b"}
NOT = re.compile(r"\b(can'?t|cannot|couldn'?t|won'?t|unable|not|no|never|no longer|if you|want me|shall i|should i|do you want|i can|let me know)\b|不能|无法|吗")


def turns(t: dict):
    for turn in t.get("turns", []):
        yield turn


def check(t: dict) -> dict:
    out = {}
    rounds = [r for turn in turns(t) for r in turn["rounds"]]
    calls = [c for r in rounds for c in r["calls"]]
    hides = (t.get("kernel") or {}).get("hides_narration", False)
    # Shown: words written with tool calls, unless the kernel shows them only as what it's doing (calls that run).
    out["narration"] = not any(r["calls"] and r["said"].strip() and (not hides or any(c["result"].startswith("Not run:") for c in r["calls"]))
                               for r in rounds)
    out["rounds-used-up"] = not any(c["result"].startswith("Not run:") or "That was the last of the" in c["result"] for c in calls)
    out["argument-error"] = not any(c.get("raw") or (c["error"] and any(m in c["result"] for m in ARG_ERRORS)) for c in calls)
    asked = " ".join(turn["user"].lower() for turn in turns(t))
    out["power-unasked"] = not any(c["name"] in POWER and not re.search(POWER[c["name"]], asked) for c in calls)
    answers = " ".join(turn.get("answer") or "" for turn in turns(t))
    out["link-markup"] = not re.search(r"\[[^\]\n]+\]\((https?://|www\.)[^)]+\)", answers)
    q_cjk, a_cjk = bool(CJK.search(asked)), bool(CJK.search(answers))
    wants_cjk = q_cjk or re.search(r"chinese|mandarin|cantonese|japanese|korean", asked)
    out["language"] = not answers.strip() or (a_cjk if q_cjk else (not a_cjk or bool(wants_cjk)))
    out["claimed-action"] = True
    for turn in turns(t):
        done = {c["name"] for r in turn["rounds"] for c in r["calls"] if not c["error"]}
        for sentence in re.split(r"(?<=[.!?\n。！？])\s*", (turn.get("answer") or "").strip().lower()):
            if not sentence or NOT.search(sentence):
                continue
            if any(re.search(rx, sentence) and tool not in done for tool, rx in DID.items()):
                out["claimed-action"] = False
    return out


def summary(rows: list[dict]) -> dict:
    names = list(rows[0]["checks"]) if rows else []
    fails = {n: [r["id"] for r in rows if not r["checks"][n]] for n in names}
    return {"traces": len(rows), "pass_all": sum(all(r["checks"].values()) for r in rows),
            "fails": {n: len(v) for n, v in fails.items()}, "failing": fails}


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("traces")
    ap.add_argument("--json")
    a = ap.parse_args()
    rows = []
    for line in open(a.traces):
        t = json.loads(line)
        if t.get("error"):
            continue
        rows.append({"id": t["id"], "checks": check(t)})
    s = summary(rows)
    if a.json:
        with open(a.json, "w") as f:
            json.dump({"summary": s, "rows": rows}, f, indent=1)
    print(json.dumps({k: v for k, v in s.items() if k != "failing"}))
    for n, ids in s["failing"].items():
        if ids:
            print(f"{n}: {', '.join(ids[:12])}{' ...' if len(ids) > 12 else ''}")
