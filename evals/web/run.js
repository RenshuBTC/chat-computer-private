// Website eval: each case is a fresh visit to chatcomputer.ai's page (docs/index.html, served from this machine, in
// Chromium with Playwright), the case's words typed in the box (a conversation's turns one after another, each after
// the answer to the one before), and what the visitor then sees recorded: the answers, what opened between the
// sidebars, Actions, and how long it took. Nothing leaves the machine: requests to other sites are answered here
// (Open-Meteo's with where a few places are, one made-up forecast and air reading; Wikipedia's with a few made-up
// articles and a day's news; the rest with an empty page). The visitor is in Singapore's time zone.
//
// Who answers (--model):
//   none  the page's own written answers (no AI in the browser), as a first visit gets them;
//   web   Chat Computer's own model in the browser (Run my model in this browser: docs/demo/engine), as a visitor who
//         chose it gets it. Hugging Face is stood in for by the model as Chat Computer makes it ready (--ccm: the
//         engine's file, made from the revision the website pins, as the app keeps it in ~/ChatComputer/models), so
//         the browser keeps that file as it would the one it makes; the browser's profile (--profile) keeps it from
//         one run to the next. Each case's trace has the model's every round: what it was given and what it said
//         (its words and tool calls), and how long each took.
//         --engine native:PORT has the same model answer the same requests through the engine's program on this
//         machine instead (chat-computer-model serve FILE.ccm --port PORT, with no refusal file beside it, as the
//         website has none): the same engine, a few times faster than in a browser, for runs of every case. The page
//         is as it is (its instructions, its tools, its turns); only where the model's words are worked out differs,
//         and each answer comes whole rather than word by word.
//
//   NODE_PATH=$(npm root -g) node evals/web/run.js OUT.json [--only id,id] [--jobs 4] [--cases FILE]
//                                        [--model none|web [--engine native:PORT] --ccm FILE.ccm --profile DIR] [--docs DIR]
const fs = require("fs");
const http = require("http");
const os = require("os");
const path = require("path");
const crypto = require("crypto");
const { chromium } = require("playwright");

const ROOT = path.resolve(__dirname, "..", "..");
const args = process.argv.slice(2);
const out = args[0];
const opt = (k, d) => { const i = args.indexOf(k); return i > 0 ? args[i + 1] : d; };
// The website's folder: this checkout's, or another version's (--docs DIR: an export of an earlier commit's docs/), to
// compare before and after with the same cases and harness.
const DOCS = path.resolve(opt("--docs", path.join(ROOT, "docs")));
const only = (opt("--only", "") || "").split(",").filter(Boolean);
const MODEL = opt("--model", "none");
const jobs = +opt("--jobs", MODEL === "web" ? "2" : "4");
const data = JSON.parse(fs.readFileSync(opt("--cases", path.join(__dirname, "cases.json")), "utf8"));
const cases = data.cases.filter((c) => !only.length || only.includes(c.id));
// How long a turn may take: a written answer comes at once; the model in a browser writes a few words a second.
const TURN_MS = MODEL === "web" ? 15 * 60000 : 60000;
const NATIVE = (/^native:(\d+)$/.exec(opt("--engine", "")) || [])[1] || "";
const DEBUG = args.includes("--debug");        // the page's console, and each round of the model's as it ends

// Open-Meteo, answered here: where a few places are (as its geocoding says), and the same forecast everywhere.
const PLACES = { tokyo: ["Tokyo", 35.6895, 139.69171, "Japan"], paris: ["Paris", 48.85341, 2.3488, "France"],
  london: ["London", 51.50853, -0.12574, "United Kingdom"], "new york": ["New York", 40.71427, -74.00597, "United States"],
  berlin: ["Berlin", 52.52437, 13.41053, "Germany"], singapore: ["Singapore", 1.28967, 103.85007, "Singapore"],
  bali: ["Bali", -8.5, 115.0, "Indonesia"] };
