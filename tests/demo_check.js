// The browser demo (docs/index.html: Chat Computer's own screen, with docs/demo/host.js standing in for the computer),
// tried in Chromium with Playwright: it opens on the word logo, with a greeting under it, both there until
// something's said, the cursor in the box; the sidebars' switches; music (made by the browser), in the middle in the conversation's place; "/" lists
// the files in front of what's open; the sidebars from the start (only their switches put them away); Escape closes
// what's open, and so does the x; a message opened as its whole conversation; an answer in words alone bringing the conversation back;
// Google, for real, typed into; this computer's model, where the Chat Computer app runs here (a stand-in for its
// engine; tools used: a map, a YouTube video, which closes when it ends, with a stand-in for YouTube's player;
// answers streamed); the app itself (a stand-in), its model getting ready and then answering with the app's own
// tools; Connect AirPods' sheet; Heart, the warmer voice (a stand-in), downloaded in pieces; a phone, laid out as the
// Claude app is there (the keyboard down after sending, a held message's menu, nothing zooming; music turned down,
// then stopped); and Connect AirPods on an iPhone, with stand-ins for Safari's listening and speaking.
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

// Chat Computer's engine (os/model), standing in: OpenAI's chat completions API, its answers streamed as the engine
// streams them ("data:" events, then "data: [DONE]"): a tool's call (a map of Tokyo, a video, the volume; its
// arguments here in two pieces), or words.
function chunk(delta, finish) {
  return "data: " + JSON.stringify({ id: "chatcmpl-1", object: "chat.completion.chunk", created: 1, model: "qwen3.5-9b",
    choices: [{ index: 0, delta: delta, finish_reason: finish || null }] }) + "\n\n";
}
function toolTurn(id, name, args) {
  const half = Math.floor(args.length / 2);
  return chunk({ role: "assistant" }) +
    chunk({ tool_calls: [{ index: 0, id: id, type: "function", function: { name: name, arguments: args.slice(0, half) } }] }) +
    chunk({ tool_calls: [{ index: 0, function: { arguments: args.slice(half) } }] }) + chunk({ content: "" }, "tool_calls") + "data: [DONE]\n\n";
}
function textTurn(words) {
  return chunk({ role: "assistant" }) + words.map((w) => chunk({ content: w })).join("") + chunk({ content: "" }, "stop") + "data: [DONE]\n\n";
}
// An AI's first answer to "a timer for 20 minutes": its thinking (Mistral's as pieces of its words), then a call of
// the page's make_timer, carrying what that AI puts on a call (Gemini: its signature); then, once the timer's made,
// its words.
function keyedToolTurn(k) {
  const call = { index: 0, id: "call_k1", type: "function", function: { name: "make_timer", arguments: '{"minutes": 20}' } };
  if (k.extra) call.extra_content = k.extra;
  const thinking = k.mistral ? chunk({ content: [{ type: "thinking", thinking: [{ type: "text", text: "Thinking it over." }] }] }) :
    chunk({ reasoning_content: "Thinking it over." });
  return chunk({ role: "assistant" }) + thinking + chunk({ tool_calls: [call] }) + chunk({ content: "" }, "tool_calls") + "data: [DONE]\n\n";
}
// OpenAI's Responses API, standing in: a call of make_timer, then words.
function responsesSSE(events) { return events.map((e) => "event: " + e.type + "\ndata: " + JSON.stringify(e) + "\n\n").join(""); }
const RESPONSES_CALL = responsesSSE([
  { type: "response.output_item.added", output_index: 0, item: { type: "function_call", id: "fc_1", call_id: "call_o1", name: "make_timer", arguments: "" } },
  { type: "response.function_call_arguments.delta", output_index: 0, delta: '{"minutes": 20}' },
  { type: "response.output_item.done", output_index: 0, item: { type: "function_call", id: "fc_1", call_id: "call_o1", name: "make_timer", arguments: '{"minutes": 20}' } },
  { type: "response.completed", response: {} }]);
const RESPONSES_WORDS = responsesSSE([{ type: "response.output_text.delta", output_index: 0, delta: "Hello from " },
  { type: "response.output_text.delta", output_index: 0, delta: "OpenAI." }, { type: "response.completed", response: {} }]);
// Anthropic's Messages API, standing in: its thinking (signed) and a call of make_timer, its input in two pieces; then words.
function claudeSSE(events) { return events.map((e) => "event: " + e.type + "\ndata: " + JSON.stringify(e) + "\n\n").join(""); }
const CLAUDE_CALL = claudeSSE([
  { type: "message_start", message: { id: "msg_1", type: "message", role: "assistant", content: [] } },
  { type: "content_block_start", index: 0, content_block: { type: "thinking", thinking: "", signature: "" } },
  { type: "content_block_delta", index: 0, delta: { type: "thinking_delta", thinking: "Thinking it over." } },
  { type: "content_block_delta", index: 0, delta: { type: "signature_delta", signature: "sig-claude" } },
  { type: "content_block_stop", index: 0 },
  { type: "content_block_start", index: 1, content_block: { type: "tool_use", id: "toolu_1", name: "make_timer", input: {} } },
  { type: "content_block_delta", index: 1, delta: { type: "input_json_delta", partial_json: '{"minu' } },
  { type: "content_block_delta", index: 1, delta: { type: "input_json_delta", partial_json: 'tes": 20}' } },
  { type: "content_block_stop", index: 1 }, { type: "message_delta", delta: { stop_reason: "tool_use" } }, { type: "message_stop" }]);
const CLAUDE_WORDS = claudeSSE([
  { type: "message_start", message: { id: "msg_2", type: "message", role: "assistant", content: [] } },
  { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } },
  { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Hello from " } },
  { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Claude." } },
  { type: "content_block_stop", index: 0 }, { type: "message_delta", delta: { stop_reason: "end_turn" } }, { type: "message_stop" }]);
// Every AI in the panel, with a key, each standing in at its own address: what it lists (null: no list there), and
// how it answers. (Kimi's key is one from its other site: its first address says no.)
const KEYED_AIS = {
  Gemini: { base: "https://generativelanguage.googleapis.com/v1beta/openai", extra: { google: { thought_signature: "sig-1" } }, effort: "High",
            models: [{ id: "models/gemini-3.8-flash" }, { id: "models/gemini-4-argon" }, { id: "models/gemini-embedding-2" }, { id: "models/gemini-nano-banana-2.1" }] },
  Claude: { base: "https://api.anthropic.com/v1", claude: true, effort: "Max", models: [{ id: "claude-haiku-5-5", created_at: "2026-10-01T00:00:00Z" },
            { id: "claude-opus-5-5", display_name: "Claude Opus 5.5", created_at: "2026-09-01T00:00:00Z", capabilities: { effort: { supported: true,
              low: { supported: true }, medium: { supported: true }, high: { supported: true }, xhigh: null, max: { supported: true } } } },
            { id: "claude-sonnet-5-5", created_at: "2026-09-15T00:00:00Z" }] },
  OpenAI: { base: "https://api.openai.com/v1", responses: true, effort: "Max", models: [{ id: "gpt-6.1-sol-pro", created: 30 }, { id: "gpt-6.1-sol", created: 20 },
            { id: "gpt-6-astra", created: 10 }, { id: "gpt-image-2", created: 40 }] },
  Meta: { base: "https://api.meta.ai/v1", effort: "Max", models: [{ id: "muse-spark-1.3-contributor", created: 30 }, { id: "muse-spark-1.3", created: 20 },
          { id: "muse-wonder-1.0", created: 10 }, { id: "muse-image-1", created: 40 }] },
  Kimi: { base: "https://api.moonshot.ai/v1", alt: "https://api.moonshot.cn/v1", effort: "Low", models: [{ id: "kimi-k3.1-pro-highspeed", created: 20 },
          { id: "kimi-k3.1-pro", created: 10 }, { id: "kimi-k2.6", created: 5 }] },
  Grok: { base: "https://api.x.ai/v1", effort: "Extra high", models: [{ id: "grok-imagine-video", created: 30 }, { id: "grok-4.7", created: 20 }, { id: "grok-4.6", created: 10 }] },
  Qwen: { base: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", effort: "Off", models: [{ id: "qwen3.8-flash" }, { id: "qwen3.8-max" },
          { id: "qwen-image-2" }, { id: "text-embedding-v4" }] },
  DeepSeek: { base: "https://api.deepseek.com", effort: "Max", models: [{ id: "deepseek-flash" },
              { id: "deepseek-v4-pro", effort: { supported_levels: ["low", "high", "max"], default_level: "high" } }] },
  GLM: { base: "https://api.z.ai/api/paas/v4", effort: "Low", models: null },
  MiMo: { base: "https://api.xiaomimimo.com/v1", effort: "Off", models: [{ id: "mimo-v2.6-flash" }, { id: "mimo-v2.6-pro-ultraspeed" }, { id: "mimo-v2.6-pro" }, { id: "mimo-v2.6-asr" }] },
  MiniMax: { base: "https://api.minimax.io/v1", effort: "High", words: ["<thi", "nk>pondering</th", "ink>Hello from ", "MiniMax."],
             models: [{ id: "MiniMax-M3", created: 10 }, { id: "MiniMax-M3.1-Flash", created: 20 }, { id: "speech-3.0", created: 30 }] },
  Mistral: { base: "https://api.mistral.ai/v1", mistral: true, effort: "High", models: [
    { id: "mistral-large-4", created: 20, capabilities: { completion_chat: true, function_calling: true } },
    { id: "mistral-ocr-3", created: 30, capabilities: { completion_chat: false } },
    { id: "mistral-medium-3-5", created: 10, capabilities: { completion_chat: true, function_calling: true } }] },
};
// OpenRouter's models, standing in: free ones, and makers' paid ones (the best of each to be offered).
const OPENROUTER_MODELS = [["a/free:free", 0, 100], ["b/free-two:free", 0, 200], ["google/gemini-3.8-flash", 1, 60], ["google/gemini-4-argon", 1, 50],
  ["anthropic/claude-haiku-5.5", 1, 20], ["anthropic/claude-opus-5.5", 1, 10], ["openai/gpt-6.1-sol-pro", 1, 40], ["openai/gpt-6.1-sol", 1, 39],
  ["openai/gpt-6-astra", 1, 30], ["meta/muse-spark-1.3-contributor", 1, 30], ["meta/muse-wonder", 1, 25], ["x-ai/grok-4.7", 1, 20],
  ["x-ai/grok-4.7:batch", 1, 20], ["deepseek/deepseek-v4.1-flash-1017", 1, 50], ["deepseek/deepseek-v4-pro-0813", 1, 30], ["deepseek/deepseek-v4-pro", 1, 5],
  ["stepfun/step-5-preview", 1, 10], ["inclusionai/ling-3.1-flash", 1, 10], ["mistralai/mistral-large-4-0", 1, 10]].map(([id, paid, created]) =>
  ({ id: id, created: created, pricing: { prompt: paid ? "0.000001" : "0", completion: paid ? "0.000002" : "0" }, supported_parameters: ["tools"] }));
