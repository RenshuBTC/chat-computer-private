// Chat Computer's own model engine in the website's chat (docs/demo/engine: os/model as WebAssembly, through
// os/webengine, in a Web Worker), tried in Chromium with Playwright, with no network: Hugging Face is stood in for by
// local files (served in pieces, as its servers serve Range requests), the page by a local server.
//
// A tiny model of random weights (tests/data/tiny-model.ccm) is got ready: downloaded, checked against its SHA-256,
// kept in the browser and loaded; the first message read ahead; then questions answered with the likeliest tokens
// (as the engine's program on Linux answers them: tests/test_web_engine.py asks os/model's kernel_check), each
// answer's pieces streamed as the program streams them; thinking; an answer stopped. Then a later visit: the model
// and the first message's reading come from the browser's copy, with nothing downloaded. Then a small model as Qwen
// publishes its models (config.json, tokenizer.json, its chat template, two safetensors files), 8 and 4 bits a weight:
// the model's file the browser makes from them, and its answer, against the program's. Then the model forgotten.
// Last, how fast it reads and writes (the tiny model; with --bench, a model of Qwen3.5 0.8B's shape too).
//
// Prints JSON.  Run: node tests/web_engine_check.js SETUP.json   (tests/test_web_engine.py writes SETUP.json)
const fs = require("fs");
const http = require("http");
const path = require("path");
const crypto = require("crypto");
const { chromium } = require("playwright");

const setup = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const DOCS = path.resolve(__dirname, "..", "docs");
const sha = (b) => crypto.createHash("sha256").update(b).digest("hex");

// The models Hugging Face stands in for: each its repository, revision and files.
function model(name, repo, bits, files) {
  return { name, label: name, repo, revision: crypto.createHash("sha1").update(name).digest("hex"), bits, base: "https://huggingface.co",
           files: Object.keys(files).sort().map((f) => ({ name: f, size: files[f].length, sha256: sha(files[f]) })), bytes: files };
}
const tinyBytes = fs.readFileSync(setup.tiny);
const published = {};
for (const f of fs.readdirSync(setup.published)) published[f] = fs.readFileSync(path.join(setup.published, f));
const MODELS = {
  tiny: model("tiny-test", "test/tiny", 8, { "tiny-model.ccm": tinyBytes }),
  published8: model("tiny-published", "test/published", 8, published),
  published4: model("tiny-published", "test/published", 4, published),
  // The tiny model with a refusal direction beside the engine (taken out as it runs); and one whose pin is wrong.
  uncensored: model("tiny-uncensored", "test/tiny", 8, { "tiny-model.ccm": tinyBytes }),
  misPinned: model("tiny-mispinned", "test/tiny", 8, { "tiny-model.ccm": tinyBytes }),
};
MODELS.published4.revision = MODELS.published8.revision;
MODELS.uncensored.revision = MODELS.misPinned.revision = MODELS.tiny.revision;
const refusalBytes = fs.readFileSync(setup.refusal);
MODELS.uncensored.refusal = { name: "tiny.refusal", size: refusalBytes.length, sha256: sha(refusalBytes) };
MODELS.misPinned.refusal = { name: "tiny.refusal", size: refusalBytes.length, sha256: sha(Buffer.from("another file")) };
let current = MODELS.tiny;
const asked = [];            // what was asked of Hugging Face (path and range)

// The page: the engine's own files from docs/, and a blank page that loads engine.js (as the website's would); and the
// tiny model, for the speed check (which loads it into the module itself).
const server = http.createServer((req, res) => {
  const p = decodeURIComponent(req.url.split("?")[0]);
  if (p === "/demo/engine/check.html") {
    res.writeHead(200, { "content-type": "text/html" });
    res.end('<!doctype html><meta charset="utf-8"><title>Engine check</title><script src="engine.js"></script>');
    return;
  }
  if (p === "/tiny-model.ccm" || p === "/demo/engine/tiny.refusal") {
    res.writeHead(200, { "content-type": "application/octet-stream" });
    res.end(p === "/tiny-model.ccm" ? tinyBytes : refusalBytes);
    return;
  }
  const f = path.join(DOCS, p);
  if (!f.startsWith(DOCS + path.sep) || !fs.existsSync(f) || !fs.statSync(f).isFile()) { res.writeHead(404); res.end(); return; }
  const type = f.endsWith(".wasm") ? "application/wasm" : f.endsWith(".js") ? "text/javascript" : "application/json";
  res.writeHead(200, { "content-type": type });
  res.end(fs.readFileSync(f));
});

const sleep = (ms) => new Promise((go) => setTimeout(go, ms));