function openMeteo(url) {
  const u = new URL(url);
  if (u.hostname === "geocoding-api.open-meteo.com") {
    const p = PLACES[(u.searchParams.get("name") || "").toLowerCase().trim()];
    return p ? { results: [{ name: p[0], latitude: p[1], longitude: p[2], country: p[3] }] } : { generationtime_ms: 0.2 };
  }
  if (u.hostname === "air-quality-api.open-meteo.com") {
    return { current: { time: "2026-10-20T16:00", us_aqi: 42, pm2_5: 9.8, pm10: 15.2 } };
  }
  if (u.hostname === "api.open-meteo.com") {
    return { current: { temperature_2m: 18.4, weather_code: 3 },
             daily: { time: ["2026-10-20", "2026-10-21"], weather_code: [61, 2], temperature_2m_max: [21.2, 19.8],
                      temperature_2m_min: [12.6, 11.9], precipitation_probability_max: [70, 15] } };
  }
  return null;
}

// Wikipedia, answered here: a few articles' openings (for the words searched), and one day's Current events.
const ARTICLES = [
  [/prime minister/i, "Prime Minister of Singapore", "The prime minister of Singapore is the head of government of Singapore. Lawrence Wong has been prime minister since 15 May 2024."],
  [/world cup/i, "2026 FIFA World Cup", "The 2026 FIFA World Cup was the 23rd FIFA World Cup, held in Canada, Mexico and the United States. Spain won the final against Argentina 1-0."],
  [/marina bay sands/i, "Marina Bay Sands", "Marina Bay Sands is an integrated resort fronting Marina Bay in Singapore. It was designed by Moshe Safdie."],
  [/population/i, "Demographics of Singapore", "Singapore's population was 6.11 million in June 2025, of whom 4.2 million were residents."],
  [/haze/i, "Southeast Asian haze", "The Southeast Asian haze is a recurring air pollution problem caused by fires in Indonesia. The 1997 and 2015 hazes were the worst."],
  [/capital of australia|canberra/i, "Canberra", "Canberra is the capital city of Australia. It was founded following the federation of the colonies of Australia."],
  [/quantum/i, "Quantum computing", "A quantum computer is a computer that exploits quantum mechanical phenomena. Its qubits can be in a superposition of states, which lets it solve some problems faster than any classical computer."],
];
const EVENTS = '<div class="current-events-content description"><p><b>Disasters and accidents</b></p><ul><li><a href="/wiki/Typhoon">Typhoon Lan</a><ul>' +
  '<li>Typhoon Lan makes landfall in the Philippines, forcing thousands from their homes. <a class="external" rel="nofollow" href="https://www.reuters.com/x">(Reuters)</a></li>' +
  '</ul></li></ul><p><b>Sports</b></p><ul><li>Spain wins a friendly against Brazil 2-1 in Madrid. <a class="external" rel="nofollow" href="https://apnews.com/y">(AP)</a></li></ul></div>';
// (Another day's: yesterday's page has its own stories, as Wikipedia's has.)
const EVENTS_BEFORE = '<div class="current-events-content description"><p><b>Business and economy</b></p><ul><li>The Bank of Japan holds its ' +
  'policy rate at 0.75%, citing uncertainty over exports. <a class="external" rel="nofollow" href="https://www.nikkei.com/z">(Nikkei)</a></li></ul>' +
  '<p><b>Science and technology</b></p><ul><li>A Japanese H3 rocket launches a weather satellite from Tanegashima Space Center. ' +
  '<a class="external" rel="nofollow" href="https://www.bbc.com/w">(BBC News)</a></li></ul></div>';
const MONTH_NAMES = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
function wikipedia(url) {
  const u = new URL(url);
  if (u.hostname !== "en.wikipedia.org") return null;
  if (u.searchParams.get("action") === "parse") {
    const d = new Date(), today = "Portal:Current_events/" + d.getUTCFullYear() + "_" + MONTH_NAMES[d.getUTCMonth()] + "_" + d.getUTCDate();
    const page = u.searchParams.get("page");
    return { parse: { title: page, text: page === today ? EVENTS : EVENTS_BEFORE } };
  }
  const q = u.searchParams.get("gsrsearch") || "";
  const pages = ARTICLES.filter((a) => a[0].test(q)).map((a, i) => ({ index: i + 1, title: a[1], extract: a[2],
    fullurl: "https://en.wikipedia.org/wiki/" + a[1].replace(/ /g, "_") }));
  return pages.length ? { query: { pages } } : { batchcomplete: true };
}

