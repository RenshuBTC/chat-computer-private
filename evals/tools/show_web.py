"""Print website traces compactly, for reading them one after another (the evals' error analysis).

    python evals/tools/show_web.py RESULTS.json [id ...]

Each case: what was typed, and (with Chat Computer's model) each round of the model's: the tools it called, with what,
and what each gave back; then the answer as the visitor saw it, what opened between the sidebars, what Actions
listed, and how long it took."""
import json
import sys


def short(v, n=160):
    s = v if isinstance(v, str) else json.dumps(v, ensure_ascii=False)
    s = " ".join(s.split())
    return s if len(s) <= n else s[:n] + "…"


def show(r):
    secs = round((r.get("total_ms") or 0) / 1000)
    print(f"=== {r['id']} [{r.get('intent')}/{r.get('phrasing')}] {secs} s" + (f", first words {round(r['first_ms'] / 1000, 1)} s" if r.get("first_ms") else ""))
    turns = r.get("turns") or [r.get("query", "")]
    answers = r.get("answers") or [r.get("answer", "")]
    msgs = r.get("messages") or []
    if msgs:
        # The conversation as the model last saw it: what was typed, its tool calls, what they gave back, its words.
        out = (r.get("rounds") or [{}])[-1].get("out") or {}
        for m in msgs + ([out] if out else []):
            role = m.get("role")
            if role == "user":
                print(f"  Q: {m.get('content')}")
            elif role == "assistant":
                for c in m.get("tool_calls") or []:
                    f = c.get("function") or {}
                    print(f"  - {f.get('name')}({short(f.get('arguments', ''), 140)})")
                if (m.get("content") or "").strip():
                    print(f"  > {short(m['content'], 400)}")
            elif role == "tool":
                print(f"    -> {short(m.get('content', ''), 220)}")
        for k, x in enumerate(r.get("rounds") or []):
            if x.get("error"):
                print(f"  round {k + 1} FAILED: {x['error']}")
    for i, (q, a) in enumerate(zip(turns, answers)):
        if not msgs:
            print(f"  Q: {q}")
        print(f"  A{i + 1 if len(answers) > 1 else ''}: {a}")
    if r.get("open"):
        print(f"  open: {short(r.get('title') or r.get('src') or '', 80)} | {short(r.get('body', ''), 160)}")
    if r.get("done_added"):
        print(f"  actions: {r['done'][-r['done_added']:]}")
    if r.get("errors"):
        print(f"  ERRORS: {short(r['errors'], 300)}")
    if r.get("rounds"):
        u = [x.get("usage") or {} for x in r["rounds"]]
        print(f"  rounds {len(r['rounds'])}: " + ", ".join(f"{round((x.get('ms') or 0) / 1000)} s ({(y.get('prompt_tokens') or 0)}+{(y.get('completion_tokens') or 0)} tok, {x.get('finish')})"
                                                     for x, y in zip(r["rounds"], u)))
    print()


def main():
    rs = json.load(open(sys.argv[1]))
    want = set(sys.argv[2:])
    for r in rs:
        if not want or r["id"] in want:
            show(r)


if __name__ == "__main__":
    main()
