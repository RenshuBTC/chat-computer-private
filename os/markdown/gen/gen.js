// Runs chatcomputer/ui/assets/md.js on the Markdown in cases.json and writes, for each case, a plain description of
// what it made (blocks, and runs of text with their styles), as os/markdown/src/cases.rs, for os/markdown's test:
//   node os/markdown/gen/gen.js chatcomputer/ui/assets/md.js os/markdown/gen/cases.json os/markdown/src/cases.rs
const fs = require("fs");
const path = process.argv[2];
global.window = {};
eval(fs.readFileSync(path, "utf8"));
const render = window.renderMarkdown;

function unesc(s) { return s.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&amp;/g, "&"); }
function q(s) { return s.replace(/\\/g, "\\\\").replace(/\n/g, "\\n").replace(/\|/g, "\\|").replace(/ /g, "\\_").replace(/\[/g, "\\[").replace(/\]/g, "\\]").replace(/;/g, "\;").replace(/,/g, "\\,").replace(/\//g, "\\/"); }
// Inline HTML (as md.js writes it) to runs: [style, text], styles B=1 I=2 C=4 S=8 L=16.
function runs(html) {
  const out = [];
  const stack = [0];
  const re = /<(\/?)(strong|em|code|del|span|br)([^>]*)>|([^<]+)/g;
  let m;
  while ((m = re.exec(html))) {
    if (m[4] !== undefined) { push(out, stack[stack.length - 1], unesc(m[4])); continue; }
    const close = m[1] === "/", tag = m[2], attrs = m[3];
    if (tag === "br") { push(out, stack[stack.length - 1], "\n"); continue; }
    if (close) { stack.pop(); continue; }
    let bit = 0;
    if (tag === "strong") bit = 1; else if (tag === "em") bit = 2; else if (tag === "code") bit = 4; else if (tag === "del") bit = 8;
    else if (tag === "span" && /class="link"/.test(attrs)) bit = 16;
    stack.push(stack[stack.length - 1] | bit);
  }
  return out.map(([s, t]) => s + ":" + q(t)).join(" ");
}
function push(out, style, text) {
  if (!text) return;
  const last = out[out.length - 1];
  if (last && last[0] === style) last[1] += text; else out.push([style, text]);
}
// Block HTML to the canonical form, one block after another.
function blocks(html) {
  const out = [];
  let rest = html;
  const take = (re) => { const m = rest.match(re); if (m) rest = rest.slice(m[0].length); return m; };
  while ((rest = rest.replace(/^\n+/, "")).length) {
    let m;
    if ((m = take(/^<p>([\s\S]*?)<\/p>/))) out.push("P " + runs(m[1]));
    else if ((m = take(/^<h(\d)>([\s\S]*?)<\/h\1>/))) out.push("H" + m[1] + " " + runs(m[2]));
    else if ((m = take(/^<pre><code>([\s\S]*?)<\/code><\/pre>/))) out.push("C " + q(unesc(m[1])));
    else if ((m = take(/^<hr>/))) out.push("HR");
    else if (rest.startsWith("<blockquote>")) {
      // (the inner HTML up to the matching close: md.js nests quotes)
      let depth = 0, i = 0;
      for (;;) {
        const o = rest.indexOf("<blockquote>", i), c = rest.indexOf("</blockquote>", i);
        if (o !== -1 && o < c) { depth++; i = o + 12; } else { depth--; i = c + 13; if (depth === 0) break; }
      }
      const inner = rest.slice(12, i - 13);
      rest = rest.slice(i);
      out.push("Q[" + blocks(inner).join(";") + "]");
    }
    else if ((m = take(/^<table><thead><tr>([\s\S]*?)<\/tr><\/thead><tbody>([\s\S]*?)<\/tbody><\/table>/))) {
      const head = [...m[1].matchAll(/<th>([\s\S]*?)<\/th>/g)].map((c) => runs(c[1]));
      const rows = [...m[2].matchAll(/<tr>([\s\S]*?)<\/tr>/g)].map((r) => [...r[1].matchAll(/<td>([\s\S]*?)<\/td>/g)].map((c) => runs(c[1])).join("|"));
      out.push("T [" + head.join("|") + "] [" + rows.join("/") + "]");
    }
    else if ((m = take(/^<(ul|ol)( start="(\d+)")?>([\s\S]*?)<\/\1>/))) {
      const items = [...m[4].matchAll(/<li( class="task( done)?")?>([\s\S]*?)<\/li>/g)].map((it) =>
        (it[1] ? (it[2] ? "T1 " : "T0 ") : "") + runs(it[3]));
      out.push("L" + (m[1] === "ol" ? "o" : "u") + (m[1] === "ol" ? (m[3] || "1") : "1") + " [" + items.join(",") + "]");
    }
    else throw new Error("unexpected: " + rest.slice(0, 80));
  }
  return out;
}
const cases = JSON.parse(fs.readFileSync(process.argv[3], "utf8"));
const lines = ["// Made by os/markdown/gen/gen.js, which runs chatcomputer/ui/assets/md.js on each case in cases.json: what it renders,",
  "// as blocks and runs of styled text (1 bold, 2 italic, 4 code, 8 struck, 16 link).", "pub const CASES: &[(&str, &str)] = &["];
for (const md of cases) {
  const got = blocks(render(md)).join("\n");
  lines.push("    (" + JSON.stringify(md) + ", " + JSON.stringify(got) + "),");
}
lines.push("];", "");
fs.writeFileSync(process.argv[4], lines.join("\n"));
console.log("wrote", cases.length, "cases");
