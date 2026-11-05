"""Traces, short, to read one after another: what was asked, each round (what the model said, the tools it called and
what they said back, cut short), and its answer.

    python3 evals/meta/show.py TRACES.jsonl [--from N] [--count N] [--only id,id] [--full]"""
from __future__ import annotations

import argparse
import json


def cut(s: str, n: int) -> str:
    s = " ".join((s or "").split())
    return s if len(s) <= n else s[: n - 1] + "…"


def show(t: dict, full: bool = False) -> str:
    out = [f"### {t['id']}" + (f"  ERROR {t['error']}" if t.get("error") else "")]
    for turn in t["turns"]:
        out.append(f"USER: {turn['user']}")
        for r in turn["rounds"]:
            if r["calls"]:
                if r["said"]:
                    out.append(f"  (said with calls) {cut(r['said'], 1000 if full else 160)}")
                for c in r["calls"]:
                    args = json.dumps(c["args"], ensure_ascii=False)
                    out.append(f"  -> {c['name']}({cut(args, 300 if full else 110)})" + (" ERR" if c["error"] else "")
                               + f" => {cut(c['result'], 2000 if full else 110)}")
            else:
                out.append(f"  ANSWER: {cut(r['said'], 4000 if full else 600)}")
    if t.get("state", {}).get("ended"):
        out.append(f"  [ends: {t['state']['ended']}]")
    return "\n".join(out)


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("traces")
    ap.add_argument("--from", dest="start", type=int, default=0)
    ap.add_argument("--count", type=int, default=1000)
    ap.add_argument("--only", default="")
    ap.add_argument("--full", action="store_true")
    a = ap.parse_args()
    rows = [json.loads(line) for line in open(a.traces)]
    if a.only:
        keep = a.only.split(",")
        rows = [r for r in rows if r["id"] in keep]
    for t in rows[a.start: a.start + a.count]:
        print(show(t, a.full))