async function main() {
  await new Promise((go) => server.listen(0, "127.0.0.1", go));
  const base = "http://127.0.0.1:" + server.address().port;
  const out = { errors: [] };
  const browser = await chromium.launch();
  const context = await browser.newContext();
  // No network: only the local page, and Hugging Face as stood in for here.
  await context.route("**/*", (route) => {
    const url = route.request().url();
    if (url.startsWith(base + "/") && !url.endsWith("/demo/engine/model.json")) return route.continue();
    if (url.endsWith("/demo/engine/model.json")) {
      const m = Object.assign({}, current);
      delete m.bytes;
      return route.fulfill({ contentType: "application/json", body: JSON.stringify(m) });
    }
    const hf = /^https:\/\/huggingface\.co\/([^/]+\/[^/]+)\/resolve\/([0-9a-f]{40})\/([^/?]+)$/.exec(url);
    const m = hf && Object.values(MODELS).find((x) => x.repo === hf[1] && x.revision === hf[2]);
    const bytes = m && m.bytes[decodeURIComponent(hf[3])];
    if (!bytes) { out.errors.push("asked for " + url); return route.abort(); }
    const range = /^bytes=(\d+)-(\d+)$/.exec(route.request().headers().range || "");
    asked.push(decodeURIComponent(hf[3]) + " " + (range ? range[1] + "-" + range[2] : "whole"));
    const headers = { "Access-Control-Allow-Origin": "*", "Access-Control-Expose-Headers": "Content-Range" };
    if (!range) return route.fulfill({ status: 200, headers, body: bytes });
    const a = +range[1], b = Math.min(+range[2], bytes.length - 1);
    headers["Content-Range"] = `bytes ${a}-${b}/${bytes.length}`;
    return route.fulfill({ status: 206, headers, body: bytes.subarray(a, b + 1) });
  });
  const open = async () => {
    const page = await context.newPage();
    page.on("pageerror", (e) => out.errors.push("page: " + e.message));
    page.on("console", (m) => { if (m.type() === "error") out.errors.push("console: " + m.text()); });
    await page.goto(base + "/demo/engine/check.html");
    return page;
  };
  // A question asked in the page ({request, stopAfter}: stopped after that many pieces of words): the answer's pieces
  // as they came, how far its reading had got, and the whole message.
  const ASK = async (a) => {
    const E = window.ChatComputerEngine, deltas = [], reading = [];
    let end = null, said = 0;
    const message = await E.chat(a.request, (d, e) => {
      deltas.push(d);
      if (e) end = e;
      if (d.content && ++said === a.stopAfter) E.stop();
    }, { onReading: (done, total) => reading.push([done, total]) });
    return { message, deltas, end, reading: reading.length ? reading[reading.length - 1] : null };
  };
  const ask = (page, request, stopAfter) => page.evaluate(ASK, { request, stopAfter });

  // ---------------------------------------------------------------- a first visit: downloaded, kept, answering
  let page = await open();
  out.first = await page.evaluate(async (a) => {
    const E = window.ChatComputerEngine, r = { progress: [] };
    r.supported = E.supported();
    r.before = await E.status();
    r.info = await E.prepare((p) => r.progress.push(p.stage));
    r.after = await E.status();
    r.ahead = await E.ahead(a.first, () => {});
    return r;
  }, setup);
  out.answers = {};
  for (const q of Object.keys(setup.questions)) out.answers[q] = await ask(page, setup.questions[q]);
  out.thinking = await ask(page, Object.assign({}, setup.questions.hello, { think: true, max_tokens: 40 }));
  out.stopped = await ask(page, Object.assign({}, setup.questions.hello, { max_tokens: 4000 }), 3);
  out.after_stop = await ask(page, setup.questions.hello);
  out.first_downloads = asked.length;
  await page.close();

  // ---------------------------------------------------------------- a later visit: from the browser's copy
  page = await open();
  out.later = await page.evaluate(async () => {
    const r = { progress: [] };
    r.info = await window.ChatComputerEngine.prepare((p) => r.progress.push(p.stage));
    return r;
  });
  out.later.answer = await ask(page, setup.questions.hello);
  out.later.downloads = asked.length - out.first_downloads;
  // A question asked while a long first message is read ahead: it takes over, from as far as the reading got.
  out.taken_over = await page.evaluate(async (a) => {
    const E = window.ChatComputerEngine;
    const ahead = E.ahead({ messages: a.request.messages.slice(0, 1), tools: a.request.tools });
    const deltas = [];
    let end = null;
    const message = await E.chat(a.request, (d, e) => { deltas.push(d); if (e) end = e; });
    return { ahead: await ahead, message, deltas, end };
  }, { request: setup.long_first });
  await page.close();

  // ---------------------------------------------------------------- Qwen's published files, made into the model here
  for (const k of ["published8", "published4"]) {
    current = MODELS[k];
    page = await open();
    const name = current.name + "-" + current.revision.slice(0, 12) + "-" + current.bits + "bit.ccm";
    out[k] = await page.evaluate(async (a) => {
      const E = window.ChatComputerEngine, r = { progress: [] };
      r.info = await E.prepare((p) => r.progress.push(p.stage));
      const dir = await (await navigator.storage.getDirectory()).getDirectoryHandle("chatcomputer-engine");
      const bytes = await (await (await dir.getFileHandle(a.name)).getFile()).arrayBuffer();
      r.kept_sha256 = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (b) => b.toString(16).padStart(2, "0")).join("");
      r.kept_bytes = bytes.byteLength;
      return r;
    }, { name });
    out[k].answer = await ask(page, setup.questions.hello);
    if (k === "published4") {
      // Forgotten: the browser's copy deleted (the others' too, for the next run: a new browser has none anyway).
      out.forgotten = await page.evaluate(async (a) => {
        const E = window.ChatComputerEngine;
        await E.forget();
        const r = { status: await E.status(), files: [] };
        const dir = await (await navigator.storage.getDirectory()).getDirectoryHandle("chatcomputer-engine");
        for await (const n of dir.keys()) r.files.push(n);
        r.chat_after = await E.chat(a.hello, () => {}).then(() => "answered", (e) => e.message);
        return r;
      }, setup.questions);
    }
    await page.close();
  }

  // ---------------------------------------------------------------- a refusal direction taken out as it runs
  for (const k of ["uncensored", "misPinned"]) {
    current = MODELS[k];
    page = await open();
    out[k] = await page.evaluate(async () => {
      try {
        const info = await window.ChatComputerEngine.prepare();
        return { info };
      } catch (e) { return { error: e.message }; }
    });
    if (k === "uncensored") out[k].answer = await ask(page, setup.questions.hello);
    await page.close();
  }

  // ---------------------------------------------------------------- how fast: the module itself, on the page
  page = await open();
  out.speed = await page.evaluate(async (a) => {
    const SPEED = async (bytesUrl, n) => {
      const { instance } = await WebAssembly.instantiateStreaming(fetch("engine.wasm"), {});
      const W = instance.exports, model = new Uint8Array(await (await fetch(bytesUrl)).arrayBuffer());
      W.cc_init();
      const mem = (p, len) => new Uint8Array(W.memory.buffer, p >>> 0, len);
      mem(W.cc_model_alloc(model.length), model.length).set(model);
      if (W.cc_load(0, 0) < 0) throw new Error("not loaded");
      // A prompt of about n tokens (a byte each, for this model), read 64 at a time; then 256 tokens written.
      const words = "the quick brown fox jumps over the lazy dog ".repeat(Math.ceil(n / 44)).slice(0, n);
      const body = new TextEncoder().encode(JSON.stringify({ messages: [{ role: "user", content: words }], think: false,
                                                              temperature: 0, max_tokens: 256 }));
      const p = W.cc_alloc(body.length);
      mem(p, body.length).set(body);
      if (W.cc_chat(p, body.length) < 0) throw new Error("not asked");
      const prompt = JSON.parse(new TextDecoder().decode(mem(W.cc_out_ptr(), W.cc_out_len()))).prompt_tokens;
      let t = performance.now(), steps = 0, r;
      for (;;) {
        r = W.cc_step(64);
        if (!new TextDecoder().decode(mem(W.cc_out_ptr(), W.cc_out_len())).startsWith('{"read"')) break;
      }
      const read = performance.now() - t;
      t = performance.now();
      while (r === 1) { r = W.cc_step(64); steps++; }
      const wrote = performance.now() - t;
      return { prompt_tokens: prompt, read_tok_s: prompt / read * 1000, written: steps + 1, write_tok_s: (steps + 1) / wrote * 1000 };
    };
    return SPEED(a.tinyUrl, 2000);
  }, { tinyUrl: "/tiny-model.ccm" }).catch((e) => ({ error: e.message }));
  await page.close();

  if (setup.bench) {
    // A model of Qwen3.5 0.8B's shape with random weights (the bench module makes it in memory): its reading 64 tokens
    // at a time, and its writing a token at a time (after a few, so the browser has made the code fast).
    page = await open();
    await page.route("**/bench.wasm", (r) => r.fulfill({ contentType: "application/wasm", body: fs.readFileSync(setup.bench) }));
    out.bench = await page.evaluate(async () => {
      const { instance } = await WebAssembly.instantiateStreaming(fetch("bench.wasm"), {});
      const W = instance.exports, r = {};
      W.cc_init();
      let t = performance.now();
      if (W.cc_bench_make() < 0) throw new Error("no bench model");
      r.made_s = (performance.now() - t) / 1000;
      r.memory_gb = W.memory.buffer.byteLength / 1e9;
      const reads = [], writes = [];
      for (let i = 0; i < 3; i++) { t = performance.now(); W.cc_bench_forward(64); reads.push(64000 / (performance.now() - t)); }
      for (let i = 0; i < 20; i++) { t = performance.now(); W.cc_bench_forward(1); writes.push(1000 / (performance.now() - t)); }
      r.read_tok_s = reads;
      r.write_tok_s = writes;
      return r;
    }).catch((e) => ({ error: e.message }));
    await page.close();
  }
  await browser.close();
  server.close();
  return out;
}

main().then((out) => console.log(JSON.stringify(out)), (e) => { console.log(JSON.stringify({ failed: String(e && e.stack || e) })); process.exit(1); });
