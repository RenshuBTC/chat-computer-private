"""LLM judges, one per failure mode that a rule can't find, each answering pass or fail, and checked against the
labels a person gave (labels/*.json) before they're trusted: the labelled traces are split once, by a hash of each
trace, into train (its examples may go in the judge's prompt), dev (to improve the prompt on) and test (looked at
once, at the end). A judge's quality is how many real failures it catches (TPR) and how many good traces it passes
(TNR), on test. The rate a judge measures on new traces is then corrected for its errors:
    true failure rate = (measured + TNR - 1) / (TPR + TNR - 1).

    META_API_KEY_FILE=... python3 evals/meta/judges.py validate extra-calls --split dev [--budget 1]
    META_API_KEY_FILE=... python3 evals/meta/judges.py run extra-calls TRACES.jsonl --tpr 0.9 --tnr 0.9 [--budget 1]

Labelled traces: each labels/NAME.json goes with the traces it labels, NAME's traces.jsonl in TRACES_DIR (default:
the evals' own copy, evals/meta/traces/NAME.jsonl)."""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import hashlib
import json
import math
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from api import Meta  # noqa: E402
from system import kernel  # noqa: E402

JUDGE_MODEL = "muse-spark-1.3"

JUDGES = {
    "extra-calls": {
        "question": "Did the assistant make tool calls that this request didn't need?",
        "fail": "FAIL when at least two of its tool calls weren't needed: searching again for what an earlier call had "
                "already found; listing folders one by one when one find_files search by a word of the name would have "
                "found it; computer_status when the question isn't about the computer itself (its drives, battery, "
                "memory, network, model or what it can do); web searches, pages or files read for something it can't do "
                "anyway, or that have nothing to do with the request.",
        "pass": "PASS when each call served the request, or when exploring was the only way (the file's name isn't known, "
                "or the question is about many folders, like what fills the drive). One unneeded call is still a PASS.",
        "examples": {"fail": ["probe-after/probe-prev-job", "before/syn-exchange", "before/os-youtube-jazz"],
                     "pass": ["after1/syn-mum-photo", "probe-before/probe-print-lease", "before/os-alarm",
                              "after1/os-usb-copy-photos"]},
    },
}


def wilson(k: int, n: int, z: float = 1.96) -> tuple[float, float]:
    if n == 0:
        return (0.0, 1.0)
    p = k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return (round(max(0.0, c - h), 3), round(min(1.0, c + h), 3))


def split_of(key: str) -> str:
    """train (15%), dev (40%) or test (45%), fixed by the trace's key."""
    h = int(hashlib.sha256(key.encode()).hexdigest()[:8], 16) % 100
    return "train" if h < 15 else "dev" if h < 55 else "test"


def labelled(traces_dir: Path) -> dict[str, dict]:
    """Every labelled trace, by key run/id: the trace and its label for each failure mode (True: failed)."""
    out = {}
    for lf in sorted((HERE / "labels").glob("*.json")):
        run = lf.stem
        tf = traces_dir / f"{run}.jsonl"
        if not tf.exists():
            continue
        traces = {json.loads(l)["id"]: json.loads(l) for l in open(tf)}
        for lab in json.loads(lf.read_text())["labels"]:
            t = traces.get(lab["id"])
            if t:
                out[f"{run}/{lab['id']}"] = {"trace": t, "modes": set(lab["modes"]), "note": lab.get("note", "")}
    return out


def cut(s: str, n: int) -> str:
    s = " ".join((s or "").split())
    return s if len(s) <= n else s[: n - 1] + "…"


def render(t: dict) -> str:
    """What the judge sees of a trace: the person's words, each tool call (its arguments, and the start of what it said
    back), and the answer: no more than the failure needs."""
    out = []
    for turn in t["turns"]:
        out.append(f"PERSON: {turn['user']}")
        n = 0
        for r in turn["rounds"]:
            for c in r["calls"]:
                n += 1
                out.append(f"  call {n}: {c['name']}({cut(json.dumps(c['args'], ensure_ascii=False), 120)}) -> {cut(c['result'], 140)}")
        out.append(f"ASSISTANT'S ANSWER: {cut(turn.get('answer') or '', 500)}")
    return "\n".join(out)


def prompt(name: str, t: dict, examples: dict[str, dict]) -> str:
    j = JUDGES[name]
    tools = "; ".join(f"{x['function']['name']}: {cut(x['function']['description'], 110)}" for x in kernel()["tools"])
    shots = []
    for verdict in ("fail", "pass"):
        for key in j["examples"][verdict]:
            ex = examples.get(key)
            if ex:
                shots.append(f"EXAMPLE ({verdict.upper()}: {ex['note']})\n{render(ex['trace'])}")
    return (f"You check one thing about an AI assistant that runs a computer through tools. {j['question']}\n\n"
            f"Its tools: {tools}\n\n{j['fail']}\n{j['pass']}\n\n" + "\n\n".join(shots) +
            f"\n\nNOW THIS ONE\n{render(t)}\n\nAnswer with one short sentence of reasons, then a last line that is "
            f"exactly VERDICT: PASS or VERDICT: FAIL.")


