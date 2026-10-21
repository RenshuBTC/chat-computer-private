// The browser demo (docs/index.html: Chat Computer's own screen, with docs/demo/host.js standing in for the computer),
// tried in Chromium with Playwright: it opens on the word logo, which fades away after a second, and then Chat
// Computer's first words come in as if just typed (hello, world and a greeting, another on another visit; none when
// something's said first), the cursor in the box; the
// sidebars' switches; music (made by the browser), in the middle in the conversation's place; "/" lists the files in
// front of what's open; no sidebars until Escape brings them in, and Escape puts them away again; the x closes it; a message opened as its whole
// conversation; an answer in words alone bringing the conversation back; Google, for real, typed into; Connect
// Claude, with a stand-in for Anthropic (tools used: a map, a YouTube video, which closes
// when it ends, with a stand-in for YouTube's player; answers streamed; thinking first, as Opus 5.5 always does);
// Connect AirPods' sheet; Chrome's built-in AI, with a stand-in (Download Gemini: its sheet, the download, then it
// answers); a phone, laid out as the Claude app is there (the keyboard down after sending, a held message's menu,
// nothing zooming; music turned down, then stopped); and Connect AirPods on an iPhone, with stand-ins for Safari's
// listening and speaking.
// Prints JSON.  Run: node tests/demo_check.js [shot-prefix]
const path = require("path");
const { chromium, devices } = require("playwright");

const page_url = "file://" + path.resolve(__dirname, "..", "docs", "index.html");
const shot = process.argv[2];

// YouTube's player, standing in: it answers the page's player messages (playing; muted or not), keeps the commands
// it's sent, and window.end() says the video ended.
const PLAYER = `<!doctype html><title>YouTube</title><body style="margin:0;background:#222;color:#fff">A YouTube player<script>
var muted = /mute=1/.test(location.search); window.commands = [];
function send(o) { parent.postMessage(JSON.stringify(o), "*"); }
addEventListener("message", function (e) {
  var d; try { d = JSON.parse(e.data); } catch (x) { return; }
  if (d.event === "listening") { send({ event: "initialDelivery", info: { playerState: 1, muted: muted } }); send({ event: "onReady", info: null }); }
  if (d.event === "command") { window.commands.push(d.func); if (d.func === "unMute") { muted = false; send({ event: "infoDelivery", info: { muted: false } }); } }
});
window.end = function () { send({ event: "onStateChange", info: 0 }); };
</script>`;

// Google, standing in: its page, with a search box to type in.
const GOOGLE = `<!doctype html><title>Google</title><body style="margin:0"><input id="q" aria-label="Search"
  style="position:absolute;left:30px;top:40px;width:300px;height:36px"></body>`;

// Anthropic's API, standing in: the key check; a map of Tokyo asked for (show_map), a video (play_youtube), or
// it quieter (set_volume); then
// the answer, once the tool's result is back. A tool's turn starts with thinking, as the real one's do: its text
// left out (display "omitted"), only its signature.
function sse(events) { return events.map((e) => "event: " + e.type + "\ndata: " + JSON.stringify(e) + "\n\n").join(""); }
const START = { type: "message_start", message: { id: "msg_1", type: "message", role: "assistant", content: [], model: "claude-sonnet-5-5",
  stop_reason: null, usage: { input_tokens: 10, output_tokens: 1 } } };
function toolTurn(id, name, json) {
  const half = Math.floor(json.length / 2);
  return sse([START,
    { type: "content_block_start", index: 0, content_block: { type: "thinking", thinking: "", signature: "" } },
    { type: "content_block_delta", index: 0, delta: { type: "signature_delta", signature: "sig-" + id } },
    { type: "content_block_stop", index: 0 },
    { type: "content_block_start", index: 1, content_block: { type: "tool_use", id: id, name: name, input: {} } },
    { type: "content_block_delta", index: 1, delta: { type: "input_json_delta", partial_json: json.slice(0, half) } },
    { type: "content_block_delta", index: 1, delta: { type: "input_json_delta", partial_json: json.slice(half) } },
    { type: "content_block_stop", index: 1 },
    { type: "message_delta", delta: { stop_reason: "tool_use" }, usage: { output_tokens: 30 } },
    { type: "message_stop" }]);
}
function textTurn(words) {
  return sse([START,
    { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } }]
    .concat(words.map((w) => ({ type: "content_block_delta", index: 0, delta: { type: "text_delta", text: w } })))
    .concat([{ type: "content_block_stop", index: 0 },
      { type: "message_delta", delta: { stop_reason: "end_turn" }, usage: { output_tokens: 5 } },
      { type: "message_stop" }]));
}
const MAP_TURN = toolTurn("toolu_1", "show_map", '{"place": "Tokyo", "west": 139.55, "south": 35.55, "east": 139.95, "north": 35.8}');
const VIDEO_TURN = toolTurn("toolu_2", "play_youtube", '{"video_id": "aBcDeFgHiJk", "title": "A test video"}');
const VOLUME_TURN = toolTurn("toolu_3", "set_volume", '{"percent": 30}');

async function state(page) {
  return page.evaluate(() => {
    const v = document.getElementById("demo-view");
    const r = v && v.style.display !== "none" ? v.getBoundingClientRect() : null;
    const frame = document.getElementById("frame").getBoundingClientRect();
    const shown = (id) => getComputedStyle(document.getElementById(id)).display !== "none";
    return {
      sheet: document.body.classList.contains("sheet"), content: document.body.classList.contains("content"),
      sheet_title: document.getElementById("sheet-title").textContent,
      view: r ? [r.left, r.top, r.width, r.height].map(Math.round) : null,
      src: v ? v.getAttribute("src") || "" : "",
      frame: [frame.left, frame.top, frame.width, frame.height].map(Math.round),
      messages: document.querySelectorAll("#messages .msg").length,
      done: Array.from(document.querySelectorAll("#done .done-what")).map((e) => e.textContent),
      last: (Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).textContent || "",
      results: Array.from(document.querySelectorAll("#results .res-text")).map((e) => e.textContent),
      panel: document.body.classList.contains("panel"),
      first_ai: (() => { const a = document.querySelector("#log > .ai"); return a ? String(a._said != null ? a._said : a.textContent).trim() : ""; })(),
      // (Faded away: not shown.)
      logo: shown("welcome") && getComputedStyle(document.getElementById("welcome")).visibility !== "hidden" ?
        document.getElementById("wordmark").textContent : null,
      said: document.querySelectorAll("#log > .you, #log > .ai").length,
      sides: [shown("messages"), shown("done")],
      composer: shown("composer"),
      toggles: ["left", "right"].map((k) => shown("toggle-" + k) && document.getElementById("toggle-" + k).getAttribute("aria-pressed")),
      bottom_toggle: !!document.getElementById("toggle-bottom"),
      full: document.body.classList.contains("full"),
      column_shown: shown("column"),
      frame_title: document.getElementById("frame-title").textContent,
      box: document.getElementById("input").value,
      typing_in: document.activeElement && document.activeElement.id,
      hint: document.getElementById("input").value === "" ? document.getElementById("input").placeholder : null,
      hint_weight: getComputedStyle(document.getElementById("input")).fontWeight,
      buttons: document.querySelectorAll("#demo-tools, #box button:not(#send)").length,     // none but the arrow
      more: document.querySelectorAll(".more").length,
    };
  });
}

// "/" in the box: what it lists (closed again with Escape), or, given a name, the one starting with it chosen
// (clicked; tapped on a touchscreen).
async function slash(page, name, tap) {
  await (tap ? page.tap("#input") : page.click("#input"));
  await page.keyboard.type("/");
  await page.waitForFunction(() => document.querySelectorAll("#results .res").length > 1);
  if (!name) {
    const rows = await page.evaluate(() => Array.from(document.querySelectorAll("#results .res:not(.ask) .res-name")).map((e) => e.textContent));
    await page.keyboard.press("Escape");
    return rows;
  }
  const row = page.locator("#results .res .res-name", { hasText: new RegExp("^" + name) }).first();
  await (tap ? row.tap() : row.click());
  await page.waitForFunction(() => !document.body.classList.contains("searching"));
  await page.waitForTimeout(150);                  // what it does happens straight after (the page's message to its host)
}

