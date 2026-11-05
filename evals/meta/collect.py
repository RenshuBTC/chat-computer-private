"""Traces: each query of a query file asked of Meta's model exactly as Chat Computer's own kernel asks it (system.py:
its system prompt, tools, rounds and request), the tools it calls answered by a stand-in PC (pc.py), until it answers
in words; then the query's follow-ups, if it has any, the same way. Every round is kept: what the model said, the
tools it called with their arguments, what each said back, tokens and time.

    META_API_KEY_FILE=... python3 evals/meta/collect.py QUERIES.json OUT_DIR [--model muse-spark-1.3] [--budget 4]
                                                        [--workers 12] [--only id,id]

Writes OUT_DIR/traces.jsonl (one trace a line) and OUT_DIR/run.json (the model, the prompt's hash, the cost)."""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import hashlib
import json
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from api import Meta, Refused  # noqa: E402
from pc import NOW, PC  # noqa: E402
from system import ROOT, claims_power, kernel, stamp, system_prompt  # noqa: E402


def one(meta: Meta, k: dict, model: str, q: dict) -> dict:
    pc = PC(q.get("notes"))
    when = time.strptime("%04d-%02d-%02d %02d:%02d" % NOW, "%Y-%m-%d %H:%M")
    messages = [{"role": "system", "content": system_prompt(k, model, q.get("notes"))}]
    trace = {"id": q["id"], "query": q["text"], "dims": q.get("dims", {}), "turns": [], "error": None}
    made = 0
    for n, text in enumerate([q["text"]] + list(q.get("followups", []))):
        messages.append({"role": "user", "content": f"{stamp(when)} {text}"})
        turn = {"user": text, "rounds": [], "answer": None}
        trace["turns"].append(turn)
        rounds = 0
        while True:
            t0 = time.time()
            got = meta.ask({"model": model, "messages": messages, "tools": k["tools"], "max_completion_tokens": k["max_tokens"]})
            choice = got["choices"][0]
            msg = choice.get("message") or {}
            calls = msg.get("tool_calls") or []
            said = msg.get("content") or ""
            rnd = {"said": said, "calls": [], "finish": choice.get("finish_reason"), "seconds": round(time.time() - t0, 2),
                   "usage": got.get("usage", {})}
            turn["rounds"].append(rnd)
            powered = any(c["name"] in ("turn_off_computer", "restart_computer") and not c["error"] for r in turn["rounds"] for c in r["calls"])
            if not calls and not turn.get("reasked") and not powered and claims_power(k, said):
                turn["reasked"] = said          # (the kernel asks again, once: this answer isn't kept)
                continue
            if not calls:
                turn["answer"] = said
                messages.append({"role": "assistant", "content": said})
                break
            out_calls = []
            for c in calls:
                made += 1
                fn = c.get("function") or {}
                out_calls.append({"id": f"call_{made}", "type": "function",
                                  "function": {"name": fn.get("name", ""), "arguments": fn.get("arguments") or "{}"}})
            messages.append({"role": "assistant", "content": said, "tool_calls": out_calls})
            rounds += 1
            for i, c in enumerate(out_calls):
                name, raw = c["function"]["name"], c["function"]["arguments"]
                try:
                    args = json.loads(raw) if raw.strip() else {}
                    bad_json = False
                except ValueError:
                    args, bad_json = {}, True
                if rounds > k["rounds"]:
                    result, error = k["not_run"], True
                else:
                    result, error = pc.run(name, args)
                    if rounds == k["rounds"] and i == len(out_calls) - 1:
                        result += "\n\n" + k["last_round"]
                rnd["calls"].append({"name": name, "args": args, "raw": raw if bad_json else None, "result": result, "error": error})
                messages.append({"role": "tool", "tool_call_id": c["id"], "content": result})
            if rounds > k["rounds"]:
                turn["answer"] = said
                break
        trace["state"] = {"notes": pc.notes, "timers": pc.timers, "text_size": pc.text_size, "volume": pc.volume, "ended": pc.ended}
    return trace


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("queries")
    ap.add_argument("out")
    ap.add_argument("--model", default=None)
    ap.add_argument("--budget", type=float, default=4.0)
    ap.add_argument("--workers", type=int, default=12)
    ap.add_argument("--only", default="")
    ap.add_argument("--rev", default=None, help="evaluate the kernel's prompt and tools as they were at this git revision")
    a = ap.parse_args()
    k = kernel(a.rev)
    model = a.model or k["model"]
    queries = json.loads(Path(a.queries).read_text())
    if a.only:
        keep = set(a.only.split(","))
        queries = [q for q in queries if q["id"] in keep]
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    meta = Meta(a.budget)
    done: dict[str, dict] = {}

    def run(q):
        try:
            tr = one(meta, k, model, q)
            tr["kernel"] = {"rev": k["rev"], "hides_narration": k["hides_narration"]}
            return tr
        except Refused as e:
            return {"id": q["id"], "query": q["text"], "dims": q.get("dims", {}), "turns": [], "error": "refused: " + str(e)}
        except Exception as e:  # noqa: BLE001 - kept in the trace, to be looked at
            return {"id": q["id"], "query": q["text"], "dims": q.get("dims", {}), "turns": [], "error": f"{type(e).__name__}: {e}"}

    with cf.ThreadPoolExecutor(a.workers) as pool:
        for tr in pool.map(run, queries):
            done[tr["id"]] = tr
    with open(out / "traces.jsonl", "w") as f:
        for q in queries:
            f.write(json.dumps(done[q["id"]], ensure_ascii=False) + "\n")
    prompt = system_prompt(k, model)
    commit = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    run_info = {"model": model, "kernel": k["rev"], "hides_narration": k["hides_narration"], "queries": a.queries, "traces": len(queries), "errors": sum(1 for t in done.values() if t["error"]),
                "prompt_sha": hashlib.sha256((prompt + json.dumps(k["tools"])).encode()).hexdigest()[:12], "commit": commit,
                "usage": {**meta.spent, "usd": round(meta.spent["usd"], 3)}}
    (out / "run.json").write_text(json.dumps(run_info, indent=1))
    print(json.dumps(run_info))


if __name__ == "__main__":
    main()
