"""review.html: the labelled traces, for the owner to read and correct the labels (the evals FAQ's custom annotation
tool). Each trace shows what the person asked, each tool call (what it said back folded away), the answer, the code
checks, and the labels, which can be changed: j and k go to the next and previous trace, 1 to 4 toggle the failure
modes, 0 clears them, and "Download labels" saves the labels in labels/*.json's form. Changes are kept in the
browser until then.

    python3 evals/meta/make_review.py   # writes evals/meta/review.html"""
from __future__ import annotations

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import checks  # noqa: E402

RUNS = ["before", "after1", "after2", "probe-before", "probe-after", "probe-after2"]
MODES = ["extra-calls", "asks-or-gives-up", "claims-or-suggests-the-impossible", "wrong-assumption"]


def data() -> list[dict]:
    out = []
    for run in RUNS:
        labels = {o["id"]: o for o in json.loads((HERE / "labels" / f"{run}.json").read_text())["labels"]}
        for line in open(HERE / "traces" / f"{run}.jsonl"):
            t = json.loads(line)
            lab = labels.get(t["id"], {"modes": [], "note": ""})
            turns = []
            for turn in t["turns"]:
                calls = [{"n": c["name"], "a": json.dumps(c["args"], ensure_ascii=False), "r": c["result"][:1500], "e": c["error"]}
                         for r in turn["rounds"] for c in r["calls"]]
                turns.append({"u": turn["user"], "c": calls, "a": turn.get("answer") or ""})
            failed = [k for k, v in checks.check(t).items() if not v]
            out.append({"run": run, "id": t["id"], "turns": turns, "checks": failed, "modes": lab["modes"], "note": lab.get("note", "")})
    return out