async function viewText(page) {
  const f = page.frameLocator("#demo-view");
  return (await f.locator("body").innerText({ timeout: 5000 })).replace(/\s+/g, " ").trim();
}
function player(page) { return page.frames().find((f) => /youtube/.test(f.url())); }

// Chat Computer's first words, once they've all come in (after a second of the word logo, its fade, and a moment's
// thinking).
async function greeted(page) {
  await page.waitForFunction(() => { const h = document.querySelector("#log > .ai.hello"); return h && h._said; }, null, { timeout: 8000 });
  return page.evaluate(() => String(document.querySelector("#log > .ai.hello")._said).trim());
}

async function ask(page, text, meanwhile) {
  const before = await page.locator("#log > .ai").count();
  await page.keyboard.type(text, { delay: 5 });
  await page.keyboard.press("Enter");
  if (meanwhile) await meanwhile();          // a look while it's being answered
  await page.waitForFunction((n) => document.querySelectorAll("#log > .ai").length > n &&
    !document.querySelector("#log > .ai.thinking"), before);
  // The words stream in: until the answer stops growing.
  let last = "";
  for (let i = 0; i < 40; i++) {
    await page.waitForTimeout(250);
    const now = await page.evaluate(() => (Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).textContent || "");
    if (now === last) break;
    last = now;
  }
  return last;
}

