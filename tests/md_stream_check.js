// Claude's words stream in, and the conversation page (chat.js) draws the finished part of a message once and
// only the part still being written again. That must give exactly what drawing the whole message at once gives,
// at every moment of the stream. Prints JSON: how many moments were checked and how many differed.
// Run:  node tests/md_stream_check.js chatcomputer/ui/assets/md.js chatcomputer/ui/assets/chat.js
const fs = require("fs");
global.window = {};
eval(fs.readFileSync(process.argv[2], "utf8"));          // md.js -> window.renderMarkdown
const chat = fs.readFileSync(process.argv[3], "utf8");
eval(chat.slice(chat.indexOf("var OPEN"), chat.indexOf("function paint")));   // OPEN, CLOSE, BLANK, finishedUpTo
const render = window.renderMarkdown;
const norm = (h) => h.replace(/>\s+</g, "><").trim();
const samples = [
  "Hello **there**.\n\nSecond paragraph\nwith two lines.\n\n- a\n- b\n\n1. one\n\n2. two\n\n3. three",
  "Code:\n\n```python\nx = 1\n\ny = 2\n```\n\nAfter the code.\n\n> quote\n> more\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nEnd.",
  "# Title\n\nText\n---\n\n```\nunclosed fence\n\nstill code",
  "- [ ] task\n- [x] done\n\n  indented continuation\n\n###not heading\n#### Heading 4\n\n``` not a fence\ntext",
  "   \n\n\nleading blanks\n\n\n\nmany blanks\n",
];
let checked = 0, bad = 0;
const examples = [];
for (const text of samples) {
  for (let step = 1; step <= 7; step++) {          // streamed in pieces of every size from 1 to 7 characters
    let parts = [], cut = 0;
    for (let n = step; n < text.length + step; n += step) {
      const shown = text.slice(0, Math.min(n, text.length));
      const c = finishedUpTo(shown, cut);
      if (c > cut) { parts.push(render(shown.slice(cut, c))); cut = c; }
      const tail = shown.slice(cut);
      const got = norm(parts.join("") + (tail ? render(tail) : ""));
      const want = norm(render(shown));
      checked++;
      if (got !== want) { bad++; if (examples.length < 3) examples.push({ shown, got, want }); }
    }
  }
}
// A numbered list that goes on after a blank line keeps its numbers.
const ol = norm(render("1. one\n\n2. two\n\n3. three"));
console.log(JSON.stringify({ checked, bad, examples, ol }));
