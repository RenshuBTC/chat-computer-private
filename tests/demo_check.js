// The browser demo (docs/index.html: Chat Computer's own screen, with docs/demo/host.js standing in for the computer),
// tried in Chromium with Playwright: it opens playing YouTube (from 0:03; the sound on at the first key) with the
// cursor in the box; "/" lists the files in front of the video; when the video ends, the conversation comes back;
// the panels' switches; a few things asked; F1 and Escape; a message opened; Connect Claude, with a stand-in for
// Anthropic (a tool used, an answer streamed); Connect AirPods' sheet; a phone-sized window.
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

// Anthropic's API, standing in: the key check, then a map of Tokyo asked for (show_map), then the answer.
function sse(events) { return events.map((e) => "event: " + e.type + "\ndata: " + JSON.stringify(e) + "\n\n").join(""); }
const START = { type: "message_start", message: { id: "msg_1", type: "message", role: "assistant", content: [], model: "claude-sonnet-5-5",
  stop_reason: null, usage: { input_tokens: 10, output_tokens: 1 } } };
const TOOL_TURN = sse([START,
  { type: "content_block_start", index: 0, content_block: { type: "tool_use", id: "toolu_1", name: "show_map", input: {} } },
  { type: "content_block_delta", index: 0, delta: { type: "input_json_delta", partial_json: '{"place": "Tokyo", "west": 139.55, ' } },
  { type: "content_block_delta", index: 0, delta: { type: "input_json_delta", partial_json: '"south": 35.55, "east": 139.95, "north": 35.8}' } },
  { type: "content_block_stop", index: 0 },
  { type: "message_delta", delta: { stop_reason: "tool_use" }, usage: { output_tokens: 30 } },
  { type: "message_stop" }]);
const TEXT_TURN = sse([START,
  { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } },
  { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Here's " } },
  { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Tokyo." } },
  { type: "content_block_stop", index: 0 },
  { type: "message_delta", delta: { stop_reason: "end_turn" }, usage: { output_tokens: 5 } },
  { type: "message_stop" }]);

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
      welcome: shown("welcome") ? Array.from(document.getElementById("welcome").children).map((e) => e.textContent) : null,
      sides: [shown("messages"), shown("done")],
      composer: shown("composer"),
      toggles: ["left", "right", "bottom"].map((k) => shown("toggle-" + k) && document.getElementById("toggle-" + k).getAttribute("aria-pressed")),
      typing_in: document.activeElement && document.activeElement.id,
      hint: shown("hint") ? document.getElementById("hint").textContent : null,
      hint_weight: getComputedStyle(document.getElementById("hint")).fontWeight,
      tools: tools ? Array.from(tools.querySelectorAll("button")).map((b) => b.getAttribute("aria-label")) : null,
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
               version: req.headers()["anthropic-version"], model: body.model, stream: !!body.stream,
               tools: (body.tools || []).map((t) => t.name), last: JSON.stringify((body.messages || []).slice(-1)) });
    if (!body.stream) return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ id: "msg_0", type: "message",
      role: "assistant", content: [{ type: "text", text: "Hi" }], stop_reason: "max_tokens", usage: { input_tokens: 8, output_tokens: 1 } }) });
    const answered = /tool_result/.test(JSON.stringify(body.messages.slice(-1)));
    return route.fulfill({ status: 200, contentType: "text/event-stream", body: answered ? TEXT_TURN : TOOL_TURN });
  });
  await page.goto(page_url);
  // It opens playing: YouTube's player between the sidebars, from 0:03, without asking; the cursor in the box.
  await page.waitForFunction(() => document.body.classList.contains("content"));
  await page.waitForTimeout(600);
  out.opening = await state(page);
  if (shot) await page.screenshot({ path: shot + "-opening.png" });
  // A click on the video itself turns the sound on too (the player is told); the pointer off it again, typing goes
  // back to the box.
  const v0 = out.opening.view;
  await page.mouse.click(v0[0] + v0[2] / 2, v0[1] + v0[3] / 2);
  await page.waitForTimeout(400);
  out.clicked_commands = await player(page).evaluate(() => window.commands);
  out.clicked_in = await page.evaluate(() => document.activeElement && document.activeElement.id);
  await page.mouse.move(v0[0] + v0[2] / 2, v0[1] + v0[3] + 30);
  await page.waitForTimeout(200);
  out.keys_back = await page.evaluate(() => document.activeElement && document.activeElement.id);
  // "/": the files, in a list in front of the video (what's at its middle is the list, not the video).
  await page.keyboard.type("/");
  await page.waitForSelector("#results .res");
  out.over = await page.evaluate(() => {
    const r = document.querySelector("#results .res").getBoundingClientRect();
    const at = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    return !!(at && at.closest("#results"));
  });
  out.over_view = (await state(page)).view;
  // The first key turned the sound on.
  out.player_commands = await player(page).evaluate(() => window.commands);
  await page.keyboard.press("Escape");                     // clears the box
  // The video ends: the conversation's screen again, the word logo in the middle.
  await player(page).evaluate(() => window.end());
  await page.waitForFunction(() => !document.body.classList.contains("content"));
  out.ended = await state(page);
  // The sidebars' switches: off, and on again (and the choice kept).
  await page.click("#toggle-left");
  await page.click("#toggle-right");
  await page.waitForTimeout(200);
  out.sides_off = await state(page);
  out.kept = await page.evaluate(() => localStorage.getItem("chatcomputer-demo-panels"));
  await page.click("#toggle-left");
  await page.click("#toggle-right");
  out.sides_on = await state(page);
  await ask(page, "play hello world");
  out.music = await state(page);
  if (shot) await page.screenshot({ path: shot + "-music.png" });
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
  out.api = api;
  if (shot) await page.screenshot({ path: shot + "-claude.png" });
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

  // A phone: no sidebars and no switches; the buttons inside the box; the conversation and the box still work.
  const phone = await browser.newPage({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  phone.on("pageerror", (e) => errors.push(String(e)));
  await phone.route(/youtube/, (route) => route.fulfill({ status: 200, contentType: "text/html", body: PLAYER }));
  await phone.goto(page_url);
  await phone.waitForFunction(() => document.body.classList.contains("content"));
  await phone.waitForTimeout(400);
  out.phone_opening = await state(phone);
  await phone.click("#input");
  await ask(phone, "hello");
  out.phone = await state(phone);
  if (shot) await phone.screenshot({ path: shot + "-phone.png" });
  out.errors = errors;
  await browser.close();
  console.log(JSON.stringify(out));
})().catch((e) => { console.log(JSON.stringify({ failed: String(e && e.stack || e) })); process.exit(1); });