PAGE = r"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Eval label review</title>
<style>
:root { --bg:#f6f6f3; --panel:#ffffff; --ink:#1d1d1b; --muted:#6b6b66; --line:#deded8; --fail:#b3261e; --pass:#1e6b3a; --accent:#2f4fb3; }
@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) { --bg:#171716; --panel:#212120; --ink:#ececea; --muted:#a3a39e; --line:#363634; --fail:#f2948c; --pass:#8fd19e; --accent:#9db0f2; } }
:root[data-theme="dark"] { --bg:#171716; --panel:#212120; --ink:#ececea; --muted:#a3a39e; --line:#363634; --fail:#f2948c; --pass:#8fd19e; --accent:#9db0f2; }
* { box-sizing:border-box } body { margin:0; background:var(--bg); color:var(--ink); font:15px/1.5 system-ui, sans-serif; }
header { display:flex; flex-wrap:wrap; gap:8px 16px; align-items:center; padding:12px 16px; border-bottom:1px solid var(--line); }
header h1 { font-size:16px; margin:0; } select, button, textarea { font:inherit; color:inherit; background:var(--panel); border:1px solid var(--line); border-radius:6px; padding:4px 8px; }
button:focus-visible, select:focus-visible, textarea:focus-visible, input:focus-visible { outline:2px solid var(--accent); outline-offset:1px; }
main { max-width:860px; margin:0 auto; padding:16px; } .meta { color:var(--muted); font-size:13px; }
.turn { background:var(--panel); border:1px solid var(--line); border-radius:8px; padding:12px; margin:12px 0; }
.you { font-weight:600; } .answer { white-space:pre-wrap; margin-top:8px; } details { margin:4px 0; font-size:13px; }
summary { cursor:pointer; } .err { color:var(--fail); } pre { white-space:pre-wrap; margin:4px 0 0 16px; color:var(--muted); }
.labels { background:var(--panel); border:1px solid var(--line); border-radius:8px; padding:12px; position:sticky; bottom:8px; }
.labels label { display:inline-flex; gap:6px; margin:2px 14px 2px 0; } textarea { width:100%; margin-top:6px; }
.fail { color:var(--fail); } .pass { color:var(--pass); } kbd { border:1px solid var(--line); border-radius:4px; padding:0 4px; font-size:12px; }
</style></head><body>
<header><h1>Eval label review</h1>
<label>Run <select id="run"></select></label>
<label>Show <select id="show"><option value="all">all</option><option value="fail">labelled failures</option><option value="changed">changed</option></select></label>
<span id="pos" class="meta"></span>
<button id="prev" aria-label="Previous trace">← <kbd>k</kbd></button><button id="next" aria-label="Next trace"><kbd>j</kbd> →</button>
<button id="dl">Download labels</button></header>
<main id="main"></main>
<script>
const DATA = __DATA__, MODES = __MODES__;
let edits = {}; try { edits = JSON.parse(localStorage.getItem("evals-meta-labels") || "{}"); } catch (e) {}
const key = t => t.run + "/" + t.id, modesOf = t => (edits[key(t)] || {modes: t.modes}).modes, noteOf = t => (edits[key(t)] || {note: t.note}).note;
const runs = [...new Set(DATA.map(t => t.run))], runSel = document.getElementById("run"), showSel = document.getElementById("show");
runs.forEach(r => runSel.add(new Option(r, r)));
let i = 0;
function list() { return DATA.filter(t => t.run === runSel.value && (showSel.value === "all" || (showSel.value === "fail" ? modesOf(t).length : key(t) in edits))); }
function esc(s) { return String(s).replace(/[&<>]/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;"}[c])); }
function save() { try { localStorage.setItem("evals-meta-labels", JSON.stringify(edits)); } catch (e) {} }
function draw() {
  const L = list(), m = document.getElementById("main");
  document.getElementById("pos").textContent = L.length ? `Trace ${i + 1} of ${L.length} · ${Object.keys(edits).length} changed` : "No traces";
  if (!L.length) { m.innerHTML = ""; return; }
  i = Math.max(0, Math.min(i, L.length - 1)); const t = L[i], ms = modesOf(t);
  let h = `<p class="meta">${esc(t.run)} / ${esc(t.id)} · code checks: ${t.checks.length ? '<span class="fail">' + esc(t.checks.join(", ")) + '</span>' : '<span class="pass">all pass</span>'}</p>`;
  for (const turn of t.turns) {
    h += `<div class="turn"><div class="you">${esc(turn.u)}</div>`;
    turn.c.forEach((c, n) => h += `<details><summary${c.e ? ' class="err"' : ''}>${n + 1}. ${esc(c.n)}(${esc(c.a.slice(0, 160))})</summary><pre>${esc(c.r)}</pre></details>`);
    h += `<div class="answer">${esc(turn.a)}</div></div>`;
  }
  h += `<div class="labels"><strong class="${ms.length ? "fail" : "pass"}">${ms.length ? "Fails" : "Passes"}</strong> · `;
  MODES.forEach((md, n) => h += `<label><input type="checkbox" data-m="${md}" ${ms.includes(md) ? "checked" : ""}> <kbd>${n + 1}</kbd> ${esc(md)}</label>`);
  h += `<textarea id="note" rows="2" aria-label="Note">${esc(noteOf(t))}</textarea></div>`;
  m.innerHTML = h;
  m.querySelectorAll("input[data-m]").forEach(b => b.onchange = () => set(t, b.dataset.m, b.checked));
  document.getElementById("note").oninput = e => { edits[key(t)] = {modes: modesOf(t), note: e.target.value}; save(); };
}
function set(t, md, on) { let ms = modesOf(t).filter(x => x !== md); if (on) ms.push(md); edits[key(t)] = {modes: ms, note: noteOf(t)}; save(); draw(); }
document.getElementById("prev").onclick = () => { i--; draw(); }; document.getElementById("next").onclick = () => { i++; draw(); };
runSel.onchange = showSel.onchange = () => { i = 0; draw(); };
document.addEventListener("keydown", e => {
  if (e.target.tagName === "TEXTAREA") return;
  const t = list()[i]; if (e.key === "j") { i++; draw(); } else if (e.key === "k") { i--; draw(); }
  else if (t && e.key >= "1" && e.key <= String(MODES.length)) { const md = MODES[+e.key - 1]; set(t, md, !modesOf(t).includes(md)); }
  else if (t && e.key === "0") { edits[key(t)] = {modes: [], note: noteOf(t)}; save(); draw(); }
});
document.getElementById("dl").onclick = () => {
  const out = {}; for (const t of DATA) { (out[t.run] = out[t.run] || {by: "the owner, reviewing", labels: []}).labels.push({id: t.id, modes: modesOf(t), first_failure: modesOf(t)[0] || null, note: noteOf(t)}); }
  const a = document.createElement("a"); a.href = URL.createObjectURL(new Blob([JSON.stringify(out, null, 1)], {type: "application/json"})); a.download = "labels-reviewed.json"; a.click();
};
draw();
</script></body></html>
"""


if __name__ == "__main__":
    html = PAGE.replace("__DATA__", json.dumps(data(), ensure_ascii=False).replace("</", "<\\/")).replace("__MODES__", json.dumps(MODES))
    (HERE / "review.html").write_text(html)
    print(f"review.html: {len(html) // 1000} KB")
