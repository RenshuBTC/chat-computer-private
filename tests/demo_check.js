// The browser demo (docs/index.html: Chat Computer's own screen, with docs/demo/host.js standing in for the computer),
// tried in Chromium with Playwright: it opens on the word logo, Chat Computer saying hello, world, the cursor in
// the box (the logo goes with the first message sent); the
// panels' switches; music (made by the browser); "/" lists the files in front of what's open; F1 and Escape; a
// message opened; Connect Claude, with a stand-in for Anthropic (tools used: a map, a YouTube video, which closes
// when it ends, with a stand-in for YouTube's player; answers streamed; thinking first, as Opus 5.5 always does);
// Connect AirPods' sheet; Chrome's built-in AI, with a stand-in (Download Gemini: its sheet, the download, then it
// answers); a phone (music turned down, then stopped).
// Prints JSON.  Run: node tests/demo_check.js [shot-prefix]
const path = require("path");
const { chromium } = require("playwright");

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

// Anthropic's API, standing in: the key check; a map of Tokyo asked for (show_map), or a video (play_youtube); then
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

async function state(page) {
  return page.evaluate(() => {
    const v = document.getElementById("demo-view");
    const r = v && v.style.display !== "none" ? v.getBoundingClientRect() : null;
    const frame = document.getElementById("frame").getBoundingClientRect();
    const shown = (id) => getComputedStyle(document.getElementById(id)).display !== "none";
    const tools = document.getElementById("demo-tools");
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
      first_ai: ((document.querySelector("#log > .ai") || {}).textContent || "").trim(),
      logo: shown("welcome") ? document.getElementById("wordmark").textContent : null,
      said: document.querySelectorAll("#log > .you, #log > .ai").length,
      sides: [shown("messages"), shown("done")],
      composer: shown("composer"),
      toggles: ["left", "right", "bottom"].map((k) => shown("toggle-" + k) && document.getElementById("toggle-" + k).getAttribute("aria-pressed")),
      typing_in: document.activeElement && document.activeElement.id,
      hint: shown("hint") ? document.getElementById("hint").textContent : null,
      hint_weight: getComputedStyle(document.getElementById("hint")).fontWeight,
      tools: tools ? Array.from(tools.querySelectorAll("button")).filter((b) => getComputedStyle(b).display !== "none").map((b) => b.getAttribute("aria-label")) : null,
      tools_at: tools ? tools.parentNode.id : null,
      more: document.querySelectorAll(".more").length,
      // What shows above the message box while something is open (the messages inside the strip's own area).
      strip: (() => {
        const c = document.getElementById("column").getBoundingClientRect();
        return Array.from(document.querySelectorAll("#log > *")).filter((n) => {
          const r = n.getBoundingClientRect();
          return getComputedStyle(n).display !== "none" && r.height > 0 && r.top >= c.top - 1 && r.bottom <= c.bottom + 1;
        }).map((n) => n.textContent.trim());
      })(),
    };
  });
}

async function viewText(page) {
  const f = page.frameLocator("#demo-view");
  return (await f.locator("body").innerText({ timeout: 5000 })).replace(/\s+/g, " ").trim();
}
function player(page) { return page.frames().find((f) => /youtube/.test(f.url())); }

async function ask(page, text) {
  const before = await page.locator("#log > .ai").count();
  await page.keyboard.type(text, { delay: 5 });
  await page.keyboard.press("Enter");
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
}