// The page, from this machine (as the website serves it: a page of its own origin, where the engine's worker runs).
function serve() {
  const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".wasm": "application/wasm",
                  ".svg": "image/svg+xml", ".png": "image/png", ".woff2": "font/woff2", ".ttf": "font/ttf", ".ico": "image/x-icon" };
  const server = http.createServer((req, res) => {
    let p = decodeURIComponent(req.url.split("?")[0]);
    if (p.endsWith("/")) p += "index.html";
    const f = path.join(DOCS, p);
    if (!f.startsWith(DOCS + path.sep) || !fs.existsSync(f) || !fs.statSync(f).isFile()) { res.writeHead(404); res.end(); return; }
    res.writeHead(200, { "content-type": types[path.extname(f)] || "application/octet-stream" });
    fs.createReadStream(f).pipe(res);
  });
  return new Promise((go) => server.listen(0, "127.0.0.1", () => go(server)));
}

// The model, for --model web: the website's own description of it (docs/demo/engine/model.json), with the engine's
// file in place of Qwen's published ones (the same model, made ready as the app makes it); Hugging Face answers with
// that file, in the pieces asked for.
function webModel(ccm) {
  const site = JSON.parse(fs.readFileSync(path.join(DOCS, "demo", "engine", "model.json"), "utf8"));
  const size = fs.statSync(ccm).size, h = crypto.createHash("sha256"), fd = fs.openSync(ccm, "r"), buf = Buffer.alloc(1 << 24);
  for (let at = 0, n; at < size; at += n) { n = fs.readSync(fd, buf, 0, buf.length, at); h.update(buf.subarray(0, n)); }
  const file = { name: path.basename(ccm), size, sha256: h.digest("hex") };
  return { fd, size, file, manifest: Object.assign({}, site, { files: [file] }) };
}
function huggingFace(route, wm) {
  const range = /^bytes=(\d+)-(\d*)$/.exec(route.request().headers().range || "");
  const headers = { "Access-Control-Allow-Origin": "*", "Access-Control-Expose-Headers": "Content-Range, Content-Length" };
  const a = range ? +range[1] : 0, b = range && range[2] ? Math.min(+range[2], wm.size - 1) : wm.size - 1;
  const body = Buffer.alloc(b - a + 1);
  fs.readSync(wm.fd, body, 0, body.length, a);
  if (range) headers["Content-Range"] = `bytes ${a}-${b}/${wm.size}`;
  return route.fulfill({ status: range ? 206 : 200, headers, contentType: "application/octet-stream", body });
}