const MAP_TURN = toolTurn("call_1", "show_map", '{"place": "Tokyo", "west": 139.55, "south": 35.55, "east": 139.95, "north": 35.8}');
const VIDEO_TURN = toolTurn("call_2", "play_youtube", '{"video_id": "aBcDeFgHiJk", "title": "A test video"}');
const VOLUME_TURN = toolTurn("call_3", "set_volume", '{"percent": 30}');
const HEALTH = JSON.stringify({ ok: true, model: "qwen3.5-9b", style: "qwen3.5",
  uncensored: { taken_out: true, checked: 24, refused_before: 18, refused_after: 0 } });
// What the page asked the engine for.
function asked(route) {
  const req = route.request(), body = JSON.parse(req.postData() || "{}");
  return { method: req.method(), path: new URL(req.url()).pathname, model: body.model, stream: body.stream, think: body.think,
           max_tokens: body.max_tokens, tools: (body.tools || []).map((t) => t.type + " " + (t.function || {}).name),
           messages: body.messages || [] };
}

const ERRORS = [];
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
      // (The last answer's words; its buttons, where it has them, apart.)
      last: (() => { const a = Array.from(document.querySelectorAll("#log > .ai")).pop(); if (!a) return "";
        const c = a.cloneNode(true); c.querySelectorAll(".offers").forEach((o) => o.remove()); return c.textContent; })(),
      offers: Array.from((Array.from(document.querySelectorAll("#log > .ai")).pop() || document.createElement("i"))
        .querySelectorAll(".offers button")).map((b) => b.textContent + (b.classList.contains("main") ? " *" : "")),
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
      buttons: Array.from(document.querySelectorAll("#demo-tools, #box button:not(#send)")).filter((b) => b.offsetParent).map((b) => b.id || b.className),
      tools: Array.from(document.querySelectorAll("#tools button")).filter((b) => b.offsetParent).map((b) => [b.getAttribute("aria-label"), b.getBoundingClientRect().left, b.getBoundingClientRect().top]),
      tools_round: Array.from(document.querySelectorAll("#tools button")).filter((b) => b.offsetParent).map((b) => { const r = b.getBoundingClientRect(), st = getComputedStyle(b);
        return [Math.round(r.width), Math.round(r.height), st.borderRadius, st.backgroundColor, Math.round(r.right)]; }),
      box_at: [document.getElementById("box").getBoundingClientRect().left, document.getElementById("box").getBoundingClientRect().right],
      send_at: [document.getElementById("send").getBoundingClientRect().left, document.getElementById("send").getBoundingClientRect().top],
      send_size: [Math.round(document.getElementById("send").getBoundingClientRect().width), Math.round(document.getElementById("send").getBoundingClientRect().height)],
      // (Between + and the spark, and between the microphone and the arrow.)
      gaps: (() => { const r = (id) => document.getElementById(id).getBoundingClientRect();
        return document.getElementById("ai").offsetParent ? [+(r("ai").left - r("add").right).toFixed(1), +(r("send").left - r("mic").right).toFixed(1)] : null; })(),
      input_top: document.getElementById("input").getBoundingClientRect().top,
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
  return (await f.locator("body").innerText({ timeout: 15000 })).replace(/\s+/g, " ").trim();
}
function player(page) { return page.frames().find((f) => /youtube/.test(f.url())); }