(async () => {
  const out = {};
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1600, height: 900 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.route(/youtube/, (route) => route.fulfill({ status: 200, contentType: "text/html", body: PLAYER }));      // no network needed
  await page.route(/www\.google\.com/, (route) => route.fulfill({ status: 200, contentType: "text/html", body: GOOGLE }));
  // Wikipedia finds nothing here (a question of fact then gets the page's honest note, as with no AI).
  await page.route(/en\.wikipedia\.org/, (route) => route.fulfill({ status: 200, contentType: "application/json",
    headers: { "access-control-allow-origin": "*" }, body: "{}" }));
  const api = [];
  await page.route("https://api.anthropic.com/**", (route) => {
    const req = route.request(), body = JSON.parse(req.postData() || "{}");
    api.push({ url: req.url(), key: req.headers()["x-api-key"], direct: req.headers()["anthropic-dangerous-direct-browser-access"],
               version: req.headers()["anthropic-version"], model: body.model, stream: !!body.stream, max_tokens: body.max_tokens,
               system: body.system ? body.system[0].text : null, thinking: body.thinking || null, effort: body.output_config || null,
               tools: (body.tools || []).map((t) => t.name), last: JSON.stringify((body.messages || []).slice(-1)),
               all: JSON.stringify(body.messages || []) });
    if (!body.stream) return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ id: "msg_0", type: "message",
      role: "assistant", content: [{ type: "text", text: "Hi" }], stop_reason: "max_tokens", usage: { input_tokens: 8, output_tokens: 1 } }) });
    const last = JSON.stringify(body.messages.slice(-1));
    const turn = /toolu_1/.test(last) ? textTurn(["Here's ", "Tokyo."]) : /toolu_2/.test(last) ? textTurn(["Playing ", "it."])
      : /toolu_3/.test(last) ? textTurn(["Quieter ", "now."]) : /quieter/.test(last) ? VOLUME_TURN
      : /video/.test(last) ? VIDEO_TURN : MAP_TURN;
    return route.fulfill({ status: 200, contentType: "text/event-stream", body: turn });
  });
  await page.goto(page_url);
  // It opens on the word logo alone; after a second it fades away (over half a second), and then Chat Computer's
  // first words come in as its answers do, as if just typed: a moment's thinking, then the words a few letters at a
  // time, fading in. The cursor in the box, under the hint.
  await page.waitForFunction(() => window.CO && document.getElementById("toggle-left"));
  out.intro = await state(page);
  const look = async (f) => (await page.waitForFunction(f, null, { polling: "raf", timeout: 6000 })).jsonValue();
  out.intro_fading = await look(() => {
    const s = getComputedStyle(document.getElementById("welcome"));
    return +s.opacity > 0 && +s.opacity < 1 && { fade: s.transitionDuration.split(",")[0].trim(), said: document.querySelectorAll("#log > .ai").length };
  });
  out.greeting_thinking = await look(() => {
    const h = document.querySelector("#log > .ai.hello.thinking");
    return h && { logo_gone: +getComputedStyle(document.getElementById("welcome")).opacity < 0.05, words: h.textContent };
  });
  out.greeting_typing = await look(() => {
    const h = document.querySelector("#log > .ai.hello"), t = h && h.textContent;
    return t && t.length < 12 && { words: t, fading: !!h.querySelector(".fresh") };
  });
  out.greeting = await greeted(page);
  out.greeting_options = await page.evaluate(() => window.CO.greetingsNow());
  out.opening = await state(page);
  // Two visits in the same browser: a different greeting each time.
  const visits = await browser.newContext({ viewport: { width: 1600, height: 900 } });
  out.visits = [];
  for (let i = 0; i < 2; i++) {
    const v = await visits.newPage();
    v.on("pageerror", (e) => errors.push(String(e)));
    await v.goto(page_url);
    out.visits.push(await greeted(v));
    await v.close();
  }
  await visits.close();
  // "/": what can be connected here, then the files.
  out.slash = await slash(page);
  if (shot) await page.screenshot({ path: shot + "-opening.png" });
  // No sidebars to start with; Escape brings Messages and Actions in.
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  out.sides_in = await state(page);
  // The sidebars' switches: off, and on again (and the choice kept).
  await page.click("#toggle-left");
  await page.click("#toggle-right");
  await page.waitForTimeout(200);
  out.sides_off = await state(page);
  out.kept = await page.evaluate(() => localStorage.getItem("chatcomputer-demo-panels"));
  await page.click("#toggle-left");
  await page.click("#toggle-right");
  out.sides_on = await state(page);
  // Music asked for.
  await ask(page, "play some piano music");
  await page.waitForTimeout(300);
  out.music = await state(page);
  if (shot) await page.screenshot({ path: shot + "-music.png" });
  // "/": the files, in a list in front of what's open (what's at its middle is the list, not the page).
  await page.keyboard.type("/");
  await page.waitForSelector("#results .res");
  out.over = await page.evaluate(() => {
    const r = document.querySelector("#results .res").getBoundingClientRect();
    const at = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    return !!(at && at.closest("#results"));
  });
  out.over_view = (await state(page)).view;
  await page.keyboard.press("Escape");                     // ends the search
  out.unslashed = await state(page);
  // Escape: the sidebars go, the middle filling the screen (what's open and the message box), and come back.
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  out.full = await state(page);
  if (shot) await page.screenshot({ path: shot + "-full.png" });
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  out.full_back = await state(page);
  // Its x: closed, the conversation back.
  await page.click("#frame-close");
  await page.waitForFunction(() => !document.body.classList.contains("content"));
  out.closed = await state(page);
  await page.click("#input");
  await page.keyboard.type("/pack", { delay: 40 });
  await page.waitForSelector("#results .res");
  out.search = await state(page);
  if (shot) await page.screenshot({ path: shot + "-search.png" });
  await page.keyboard.press("Enter");
  await page.waitForTimeout(400);
  out.opened = await state(page);
  out.opened_view = await viewText(page);
  await page.click("#frame-close");
  // A message: its whole conversation, in the middle, the conversation with Chat Computer put away.
  await page.click("#messages .msg:has-text('Jane')");
  await page.waitForTimeout(400);
  out.jane = await viewText(page);
  out.jane_state = await state(page);
  if (shot) await page.screenshot({ path: shot + "-message.png" });
  await page.click("#frame-close");
  await page.click("#input");
  await ask(page, "make me a 5 minute timer");
  out.timer = await viewText(page);
  out.after_timer = await state(page);
  // Asked something else, answered in words alone: what's open is put away and the conversation has the middle.
  await ask(page, "what's the meaning of life?");
  out.fallback = await state(page);
  if (shot) await page.screenshot({ path: shot + "-long.png" });
  // Google, for real: in the middle, and a click on its box, and typing goes there.
  await ask(page, "google.com");
  out.google = await state(page);
  const g = await (await page.$("#demo-view")).contentFrame();
  await g.waitForSelector("#q");
  await g.click("#q");
  await page.keyboard.type("cats", { delay: 20 });
  out.google_typed = { frame: await g.evaluate(() => document.getElementById("q").value), box: (await state(page)).box };
  if (shot) await page.screenshot({ path: shot + "-google.png" });
  await page.click("#frame-close");
  await page.click("#input");
  // Any other address typed, as in a browser's address bar: a site that won't show inside another page opens at once
  // in a new tab (here, a stand-in for it); a YouTube video's address plays in YouTube's player, in the middle.
  await page.context().route(/^https:\/\/(youtube\.com|bbc\.co\.uk)\//, (route) =>
    route.fulfill({ status: 200, contentType: "text/html", body: "<title>a site</title>a site" }));
  const tabbed = [];
  for (const typed of ["youtube.com", "open bbc.co.uk please"]) {
    const popup = page.waitForEvent("popup", { timeout: 8000 }).catch(() => null);
    await ask(page, typed);
    const tab = await popup;
    if (tab) { await tab.waitForLoadState().catch(() => {}); tabbed.push({ url: tab.url(), opener: await tab.evaluate(() => window.opener === null) }); await tab.close(); }
    else tabbed.push(null);
    const s = await state(page);
    tabbed[tabbed.length - 1] = { tab: tabbed[tabbed.length - 1], last: s.last, done: s.done[s.done.length - 1], content: s.content };
  }
  out.typed_sites = tabbed;
  await ask(page, "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1m5s");
  out.typed_video = await state(page);
  await page.click("#frame-close");
  await page.click("#input");

  // Connect Claude ("/"): its sheet, a key, and then Claude (a stand-in) answers, using a tool of the demo's.
  await slash(page, "Connect Claude");
  out.claude_sheet = await state(page);
  await page.keyboard.type("sk-ant-api03-demo-test-key");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => !document.body.classList.contains("sheet"));
  out.claude_on = await state(page);
  out.claude_on_slash = await slash(page);
  out.stored = await page.evaluate(() => localStorage.getItem("chatcomputer-demo-claude-key"));
  await ask(page, "show me Tokyo");
  out.claude_map = await state(page);
  if (shot) await page.screenshot({ path: shot + "-claude.png" });
  // A video, played by Claude: YouTube's player (a stand-in) between the sidebars; when it ends, the conversation.
  await ask(page, "play a video");
  out.claude_video = await state(page);
  await page.waitForTimeout(300);
  // Turned down by Claude (set_volume): YouTube's player is told, and the video stays.
  await ask(page, "a bit quieter");
  out.claude_quieter = { ...(await state(page)), player: await player(page).evaluate(() => window.commands) };
  await player(page).evaluate(() => window.end());
  await page.waitForFunction(() => !document.body.classList.contains("content"));
  out.video_ended = await state(page);
  out.api = api;
  await slash(page, "Disconnect Claude");
  out.claude_off_sheet = await state(page);
  await page.click("#sheet-ok");
  await page.waitForFunction(() => !document.body.classList.contains("sheet"));
  out.claude_off = await state(page);
  out.claude_off_slash = await slash(page);
  out.stored_after = await page.evaluate(() => localStorage.getItem("chatcomputer-demo-claude-key"));
  // Connect AirPods ("/"): what it does, before the microphone is asked for.
  await slash(page, "Connect AirPods");
  out.talk_sheet = await state(page);
  await page.keyboard.press("Escape");

  // Chrome's built-in AI, standing in: it can be downloaded; "Download Gemini", its sheet, the download (its progress on
  // the button), then it answers what the written answers can't, with a page on the screen. Written answers still
  // answer what they can (music).
  const ai = await browser.newPage({ viewport: { width: 1600, height: 900 } });
  ai.on("pageerror", (e) => errors.push(String(e)));
  await ai.addInitScript(() => {
    window.__ai = { created: [], prompts: [] };
    let ready = false;
    window.LanguageModel = {
      availability: () => Promise.resolve(ready ? "available" : "downloadable"),
      create: (o) => {
        window.__ai.created.push({ system: o.initialPrompts && o.initialPrompts[0].role === "system" && o.initialPrompts[0].content,
                                   inputs: o.expectedInputs, monitored: typeof o.monitor === "function" });
        const m = new EventTarget();
        if (o.monitor) o.monitor(m);
        return new Promise((ok) => {
          const step = (loaded) => { const e = new Event("downloadprogress"); e.loaded = loaded; m.dispatchEvent(e); };
          setTimeout(() => step(0.5), 100);
          window.__ai.half = () => { step(1); ready = true; ok({
            prompt: (text, opts) => { window.__ai.prompts.push({ text, schema: !!(opts && opts.responseConstraint) });
              return Promise.resolve(JSON.stringify({ reply: "Here's a short poem about the sea.", action: "show_page", title: "The sea",
                                                       markdown: "The tide comes in on silver feet." })); },
            destroy: () => {} }); };
        });
      },
    };
  });
  await ai.goto(page_url);
  await ai.waitForFunction(() => window.CO && document.getElementById("toggle-left"));
  await ai.waitForTimeout(400);
  out.ai_opening = await slash(ai);
  await slash(ai, "Download Gemini");
  out.ai_sheet = await state(ai);
  await ai.click("#sheet-ok");
  await ai.waitForFunction(() => /50%/.test(document.getElementById("input").placeholder));
  out.ai_downloading = await state(ai);
  out.ai_downloading_slash = await slash(ai);
  await ai.evaluate(() => window.__ai.half());
  await ai.waitForFunction(() => !/Downloading/.test(document.getElementById("input").placeholder));
  out.ai_on = await state(ai);
  out.ai_on_slash = await slash(ai);
  await ai.click("#input");
  await ask(ai, "write a short poem about the sea");
  out.ai_poem = await state(ai);
  out.ai_poem_view = await viewText(ai);
  await ask(ai, "play some piano music");
  out.ai_music = await state(ai);
  out.ai_calls = await ai.evaluate(() => window.__ai);
  if (shot) await ai.screenshot({ path: shot + "-ai.png" });

  // Google's Gemma where the browser has no AI of its own but a graphics chip it can use (WebGPU), with a stand-in
  // for Transformers.js (from jsDelivr): "Download Gemma" in the "/" list, its sheet, the download's progress, then it
  // answers what the written answers can't, its words streamed, knowing what was said before; the written answers
  // still answer what they can. On a later visit it's there (kept by the browser: the stand-in keeps its files where
  // Transformers.js does) and starts when it's first needed, from the browser's copy; a page that stopped while it
  // was starting doesn't start it again by itself; Remove Gemma frees the room.
  const LM = `export class TextStreamer { constructor(tokenizer, o) { this.o = o; } }
export async function pipeline(task, model, opts) {
  const lm = window.__lm, base = "https://huggingface.co/" + model + "/resolve/main/";
  const files = [["onnx/model_" + opts.dtype + ".onnx", 461341], ["tokenizer.json", 20323013]];
  const c0 = await caches.open("transformers-cache"), data = await c0.match(base + "onnx/model_" + opts.dtype + ".onnx_data");
  const kept = !!(await c0.match(base + files[0][0])) && !!data;
  lm.loads.push({ task, model, device: opts.device, dtype: opts.dtype, kept, data: data ? (await data.blob()).size : 0 });
  if (kept) await new Promise((ok) => setTimeout(ok, 400));       // started from the browser's copy, as Transformers.js does
  else {
    files.forEach(([file, total]) => opts.progress_callback({ status: "progress", file, loaded: total / 2, total }));
    await new Promise((ok) => { lm.finish = ok; });
    const c = await caches.open("transformers-cache");
    for (const [file, total] of files) {
      opts.progress_callback({ status: "progress", file, loaded: total, total });
      opts.progress_callback({ status: "done", file });
      await c.put(base + file, new Response("x"));
    }
  }
  const gen = async (messages, o) => {
    lm.asked.push(messages);
    for (const w of lm.words) { await new Promise((r) => setTimeout(r, 15)); o.streamer.o.callback_function(w); }
    return [];
  };
  gen.tokenizer = {};
  gen.dispose = async () => { lm.disposed = true; };
  return gen;
}`;
  const lmCtx = await browser.newContext({ viewport: { width: 1600, height: 900 } });
  const DATA = 40 * 1024 * 1024, ranges = [], hub = { hold: true, fail: 0, held: null };
  await lmCtx.route(/^https:\/\/huggingface\.co\/.*\.onnx_data$/, async (route) => {
    const req = route.request();
    if (req.method() === "OPTIONS") return route.fulfill({ status: 204, headers: { "access-control-allow-origin": "*",
      "access-control-allow-headers": "range", "access-control-allow-methods": "GET" } });
    const range = req.headers()["range"] || "", m = /bytes=(\d+)-(\d+)/.exec(range);
    ranges.push(range);
    if (!m) return route.fulfill({ status: 200, body: Buffer.alloc(16) });
    const from = +m[1], to = Math.min(+m[2], DATA - 1);
    if (from >= 32 * 1024 * 1024 && hub.hold) { hub.held = route; return; }       // the third piece: never comes
    if (from >= 32 * 1024 * 1024 && hub.fail > 0) { hub.fail--; return route.abort("connectionreset"); }   // cut off once
    return route.fulfill({ status: 206, headers: { "access-control-allow-origin": "*", "access-control-expose-headers": "*",
      "content-range": "bytes " + from + "-" + to + "/" + DATA, "content-type": "application/octet-stream" },
      body: Buffer.alloc(to - from + 1, from >> 24) });
  });
  await lmCtx.addInitScript(() => {
    window.__lm = { loads: [], asked: [], words: ["Dark cup ", "in my hands, ", "the day ", "starts with you."] };
    Object.defineProperty(navigator, "gpu", { value: { requestAdapter: () => Promise.resolve({ features: new Set(["shader-f16"]) }) } });
  });
  await lmCtx.route("https://cdn.jsdelivr.net/npm/@huggingface/transformers@4.3.1", (route) => route.fulfill({ status: 200,
    contentType: "text/javascript", headers: { "access-control-allow-origin": "*" }, body: LM }));
  await lmCtx.route(/en\.wikipedia\.org/, (route) => route.fulfill({ status: 200, contentType: "application/json",
    headers: { "access-control-allow-origin": "*" }, body: "{}" }));
  const lm = await lmCtx.newPage();
  lm.on("pageerror", (e) => errors.push(String(e)));
  await lm.goto(page_url);
  await greeted(lm);
  out.lm_opening = await slash(lm);
  await lm.click("#input");
  await ask(lm, "write a haiku about coffee");
  out.lm_before = (await state(lm)).last;
  await slash(lm, "Download Gemma");
  out.lm_sheet = await state(lm);
  await lm.click("#sheet-ok");
  const kept = (p) => p.evaluate(() => caches.open("chatcomputer-gemma-parts").then((c) => c.keys()).then((k) => k.map((r) => r.url.split("?")[1]).sort()));
  // (asked again until so: waitForFunction takes a promise for a yes)
  for (let t = Date.now(); (await kept(lm)).length < 3; ) {
    if (Date.now() - t > 20000) throw new Error("the pieces never came");
    await new Promise((ok) => setTimeout(ok, 100));
  }
  await lm.waitForFunction(() => /\(\d+%\)/.test(document.getElementById("input").placeholder), null, { timeout: 5000 });
  out.lm_downloading = await state(lm);
  out.lm_downloading_slash = await slash(lm);
  await lm.click("#input");
  out.lm_stuck = await ask(lm, "are you stuck downloading?");
  out.lm_pieces_kept = await kept(lm);
  out.lm_ranges_first = ranges.splice(0);
  await lm.close();                                         // (left mid-download: the third piece never came)
  hub.hold = false;
  hub.fail = 1;
  const lm1 = await lmCtx.newPage();
  lm1.on("pageerror", (e) => errors.push(String(e)));
  await lm1.goto(page_url);
  await greeted(lm1);
  out.lm_again_slash = await slash(lm1);
  await slash(lm1, "Download Gemma");
  await lm1.click("#sheet-ok");
  await lm1.waitForFunction(() => window.__lm.loads.length === 1, null, { timeout: 20000 });
  out.lm_ranges_again = ranges.splice(0);
  out.lm_pieces_after = await kept(lm1);
  await lm1.evaluate(() => window.__lm.finish());
  await lm1.waitForFunction(() => !/Downloading/.test(document.getElementById("input").placeholder));
  out.lm_on = await state(lm1);
  out.lm_on_slash = await slash(lm1);
  await lm1.click("#input");
  await ask(lm1, "write a haiku about coffee");
  out.lm_haiku = await state(lm1);
  await ask(lm1, "play some piano music");
  out.lm_music = (await state(lm1)).last;
  await lm1.evaluate(() => { window.__lm.words = ["Here's ", "a longer one."]; });
  await ask(lm1, "make it longer");
  out.lm_calls = await lm1.evaluate(() => ({ loads: window.__lm.loads, asked: window.__lm.asked.map((m) => m.map((x) => [x.role, x.content])) }));
  await lm1.close();
  // A later visit: it's here, started (from the browser's copy) when it's first needed.
  const lm2 = await lmCtx.newPage();
  lm2.on("pageerror", (e) => errors.push(String(e)));
  await lm2.goto(page_url);
  await greeted(lm2);
  out.lm_later_slash = await slash(lm2);
  await lm2.click("#input");
  await ask(lm2, "write a haiku about tea", async () => {
    out.lm_starting = await (await lm2.waitForFunction(() => { const h = document.getElementById("input").placeholder;
      return /Starting/.test(h) && h; }, null, { polling: "raf", timeout: 5000 })).jsonValue();
  });
  out.lm_later = await lm2.evaluate(() => ({ last: (Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).textContent,
    loads: window.__lm.loads }));
  await lm2.close();
  // The page stopped while it was starting (out of memory): offered again rather than started by itself; removed.
  const lm3 = await lmCtx.newPage();
  lm3.on("pageerror", (e) => errors.push(String(e)));
  await lm3.addInitScript(() => localStorage.setItem("chatcomputer-demo-gemma-starting", "true"));
  await lm3.goto(page_url);
  await greeted(lm3);
  out.lm_stopped_slash = await slash(lm3);
  await slash(lm3, "Start Gemma");
  out.lm_stopped_sheet = await lm3.evaluate(() => ({ title: document.getElementById("sheet-title").textContent,
    text: document.getElementById("sheet-text").innerText, ok: document.getElementById("sheet-ok").textContent }));
  await lm3.keyboard.press("Escape");
  await lm3.waitForFunction(() => !document.body.classList.contains("sheet"));
  await slash(lm3, "Start Gemma");
  await lm3.click("#sheet-ok");
  await lm3.waitForFunction(() => window.__lm.loads.length === 1);
  await lm3.waitForFunction(() => !/Starting/.test(document.getElementById("input").placeholder));
  out.lm_restarted = await lm3.evaluate(() => window.__lm.loads);
  await slash(lm3, "Remove Gemma");
  out.lm_remove_sheet = await state(lm3);
  await lm3.click("#sheet-ok");
  await lm3.waitForFunction(() => /Removed Gemma/.test(document.getElementById("done").textContent));
  out.lm_removed = { slash: await slash(lm3), disposed: await lm3.evaluate(() => !!window.__lm.disposed),
    kept: await lm3.evaluate(() => caches.open("transformers-cache").then((c) => c.keys()).then((k) => k.length)) };
  await lmCtx.close();

  // Read aloud's voice, where the graphics chip can make Heart's (WebGPU; a stand-in for kokoro-js, and the browser's
  // voices standing in too). The first time Read aloud is pressed, Heart is offered: closed, the browser's best voice
  // reads (Edge's natural Ava before Google's and the usual one, never a novelty voice), and Heart isn't offered that
  // way again (the "/" list has it). Chosen, it downloads in pieces (its progress in the box and the list), then reads
  // what was asked, in Heart, not the browser's voice; a later visit starts it from the browser's copy; Remove Heart.
  const KOKORO = `export class KokoroTTS {
  static async from_pretrained(model, opts) {
    const h = window.__heart, c = await caches.open("transformers-cache");
    const data = await c.match("https://huggingface.co/" + model + "/resolve/main/onnx/model.onnx");
    h.loads.push({ model, dtype: opts.dtype, device: opts.device, data: data ? (await data.blob()).size : 0 });
    return { model: { dispose: async () => { h.disposed = true; } }, generate: async (text, o) => {
      h.said.push({ text, voice: o.voice });
      await new Promise((r) => setTimeout(r, 10));
      return { audio: new Float32Array(1200), sampling_rate: 24000 };
    } };
  }
}`;
  const VDATA = 20 * 1024 * 1024, vranges = [], vhub = { hold: false, held: null };
  function fulfilPiece(route, from, to) {
    return route.fulfill({ status: 206, headers: { "access-control-allow-origin": "*", "access-control-expose-headers": "*",
      "content-range": "bytes " + from + "-" + to + "/" + VDATA, "content-type": "application/octet-stream" }, body: Buffer.alloc(to - from + 1, 7) });
  }
  async function voiceContext() {
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    await ctx.route(/^https:\/\/huggingface\.co\/onnx-community\/Kokoro-82M-v1\.0-ONNX\/resolve\/main\/onnx\/model\.onnx$/, async (route) => {
      const req = route.request();
      if (req.method() === "OPTIONS") return route.fulfill({ status: 204, headers: { "access-control-allow-origin": "*",
        "access-control-allow-headers": "range", "access-control-allow-methods": "GET" } });
      const range = req.headers()["range"] || "", m = /bytes=(\d+)-(\d+)/.exec(range);
      vranges.push(range);
      if (!m) return route.fulfill({ status: 200, body: Buffer.alloc(16) });
      const from = +m[1], to = Math.min(+m[2], VDATA - 1);
      if (from > 0 && vhub.hold) { vhub.held = () => fulfilPiece(route, from, to); return; }   // the second piece: when let go
      return fulfilPiece(route, from, to);
    });
    await ctx.route("https://cdn.jsdelivr.net/npm/kokoro-js@1.2.1/dist/kokoro.web.js", (route) => route.fulfill({ status: 200,
      contentType: "text/javascript", headers: { "access-control-allow-origin": "*" }, body: KOKORO }));
    await ctx.route(/en\.wikipedia\.org/, (route) => route.fulfill({ status: 200, contentType: "application/json",
      headers: { "access-control-allow-origin": "*" }, body: "{}" }));
    await ctx.addInitScript(() => {
      window.__heart = { loads: [], said: [] };
      window.__spoken = [];
      Object.defineProperty(navigator, "gpu", { value: { requestAdapter: () => Promise.resolve({ features: new Set() }) } });
      const voices = [{ name: "Albert", lang: "en-US" }, { name: "Samantha", lang: "en-US", default: true },
        { name: "Google US English", lang: "en-US" }, { name: "Microsoft Ava Online (Natural) - English (United States)", lang: "en-US" },
        { name: "Thomas", lang: "fr-FR" }];
      window.speechSynthesis.getVoices = () => voices;
      window.SpeechSynthesisUtterance = function (t) { this.text = t; };
      window.speechSynthesis.speak = (u) => { window.__spoken.push({ text: u.text, voice: u.voice && u.voice.name });
        setTimeout(() => u.onend && u.onend(), 5); };
      window.speechSynthesis.cancel = () => {};
    });
    return ctx;
  }
  const speakLast = (p) => p.click("#log > .acts.theirs.last .act[data-act=speak]");
  const parts = (p) => p.evaluate(() => caches.open("chatcomputer-heart-parts").then((c) => c.keys()).then((k) => k.map((r) => r.url.split("?")[1]).sort()));
  async function answered(p) {
    const page = await p.newPage();
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto(page_url);
    await greeted(page);
    await page.waitForFunction(() => document.querySelector("#log > .ai.hello") && window.CO);
    await page.click("#input");
    await ask(page, "what is chat computer");
    return page;
  }
  // Closed: the browser's voice.
  const vc1 = await voiceContext();
  const v1 = await answered(vc1);
  out.voice_slash = await slash(v1);
  await v1.click("#input");
  await speakLast(v1);
  await v1.waitForFunction(() => document.body.classList.contains("sheet"));
  out.voice_sheet = await v1.evaluate(() => ({ title: document.getElementById("sheet-title").textContent,
    text: document.getElementById("sheet-text").innerText, ok: document.getElementById("sheet-ok").textContent, spoken: window.__spoken.length }));
  await v1.click("#sheet-cancel");
  await v1.waitForFunction(() => window.__spoken.length > 0);
  out.voice_system = await v1.evaluate(() => ({ spoken: window.__spoken.slice(), answer: Array.from(document.querySelectorAll("#log > .ai")).pop().textContent,
    sheet: document.body.classList.contains("sheet"), heart: window.__heart.loads.length }));
  await v1.close();
  const v1b = await answered(vc1);                           // another visit: no sheet, the browser's voice straight away
  await speakLast(v1b);
  await v1b.waitForFunction(() => window.__spoken.length > 0);
  out.voice_again = await v1b.evaluate(() => ({ sheet: document.body.classList.contains("sheet"), spoken: window.__spoken.length }));
  out.voice_again_slash = await slash(v1b);
  await vc1.close();
  // Chosen: downloaded (the second piece held a moment: its progress shown), then the answer read in Heart.
  const vc2 = await voiceContext();
  const v2 = await answered(vc2);
  vhub.hold = true;
  await speakLast(v2);
  await v2.waitForFunction(() => document.body.classList.contains("sheet"));
  await v2.click("#sheet-ok");
  for (let t = Date.now(); !(await parts(v2)).includes("piece=0"); ) {
    if (Date.now() - t > 20000) throw new Error("Heart's first piece never came");
    await new Promise((ok) => setTimeout(ok, 100));
  }
  await v2.waitForFunction(() => /\(\d+%\)/.test(document.getElementById("input").placeholder), null, { timeout: 5000 });
  out.voice_downloading = { hint: await v2.evaluate(() => document.getElementById("input").placeholder), slash: await slash(v2) };
  vhub.hold = false;
  await vhub.held();
  await v2.waitForFunction(() => window.__heart.said.length >= 1, null, { timeout: 20000 });
  out.voice_heart = await v2.evaluate(() => ({ loads: window.__heart.loads, said: window.__heart.said.slice(), spoken: window.__spoken.length,
    answer: Array.from(document.querySelectorAll("#log > .ai")).pop().textContent, done: document.getElementById("done").textContent,
    hint: document.getElementById("input").placeholder }));
  out.voice_ranges = vranges.splice(0);
  out.voice_parts_after = await parts(v2);
  out.voice_on_slash = await slash(v2);
  await v2.close();
  // A later visit: from the browser's copy (nothing downloaded), in Heart; then removed.
  const v3 = await answered(vc2);
  await speakLast(v3);
  await v3.waitForFunction(() => window.__heart.said.length >= 1, null, { timeout: 20000 });
  out.voice_later = await v3.evaluate(() => ({ loads: window.__heart.loads, spoken: window.__spoken.length, sheet: document.body.classList.contains("sheet") }));
  out.voice_later_ranges = vranges.splice(0);
  await slash(v3, "Remove Heart");
  out.voice_remove_sheet = await v3.evaluate(() => document.getElementById("sheet-title").textContent);
  await v3.click("#sheet-ok");
  await v3.waitForFunction(() => /Removed the Heart voice/.test(document.getElementById("done").textContent));
  out.voice_removed = { slash: await slash(v3), kept: await v3.evaluate(() => caches.open("transformers-cache").then((c) => c.keys())
    .then((k) => k.filter((r) => /Kokoro/.test(r.url)).length)), disposed: await v3.evaluate(() => !!window.__heart.disposed) };
  await vc2.close();

  // Under each message, as in the Claude app: yours has when, ask again, edit and copy; the answer's has copy, read
  // aloud, good, bad, ask again and when, and the latest answer's row shows. While an answer comes in, its newest
  // words fade in. Good is kept in this browser; edit puts your words back in the box; ask again asks them again.
  const rows = await browser.newPage({ viewport: { width: 1600, height: 900 } });
  rows.on("pageerror", (e) => errors.push(String(e)));
  await rows.route(/en\.wikipedia\.org/, (route) => route.fulfill({ status: 200, contentType: "application/json",
    headers: { "access-control-allow-origin": "*" }, body: "{}" }));
  await rows.goto(page_url);
  await rows.waitForFunction(() => window.CO && document.getElementById("toggle-left"));
  await rows.waitForTimeout(300);
  await rows.click("#input");
  await rows.keyboard.type("is it free");
  await rows.keyboard.press("Enter");
  await rows.waitForTimeout(900);
  out.fading = await rows.evaluate(() => Array.from(document.querySelectorAll("#log > .ai .fresh")).map((s) => getComputedStyle(s).animationName));
  await rows.waitForFunction(() => document.querySelector("#log > .acts.theirs"));
  await rows.waitForTimeout(500);
  out.rows = await rows.evaluate(() => Array.from(document.querySelectorAll("#log > .acts")).map((r) => ({
    kind: r.classList.contains("mine") ? "mine" : "theirs", last: r.classList.contains("last"), when: (r.querySelector(".when") || {}).textContent,
    acts: Array.from(r.querySelectorAll(".act")).map((b) => b.getAttribute("aria-label")), shown: getComputedStyle(r).opacity,
    after: r.previousElementSibling.className.split(" ")[0] })));
  await rows.click("#log > .acts.theirs .act[data-act=good]");
  await rows.waitForTimeout(150);                          // the page keeps it a moment later (its host's message)
  out.feedback = await rows.evaluate(() => ({ kept: JSON.parse(localStorage.getItem("chatcomputer-demo-feedback") || "[]"),
    on: Array.from(document.querySelectorAll("#log > .acts.theirs .act.on")).map((b) => b.dataset.act) }));
  await rows.hover("#log > .you");
  await rows.click("#log > .acts.mine .act[data-act=edit]");
  out.edit = await rows.evaluate(() => ({ box: document.getElementById("input").value, focused: document.activeElement.id }));
  await rows.fill("#input", "");
  await rows.hover("#log > .you");
  await rows.click("#log > .acts.mine .act[data-act=retry]");
  await rows.waitForFunction(() => document.querySelectorAll("#log > .acts.theirs").length === 2);
  out.retry = await rows.evaluate(() => ({ yous: Array.from(document.querySelectorAll("#log > .you")).map((y) => y.textContent),
    last: Array.from(document.querySelectorAll("#log > .acts")).filter((r) => r.classList.contains("last")).length }));
  out.rows_hello = await rows.evaluate(() => document.querySelectorAll("#log > .ai.hello").length);
  if (shot) await rows.screenshot({ path: shot + "-rows.png" });

  // The text's size, asked for in words: the way asked ("increase the font size" bigger, "decrease" smaller); "font"
  // alone asks which way; what the font is, named; back to the usual size.
  const sizes = await browser.newPage({ viewport: { width: 1600, height: 900 } });
  sizes.on("pageerror", (e) => errors.push(String(e)));
  await sizes.goto(page_url);
  await greeted(sizes);
  const rem = () => sizes.evaluate(() => parseFloat(getComputedStyle(document.documentElement).fontSize));
  const sized = async (text) => { await ask(sizes, text); return { rem: await rem(), last: (await state(sizes)).last }; };
  await sizes.click("#input");
  out.sizes = { start: await rem() };
  out.sizes.increase = await sized("Increase font size");
  out.sizes.font = await sized("font");
  out.sizes.decrease = await sized("decrease the font size");
  out.sizes.smaller = await sized("smaller text please");
  out.sizes.which = await sized("what font is this?");
  out.sizes.normal = await sized("normal text size");
  await sizes.close();

  // A phone: no sidebars and no switches; nothing in the box but the words and the arrow; it works.
  const phone = await browser.newPage({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  phone.on("pageerror", (e) => errors.push(String(e)));
  await phone.route(/youtube/, (route) => route.fulfill({ status: 200, contentType: "text/html", body: PLAYER }));
  await phone.goto(page_url);
  await phone.waitForFunction(() => window.CO && document.getElementById("toggle-left"));
  await greeted(phone);
  out.phone_opening = await state(phone);
  // As in the Claude app (measured on an iPhone): the text a eighteenth of the screen's width, never under 16px (no
  // zooming in on the box); the conversation and the box 0.77rem from the sides; the box 100px tall at 375px across,
  // the words above and the arrow under them at the right, 8px from the box's edges; nothing past the screen's edges.
  out.phone_layout = await phone.evaluate(() => {
    const r = (id) => document.getElementById(id).getBoundingClientRect();
    const box = r("box"), input = r("input"), send = r("send"), rem = parseFloat(getComputedStyle(document.documentElement).fontSize);
    return { phone: document.body.classList.contains("phone"), rem: rem,
      input_px: parseFloat(getComputedStyle(document.getElementById("input")).fontSize), box_w: Math.round(box.width), width: innerWidth,
      box_left: Math.round(box.left), box_h: Math.round(box.height / rem * 100) / 100, send_under: send.top >= input.bottom,
      send_right: Math.round(box.right - send.right), words_left: Math.round(input.left - box.left),
      scroll_w: document.documentElement.scrollWidth, typing_in: document.activeElement && document.activeElement.id };
  });
  await phone.tap("#input");
  out.phone_tapped = await phone.evaluate(() => document.activeElement.id);
  // The keyboard (a stand-in: what's on the screen, the visual viewport, made 300px shorter, as Safari's is on an
  // iPhone, where the page itself isn't made smaller): the box just above it, never under it, whether Safari leaves
  // what's on the screen at the page's top or scrolls it down to the box; down again, the box back at the bottom.
  out.phone_keyboard = await phone.evaluate(async () => {
    const vv = window.visualViewport, H = innerHeight, frames = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    const put = (height, top, kind) => {
      Object.defineProperty(vv, "height", { configurable: true, get: () => height });
      Object.defineProperty(vv, "offsetTop", { configurable: true, get: () => top });
      vv.dispatchEvent(new Event(kind));
      return frames();
    };
    const box = () => Math.round(document.getElementById("composer").getBoundingClientRect().bottom);
    const room = () => Math.round(parseFloat(getComputedStyle(document.getElementById("column")).paddingBottom));
    const before = room();
    await put(H - 300, 0, "resize");
    const up = { box: box(), seen: H - 300, room: room() - before, keyboard: document.body.classList.contains("keyboard") };
    await put(H - 300, 300, "scroll");
    const scrolled = { box: box(), seen: H };
    await put(H, 0, "resize");
    const down = { box: box(), seen: H, room: room() - before, keyboard: document.body.classList.contains("keyboard") };
    delete vv.height; delete vv.offsetTop;
    return { up, scrolled, down };
  });
  await ask(phone, "hello");
  out.phone = await state(phone);
  if (shot) await phone.screenshot({ path: shot + "-phone.png" });
  // Holding a message: what can be done with it, over the screen dimmed (fingers, through the browser's own touch
  // events: Playwright has no holding).
  const touch = await phone.context().newCDPSession(phone);
  const fingers = (type, points) => touch.send("Input.dispatchTouchEvent", { type: type, touchPoints: points });
  async function holdOn(sel) {
    const at = await phone.evaluate((s) => {
      const m = Array.from(document.querySelectorAll(s)).pop();
      m.scrollIntoView({ block: "center" });
      const r = m.getBoundingClientRect();
      return { x: Math.round(r.left + Math.min(40, r.width / 2)), y: Math.round(r.top + Math.min(16, r.height / 2)) };
    }, sel);
    await fingers("touchStart", [at]);
    await phone.waitForTimeout(650);
    const seen = await phone.evaluate(() => ({ holding: document.body.classList.contains("holding"),
      held: (document.querySelector(".held") || {}).className || "",
      items: Array.from(document.querySelectorAll("#hold-menu button")).filter((b) => !b.hidden).map((b) => b.textContent.trim()),
      selected: String(window.getSelection()) }));
    await fingers("touchEnd", []);
    return seen;
  }
  out.phone_hold_you = await holdOn("#log > .you");
  if (shot) await phone.screenshot({ path: shot + "-phone-hold.png" });
  await phone.tap("#hold-menu [data-do=edit]");
  out.phone_edit = await phone.evaluate(() => ({ holding: document.body.classList.contains("holding"),
    box: document.getElementById("input").value, typing_in: document.activeElement.id }));
  await phone.evaluate(() => { const i = document.getElementById("input"); i.value = ""; i.dispatchEvent(new Event("input")); i.blur(); });
  out.phone_hold_ai = await holdOn("#log > .ai:not(.hello)");
  await phone.tap("#hold-menu [data-do=select]");
  out.phone_select = await phone.evaluate(() => ({ reading: document.body.classList.contains("reading"),
    text: document.getElementById("reader-text").textContent.slice(0, 40), select: getComputedStyle(document.getElementById("reader-text")).userSelect }));
  await phone.tap("#reader-close");
  await holdOn("#log > .ai:not(.hello)");
  await phone.touchscreen.tap(4, 4);                      // the dimmed screen, beside the held message
  out.phone_put_away = await phone.evaluate(() => document.body.classList.contains("holding") || !!document.querySelector(".held"));
  await holdOn("#log > .you");
  await phone.tap("#log > .you.held");                     // the held message itself
  out.phone_put_away_too = await phone.evaluate(() => document.body.classList.contains("holding") || !!document.querySelector(".held"));
  out.phone_shade = await phone.evaluate(() => {
    document.body.classList.add("holding");
    const s = getComputedStyle(document.getElementById("hold-shade")), m = getComputedStyle(document.getElementById("hold-menu"));
    document.body.classList.remove("holding");
    return { blur: s.backdropFilter, menu_blur: m.backdropFilter };
  });
  // Two fingers pinched: nothing zooms, and the text stays its size.
  await fingers("touchStart", [{ x: 150, y: 300, id: 1 }, { x: 240, y: 300, id: 2 }]);
  for (let i = 1; i <= 8; i++) await fingers("touchMove", [{ x: 150 - 12 * i, y: 300, id: 1 }, { x: 240 + 12 * i, y: 300, id: 2 }]);
  await fingers("touchEnd", []);
  await phone.waitForTimeout(150);
  out.phone_pinched = await phone.evaluate(() => ({ rem: parseFloat(getComputedStyle(document.documentElement).fontSize),
    zoom: window.visualViewport ? visualViewport.scale : 1, scroll_w: document.documentElement.scrollWidth }));
  // Music, and then asked about it: turned down (the same music, quieter: not started again), then stopped.
  await ask(phone, "play some piano music");
  out.phone_music = await state(phone);
  const piano = await (await phone.$("#demo-view")).contentFrame();
  await piano.waitForSelector("#play");
  await piano.evaluate(() => { window.same = true; });
  await ask(phone, "turn the music down");
  out.phone_down = await state(phone);
  out.phone_down_piano = await piano.evaluate(() => ({ same: window.same === true, vol: window.vol }));
  await ask(phone, "stop the music");
  out.phone_stopped = await state(phone);

  // The buttons under messages, pressed with a finger (held a moment, as fingers are) on a phone: with the keyboard
  // up (a stand-in: what's on the screen 300px shorter while the box has the keys, given back a moment after), a press
  // still does what it says, and the keyboard goes down after it; an earlier answer tapped (its row shown) and tapped
  // again (put away) leaves no row showing whose buttons can't be pressed.
  const thumbs = await browser.newContext({ ...devices["iPhone 13"] });
  const th = await thumbs.newPage();
  th.on("pageerror", (e) => errors.push(String(e)));
  await th.addInitScript(() => {
    window.__spoken = [];
    window.speechSynthesis.speak = (u) => { window.__spoken.push(u.text); };
    window.speechSynthesis.cancel = () => {};
    document.addEventListener("DOMContentLoaded", () => {
      const input = document.getElementById("input"), vv = window.visualViewport;
      const put = (up) => {
        const H = innerHeight;
        Object.defineProperty(vv, "height", { configurable: true, get: () => (up ? H - 300 : H) });
        Object.defineProperty(vv, "offsetTop", { configurable: true, get: () => 0 });
        setTimeout(() => vv.dispatchEvent(new Event("resize")), 30);
      };
      input.addEventListener("focus", () => put(true));
      input.addEventListener("blur", () => put(false));
    });
  });
  await th.goto(page_url);
  await greeted(th);
  await th.tap("#input");
  await ask(th, "is it free");
  await th.tap("#input");
  await ask(th, "what is chat computer");
  const cdp = await thumbs.newCDPSession(th);
  const finger = async (sel) => {
    const bb = await th.locator(sel).boundingBox();
    const pt = { x: Math.round(bb.x + bb.width / 2), y: Math.round(bb.y + bb.height / 2) };
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [pt] });
    await th.waitForTimeout(120);
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await th.waitForTimeout(300);
  };
  const pressed = () => th.evaluate(() => ({ spoken: window.__spoken.length, keyboard: document.body.classList.contains("keyboard"),
    on: Array.from(document.querySelectorAll("#log > .acts .act.on")).map((b) => b.dataset.act) }));
  await th.tap("#input");
  await th.waitForTimeout(200);
  out.thumb_keyboard = (await pressed()).keyboard;
  await finger("#log > .acts.theirs.last .act[data-act=speak]");
  out.thumb_speak = await pressed();
  await th.tap("#input");
  await th.waitForTimeout(200);
  await finger("#log > .acts.theirs.last .act[data-act=good]");
  out.thumb_good = await pressed();
  const earlier = th.locator("#log > .ai").nth(1);
  await earlier.scrollIntoViewIfNeeded();
  await earlier.tap();
  await th.waitForTimeout(250);
  await earlier.tap();
  await th.waitForTimeout(250);
  out.thumb_twice = await th.evaluate(() => { const row = document.querySelector("#log > .acts.theirs");
    return { shown: row.classList.contains("shown"), opacity: getComputedStyle(row).opacity }; });
  await thumbs.close();

  // Connect AirPods on an iPhone, with stand-ins for Safari's listening and speaking, the microphone and the wake lock:
  // one thing heard at a time; what's heard said once it stops changing, though Safari never calls it final; heard
  // twice over, asked once; the answer read out (the tap having let it speak), then listening again; the screen kept
  // on meanwhile; and refused, what to switch on.
  const iphone = await browser.newContext({ ...devices["iPhone 13"] });
  await iphone.addInitScript(() => {
    window.recs = [];
    function Rec() { window.recs.push(this); }
    Rec.prototype.start = function () { this.started = true; };
    Rec.prototype.stop = Rec.prototype.abort = function () { this.ended = true; const r = this; setTimeout(() => r.onend && r.onend(), 5); };
    Rec.prototype.hear = function (text, final) {
      this.onresult({ resultIndex: 0, results: [{ 0: { transcript: text }, isFinal: !!final, length: 1 }] });
    };
    window.SpeechRecognition = window.webkitSpeechRecognition = Rec;
    window.spoken = [];
    window.SpeechSynthesisUtterance = function (t) { this.text = t; this.volume = 1; };
    Object.defineProperty(window, "speechSynthesis", { configurable: true, value: { cancel() {},
      speak(u) { window.spoken.push({ text: u.text, volume: u.volume }); setTimeout(() => u.onend && u.onend(), 50); } } });
    Object.defineProperty(navigator, "mediaDevices", { configurable: true, value: { getUserMedia: () => Promise.resolve({
      getAudioTracks: () => [{ label: "AirPods Pro" }], getTracks: () => [{ stop() {} }] }) } });
    window.awake = 0;
    Object.defineProperty(navigator, "wakeLock", { configurable: true, value: { request() {
      window.awake++; return Promise.resolve({ release() { window.awake--; }, addEventListener() {} }); } } });
  });
  const ip = await iphone.newPage();
  ip.on("pageerror", (e) => errors.push(String(e)));
  await ip.goto(page_url);
  await ip.waitForFunction(() => window.CO && document.getElementById("toggle-left"));
  await slash(ip, "Connect AirPods", true);
  await ip.tap("#sheet-ok");
  await ip.waitForFunction(() => window.recs.length === 1 && window.recs[0].started);
  out.talk_phone = await ip.evaluate(() => ({ continuous: window.recs[0].continuous, spoken: window.spoken.slice(), awake: window.awake,
    hint: document.getElementById("input").placeholder }));
  await ip.evaluate(() => window.recs[0].hear("what time"));
  await ip.waitForTimeout(300);
  await ip.evaluate(() => window.recs[0].hear("what time is it"));
  out.talk_hearing = await ip.evaluate(() => document.getElementById("live-text").textContent);
  await ip.waitForFunction(() => document.querySelectorAll("#log > .you").length === 1, null, { timeout: 5000 });
  await ip.waitForFunction(() => window.spoken.length === 2 && window.recs.length === 2 && window.recs[1].started, null, { timeout: 15000 });
  out.talk_answered = await ip.evaluate(() => ({ asked: document.querySelector("#log > .you").textContent,
    read_out: window.spoken[1].text, answer: Array.from(document.querySelectorAll("#log > .ai")).pop().textContent,
    first_ended: !!window.recs[0].ended }));
  await ip.evaluate(() => window.recs[1].hear("What time is it in Tokyo what time is it in Tokyo", true));
  await ip.waitForFunction(() => document.querySelectorAll("#log > .you").length === 2, null, { timeout: 5000 });
  out.talk_twice = await ip.evaluate(() => Array.from(document.querySelectorAll("#log > .you")).pop().textContent);
  await ip.waitForFunction(() => window.recs.length === 3 && window.recs[2].started, null, { timeout: 15000 });
  await ip.evaluate(() => window.recs[2].onerror({ error: "service-not-allowed" }));
  out.talk_refused = await ip.evaluate(() => ({ title: document.getElementById("sheet-title").textContent,
    text: document.getElementById("sheet-text").textContent, hint: document.getElementById("input").placeholder,
    awake: window.awake }));
  await iphone.close();

  // The greeting, as the Claude app's: for the time and the day, with the name you go by, and for where you are and
  // its weather (Open-Meteo standing in: Singapore, your time zone's city; Penang when you say you live there). A
  // Friday evening, rain, a warm night, called Ren; a Friday morning, sunny, no name. Then "what's my name?",
  // "call me", "call me back later" (not a name), "I live in", the weather there, and forgetting the name.
  const sky = { code: 61, temp: 27.4, day: 0 }, meteo = [];
  async function singapore(at) {
    const ctx = await browser.newContext({ timezoneId: "Asia/Singapore", viewport: { width: 1280, height: 800 } });
    await ctx.route(/open-meteo\.com/, (route) => {
      const u = new URL(route.request().url()), name = u.searchParams.get("name") || "";
      meteo.push(u.hostname.split(".")[0] + (name ? " " + name : ""));
      const json = u.hostname.startsWith("geocoding") ? { results: [/penang/i.test(name) ?
        { name: "George Town", country: "Malaysia", latitude: 5.41, longitude: 100.33 } : { name: "Singapore", country: "Singapore", latitude: 1.29, longitude: 103.85 }] } :
        { current: { temperature_2m: sky.temp, weather_code: sky.code, is_day: sky.day }, daily: { weather_code: [sky.code, 3],
          temperature_2m_max: [31, 32], temperature_2m_min: [26, 26], precipitation_probability_max: [80, 40] } };
      return route.fulfill({ status: 200, contentType: "application/json", headers: { "access-control-allow-origin": "*" }, body: JSON.stringify(json) });
    });
    const p = await ctx.newPage();
    p.on("pageerror", (e) => errors.push(String(e)));
    await p.clock.setFixedTime(new Date(at));
    await p.goto(page_url);
    await greeted(p);
    return [ctx, p];
  }
  const [eveCtx, eve] = await singapore("2026-10-23T20:30:00+08:00");          // a Friday evening
  await eve.evaluate(() => localStorage.setItem("chatcomputer-demo-name", JSON.stringify("Ren")));
  await eve.reload();                                                            // (the weather kept from the first)
  out.greet_evening = { said: await greeted(eve), now: await eve.evaluate(() => window.CO.greetingsNow()), meteo: meteo.slice() };
  await eve.click("#input");
  out.name_asked = await ask(eve, "what's my name?");
  out.name_given = { said: await ask(eve, "call me ren shu"), time: (await eve.evaluate(() => window.CO.greetingsNow())).time };
  out.name_not = await ask(eve, "call me back later");
  out.place_given = { said: await ask(eve, "I live in penang"), here: (await eve.evaluate(() => window.CO.greetingsNow())).here };
  out.place_weather = await ask(eve, "weather");
  out.name_forgotten = { said: await ask(eve, "forget my name"), time: (await eve.evaluate(() => window.CO.greetingsNow())).time,
    done: await eve.evaluate(() => Array.from(document.querySelectorAll("#done .done-what")).map((e) => e.textContent)) };
  out.greet_meteo = meteo.slice();
  await eveCtx.close();
  Object.assign(sky, { code: 0, temp: 29, day: 1 });
  const [mornCtx, morn] = await singapore("2026-10-23T08:15:00+08:00");         // a Friday morning, sunny, no name
  out.greet_morning = { said: await greeted(morn), now: await morn.evaluate(() => window.CO.greetingsNow()) };
  await mornCtx.close();
  Object.assign(sky, { code: 3, temp: 26.5, day: 0 });
  const [dawnCtx, dawn] = await singapore("2026-10-21T06:15:00+08:00");         // a Wednesday, cloudy, still dark
  out.greet_dawn = (await dawn.evaluate(() => window.CO.greetingsNow())).here;
  await dawnCtx.close();

  out.errors = errors;
  await browser.close();
  console.log(JSON.stringify(out));
})().catch((e) => { console.log(JSON.stringify({ failed: String(e && e.stack || e) })); process.exit(1); });