// In each page, before its own scripts: the engine's questions and answers recorded (each round: how many messages
// it was given, what it said, how long it took, its end), and whether it's got ready; for a case, a fresh visit:
// this browser's storage as a first visit's, but for the model it keeps (and that it was chosen).
const RECORD = (native) => `(() => {
  const NATIVE = ${JSON.stringify(!!native)};
  const keep = ["chatcomputer-web-model", "chatcomputer-web-kept"];
  try { Object.keys(localStorage).forEach((k) => { if (keep.indexOf(k) < 0) localStorage.removeItem(k); }); } catch (e) {}
  const ev = window.__eval = { rounds: [], ready: false, last: null, done: 0 };
  let real = null;
  // (Natively: the model is there at once, and each request goes to the engine's program, through the harness.)
  function program(request, onChunk) {
    return fetch("https://engine.eval.invalid/v1/chat/completions", { method: "POST", body: JSON.stringify(Object.assign({}, request, { stream: false })) })
      .then((r) => r.json()).then((j) => {
        if (j.error || !j.choices) throw new Error((j.error && j.error.message) || "the engine didn't answer");
        const m = j.choices[0].message;
        if (m.content && onChunk) onChunk({ content: m.content });
        if (onChunk) onChunk({ content: "" }, { finish_reason: j.choices[0].finish_reason, usage: j.usage });
        return m;
      });
  }
  Object.defineProperty(window, "ChatComputerEngine", { configurable: true, get() { return real; }, set(e) {
    if (NATIVE) {
      e = Object.assign({}, e, {
        status: () => Promise.resolve({ name: "qwen3.5-0.8b", label: "Qwen3.5 0.8B", download: 0, kept: true, kept_bytes: 1, loaded: true }),
        prepare: () => Promise.resolve({ label: "Qwen3.5 0.8B" }),
        ahead: () => Promise.resolve({ tokens: 0, read: true }),
        chat: (request, onChunk) => program(request, onChunk),
        stop: () => {}, forget: () => Promise.resolve() });
    }
    real = Object.assign({}, e, {
      ahead(request, onReading) { return e.ahead(request, onReading).then((r) => { ev.ready = true; return r; }); },
      chat(request, onChunk, options) {
        const r = { at: Date.now(), given: request.messages.length, end: null };
        ev.last = JSON.parse(JSON.stringify(request.messages));
        return e.chat(request, (d, end) => { if (end) r.end = end; if (onChunk) onChunk(d, end); }, options).then((m) => {
          r.ms = Date.now() - r.at; r.out = JSON.parse(JSON.stringify(m)); ev.rounds.push(r); return m;
        }, (err) => { r.ms = Date.now() - r.at; r.error = String(err && err.message || err); ev.rounds.push(r); throw err; });
      } });
  } });
})();`;

async function route(target, wm) {
  await target.route(/^https?:\/\//, (r) => {
    const url = r.request().url();
    if (/^http:\/\/127\.0\.0\.1:\d+\//.test(url)) {
      if (wm && /\/demo\/engine\/model\.json$/.test(url)) {
        return r.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(wm.manifest) });
      }
      return r.continue();
    }
    if (wm && url.indexOf("https://huggingface.co/" + wm.manifest.repo + "/resolve/" + wm.manifest.revision + "/") === 0) return huggingFace(r, wm);
    if (NATIVE && url.indexOf("https://engine.eval.invalid/") === 0) {
      return fetch("http://127.0.0.1:" + NATIVE + new URL(url).pathname, { method: "POST", headers: { "content-type": "application/json" },
                                                                            body: r.request().postData() })
        .then((res) => res.text().then((body) => r.fulfill({ status: res.status, contentType: "application/json",
                                                               headers: { "access-control-allow-origin": "*" }, body })),
              (e) => r.fulfill({ status: 502, contentType: "application/json", headers: { "access-control-allow-origin": "*" },
                                 body: JSON.stringify({ error: { message: "no engine on port " + NATIVE + ": " + e.message } }) }));
    }
    const om = openMeteo(url) || wikipedia(url);
    if (om) return r.fulfill({ status: 200, contentType: "application/json", headers: { "access-control-allow-origin": "*" },
                               body: JSON.stringify(om) });
    r.fulfill({ status: 200, contentType: "text/html", body: "<!doctype html><title>elsewhere</title>" });
  });
}

