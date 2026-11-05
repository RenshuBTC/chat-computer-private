"""Muse Spark (and Meta's other models) on Chat Computer's own system: can they run the computer with its tools?

Each of evals/os/cases.json's everyday requests ("Find the tax letter I got in March and open it") is asked, many
times, of Meta's models through Meta's Model API, with exactly what Chat Computer's own kernel gives its model: the
operating system's tools (os/kernel/src/tools.json) and its instructions. What each answer does (the tools it calls,
or that it says it can't) is graded by Muse Spark against the case's expectation, knowing which tools exist: done (a
fitting tool called), honest (it can't be done with these tools, and the answer says so), or wrong. The results say
which everyday requests the own system can already do with Muse Spark, and which tools it still lacks.

    META_API_KEY_FILE=... python3 evals/meta/run.py OUT_DIR [--repeat N] [--budget DOLLARS] [--until HH:MM] [--workers N]

The cost is estimated from the tokens each answer reports, at the prices public reports give (Muse Spark 1.3: $1.25
a million tokens read, $4.25 written); it stops at --budget, at --until (local time), or when the API refuses for
want of credit. The key is read from the file and never written anywhere."""
import argparse
import concurrent.futures as cf
import json
import os
import random
import re
import ssl
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
API = "https://api.meta.ai/v1/chat/completions"
PRICE = {"muse-spark-1.3": (1.25, 4.25), "muse-spark-1.2": (1.25, 4.25), "muse-wonder-1.0": (5.0, 15.0)}
SYSTEM = ("You are Chat Computer: you run this computer for the person using it, who talks to you by typing here. "
          "You are {model}, Meta's AI model, answering over the internet through Meta's Model API. With your tools you "
          "can see how this computer is doing, change the text size, set timers, reminders and alarms, read the files on "
          "its drives, search the web and read web pages, show the person a picture, note things about them to remember, "
          "start a new conversation, and restart this computer or turn it off. Use a tool rather than saying how; don't "
          "say you're about to use one, just use it. Answer briefly. Don't make things up: say so when you can't do "
          "something with your tools.")
GRADER = ("You grade an AI assistant that runs a computer through tools. The tools it has: {tools}.\n"
          "The person asked: {query}\nWhat should happen: {expect}\nThe assistant's first turn: it said {said!r} and "
          "called {calls}.\nAnswer with one word: DONE if its tool calls are a fitting first step towards what should "
          "happen; HONEST if no combination of its tools could do this and it said so instead of pretending; WRONG "
          "otherwise (a tool that doesn't fit, claiming to have done what it can't, or asking needlessly).")


