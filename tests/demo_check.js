// The browser demo (docs/index.html: ClaudeOS's own screen, with docs/demo/host.js standing in for the computer),
// tried in Chromium with Playwright: the intro, a few things asked, F1 and Escape, "/" search, a message opened, a
// phone-sized window. Prints JSON.  Run: node tests/demo_check.js [shot-prefix]
const path = require("path");
const { chromium } = require("playwright");

const page_url = "file://" + path.resolve(__dirname, "..", "docs", "index.html");
const shot = process.argv[2];

async function state(page) {
  return page.evaluate(() => {
    const v = document.getElementById("demo-view");
    const r = v && v.style.display !== "none" ? v.getBoundingClientRect() : null;
    const frame = document.getElementById("frame").getBoundingClientRect();
    return {
      sheet: document.body.classList.contains("sheet"), content: document.body.classList.contains("content"),
      view: r ? [r.left, r.top, r.width, r.height].map(Math.round) : null,
      frame: [frame.left, frame.top, frame.width, frame.height].map(Math.round),
      messages: document.querySelectorAll("#messages .msg").length,
      done: Array.from(document.querySelectorAll("#done .done-what")).map((e) => e.textContent),
      last: (Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).textContent || "",
      notices: Array.from(document.querySelectorAll("#log > .notice")).map((e) => e.textContent),
      results: Array.from(document.querySelectorAll("#results .res-text")).map((e) => e.textContent),
      panel: document.body.classList.contains("panel"),
    };
  });
}

async function viewText(page) {
  const f = page.frameLocator("#demo-view");
  return (await f.locator("body").innerText({ timeout: 5000 })).replace(/\s+/g, " ").trim();
}

async function ask(page, text) {
  const before = await page.locator("#log > .ai").count();
  await page.keyboard.type(text, { delay: 5 });
  await page.keyboard.press("Enter");
  await page.waitForFunction((n) => document.querySelectorAll("#log > .ai").length > n &&
    !document.querySelector("#log > .ai.thinking"), before);
  await page.waitForTimeout(700);              // the words stream in
}

(async () => {
  const out = {};
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1600, height: 900 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(page_url);
  await page.waitForSelector("body.sheet");
  out.intro = await page.locator("#sheet-title").innerText();
  await page.click("#sheet-ok");
  await page.waitForFunction(() => !document.body.classList.contains("sheet"));
  out.start = await state(page);
  await ask(page, "play some piano music");
  out.music = await state(page);
  out.music_view = await viewText(page);
  if (shot) await page.screenshot({ path: shot + "-music.png" });
  await page.keyboard.press("F1");
  out.f1 = await state(page);
  await page.keyboard.press("F1");
  out.f1_back = await state(page);
  await page.keyboard.press("Escape");
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
  await page.keyboard.press("Escape");
  await ask(page, "what's the meaning of life?");
  out.fallback = (await state(page)).last;
  if (shot) await page.screenshot({ path: shot + "-chat.png" });

  // A phone: no sidebars; the conversation and the box still work.
  const phone = await browser.newPage({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  phone.on("pageerror", (e) => errors.push(String(e)));
  await phone.goto(page_url);
  await phone.waitForSelector("body.sheet");
  await phone.click("#sheet-ok");
  await phone.waitForFunction(() => !document.body.classList.contains("sheet"));
  await phone.click("#input");
  await ask(phone, "hello");
  out.phone = await state(phone);
  if (shot) await phone.screenshot({ path: shot + "-phone.png" });
  out.errors = errors;
  await browser.close();
  console.log(JSON.stringify(out));
})().catch((e) => { console.log(JSON.stringify({ failed: String(e && e.stack || e) })); process.exit(1); });
