"""Print desktop traces compactly, for reading them one after another (the evals' error analysis).

    python evals/tools/show_os.py RUN_DIR [id ...]"""
import json
import os
import sys


def short(v, n=160):
    s = v if isinstance(v, str) else json.dumps(v, ensure_ascii=False)
    s = " ".join(s.split())
    return s if len(s) <= n else s[:n] + "…"


def show(d):
    st = d.get("state", {})
    bits = [st.get("device", "")]
    for k in ("printers", "usb", "open_page"):
        if st.get(k):
            bits.append(f"{k}={short(st[k], 60)}")
    print(f"=== {d['id']} [{d.get('modality')}] ({', '.join(bits)}) {d.get('seconds')} s")
    print(f"  Q: {d['query']}")
    for t in d.get("tools", []):
        print(f"  - {t['name']}({short(t['input'], 140)}) {'ok' if t['ok'] else 'FAILED'} -> {short(t['result'], 220)}")
    for c in d.get("confirmations", []):
        print(f"  ? {short(c['question'], 200)}")
    if d.get("platform_calls"):
        print(f"  machine: {short(d['platform_calls'], 200)}")
    f = d.get("files", {})
    if f.get("added") or f.get("removed"):
        print(f"  files +{f.get('added')} -{f.get('removed')}")
    if d.get("trash"):
        print(f"  trash: {d['trash']}")
    sh = [s for s in d.get("screen", []) if s[0] in ("show_url", "show_html", "notice")]
    if sh:
        print(f"  screen: {short(sh, 200)}")
    if d.get("errors"):
        print(f"  ERRORS: {short(d['errors'], 300)}")
    print(f"  A: {d.get('reply', '')}")


if __name__ == "__main__":
    run = sys.argv[1]
    order = [c["id"] for c in json.load(open(os.path.join(os.path.dirname(__file__), "..", "os", "cases.json")))["cases"]]
    want = sys.argv[2:] or order
    for i in want:
        p = os.path.join(run, "traces", i + ".json")
        if os.path.exists(p):
            show(json.load(open(p)))
