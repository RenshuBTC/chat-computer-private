// The browser demo (docs/index.html: Chat Computer's own screen, with docs/demo/host.js standing in for the computer),
// tried in Chromium with Playwright: it opens playing YouTube, Escape shows the empty screen (the word logo), the
// panels' switches, a few things asked, F1 and Escape, "/" search, a message opened, a phone-sized window.
// Prints JSON.  Run: node tests/demo_check.js [shot-prefix]
const path = require("path");
const { chromium } = require("playwright");

const page_url = "file://" + path.resolve(__dirname, "..", "docs", "index.html");
const shot = process.argv[2];

async function state(page) {
  return page.evaluate(() => {
    const v = document.getElementById("demo-view");
    const r = v && v.style.display !== "none" ? v.getBoundingClientRect() : null;
    const frame = document.getElementById("frame").getBoundingClientRect();
    const shown = (id) => getComputedStyle(document.getElementById(id)).display !== "none";
    return {
      sheet: document.body.classList.contains("sheet"), content: document.body.classList.contains("content"),
      view: r ? [r.left, r.top, r.width, r.height].map(Math.round) : null,
      src: v ? v.getAttribute("src") || "" : "",
      frame: [frame.left, frame.top, frame.width, frame.height].map(Math.round),
      messages: document.querySelectorAll("#messages .msg").length,
      done: Array.from(document.querySelectorAll("#done .done-what")).map((e) => e.textContent),
      last: (Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).textContent || "",
      results: Array.from(document.querySelectorAll("#results .res-text")).map((e) => e.textContent),
      panel: document.body.classList.contains("panel"),
      welcome: shown("welcome") ? [document.getElementById("wordmark").textContent, document.getElementById("hello").textContent] : null,
      sides: [shown("messages"), shown("done")],
      composer: shown("composer"),
      toggles: ["left", "right", "bottom"].map((k) => shown("toggle-" + k) && document.getElementById("toggle-" + k).getAttribute("aria-pressed")),
      line: (document.getElementById("demo-line") || {}).textContent || "",
      // What shows above the message box while something is open (the lines inside the strip's own area).
      strip: (() => {
        const c = document.getElementById("column").getBoundingClientRect();
        return Array.from(document.querySelectorAll("#log > *")).filter((n) => {
          const r = n.getBoundingClientRect();
          return getComputedStyle(n).display !== "none" && r.height > 0 && r.top >= c.top - 1 && r.bottom <= c.bottom + 1;
        }).map((n) => n.textContent.replace(/\u2026 more$|less$/, "").trim());
      })(),
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
  await page.route(/youtube/, (route) => route.fulfill({ status: 200, contentType: "text/html",
    body: "<!doctype html><title>YouTube</title><p>A YouTube player</p>" }));      // no network needed
  await page.goto(page_url);
  // It opens playing: YouTube's player between the sidebars, without asking.
  await page.waitForFunction(() => document.body.classList.contains("content"));
  out.opening = await state(page);
  if (shot) await page.screenshot({ path: shot + "-opening.png" });
  // Escape: the empty screen, the word logo and hello, world.
  await page.keyboard.press("Escape");
  out.empty = await state(page);
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
  await page.keyboard.press("Escape");
  await ask(page, "what's the meaning of life?");
  out.fallback = (await state(page)).last;
  if (shot) await page.screenshot({ path: shot + "-chat.png" });

  // A phone: no sidebars and no switches; the conversation and the box still work.
  const phone = await browser.newPage({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  phone.on("pageerror", (e) => errors.push(String(e)));
  await phone.route(/youtube/, (route) => route.fulfill({ status: 200, contentType: "text/html", body: "<p>A YouTube player</p>" }));
  await phone.goto(page_url);
  await phone.waitForFunction(() => document.body.classList.contains("content"));
  out.phone_opening = await state(phone);
  await phone.click("#input");
  await ask(phone, "hello");
  out.phone = await state(phone);
  if (shot) await phone.screenshot({ path: shot + "-phone.png" });
  out.errors = errors;
  await browser.close();
  console.log(JSON.stringify(out));
})().catch((e) => { console.log(JSON.stringify({ failed: String(e && e.stack || e) })); process.exit(1); });