// Chat Computer's first words, once they've all come in (after a second of the word logo, its fade, and a moment's
// thinking).
// The greeting under the word logo, once it's chosen.
async function greeted(page) {
  await page.waitForFunction(() => { const g = document.getElementById("greeting"); return g && g.classList.contains("shown") && g.textContent; },
    null, { timeout: 8000 });
  return page.evaluate(() => document.getElementById("greeting").textContent);
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
  // (Chromium's own stand-in microphone: a tone, for the microphone button's bar.)
  const browser = await chromium.launch({ args: ["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"] });
  const page = await (await browser.newContext({ viewport: { width: 1600, height: 900 } })).newPage();    // (later visits too)
  const errors = ERRORS;
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.route(/youtube/, (route) => route.fulfill({ status: 200, contentType: "text/html", body: PLAYER }));      // no network needed
  await page.route(/www\.google\.com/, (route) => route.fulfill({ status: 200, contentType: "text/html", body: GOOGLE }));
  // Wikipedia finds nothing here (a question of fact then gets the page's honest note, as with no AI).
  await page.route(/en\.wikipedia\.org/, (route) => route.fulfill({ status: 200, contentType: "application/json",
    headers: { "access-control-allow-origin": "*" }, body: "{}" }));
  // DuckDuckGo's icon service, standing in: a small picture for each service's icon, but none for LinkedIn's (its
  // messages then show Chat Computer's own icon).
  const ICON_PNG = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64");
  const iconsAsked = [];
  await page.route(/ssl\.gstatic\.com/, (route) => route.fulfill({ status: 200, contentType: "image/png", body: ICON_PNG }));
  await page.route(/icons\.duckduckgo\.com/, (route) => {
    const u = route.request().url();
    iconsAsked.push(u.replace("https://icons.duckduckgo.com/ip3/", ""));
    return /linkedin/.test(u) ? route.fulfill({ status: 404, body: "" }) : route.fulfill({ status: 200, contentType: "image/png", body: ICON_PNG });
  });
  await page.goto(page_url);
  // It opens on the word logo, and under it, once it's chosen, the greeting, fading in (at this width, on one line);
  // nothing said, and both stay (three seconds on, still there). The cursor in the box, under the hint.
  await page.waitForFunction(() => window.CO && document.getElementById("toggle-left"));
  out.intro = await state(page);
  out.greeting = await greeted(page);
  out.greeting_options = await page.evaluate(() => window.CO.greetingsNow());
  await page.waitForTimeout(3000);
  out.opening = await state(page);
  out.greeting_line = await page.evaluate(() => {
    const g = document.getElementById("greeting"), w = document.getElementById("wordmark"), r = g.getBoundingClientRect();
    return { below: r.top > w.getBoundingClientRect().bottom, one_line: r.height < 2 * parseFloat(getComputedStyle(g).lineHeight),
             opacity: getComputedStyle(g).opacity, logo: getComputedStyle(document.getElementById("welcome")).opacity };
  });
  // The sidebars' switches: each at the top of its sidebar on the edge nearest the middle, centred on the same line
  // as the sidebar's title (and Messages' +); Messages' connect rows out of sight until + is pressed.
  out.switches = await page.evaluate(() => {
    const box = (e) => e.getBoundingClientRect(), mid = (e) => +((box(e).top + box(e).bottom) / 2).toFixed(1);
    const L = document.getElementById("toggle-left"), R = document.getElementById("toggle-right");
    const M = document.getElementById("messages"), D = document.getElementById("done");
    return { centres: [mid(L), mid(document.querySelector("#messages .side-title")), mid(document.getElementById("add-account")),
                       mid(R), mid(document.getElementById("done-title"))],
             left_gap: Math.round(box(M).right - box(L).right), right_gap: Math.round(box(R).left - box(D).left),
             connect: !!document.getElementById("connect") };
  });
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
  // Messages' +: everything that can be connected, in one panel, by kind, each with its own logo; one chosen, said it
  // can't be here (the app connects it); closed with its x, at its top right (no back arrow: it came from no panel).
  await page.click("#add-account");
  await page.waitForFunction(() => document.body.classList.contains("sheet"));
  const sheetNow = () => page.evaluate(() => ({ title: document.getElementById("sheet-title").textContent,
    items: Array.from(document.querySelectorAll("#sheet-fields .choices > *")).map((e) => e.tagName === "P" ? "~ " + e.textContent :
      e.textContent + (e.classList.contains("on") ? " *" : "") + (e.querySelector("img") ? " @" : "")).filter((t) => t !== "~ "),
    error: document.getElementById("sheet-error").textContent, ok: getComputedStyle(document.getElementById("sheet-buttons")).display,
    back: getComputedStyle(document.getElementById("sheet-back")).display,
    x: (() => { const x = document.getElementById("sheet-cancel").getBoundingClientRect(), f = document.getElementById("sheet-form").getBoundingClientRect();
      return { label: document.getElementById("sheet-cancel").getAttribute("aria-label"), top_right: x.top - f.top < 40 && f.right - x.right < 40,
               text: document.getElementById("sheet-cancel").textContent }; })() }));
  out.connect_panel = await sheetNow();
  await page.click('#sheet-fields .choices button:text-is("Gmail")');
  await page.waitForFunction(() => document.getElementById("sheet-error").textContent !== "");
  out.connect_gmail = await sheetNow();
  await page.click("#sheet-cancel");
  await page.waitForFunction(() => !document.body.classList.contains("sheet"));
  // "/": what can be connected here, then the files.
  out.slash = await slash(page);
  if (shot) await page.screenshot({ path: shot + "-opening.png" });
  // The sidebars show from the start.
  out.sides_in = await state(page);
  // The sidebars' switches: off, and on again (and the choice kept). What's in the middle stays where it was.
  const middle = () => page.evaluate(() => ["column", "box", "wordmark"].map((id) => {
    const r = document.getElementById(id).getBoundingClientRect();
    return [Math.round(r.left), Math.round(r.right)];
  }));
  out.middle_in = await middle();
  await page.click("#toggle-left");
  await page.waitForTimeout(100);
  out.middle_left_off = await middle();
  await page.click("#toggle-right");
  await page.waitForTimeout(200);
  out.middle_off = await middle();
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
  // Escape again: what's open is closed, the conversation back in the middle; the sidebars stay (their switches
  // are for them).
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => !document.body.classList.contains("content"));
  out.escaped = await state(page);
  if (shot) await page.screenshot({ path: shot + "-escaped.png" });
  // With nothing open, Escape leaves the sidebars as they are.
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  out.escaped_again = await state(page);
  await page.click("#input");
  await page.keyboard.type("/pack", { delay: 40 });
  await page.waitForSelector("#results .res");
  out.search = await state(page);
  if (shot) await page.screenshot({ path: shot + "-search.png" });
  await page.keyboard.press("Enter");
  await page.waitForTimeout(400);
  out.opened = await state(page);
  out.opened_view = await viewText(page);
  // Its x: closed, the conversation back.
  await page.click("#frame-close");
  await page.waitForFunction(() => !document.body.classList.contains("content"));
  out.closed = await state(page);
  // A message: its whole conversation, in the middle, the conversation with Chat Computer put away.
  // Each message with its service's own icon (from DuckDuckGo's icon service, standing in), or Chat Computer's own
  // where there's none (LinkedIn's, here).
  await page.waitForFunction(() => Array.from(document.querySelectorAll("#messages .msg-top")).every((t) =>
    t.querySelector("svg") || (t.querySelector("img") && t.querySelector("img").complete)), null, { timeout: 10000 });
  await page.waitForTimeout(300);
  out.msg_icons = await page.evaluate(() => Array.from(document.querySelectorAll("#messages .msg")).map((m) => {
    const img = m.querySelector(".msg-top img");
    return [m.querySelector(".msg-src").textContent.replace(/: $/, ""), img ? img.getAttribute("src").replace("https://icons.duckduckgo.com/ip3/", "") :
      m.querySelector(".msg-top svg") ? "own" : "none", img ? img.referrerPolicy : ""];
  }));
  out.icons_asked = Array.from(new Set(iconsAsked)).sort();
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

  // This computer's model, where the Chat Computer app runs here: a stand-in for its engine at 127.0.0.1:8765. "/"
  // lists Use this computer's model: chosen while the app isn't running, its sheet says so; with it running, the
  // model answers, using the page's tools (a map; a YouTube video, with a stand-in for YouTube's player, which closes
  // when it ends; the volume); on a later visit it's used again by itself; when the app stops, the page says so and
  // the written answers are back; Stop using it.
  const engine = { up: false, asked: [], health: 0 };
  await page.context().route("http://127.0.0.1:8765/**", (route) => {
    if (!engine.up) return route.abort("connectionrefused");
    if (new URL(route.request().url()).pathname === "/health") {
      engine.health++;
      return route.fulfill({ status: 200, contentType: "application/json", body: HEALTH });
    }
    const a = asked(route), last = a.messages[a.messages.length - 1] || {};
    engine.asked.push(a);
    const said = String((a.messages.filter((m) => m.role === "user").pop() || {}).content || "");
    const turn = last.role === "tool" ? textTurn(last.tool_call_id === "call_1" ? ["Here's ", "Tokyo."] : last.tool_call_id === "call_2" ?
      ["Playing ", "it."] : ["Quieter ", "now."]) : /quieter/.test(said) ? VOLUME_TURN : /video/.test(said) ? VIDEO_TURN :
      /Tokyo/.test(said) ? MAP_TURN : textTurn(["Hello ", "from ", "Qwen."]);
    return route.fulfill({ status: 200, contentType: "text/event-stream", body: turn });
  });
  await slash(page, "Use this computer");
  out.local_sheet = await state(page);
  out.local_sheet_text = await page.evaluate(() => document.getElementById("sheet-text").innerText);
  await page.click("#sheet-ok");
  await page.waitForFunction(() => document.getElementById("sheet-error").textContent !== "");
  out.local_refused = await page.evaluate(() => ({ sheet: document.body.classList.contains("sheet"),
    error: document.getElementById("sheet-error").textContent }));
  engine.up = true;
  await page.click("#sheet-ok");
  await page.waitForFunction(() => !document.body.classList.contains("sheet"));
  out.local_on = await state(page);
  out.local_on_slash = await slash(page);
  out.local_stored = await page.evaluate(() => localStorage.getItem("chatcomputer-local-model"));
  await ask(page, "show me Tokyo");
  out.local_map = await state(page);
  if (shot) await page.screenshot({ path: shot + "-local.png" });
  // A video, played by the model: YouTube's player (a stand-in) between the sidebars; when it ends, the conversation.
  await ask(page, "play a video");
  out.local_video = await state(page);
  await page.waitForTimeout(300);
  // Turned down by the model (set_volume): YouTube's player is told, and the video stays.
  await ask(page, "a bit quieter");
  out.local_quieter = { ...(await state(page)), player: await player(page).evaluate(() => window.commands) };
  await player(page).evaluate(() => window.end());
  await page.waitForFunction(() => !document.body.classList.contains("content"));
  out.video_ended = await state(page);
  out.asked = engine.asked.splice(0);
  // A later visit: used again by itself (the app still running).
  const later = await page.context().newPage();
  later.on("pageerror", (e) => errors.push(String(e)));
  const probes = engine.health;
  await later.goto(page_url);
  await greeted(later);
  for (let t = Date.now(); engine.health === probes; ) {
    if (Date.now() - t > 8000) throw new Error("the later visit didn't look for the model");
    await new Promise((ok) => setTimeout(ok, 100));
  }
  await later.waitForTimeout(200);
  out.local_later_slash = await slash(later);
  out.local_later = await ask(later, "hello there");
  out.local_later_asked = engine.asked.splice(0).map((a) => a.messages.slice(1).map((m) => [m.role, m.content]));
  // The app stops: said so; the written answers are back, and "/" offers it again.
  engine.up = false;
  out.local_gone = await ask(later, "what's new?");
  out.local_gone_slash = await slash(later);
  out.local_gone_after = await ask(later, "hello");
  // Back, and then Stop using it: Actions says so, and the browser forgets it.
  engine.up = true;
  await slash(later, "Use this computer");
  await later.click("#sheet-ok");
  await later.waitForFunction(() => !document.body.classList.contains("sheet"));
  await slash(later, "Stop using");
  out.local_stopped = { done: (await state(later)).done, slash: await slash(later),
    stored: await later.evaluate(() => localStorage.getItem("chatcomputer-local-model")) };
  await later.close();
  // Connect AirPods ("/"): what it does, before the microphone is asked for.
  await slash(page, "Connect AirPods");
  out.talk_sheet = await state(page);
  await page.keyboard.press("Escape");

  // The microphone button: listening at once (no sheet), with Chromium's own stand-in microphone (a real stream, its
  // tone heard) and a stand-in for the browser's listening; the button's one bar rising and falling with what's
  // heard; pressed again, stopped, the microphone let go of.
  const micCtx = await browser.newContext({ viewport: { width: 1280, height: 800 }, permissions: ["microphone"] });
  await micCtx.addInitScript(() => {
    window.recs = [];
    function Rec() { window.recs.push(this); }
    Rec.prototype.start = function () { this.started = true; };
    Rec.prototype.stop = Rec.prototype.abort = function () { this.ended = true; const r = this; setTimeout(() => r.onend && r.onend(), 5); };
    window.SpeechRecognition = window.webkitSpeechRecognition = Rec;
    const real = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
    window.streams = [];
    navigator.mediaDevices.getUserMedia = (c) => real(c).then((s) => { window.streams.push(s); return s; });
  });
  const mic = await micCtx.newPage();
  mic.on("pageerror", (e) => errors.push(String(e)));
  await mic.goto(page_url);
  await mic.waitForFunction(() => window.CO && document.getElementById("toggle-left"));
  await mic.click("#mic");
  await mic.waitForFunction(() => window.recs.length === 1 && window.recs[0].started, null, { timeout: 10000 });
  const levels = [];
  for (let i = 0; i < 25; i++) {
    levels.push(await mic.evaluate(() => +getComputedStyle(document.getElementById("mic")).getPropertyValue("--level") || 0));
    await mic.waitForTimeout(80);
  }
  out.mic_now = await mic.evaluate(() => ({ sheet: document.body.classList.contains("sheet"),
    pressed: document.getElementById("mic").getAttribute("aria-pressed"), title: document.getElementById("mic").title,
    bar: getComputedStyle(document.querySelector("#mic .level")).display, icon: getComputedStyle(document.querySelector("#mic svg")).display,
    live: window.streams.length && window.streams[0].getAudioTracks()[0].readyState }));
  out.mic_now.low = Math.min(...levels);
  out.mic_now.high = Math.max(...levels);
  await mic.click("#mic");
  await mic.waitForTimeout(200);
  out.mic_stopped = await mic.evaluate(() => ({ pressed: document.getElementById("mic").getAttribute("aria-pressed"),
    bar: getComputedStyle(document.querySelector("#mic .level")).display,
    live: window.streams[0].getAudioTracks()[0].readyState, aborted: !!window.recs[0].ended }));
  await micCtx.close();

  // In the app (a stand-in for it: Tauri's invoke, and what the app tells the page), the model comes with it: while
  // it gets ready, the empty box says how it's going, and so do "/" (its sheet) and an answer it can't give yet; then
  // it answers, on the port the app says, with the app's own tools (the files on this computer).
  const appCtx = await browser.newContext({ viewport: { width: 1600, height: 900 } });
  const appEngine = { asked: [] };
  await appCtx.route("http://127.0.0.1:8766/**", (route) => {
    if (new URL(route.request().url()).pathname === "/health") return route.fulfill({ status: 200, contentType: "application/json", body: HEALTH });
    const a = asked(route), last = a.messages[a.messages.length - 1] || {};
    appEngine.asked.push(a);
    return route.fulfill({ status: 200, contentType: "text/event-stream", body: last.role === "tool" ?
      textTurn(["It's ", "in Documents."]) : toolTurn("call_9", "find_files", '{"query": "tax"}') });
  });
  await appCtx.route(/en\.wikipedia\.org/, (route) => route.fulfill({ status: 200, contentType: "application/json",
    headers: { "access-control-allow-origin": "*" }, body: "{}" }));
  await appCtx.addInitScript(() => {
    window.__app = { calls: [] };
    window.__TAURI__ = { core: { invoke: (command, args) => {
      window.__app.calls.push([command, args || {}]);
      if (command === "about") return Promise.resolve({ name: "Sam Lee" });
      if (command === "model_status") return Promise.resolve({ state: "preparing", label: "", preparing: "Qwen3.5 9B",
        stage: "downloading", done: 1.2e9, total: 9e9, port: 8765, detail: "" });
      if (command === "search_files") return Promise.resolve([{ name: "Taxes 2025.pdf", where: "Documents", kind: "pdf",
        ts: 1760000000, path: "/home/sam/Documents/Taxes 2025.pdf" }]);
      return Promise.resolve(null);
    } } };
  });
  const app = await appCtx.newPage();
  app.on("pageerror", (e) => errors.push(String(e)));
  await app.goto(page_url);
  await greeted(app);
  await app.waitForFunction(() => /Getting my model ready/.test(document.getElementById("input").placeholder));
  out.app_preparing = await state(app);
  out.app_slash = await slash(app);
  await slash(app, "Chat Computer's model");
  out.app_sheet = await app.evaluate(() => ({ title: document.getElementById("sheet-title").textContent,
    text: document.getElementById("sheet-text").innerText, ok: document.getElementById("sheet-ok").textContent }));
  await app.click("#sheet-ok");
  await app.waitForFunction(() => !document.body.classList.contains("sheet"));
  out.app_before = await ask(app, "write a haiku about coffee");
  await app.evaluate(() => window.chatcomputerApp({ model: { state: "preparing", label: "", preparing: "Qwen3.5 9B",
    stage: "uncensoring", done: 30, total: 120, port: 8765, detail: "" } }));
  out.app_uncensoring = (await state(app)).hint;
  await app.evaluate(() => window.chatcomputerApp({ model: { state: "ready", label: "Qwen3.5 9B", preparing: "",
    stage: "", done: 0, total: 0, port: 8766, detail: "" } }));
  out.app_ready = await state(app);
  out.app_ready_slash = await slash(app);
  out.app_answer = await ask(app, "find my tax return");
  out.app_asked = appEngine.asked;
  out.app_calls = await app.evaluate(() => window.__app.calls);
  out.app_done = (await state(app)).done;
  await appCtx.close();

  // Your ChatGPT plan, in the app (a stand-in for it: its sign-in, which waits for OpenAI's page, and its asking OpenAI,
  // streamed through a channel as the Responses API's events): "Continue with ChatGPT" in the spark's list; signed in,
  // the plan's own notice the first time; then an answer that calls one of the page's tools and, given its result,
  // says words; the request as the Responses API takes it (instructions, the conversation, the page's tools as
  // functions, nothing kept by OpenAI); its plan's list (Manage usage first); signed out, the app told.
  const gptCtx = await browser.newContext({ viewport: { width: 1600, height: 900 } });
  await gptCtx.route(/en\.wikipedia\.org/, (route) => route.fulfill({ status: 200, contentType: "application/json",
    headers: { "access-control-allow-origin": "*" }, body: "{}" }));
  await gptCtx.route(/icons\.duckduckgo\.com|ssl\.gstatic\.com/, (route) => route.fulfill({ status: 404, body: "" }));
  await gptCtx.addInitScript(() => {
    window.__gpt = { calls: [], asked: [] };
    const sse = (events) => events.map((e) => "event: " + e.type + "\ndata: " + JSON.stringify(e) + "\n\n").join("");
    function Channel() { window.__gpt.channel = this; }
    window.__TAURI__ = { core: { Channel, invoke: (command, args) => {
      window.__gpt.calls.push(command);
      if (command === "about") return Promise.resolve({ name: "" });
      if (command === "model_status") return Promise.resolve({ state: "none", label: "", preparing: "", stage: "", done: 0, total: 0, port: 8765, detail: "x" });
      if (command === "chatgpt_sign_in") return new Promise((r) => setTimeout(() => r({ signed_in: true, email: "sam@example.com" }), 200));
      if (command === "chatgpt_status") return Promise.resolve({ signed_in: true, email: "sam@example.com" });
      if (command === "chatgpt_sign_out") return Promise.resolve({ signed_in: false, email: null });
      if (command === "chatgpt_ask") {
        const send = (t) => args.events.onmessage(t);
        if (args.path === "models") {
          send(JSON.stringify({ models: [{ slug: "gpt-hidden", visibility: "hide" }, { slug: "gpt-9-plan", display_name: "GPT-9", visibility: "list",
            supported_reasoning_levels: [{ effort: "low" }, { effort: "medium" }, { effort: "high" }], default_reasoning_level: "medium" }] }));
          return Promise.resolve(200);
        }
        const body = JSON.parse(args.body), last = body.input[body.input.length - 1];
        window.__gpt.asked.push(body);
        const text = last.type === "function_call_output" ?
          sse([{ type: "response.output_text.delta", output_index: 0, delta: "A 20-minute " }, { type: "response.output_text.delta", output_index: 0, delta: "timer is on." },
               { type: "response.completed", response: {} }]) :
          sse([{ type: "response.output_item.added", output_index: 0, item: { type: "function_call", id: "fc_1", call_id: "call_g1", name: "make_timer", arguments: "" } },
               { type: "response.function_call_arguments.delta", output_index: 0, delta: "{\"minu" },
               { type: "response.function_call_arguments.delta", output_index: 0, delta: "tes\": 20}" },
               { type: "response.output_item.done", output_index: 0, item: { type: "function_call", id: "fc_1", call_id: "call_g1", name: "make_timer", arguments: "{\"minutes\": 20}" } },
               { type: "response.completed", response: {} }]);
        // (In pieces, one cut inside an event, as a stream comes.)
        send(text.slice(0, 37)); send(text.slice(37, 120)); send(text.slice(120));
        return Promise.resolve(200);
      }
      return Promise.resolve(null);
    } } };
  });
  const gpt = await gptCtx.newPage();
  gpt.on("pageerror", (e) => errors.push(String(e)));
  await gpt.goto(page_url);
  await gpt.waitForFunction(() => window.CO && document.getElementById("toggle-left"));
  await gpt.click("#ai");
  await gpt.waitForFunction(() => document.body.classList.contains("sheet"));
  out.gpt_menu = await gpt.evaluate(() => Array.from(document.querySelectorAll("#sheet-fields .choices button")).map((b) => b.textContent));
  await gpt.click("#sheet-fields .choices button:has-text('Continue with ChatGPT')");
  await gpt.waitForFunction(() => document.getElementById("sheet-title").textContent === "You're using your ChatGPT plan", null, { timeout: 10000 });
  out.gpt_told = await gpt.evaluate(() => ({ text: document.getElementById("sheet-text").innerText, ok: document.getElementById("sheet-ok").textContent,
    spark: document.getElementById("ai").title, kept: JSON.parse(localStorage.getItem("chatcomputer-own-ai")) }));
  await gpt.click("#sheet-ok");
  await gpt.waitForFunction(() => !document.body.classList.contains("sheet"));
  await gpt.fill("#input", "a timer for 20 minutes");
  await gpt.keyboard.press("Enter");
  await gpt.waitForFunction(() => /A 20-minute timer is on\./.test((Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).textContent || ""),
    null, { timeout: 10000 });
  out.gpt_asked = await gpt.evaluate(() => window.__gpt.asked.map((b) => ({ model: b.model, store: b.store, stream: b.stream,
    told: b.instructions.slice(0, 160), input: b.input.map((i) => i.type || i.role), tools: b.tools.slice(0, 2).map((t) => [t.type, t.name, !!t.parameters]),
    last: b.input[b.input.length - 1] })));
  out.gpt_done = (await state(gpt)).done;
  await gpt.click("#ai");
  await gpt.waitForFunction(() => document.body.classList.contains("sheet"));
  out.gpt_menu_in = await gpt.evaluate(() => ({ choices: Array.from(document.querySelectorAll("#sheet-fields .choices button")).map((b) => b.textContent),
    text: document.getElementById("sheet-text").innerText }));
  await gpt.click("#sheet-fields .choices button:has-text('Sign out of ChatGPT')");
  await gpt.waitForFunction(() => !document.getElementById("ai").classList.contains("on"));
  out.gpt_calls = await gpt.evaluate(() => window.__gpt.calls.filter((c) => c.startsWith("chatgpt_")));
  await gptCtx.close();

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
  const VDATA = 20 * 1024 * 1024, vranges = [], vhub = { hold: false, held: null, fail: 0 };
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
      if (from > 0 && vhub.fail > 0) { vhub.fail--; return route.abort("connectionreset"); }   // cut off once
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
    await page.waitForFunction(() => window.CO);
    await page.click("#input");
    await ask(page, "my dad struggles with computers, would this help?");     // (an answer of a few sentences)
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
  // Chosen: downloading (its progress shown), when the page is left, the second piece never having come; on the next
  // visit, chosen again, only that piece is asked for (cut off once, and asked for again); then the answer is read
  // in Heart.
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
  out.voice_parts_kept = await parts(v2);
  out.voice_ranges_first = vranges.splice(0);
  await v2.close();
  vhub.hold = false;
  vhub.held = null;
  vhub.fail = 1;
  const v2b = await answered(vc2);
  await speakLast(v2b);
  await v2b.waitForFunction(() => document.body.classList.contains("sheet"));
  await v2b.click("#sheet-ok");
  await v2b.waitForFunction(() => window.__heart.said.length >= 1, null, { timeout: 20000 });
  out.voice_heart = await v2b.evaluate(() => ({ loads: window.__heart.loads, said: window.__heart.said.slice(), spoken: window.__spoken.length,
    answer: Array.from(document.querySelectorAll("#log > .ai")).pop().textContent, done: document.getElementById("done").textContent,
    hint: document.getElementById("input").placeholder }));
  out.voice_ranges = vranges.splice(0);
  out.voice_parts_after = await parts(v2b);
  out.voice_on_slash = await slash(v2b);
  await v2b.close();
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
  out.rows_hello = await rows.evaluate(() => ({ first: (document.querySelector("#log > *") || {}).className, logo: document.body.classList.contains("has-log") }));
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
  // The greeting whole on a phone, however long: on a second line where it needs one, centred, never cut off.
  out.phone_greeting = await phone.evaluate(() => {
    const g = document.getElementById("greeting"), was = g.textContent, rows = [];
    for (const t of ["Hello, cloudy Singapore! How's your morning?", "Good morning"]) {
      g.textContent = t;
      const r = g.getBoundingClientRect();
      rows.push({ lines: Math.round(r.height / parseFloat(getComputedStyle(g).lineHeight)), whole: g.scrollWidth <= g.clientWidth + 1,
                  inside: r.left >= 0 && r.right <= innerWidth, centred: getComputedStyle(g).textAlign === "center" });
    }
    g.textContent = was;
    return rows;
  });
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
    // (The conversation pulled down past its top, as Chrome on an iPhone lets it be to reload: what's on the screen
    // then starts above the page's top. While typing, the box stays just above the keyboard; not typing, at the
    // bottom: nothing lifts it.)
    await put(H - 300, -120, "scroll");
    const pulled_typing = { box: box(), seen: H - 300 };
    await put(H, 0, "resize");
    const down = { box: box(), seen: H, room: room() - before, keyboard: document.body.classList.contains("keyboard") };
    document.activeElement.blur();
    await put(H, -120, "scroll");
    const pulled = { box: box(), seen: H, keyboard: document.body.classList.contains("keyboard") };
    await put(H, 0, "scroll");
    delete vv.height; delete vv.offsetTop;
    return { up, scrolled, pulled_typing, down, pulled };
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

  // Your own AI (the spark beside +): signing in with OpenRouter (its pages standing in: its sign-in, which comes back
  // with a code; the code traded for a key; its models, the newest free one that uses tools chosen), then its answers,
  // on the visitor's key, with the page's tools and none of the engine's own options; signed out, gone. And a key
  // of the visitor's own (OpenAI's, standing in), checked against its models.
  const ownCtx = await browser.newContext({ viewport: { width: 1600, height: 900 } });
  const orSeen = { auth: null, traded: null, asked: [] };
  await ownCtx.route(/icons\.duckduckgo\.com/, (route) => route.fulfill({ status: 404, body: "" }));
  await ownCtx.route("https://openrouter.ai/auth**", (route) => {
    orSeen.auth = Object.fromEntries(new URL(route.request().url()).searchParams);
    return route.fulfill({ status: 200, contentType: "text/html", body: "<title>OpenRouter</title>Sign in" });
  });
  await ownCtx.route("https://openrouter.ai/api/v1/auth/keys", (route) => {
    orSeen.traded = JSON.parse(route.request().postData() || "{}");
    return route.fulfill({ status: 200, contentType: "application/json", headers: { "access-control-allow-origin": "*" },
                           body: JSON.stringify({ key: "test-openrouter-key" }) });
  });
  await ownCtx.route("https://openrouter.ai/api/v1/models", (route) => route.fulfill({ status: 200, contentType: "application/json",
    headers: { "access-control-allow-origin": "*" }, body: JSON.stringify({ data: [
      { id: "old/free-model:free", created: 100, pricing: { prompt: "0", completion: "0" }, supported_parameters: ["tools"] },
      { id: "new/paid-model", created: 300, pricing: { prompt: "0.000001", completion: "0.000002" }, supported_parameters: ["tools"] },
      { id: "new/free-model:free", created: 200, pricing: { prompt: "0", completion: "0" }, supported_parameters: ["tools", "temperature"] },
      { id: "newest/free-no-tools:free", created: 400, pricing: { prompt: "0", completion: "0" }, supported_parameters: ["temperature"] }] }) }));
  await ownCtx.route("https://openrouter.ai/api/v1/chat/completions", (route) => {
    const req = route.request(), body = JSON.parse(req.postData() || "{}");
    orSeen.asked.push({ auth: req.headers()["authorization"], title: req.headers()["x-title"], model: body.model, stream: body.stream,
                        think: "think" in body, max: "max_tokens" in body, tools: (body.tools || []).length,
                        system: String((body.messages[0] || {}).content || "").slice(0, 160), last: body.messages[body.messages.length - 1] });
    return route.fulfill({ status: 200, contentType: "text/event-stream", headers: { "access-control-allow-origin": "*" },
                           body: textTurn(["Hello from ", "OpenRouter."]) });
  });
  const ownPage = await ownCtx.newPage();
  ownPage.on("pageerror", (e) => errors.push(String(e)));
  await ownPage.goto(page_url);
  await ownPage.waitForFunction(() => window.CO && document.getElementById("toggle-left"));
  await ownPage.click("#ai");
  await ownPage.waitForFunction(() => document.body.classList.contains("sheet"));
  const sheetOf = (pg) => pg.evaluate(() => ({ title: document.getElementById("sheet-title").textContent,
    choices: Array.from(document.querySelectorAll("#sheet-fields .choices button")).map((b) => b.textContent) }));
  out.own_menu = await sheetOf(ownPage);
  await ownPage.click("#sheet-fields .choices button:has-text('Continue with OpenRouter')");
  await ownPage.waitForURL(/openrouter\.ai\/auth/);
  await ownPage.goto(page_url + "?code=test-code");                  // (OpenRouter sends the visitor back with a code)
  await ownPage.waitForFunction(() => document.getElementById("ai").classList.contains("on"), null, { timeout: 10000 });
  out.own_in = await ownPage.evaluate(() => ({ title: document.getElementById("ai").title, search: location.search,
    kept: JSON.parse(localStorage.getItem("chatcomputer-own-ai")) }));
  out.own_auth = orSeen.auth && { back: orSeen.auth.callback_url, method: orSeen.auth.code_challenge_method,
                                  challenge: (orSeen.auth.code_challenge || "").length };
  out.own_traded = orSeen.traded && { code: orSeen.traded.code, method: orSeen.traded.code_challenge_method,
                                      verifier: (orSeen.traded.code_verifier || "").length };
  await ownPage.fill("#input", "hi there");
  await ownPage.keyboard.press("Enter");
  await ownPage.waitForFunction(() => /Hello from OpenRouter\./.test((Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).textContent || ""),
    null, { timeout: 10000 });
  out.own_asked = orSeen.asked;
  await ownPage.click("#ai");
  await ownPage.waitForFunction(() => document.body.classList.contains("sheet"));
  out.own_menu_in = await sheetOf(ownPage);
  await ownPage.click("#sheet-fields .choices button:has-text('Sign out of OpenRouter')");
  await ownPage.waitForFunction(() => !document.getElementById("ai").classList.contains("on"));
  out.own_out = await ownPage.evaluate(() => ({ title: document.getElementById("ai").title, kept: localStorage.getItem("chatcomputer-own-ai"),
    done: Array.from(document.querySelectorAll("#done-list .done-what")).map((d) => d.textContent).filter((t) => /OpenRouter/.test(t)) }));
  // A key of one's own: OpenAI's (standing in), its chat models listed, the newest chosen; an empty key, said so.
  await ownCtx.route("https://api.openai.com/v1/models", (route) => route.fulfill({ status: 200, contentType: "application/json",
    headers: { "access-control-allow-origin": "*" }, body: JSON.stringify({ data: [
      { id: "whisper-1", created: 900 }, { id: "gpt-4.1", created: 100 }, { id: "gpt-9", created: 500 }, { id: "gpt-9-realtime", created: 600 }] }) }));
  await ownPage.click("#ai");
  await ownPage.click('#sheet-fields .choices button:text-is("OpenAI")');
  await ownPage.waitForSelector("#sheet-fields input[name=key]");
  await ownPage.click("#sheet-ok");
  await ownPage.waitForFunction(() => document.getElementById("sheet-error").textContent !== "");
  out.own_empty = await ownPage.evaluate(() => document.getElementById("sheet-error").textContent);
  await ownPage.fill("#sheet-fields input[name=key]", "test-openai-key");
  await ownPage.click("#sheet-ok");
  await ownPage.waitForFunction(() => document.getElementById("ai").classList.contains("on"), null, { timeout: 10000 });
  out.own_key = await ownPage.evaluate(() => ({ title: document.getElementById("ai").title, sheet: document.body.classList.contains("sheet"),
    type: (document.querySelector("#sheet-fields input[name=key]") || {}).type }));
  // OpenRouter's models to choose from: its free ones, then the best of each maker in the panel (on your credits).
  await ownCtx.unroute("https://openrouter.ai/api/v1/models");
  await ownCtx.route("https://openrouter.ai/api/v1/models", (route) => route.fulfill({ status: 200, contentType: "application/json",
    headers: { "access-control-allow-origin": "*" }, body: JSON.stringify({ data: OPENROUTER_MODELS }) }));
  await ownPage.evaluate(() => { localStorage.setItem("chatcomputer-own-ai", JSON.stringify({ provider: "openrouter", key: "k", model: "a/free:free" })); });
  await ownPage.reload();
  await ownPage.waitForFunction(() => window.CO && document.getElementById("ai").classList.contains("on"));
  const PNG = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==", "base64");
  await ownCtx.route(/icons\.duckduckgo\.com/, (route) => route.fulfill({ status: 200, contentType: "image/png", body: PNG }));
  await ownPage.click("#ai");
  await ownPage.waitForFunction(() => /The best of each maker/.test(document.getElementById("sheet-fields").textContent));
  out.or_models = await ownPage.evaluate(() => Array.from(document.querySelectorAll("#sheet-fields .choices > *")).map((e) =>
    e.tagName === "P" ? "~ " + e.textContent : e.textContent + (e.classList.contains("on") ? " *" : "") +
    (e.querySelector("img") ? " @" + new URL(e.querySelector("img").src).pathname.slice(5) : "")).filter((t) => t !== "~ "));
  await ownCtx.close();

  // Chat Computer's model in this browser (docs/demo/engine's API, standing in): offered with / and when an answer
  // needs a model; chosen, its download said as it goes, its few lines of instructions (the same every visit, with no
  // tools) read ahead while it gets ready; then the page answers what it can itself (a timer: no model asked), and the
  // model the rest, knowing what was said before (the timer's question and answer), with no time stamp; a later visit
  // starts it from the browser's copy without asking or downloading; stopped; removed (the browser's copy deleted).
  const wCtx = await browser.newContext({ viewport: { width: 1600, height: 900 } });
  await wCtx.route(/icons\.duckduckgo\.com/, (route) => route.fulfill({ status: 404, body: "" }));
  await wCtx.addInitScript(() => {
    let kept = !!localStorage.getItem("fake-engine-kept");
    const log = window.__engine = [];
    const fake = {
      supported: () => ({ ok: true, why: "" }),
      status: () => Promise.resolve({ name: "qwen3.5-0.8b", label: "Qwen3.5 0.8B", download: 1759777953, kept: kept, loaded: false }),
      prepare: (on) => {
        log.push(kept ? "load" : "download");
        if (!kept) on({ stage: "downloading", done: 880000000, total: 1759777953 });
        return new Promise((r) => setTimeout(() => {
          on({ stage: "loading" });
          kept = true;
          localStorage.setItem("fake-engine-kept", "1");
          setTimeout(() => r({ label: "Qwen3.5 0.8B", layers: 24 }), 200);
        }, kept ? 50 : 600));
      },
      ahead: (req, onReading) => { log.push({ ahead: req }); onReading(40, 100); return new Promise((r) => setTimeout(r, 300)); },
      chat: (req, onChunk) => {
        log.push({ chat: req });
        onChunk({ content: "Five minutes, " }); onChunk({ content: "ticking softly." });
        return Promise.resolve({ role: "assistant", content: "Five minutes, ticking softly." });
      },
      stop: () => {},
      forget: () => { log.push("forget"); kept = false; localStorage.removeItem("fake-engine-kept"); return Promise.resolve(); },
    };
    // (engine.js, loaded after, sets its own: this one stays.)
    Object.defineProperty(window, "ChatComputerEngine", { get: () => fake, set: () => {}, configurable: false });
  });
  const wp = await wCtx.newPage();
  wp.on("pageerror", (e) => errors.push(String(e)));
  await wp.goto(page_url);
  await wp.waitForFunction(() => window.CO && document.getElementById("toggle-left"));
  out.web_slash = await slash(wp);
  await wp.fill("#input", "write a poem about the sea");
  await wp.keyboard.press("Enter");
  await wp.waitForFunction(() => { const a = Array.from(document.querySelectorAll("#log > .ai")).pop(); return a && a.querySelector(".offers button"); },
    null, { timeout: 10000 });
  out.web_offer = await wp.evaluate(() => { const a = Array.from(document.querySelectorAll("#log > .ai")).pop();
    return { text: a.querySelector(".md").textContent, buttons: Array.from(a.querySelectorAll(".offers button")).map((b) => b.textContent +
      (b.classList.contains("main") ? " *" : "")) }; });
  await slash(wp, "Run my model in this browser");
  await wp.waitForFunction(() => document.body.classList.contains("sheet"));
  out.web_form = await wp.evaluate(() => ({ title: document.getElementById("sheet-title").textContent, ok: document.getElementById("sheet-ok").textContent,
    text: document.getElementById("sheet-text").innerText }));
  await wp.click("#sheet-ok");
  await wp.waitForFunction(() => /downloading/.test(document.getElementById("input").placeholder));
  out.web_getting = await wp.evaluate(() => document.getElementById("input").placeholder);
  out.web_getting_slash = await slash(wp);
  await wp.waitForFunction(() => /Using my model in this browser/.test(document.getElementById("done-list").textContent), null, { timeout: 10000 });
  // What was asked while it wasn't here (the poem), answered once it is, under it; the buttons gone.
  await wp.waitForFunction(() => window.__engine.filter((e) => e.chat).length === 1 &&
    /ticking softly\./.test((Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).textContent || ""), null, { timeout: 10000 });
  out.web_waited = await wp.evaluate(() => ({ asked: window.__engine.filter((e) => e.chat).map((e) => e.chat.messages[e.chat.messages.length - 1].content),
    offers: document.querySelectorAll("#log .offers").length }));
  const chatsBefore = await wp.evaluate(() => window.__engine.filter((e) => e.chat).length);
  await wp.fill("#input", "a five minute timer");
  await wp.keyboard.press("Enter");
  await wp.waitForFunction(() => /A 5-minute timer/.test((Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).textContent || ""),
    null, { timeout: 10000 });
  out.web_timer_asked = await wp.evaluate((n) => window.__engine.filter((e) => e.chat).length - n, chatsBefore);
  await wp.fill("#input", "write a short poem about the timer");
  await wp.keyboard.press("Enter");
  await wp.waitForFunction(() => /ticking softly\./.test((Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).textContent || ""),
    null, { timeout: 10000 });
  out.web_asked = await wp.evaluate(() => window.__engine.filter((e) => e.chat || e.ahead).map((e) => {
    const r = e.chat || e.ahead;
    return { kind: e.chat ? "chat" : "ahead", think: r.think, max: r.max_tokens, tools: (r.tools || []).map((t) => t.function.name),
             system: r.messages[0].content.slice(0, 120), roles: r.messages.map((m) => m.role),
             said: r.messages.slice(1).map((m) => String(m.content || "").slice(0, 40)), last: String(r.messages[r.messages.length - 1].content || "").slice(-60) };
  }));
  out.web_on_slash = await slash(wp);
  out.web_log = await wp.evaluate(() => window.__engine.filter((e) => typeof e === "string"));
  // A later visit: started from the browser's copy, at once, nothing downloaded. (A page at a file:// address, as this
  // one is, sometimes comes back from a reload with its storage empty, as Chromium keeps file pages' now and then,
  // the same before and after the page's changes: what the page kept is put back, and it's opened again.)
  const keptBefore = await wp.evaluate(() => JSON.stringify(Object.assign({}, localStorage)));
  await wp.reload();
  if (await wp.evaluate(() => !localStorage.getItem("chatcomputer-web-model"))) {
    await wp.evaluate((k) => { const o = JSON.parse(k); for (const x in o) localStorage.setItem(x, o[x]); }, keptBefore);
    await wp.reload();
  }
  await wp.waitForFunction(() => window.CO && (window.__engine || []).some((e) => e.ahead), null, { timeout: 20000, polling: 100 }).catch(async (e) => {
    throw new Error(e.message + " " + JSON.stringify(await wp.evaluate(() => ({ log: (window.__engine || []).map((x) => typeof x === "string" ? x : Object.keys(x)[0]),
      ls: Object.fromEntries(Object.keys(localStorage).filter((k) => /web|fake|own|local/.test(k)).map((k) => [k, localStorage.getItem(k).slice(0, 80)])),
      placeholder: document.getElementById("input").placeholder }))));
  });
  await wp.waitForFunction(() => document.getElementById("input").placeholder === "Type anything", null, { timeout: 20000, polling: 100 });
  out.web_later = await wp.evaluate(() => ({ log: window.__engine.filter((e) => typeof e === "string"),
    done: Array.from(document.querySelectorAll("#done-list .done-what")).map((d) => d.textContent).filter((t) => /browser/.test(t)) }));
  out.web_later_slash = await slash(wp);
  await slash(wp, "Stop using my model in this browser");
  out.web_stopped = await slash(wp);
  await slash(wp, "Remove my model from this browser");
  await wp.waitForFunction(() => window.__engine.indexOf("forget") >= 0);
  await wp.waitForTimeout(200);
  out.web_removed = { slash: await slash(wp), kept: await wp.evaluate(() => [localStorage.getItem("chatcomputer-web-kept"), localStorage.getItem("chatcomputer-web-model")]) };
  await wCtx.close();

  // Every AI in the panel, on a key (each at its own address, standing in): its models listed with the key, the best
  // chat model chosen (where there's no list, its best by name; a key from its other site, at that site's address);
  // asked there, streamed, with the page's tools; its call of make_timer run, and asked again with the timer's
  // answer (its thinking sent back only to an AI that wants it; Gemini's signature on the call, only to Gemini;
  // Mistral's thinking as Mistral gave it); its words shown (any <think> in them taken out); signed out. A key one
  // refuses (Google's way: a 400), said so. And the page that makes a key, in a new tab.
  const kCtx = await browser.newContext({ viewport: { width: 1600, height: 900 } });
  await kCtx.route(/icons\.duckduckgo\.com/, (route) => route.fulfill({ status: 404, body: "" }));
  await kCtx.route("https://platform.claude.com/**", (route) => route.fulfill({ status: 200, contentType: "text/html", body: "<title>Keys</title>" }));
  const kSeen = {}, CORS = { "access-control-allow-origin": "*" };
  for (const [name, k] of Object.entries(KEYED_AIS)) {
    kSeen[name] = { models: [], asked: [] };
    const answers = k.alt || k.base;
    for (const base of [k.base].concat(k.alt ? [k.alt] : [])) {
      await kCtx.route(base + "/models**", (route) => {
        const h = route.request().headers();
        kSeen[name].models.push({ base: base, key: h["authorization"] });
        if (/bad-key/.test(h["authorization"] || "")) return route.fulfill({ status: 400, contentType: "application/json", headers: CORS,
          body: JSON.stringify([{ error: { code: 400, message: "API key not valid. Please pass a valid API key.", status: "INVALID_ARGUMENT" } }][0]) });
        if (base !== answers) return route.fulfill({ status: 401, contentType: "application/json", headers: CORS, body: '{"error":{"message":"Incorrect API key provided"}}' });
        if (!k.models) return route.fulfill({ status: 404, contentType: "application/json", headers: CORS, body: '{"error":{"message":"Not found"}}' });
        return route.fulfill({ status: 200, contentType: "application/json", headers: CORS, body: JSON.stringify({ object: "list", data: k.models }) });
      });
    }
    await kCtx.route(answers + (k.responses ? "/responses" : k.claude ? "/messages" : "/chat/completions"), (route) => {
      const req = route.request(), body = JSON.parse(req.postData() || "{}");
      kSeen[name].asked.push({ url: req.url(), headers: req.headers(), body: body });
      const last = k.responses ? body.input[body.input.length - 1] : body.messages[body.messages.length - 1];
      const again = k.responses ? last.type === "function_call_output" : k.claude ? last.content.some((b) => b.type === "tool_result") : last.role === "tool";
      return route.fulfill({ status: 200, contentType: "text/event-stream", headers: CORS, body: k.responses ? (again ? RESPONSES_WORDS : RESPONSES_CALL) :
        k.claude ? (again ? CLAUDE_WORDS : CLAUDE_CALL) : again ? textTurn(k.words || ["Hello from ", name + "."]) : keyedToolTurn(k) });
    });
  }
  const kp = await kCtx.newPage();
  kp.on("pageerror", (e) => errors.push(String(e)));
  await kp.goto(page_url);
  await kp.waitForFunction(() => window.CO && document.getElementById("toggle-left"));
  out.keyed = {};
  for (const name of Object.keys(KEYED_AIS)) {
    await kp.click("#ai");
    await kp.click(`#sheet-fields .choices button:text-is("${name}")`);
    await kp.waitForSelector("#sheet-fields input[name=key]");
    if (name === "Gemini") out.key_panel = await kp.evaluate(() => ({ title: document.getElementById("sheet-title").textContent,
      on: Array.from(document.querySelectorAll("#sheet-fields .choices button.on")).map((b) => b.textContent),
      field: document.querySelector("#sheet-fields label").textContent, ok: document.getElementById("sheet-ok").textContent,
      more: Array.from(document.querySelectorAll("#sheet-fields .choices")).pop().textContent,
      order: Array.from(document.getElementById("sheet-fields").children).map((e) => e.tagName + (e.className ? "." + e.className : "")) }));
    if (name === "Gemini") {
      await kp.fill("#sheet-fields input[name=key]", "bad-key");
      await kp.click("#sheet-ok");
      await kp.waitForFunction(() => document.getElementById("sheet-error").textContent !== "");
      out.key_refused = await kp.evaluate(() => document.getElementById("sheet-error").textContent);
    }
    await kp.fill("#sheet-fields input[name=key]", "key-of-" + name);
    await kp.click("#sheet-ok");
    await kp.waitForFunction(() => document.getElementById("ai").classList.contains("on"), null, { timeout: 10000 });
    await kp.click("#ai");
    await kp.waitForFunction(() => document.body.classList.contains("sheet"));
    const panel = await kp.evaluate(() => Array.from(document.querySelectorAll("#sheet-fields .choices > *")).map((e) =>
      e.tagName === "P" ? "~ " + e.textContent : e.textContent + (e.classList.contains("on") ? " *" : "")).filter((t) => t !== "~ "));
    await kp.click(`#sheet-fields .choices button:text-is("${KEYED_AIS[name].effort}")`);
    await kp.waitForFunction((e) => Array.from(document.querySelectorAll("#sheet-fields .choices button.on")).some((b) => b.textContent === e),
      KEYED_AIS[name].effort);
    const spark = await kp.evaluate(() => document.getElementById("ai").title);
    await kp.click("#sheet-cancel");
    await kp.fill("#input", "a timer for 20 minutes");
    await kp.keyboard.press("Enter");
    await kp.waitForFunction((n) => ((Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).textContent || "").indexOf("Hello from " + n + ".") >= 0,
      name, { timeout: 10000 });
    const said = await kp.evaluate(() => (Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).textContent);
    const seen = kSeen[name], [first, second] = seen.asked, h = first.headers, kept = await kp.evaluate(() => JSON.parse(localStorage.getItem("chatcomputer-own-ai")));
    const r = { panel: panel, spark: spark, model: first.body.model, kept_base: kept.base || null, at: first.url, stream: first.body.stream, tools: (first.body.tools || []).length,
      key: h["authorization"] || null, models: seen.models.map((m) => m.base + " " + m.key), asked: seen.asked.length, said: said,
      browser: h["anthropic-dangerous-direct-browser-access"] || null, version: h["anthropic-version"] || null };
    if (KEYED_AIS[name].responses) {
      const last = second.body.input[second.body.input.length - 1];
      Object.assign(r, { store: first.body.store, input: second.body.input.map((i) => i.type || i.role), result: last.output.slice(0, 20),
        effort: first.body.reasoning || null, told: first.body.instructions });
    } else if (KEYED_AIS[name].claude) {
      const msgs = second.body.messages, back = msgs[msgs.length - 2], last = msgs[msgs.length - 1];
      Object.assign(r, { max: first.body.max_tokens, effort: first.body.output_config || null, told: first.body.system, roles: msgs.map((m) => m.role),
        back: back.content.map((b) => b.type + (b.signature ? ":" + b.signature : "") + (b.input ? ":" + JSON.stringify(b.input) : "")),
        result_first: last.content[0].type, result: String(last.content[0].content).slice(0, 20), tool_names: first.body.tools.slice(0, 1).map((t) => [t.name, !!t.input_schema]) });
    } else {
      const back = second.body.messages.filter((m) => m.role === "assistant" && m.tool_calls).pop();
      Object.assign(r, { effort: first.body.reasoning_effort || first.body.thinking || null, told: first.body.messages[0].content,
        max: first.body.max_tokens || null, split: first.body.reasoning_split || null, thinking_back: back.reasoning_content || null,
        content_back: Array.isArray(back.content) ? back.content.map((c) => c.type) : back.content, extra_back: back.tool_calls[0].extra_content || null,
        result: second.body.messages[second.body.messages.length - 1].content.slice(0, 20) });
    }
    out.keyed[name] = r;
    await kp.click("#ai");
    await kp.click("#sheet-fields .choices button:has-text('Sign out of')");
    await kp.waitForFunction(() => !document.getElementById("ai").classList.contains("on"));
  }
  // Another AI's key: its address, model and key, in the same panel; signed in, Change asks for them again, with a
  // back arrow to the panel it came from.
  await kp.click("#ai");
  await kp.click('#sheet-fields .choices button:text-is("Another AI")');
  await kp.waitForSelector("#sheet-fields input[name=base]");
  await kp.fill("#sheet-fields input[name=base]", "https://ai.example.com/v1");
  await kp.fill("#sheet-fields input[name=model]", "its-model-2");
  await kp.fill("#sheet-fields input[name=key]", "key-of-another");
  await kp.click("#sheet-ok");
  await kp.waitForFunction(() => document.getElementById("ai").classList.contains("on"));
  await kp.click("#ai");
  await kp.waitForFunction(() => document.body.classList.contains("sheet"));
  await kp.click('#sheet-fields .choices button:text-is("Change")');
  await kp.waitForSelector("#sheet-fields input[name=base]");
  out.other_change = await kp.evaluate(() => ({ back: getComputedStyle(document.getElementById("sheet-back")).display,
    on: Array.from(document.querySelectorAll("#sheet-fields .choices button.on")).map((b) => b.textContent) }));
  await kp.click("#sheet-back");
  await kp.waitForFunction(() => /Sign out of/.test(document.getElementById("sheet-fields").textContent));
  out.other_back = await kp.evaluate(() => ({ back: getComputedStyle(document.getElementById("sheet-back")).display,
    items: Array.from(document.querySelectorAll("#sheet-fields .choices button")).map((b) => b.textContent),
    spark: document.getElementById("ai").title }));
  await kp.click('#sheet-fields .choices button:has-text("Sign out of")');
  await kp.waitForFunction(() => !document.getElementById("ai").classList.contains("on"));
  // (Get a key: the page that makes one, in a new tab.)
  await kp.click("#ai");
  await kp.click('#sheet-fields .choices button:text-is("Claude")');
  await kp.waitForSelector("#sheet-fields input[name=key]");
  const [keyTab] = await Promise.all([kCtx.waitForEvent("page"), kp.click("#sheet-fields .choices button:has-text('Get a key')")]);
  await keyTab.waitForURL(/platform\.claude\.com/).catch(() => {});
  out.key_tab = { url: keyTab.url(), still: await kp.evaluate(() => document.getElementById("sheet-title").textContent) };
  await kCtx.close();

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
  // Friday evening, rain, a warm night, called Sam; a Friday morning, sunny, no name. Then "what's my name?",
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
  await eve.evaluate(() => localStorage.setItem("chatcomputer-demo-name", JSON.stringify("Sam")));
  await eve.reload();                                                            // (the weather kept from the first)
  out.greet_evening = { said: await greeted(eve), now: await eve.evaluate(() => window.CO.greetingsNow()), meteo: meteo.slice() };
  await eve.click("#input");
  out.name_asked = await ask(eve, "what's my name?");
  out.name_given = { said: await ask(eve, "call me sam lee"), time: (await eve.evaluate(() => window.CO.greetingsNow())).time };
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

  // Revert, in Actions: what can be taken back has it, under the pointer, beside its pin. The demo's saved file taken
  // away, then put back (its revert reverted), as "/" shows; yesterday's volume; the text made bigger, and back; a
  // timer closed; one closed already, by its x, has nothing left to take back (no Revert).
  const rv = await browser.newPage({ viewport: { width: 1600, height: 900 } });
  rv.on("pageerror", (e) => errors.push(String(e)));
  await rv.goto(page_url);
  await greeted(rv);
  const acts = () => rv.evaluate(() => Array.from(document.querySelectorAll("#done-list > li.done-item")).map((li) => ({
    id: li.dataset.id, text: li.querySelector(".done-what").textContent, button: !!li.querySelector(".revert"),
    marks: ["revertable", "reverted", "reverting"].filter((c) => li.classList.contains(c)),
    struck: getComputedStyle(li.querySelector(".done-what")).textDecorationLine === "line-through" })));
  const named = (list, text) => list.filter((a) => a.text === text).pop();
  const typingIn = () => rv.evaluate(() => document.activeElement && document.activeElement.id);
  async function revertIt(id) {
    const li = rv.locator('#done-list > li[data-id="' + id + '"]');
    await li.hover();
    const b = li.locator(".revert");
    const seen = await b.evaluate((e) => getComputedStyle(e).visibility);
    await b.click();
    await rv.waitForTimeout(250);
    return seen;
  }
  out.revert_start = { acts: await acts(), hidden: await rv.evaluate(() =>
    getComputedStyle(document.querySelector('#done-list li[data-id="d4"] .revert')).visibility) };
  out.revert_seen = await revertIt("d4");
  out.reverted_file = { acts: await acts(), typing_in: await typingIn() };
  if (shot) await rv.screenshot({ path: shot + "-revert.png" });
  out.reverted_file.files = await slash(rv);
  await revertIt(named(out.reverted_file.acts, "Reverted “Saved Packing list.md”").id);
  out.unreverted_file = { acts: await acts(), files: await slash(rv) };
  await revertIt("d2");
  out.reverted_volume = await acts();
  const textSize = () => rv.evaluate(() => getComputedStyle(document.documentElement).fontSize);
  out.text_sizes = [await textSize()];
  await rv.click("#input");
  await ask(rv, "make the text bigger");
  out.text_sizes.push(await textSize());
  await revertIt(named(await acts(), "Made the text bigger").id);
  out.text_sizes.push(await textSize());
  out.reverted_size = (await acts()).slice(-2);
  await rv.click("#input");
  await ask(rv, "make me a 5 minute timer");
  const timerAct = named(await acts(), "Made a timer");
  out.timer_made = { act: timerAct, open: await rv.evaluate(() => document.body.classList.contains("content")) };
  await revertIt(timerAct.id);
  out.timer_reverted = { acts: (await acts()).slice(-2), open: await rv.evaluate(() => document.body.classList.contains("content")) };
  await rv.click("#input");
  await ask(rv, "make me a 2 minute timer");
  out.timer_kept = (await acts()).pop();
  await rv.click("#frame-close");
  await rv.waitForFunction(() => !document.body.classList.contains("content"));
  out.timer_closed = (await acts()).pop();
  out.revert_notices = await rv.evaluate(() => Array.from(document.querySelectorAll("#log > .notice")).map((n) => n.textContent));
  await rv.close();

  out.errors = errors;
  await browser.close();
  console.log(JSON.stringify(out));
})().catch((e) => { console.log(JSON.stringify({ failed: String(e && e.stack || e) + (ERRORS.length ? "\npage errors: " + ERRORS.join("\n") : "") })); process.exit(1); });