def judge(meta: Meta, name: str, t: dict, examples: dict[str, dict]) -> tuple[bool | None, str]:
    """True if the judge says the trace failed; None if its answer had no verdict."""
    text = ""
    for _ in range(2):          # (an answer can come back empty, its tokens all spent thinking: asked once more)
        got = meta.ask({"model": JUDGE_MODEL, "max_completion_tokens": 8192,
                        "messages": [{"role": "user", "content": prompt(name, t, examples)}]})
        text = (got["choices"][0]["message"].get("content") or "").strip()
        m = re.search(r"VERDICT:\s*(PASS|FAIL)", text.upper())
        if m:
            return m.group(1) == "FAIL", text
    return None, text


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("what", choices=["validate", "run"])
    ap.add_argument("name", choices=list(JUDGES))
    ap.add_argument("traces", nargs="?")
    ap.add_argument("--split", default="dev", choices=["dev", "test"])
    ap.add_argument("--traces-dir", default=str(HERE / "traces"))
    ap.add_argument("--tpr", type=float)
    ap.add_argument("--tnr", type=float)
    ap.add_argument("--budget", type=float, default=1.0)
    ap.add_argument("--workers", type=int, default=12)
    ap.add_argument("--out")
    a = ap.parse_args()
    meta = Meta(a.budget)
    data = labelled(Path(a.traces_dir))
    train = {k: v for k, v in data.items() if split_of(k) == "train"}
    for verdict in ("fail", "pass"):
        for key in JUDGES[a.name]["examples"][verdict]:
            assert key in train, f"{key}: an example must come from train"
    if a.what == "validate":
        items = [(k, v) for k, v in data.items() if split_of(k) == a.split]
        with cf.ThreadPoolExecutor(a.workers) as pool:
            got = list(pool.map(lambda kv: judge(meta, a.name, kv[1]["trace"], train), items))
        rows, tp, fn, tn, fp, none = [], 0, 0, 0, 0, 0
        for (k, v), (said_fail, text) in zip(items, got):
            truth = a.name in v["modes"]
            if said_fail is None:
                none += 1
            elif truth:
                tp += said_fail
                fn += not said_fail
            else:
                tn += not said_fail
                fp += said_fail
            rows.append({"key": k, "label_fail": truth, "judge_fail": said_fail, "judge": text[-400:], "note": v["note"]})
        res = {"judge": a.name, "split": a.split, "labelled": len(items), "fails": tp + fn, "passes": tn + fp,
               "no_verdict": none, "tpr": round(tp / max(1, tp + fn), 3), "tpr_95": wilson(tp, tp + fn),
               "tnr": round(tn / max(1, tn + fp), 3), "tnr_95": wilson(tn, tn + fp), "usage": meta.spent}
        print(json.dumps(res))
        for r in rows:
            if r["judge_fail"] is not None and r["judge_fail"] != r["label_fail"]:
                print(f"DISAGREE {r['key']} label={'FAIL' if r['label_fail'] else 'PASS'}: {r['note']} || judge: {cut(r['judge'], 300)}")
        if a.out:
            Path(a.out).write_text(json.dumps({"result": res, "rows": rows}, indent=1, ensure_ascii=False))
        return
    traces = [json.loads(l) for l in open(a.traces) if not json.loads(l).get("error")]
    with cf.ThreadPoolExecutor(a.workers) as pool:
        got = list(pool.map(lambda t: judge(meta, a.name, t, train), traces))
    verdicts = [g for g, _ in got if g is not None]
    measured = sum(verdicts) / max(1, len(verdicts))
    res = {"judge": a.name, "traces": len(traces), "judged": len(verdicts), "measured_fail_rate": round(measured, 3),
           "failing": [t["id"] for t, (g, _) in zip(traces, got) if g]}
    if a.tpr and a.tnr and a.tpr + a.tnr > 1:
        res["corrected_fail_rate"] = round(min(1.0, max(0.0, (measured + a.tnr - 1) / (a.tpr + a.tnr - 1))), 3)
    res["usage"] = meta.spent
    print(json.dumps(res))
    if a.out:
        Path(a.out).write_text(json.dumps({"result": res, "verdicts": [{"id": t["id"], "fail": g, "judge": x[-400:]}
                                                                        for t, (g, x) in zip(traces, got)]}, indent=1, ensure_ascii=False))


if __name__ == "__main__":
    main()