(async () => {
  const out = {};
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1600, height: 900 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.route(/youtube/, (route) => route.fulfill({ status: 200, contentType: "text/html", body: PLAYER }));      // no network needed
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
      : /video/.test(last) ? VIDEO_TURN : MAP_TURN;
    return route.fulfill({ status: 200, contentType: "text/event-stream", body: turn });
  });
  await page.goto(page_url);
  // It opens on the word logo and Chat Computer's first words, hello, world; the cursor in the box, under the hint.
  await page.waitForFunction(() => document.getElementById("demo-tools"));
  await page.waitForTimeout(500);
  out.opening = await state(page);
  if (shot) await page.screenshot({ path: shot + "-opening.png" });
  // The sidebars' switches: off, and on again (and the choice kept).
  await page.click("#toggle-left");
  await page.click("#toggle-right");
  await page.waitForTimeout(200);
  out.sides_off = await state(page);
  out.kept = await page.evaluate(() => localStorage.getItem("chatcomputer-demo-panels"));
  await page.click("#toggle-left");
  await page.click("#toggle-right");
  out.sides_on = await state(page);
  await ask(page, "play some piano music");
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
  await page.keyboard.press("Escape");                     // clears the box
  await page.keyboard.press("F1");
  out.f1 = await state(page);
  await page.keyboard.press("F1");
  out.f1_back = await state(page);
  // The conversation's switch: the page takes its room; typing brings it back.
  await page.click("#toggle-bottom");
  await page.waitForTimeout(200);
  out.bottom_off = await state(page);
  await page.keyboard.type("x");
  await page.waitForTimeout(200);
  out.bottom_back = await state(page);
  await page.keyboard.press("Escape");                     // clears the box
  await page.keyboard.press("Escape");                     // closes what's open
  out.escaped = await state(page);
  await page.keyboard.type("/pack", { delay: 40 });
  await page.waitForSelector("#results .res");
  out.search = await state(page);
  if (shot) await page.screenshot({ path: shot + "-search.png" });
  await page.keyboard.press("Enter");
  await page.waitForTimeout(400);
  out.opened = await state(page);
  out.opened_view = await viewText(page);
  await page.keyboard.press("Escape");
  await page.click("#messages .msg:has-text('Jane')");
  await page.waitForTimeout(400);
  out.jane = await viewText(page);
  await page.keyboard.press("Escape");
  await ask(page, "make me a 5 minute timer");
  out.timer = await viewText(page);
  out.after_timer = await state(page);
  // A long answer while something is open: both messages in full above the box, no "more".
  await ask(page, "what's the meaning of life?");
  out.fallback = await state(page);
  if (shot) await page.screenshot({ path: shot + "-long.png" });
  await page.keyboard.press("Escape");

  // Connect Claude: its sheet, a key, and then Claude (a stand-in) answers, using a tool of the demo's.
  await page.click("#demo-claude");
  out.claude_sheet = await state(page);
  await page.keyboard.type("sk-ant-api03-demo-test-key");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => !document.body.classList.contains("sheet"));
  out.claude_on = await state(page);
  out.stored = await page.evaluate(() => localStorage.getItem("chatcomputer-demo-claude-key"));
  await ask(page, "show me Tokyo");
  out.claude_map = await state(page);
  if (shot) await page.screenshot({ path: shot + "-claude.png" });
  // A video, played by Claude: YouTube's player (a stand-in) between the sidebars; when it ends, the conversation.
  await ask(page, "play a video");
  out.claude_video = await state(page);
  await page.waitForTimeout(300);
  await player(page).evaluate(() => window.end());
  await page.waitForFunction(() => !document.body.classList.contains("content"));
  out.video_ended = await state(page);
  out.api = api;
  await page.click("#demo-claude");                        // disconnecting
  out.claude_off_sheet = await state(page);
  await page.click("#sheet-ok");
  await page.waitForFunction(() => !document.body.classList.contains("sheet"));
  out.claude_off = await state(page);
  out.stored_after = await page.evaluate(() => localStorage.getItem("chatcomputer-demo-claude-key"));
  // Connect AirPods: what it does, before the microphone is asked for.
  await page.click("#demo-airpods");
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
  await ai.waitForFunction(() => document.getElementById("demo-tools") && !document.getElementById("demo-nano").hidden);
  out.ai_opening = await state(ai);
  await ai.click("#demo-nano");
  out.ai_sheet = await state(ai);
  await ai.click("#sheet-ok");
  await ai.waitForFunction(() => /50%/.test(document.getElementById("demo-nano").getAttribute("aria-label")));
  out.ai_downloading = await state(ai);
  await ai.evaluate(() => window.__ai.half());
  await ai.waitForFunction(() => getComputedStyle(document.getElementById("demo-nano")).display === "none");
  out.ai_on = await state(ai);
  await ai.click("#input");
  await ask(ai, "write a short poem about the sea");
  out.ai_poem = await state(ai);
  out.ai_poem_view = await viewText(ai);
  await ask(ai, "play some piano music");
  out.ai_music = await state(ai);
  out.ai_calls = await ai.evaluate(() => window.__ai);
  if (shot) await ai.screenshot({ path: shot + "-ai.png" });

  // A phone: no sidebars and no switches; the buttons inside the box; the conversation and the box still work.
  const phone = await browser.newPage({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  phone.on("pageerror", (e) => errors.push(String(e)));
  await phone.route(/youtube/, (route) => route.fulfill({ status: 200, contentType: "text/html", body: PLAYER }));
  await phone.goto(page_url);
  await phone.waitForFunction(() => document.getElementById("demo-tools"));
  await phone.waitForTimeout(400);
  out.phone_opening = await state(phone);
  await phone.click("#input");
  await ask(phone, "hello");
  out.phone = await state(phone);
  if (shot) await phone.screenshot({ path: shot + "-phone.png" });
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
  out.errors = errors;
  await browser.close();
  console.log(JSON.stringify(out));
})().catch((e) => { console.log(JSON.stringify({ failed: String(e && e.stack || e) })); process.exit(1); });