def post(key: str, body: dict, ctx: ssl.SSLContext) -> dict:
    req = urllib.request.Request(API, data=json.dumps(body).encode(), method="POST", headers={
        "Authorization": "Bearer " + key, "Content-Type": "application/json"})
    for attempt in range(5):
        try:
            with urllib.request.urlopen(req, timeout=120, context=ctx) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            text = e.read().decode(errors="replace")[:300]
            if e.code in (402, 403) or "credit" in text.lower() or "quota" in text.lower():
                raise RuntimeError(f"refused {e.code}: {text}")
            if e.code in (404, 429) or e.code >= 500:
                time.sleep(1.5 * (attempt + 1) + random.random())
                continue
            raise RuntimeError(f"error {e.code}: {text}")
        except (urllib.error.URLError, TimeoutError, ConnectionError):
            time.sleep(2 * (attempt + 1))
    raise RuntimeError("no answer after 5 tries")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--repeat", type=int, default=20)
    ap.add_argument("--budget", type=float, default=140.0)
    ap.add_argument("--until", default="")
    ap.add_argument("--workers", type=int, default=32)
    ap.add_argument("--models", default="muse-spark-1.3,muse-wonder-1.0,muse-spark-1.2")
    a = ap.parse_args()
    key = Path(os.environ["META_API_KEY_FILE"]).read_text().strip()
    ctx = ssl.create_default_context(cafile=os.environ.get("SSL_CERT_FILE") or None)
    tools = json.loads((ROOT / "os/kernel/src/tools.json").read_text())
    names = ", ".join(t["function"]["name"] for t in tools)
    cases = json.loads((ROOT / "evals/os/cases.json").read_text())["cases"]
    models = a.models.split(",")
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    log = open(out / "calls.jsonl", "a")
    lock = threading.Lock()
    spent = {"usd": 0.0, "in": 0, "out": 0, "calls": 0, "stop": ""}
    deadline = None
    if a.until:
        h, m = map(int, a.until.split(":"))
        now = time.localtime()
        deadline = time.mktime((now.tm_year, now.tm_mon, now.tm_mday, h, m, 0, 0, 0, -1))

    def charge(model, usage):
        i, o = usage.get("prompt_tokens", 0), usage.get("completion_tokens", 0)
        cached = (usage.get("prompt_tokens_details") or {}).get("cached_tokens", 0) or 0
        pin, pout = PRICE.get(model, (5.0, 15.0))
        with lock:
            spent["in"] += i
            spent["out"] += o
            spent["calls"] += 1
            spent["usd"] += ((i - cached) * pin + cached * pin * 0.12 + o * pout) / 1e6

    def stop() -> bool:
        if spent["stop"]:
            return True
        if spent["usd"] >= a.budget:
            spent["stop"] = "budget"
        elif deadline and time.time() >= deadline:
            spent["stop"] = "time"
        return bool(spent["stop"])

    def one(model, case, n):
        if stop():
            return
        try:
            r = post(key, {"model": model, "tools": tools, "max_completion_tokens": 4096, "messages": [
                {"role": "system", "content": SYSTEM.format(model=model)},
                {"role": "user", "content": case["query"]}]}, ctx)
            charge(model, r.get("usage", {}))
            msg = r["choices"][0]["message"]
            calls = [(c["function"]["name"], c["function"].get("arguments", "")) for c in msg.get("tool_calls") or []]
            said = (msg.get("content") or "").strip()
            g = post(key, {"model": "muse-spark-1.3", "max_completion_tokens": 4096, "messages": [{"role": "user",
                "content": GRADER.format(tools=names, query=case["query"], expect=case["expect"], said=said[:600],
                                         calls=json.dumps(calls)[:600] or "nothing")}]}, ctx)
            charge("muse-spark-1.3", g.get("usage", {}))
            word = re.search(r"\b(DONE|HONEST|WRONG)\b", (g["choices"][0]["message"].get("content") or "").upper())
            row = {"model": model, "case": case["id"], "n": n, "calls": [c[0] for c in calls], "said": said[:300],
                   "grade": word.group(1) if word else "UNGRADED"}
        except RuntimeError as e:
            if str(e).startswith("refused"):
                spent["stop"] = "refused: " + str(e)[:120]
            row = {"model": model, "case": case["id"], "n": n, "error": str(e)[:200]}
        with lock:
            log.write(json.dumps(row) + "\n")
            log.flush()

    jobs = [(m, c, n) for n in range(a.repeat) for m in models for c in cases]
    random.Random(1).shuffle(jobs)
    t0 = time.time()
    with cf.ThreadPoolExecutor(a.workers) as pool:
        futures = [pool.submit(one, *j) for j in jobs]
        for i, _ in enumerate(cf.as_completed(futures)):
            if i % 200 == 0:
                print(f"{i}/{len(jobs)} answers, {spent['calls']} calls, ~${spent['usd']:.2f}, "
                      f"{time.time() - t0:.0f} s{', stopping: ' + spent['stop'] if spent['stop'] else ''}", flush=True)
    summary = summarize(out / "calls.jsonl", cases)
    summary["usage"] = {k: v for k, v in spent.items()}
    (out / "summary.json").write_text(json.dumps(summary, indent=1))
    print(json.dumps(summary["models"], indent=1))
    print("usage", json.dumps(summary["usage"]))


def summarize(path: Path, cases) -> dict:
    rows = [json.loads(l) for l in open(path)]
    models, per_case = {}, {}
    for r in rows:
        if "grade" not in r:
            continue
        m = models.setdefault(r["model"], {"answers": 0, "DONE": 0, "HONEST": 0, "WRONG": 0, "UNGRADED": 0})
        m["answers"] += 1
        m[r["grade"]] += 1
        c = per_case.setdefault(r["case"], {}).setdefault(r["model"], {"answers": 0, "DONE": 0, "HONEST": 0, "WRONG": 0,
                                                                       "UNGRADED": 0, "tools": {}})
        c["answers"] += 1
        c[r["grade"]] += 1
        for t in r["calls"]:
            c["tools"][t] = c["tools"].get(t, 0) + 1
    for m in models.values():
        m["done_or_honest"] = round((m["DONE"] + m["HONEST"]) / max(1, m["answers"]), 3)
    queries = {c["id"]: c["query"] for c in cases}
    return {"models": models, "cases": {k: {"query": queries.get(k, ""), **v} for k, v in sorted(per_case.items())}}


if __name__ == "__main__":
    main()