// The model got ready once (downloaded, kept and its instructions read, the first time; from the browser's copy after).
async function prepareModel(ctx, base) {
  const page = await ctx.newPage(), t0 = Date.now();
  page.on("pageerror", (e) => console.log("page error:", String(e)));
  await page.goto(base + "/index.html");
  await page.waitForFunction(() => window.CO && window.webkit && document.getElementById("toggle-left"));
  await page.evaluate(() => window.webkit.messageHandlers.chatcomputer.postMessage(JSON.stringify({ type: "account", form: "web", values: {} })));
  let last = "";
  for (;;) {
    const st = await page.evaluate(() => ({ ready: window.__eval.ready,
                                            status: (document.body.innerText.match(/(Getting my model ready|My model couldn't)[^\n]*/) || [""])[0] }));
    if (st.ready) break;
    const s = String(st.status);
    if (s && s !== last) { console.log(Math.round((Date.now() - t0) / 1000) + " s: " + s); last = s; }
    if (/couldn't get ready|failed/i.test(s)) throw new Error(s);
    if (Date.now() - t0 > 60 * 60000) throw new Error("the model didn't get ready in an hour");
    await page.waitForTimeout(2000);
  }
  await page.waitForTimeout(500);
  await page.close();
  return Date.now() - t0;
}

async function visit(ctx, base, c, routed) {
  const page = await ctx.newPage();
  const errors = [], requests = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  if (DEBUG) page.on("console", (m) => console.log("  [" + c.id + " console] " + m.text().slice(0, 300)));
  page.on("request", (r) => { if (!/^http:\/\/127\.0\.0\.1:/.test(r.url()) && !/^(data|blob):/.test(r.url())) requests.push(r.url()); });
  if (!routed) await route(page, null);
  await page.goto(base + "/index.html");
  await page.waitForFunction(() => window.CO && document.getElementById("toggle-left"));
  // (Each answer ends with aiDone: counted.)
  await page.evaluate(() => { const d = window.CO.aiDone; window.CO.aiDone = function () { window.__eval.done++; return d.apply(this, arguments); }; });
  let ready_ms = null;
  if (MODEL === "web") {
    const t = Date.now();
    await page.evaluate(() => window.webkit.messageHandlers.chatcomputer.postMessage(JSON.stringify({ type: "account", form: "web", values: {} })));
    await page.waitForFunction(() => window.__eval.ready, null, { timeout: 10 * 60000, polling: 250 });
    ready_ms = Date.now() - t;
  }
  await page.waitForTimeout(300);
  const before = await page.evaluate(() => document.querySelectorAll("#done .done-what").length);
  const t0 = Date.now(), turns = c.turns || [c.query], answers = [], firsts = [], times = [];
  for (const q of turns) {
    const n = await page.evaluate(() => window.__eval.done), t1 = Date.now();
    const shown = await page.locator("#log > .ai").count();
    await page.click("#input", { timeout: 5000 }).catch(() => {});
    await page.keyboard.type(q, { delay: 2 });
    await page.keyboard.press("Enter");
    let first = null, text = "";
    try {
      // The first words (or the end), then the end.
      await page.waitForFunction(([k, d]) => {
        const a = document.querySelectorAll("#log > .ai");
        return a.length > k && /\S/.test(a[a.length - 1].innerText || "") || window.__eval.done > d;
      }, [shown, n], { timeout: TURN_MS, polling: 100 }).catch(() => {});
      first = Date.now() - t1;
      if (DEBUG) {
        for (let seen = 0; !(await page.evaluate((k) => window.__eval.done > k, n)); await page.waitForTimeout(1000)) {
          const rs = await page.evaluate(() => window.__eval.rounds);
          for (; seen < rs.length; seen++) console.log("  [" + c.id + " round " + (seen + 1) + "] " + JSON.stringify(rs[seen].out || rs[seen].error).slice(0, 400));
          if (Date.now() - t1 > TURN_MS) throw new Error("timed out");
        }
      }
      await page.waitForFunction((k) => window.__eval.done > k, n, { timeout: Math.max(1000, TURN_MS - (Date.now() - t1)), polling: 250 });
      await page.waitForTimeout(400);
    } catch (e) { errors.push("no end to the answer to " + q + " in " + Math.round(TURN_MS / 60000) + " min: " + String(e).slice(0, 160)); }
    text = await page.evaluate(() => (Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).innerText || "");
    answers.push(text.trim());
    firsts.push(first);
    times.push(Date.now() - t1);
  }
  const seen = await page.evaluate((k) => {
    const v = document.getElementById("demo-view");
    let title = "", body = "";
    try { title = v && v.contentDocument ? v.contentDocument.title : ""; body = v && v.contentDocument && v.contentDocument.body ? v.contentDocument.body.innerText.slice(0, 600) : ""; } catch (e) {}
    return {
      open: document.body.classList.contains("content"),
      src: v ? v.getAttribute("src") || "" : "", title: title, body: body,
      done: Array.from(document.querySelectorAll("#done .done-what")).slice(0).map((e) => e.textContent).slice(-5),
      done_added: document.querySelectorAll("#done .done-what").length - k,
      rounds: window.__eval.rounds, messages: window.__eval.last,
    };
  }, before);
  await page.close();
  // The model's trace: each round's output, and the conversation as it last saw it (without its instructions).
  const rounds = (seen.rounds || []).map((r) => ({ given: r.given, ms: r.ms, out: r.out || null, error: r.error || null,
    finish: r.end && r.end.finish_reason, usage: r.end && r.end.usage }));
  const messages = (seen.messages || []).filter((m) => m.role !== "system");
  delete seen.rounds;
  delete seen.messages;
  return { id: c.id, intent: c.intent, phrasing: c.phrasing, query: c.query || turns.join(" / "), turns: c.turns || null,
           expect: c.expect, model: MODEL, answers, answer: answers[answers.length - 1] || "", first_ms: firsts[0], firsts, times,
           total_ms: Date.now() - t0, ready_ms, ...seen, rounds, messages, requests: requests.slice(0, 10), errors };
}

(async () => {
  const server = await serve(), base = "http://127.0.0.1:" + server.address().port;
  const look = { viewport: { width: 1440, height: 900 }, timezoneId: "Asia/Singapore", locale: "en-SG" };
  let browser = null, shared = null, wm = null;
  const t0 = Date.now(), meta = { model: MODEL, started: new Date().toISOString() };
  if (MODEL === "web" && !NATIVE) {
    const ccm = opt("--ccm", path.join(os.homedir(), "ChatComputer", "models", "qwen3.5-0.8b.ccm"));
    wm = webModel(ccm);
    shared = await chromium.launchPersistentContext(opt("--profile", path.join(os.tmpdir(), "chatcomputer-eval-profile")), look);
    await shared.addInitScript(RECORD(false));
    await route(shared, wm);
    meta.ccm = wm.file;
    meta.prepare_ms = await prepareModel(shared, base);
    console.log("model ready in " + Math.round(meta.prepare_ms / 1000) + " s");
  } else browser = await chromium.launch();
  if (NATIVE) {
    const h = await fetch("http://127.0.0.1:" + NATIVE + "/health").then((r) => r.json());
    meta.engine = Object.assign({ port: +NATIVE }, h);
    if (h.uncensored) console.log("note: this engine's model has its refusals taken out; the website's hasn't");
  }
  const results = [];
  let next = 0;
  async function worker() {
    while (next < cases.length) {
      const c = cases[next++];
      let ctx = shared;
      try {
        if (!shared) { ctx = await browser.newContext(look); await ctx.addInitScript(RECORD(!!NATIVE)); }
        results.push(await visit(ctx, base, c, !!shared));
      } catch (e) { results.push({ id: c.id, query: c.query, model: MODEL, errors: [String(e)] }); }
      if (!shared && ctx) await ctx.close().catch(() => {});
      process.stdout.write(MODEL === "web" ? c.id + " (" + Math.round((results[results.length - 1].total_ms || 0) / 1000) + " s)\n" : ".");
      // (Kept as it goes: a long run's results so far are there if it stops.)
      if (MODEL === "web") fs.writeFileSync(out, JSON.stringify(results, null, 1));
    }
  }
  await Promise.all(Array.from({ length: jobs }, worker));
  if (shared) await shared.close();
  if (browser) await browser.close();
  server.close();
  results.sort((a, b) => cases.findIndex((c) => c.id === a.id) - cases.findIndex((c) => c.id === b.id));
  meta.minutes = Math.round((Date.now() - t0) / 60000);
  fs.writeFileSync(out, JSON.stringify(results, null, 1));
  fs.writeFileSync(out.replace(/\.json$/, "") + ".run.json", JSON.stringify(meta, null, 1));
  console.log("\n" + results.length + " cases -> " + out);
})();
