// The Chrome extension (apps/chrome), loaded into Chromium as it is into Chrome, with chatcomputer.ai standing in from
// docs/ here: its side panel page shows Chat Computer (the website in its frame, ?app=chrome) at a side panel's text
// size; a site asked for opens in a real tab, without a click, typed as an address and asked for in words; other
// pages can't make it open tabs. Prints JSON.  Run: node tests/chrome_check.js
const fs = require("fs");
const os = require("os");
const path = require("path");
const { chromium } = require("playwright");

const ROOT = path.resolve(__dirname, "..");
const EXT = path.join(ROOT, "apps", "chrome");
const DOCS = path.join(ROOT, "docs");
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".ttf": "font/ttf", ".json": "application/json" };

(async () => {
  const out = {};
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "chatcomputer-chrome-"));
  const ctx = await chromium.launchPersistentContext(dir, {
    channel: "chromium", headless: true, viewport: { width: 420, height: 860 },
    args: ["--disable-extensions-except=" + EXT, "--load-extension=" + EXT],
  });
  const errors = [];
  try {
    // chatcomputer.ai, from docs/ here; the sites asked for, stand-ins.
    await ctx.route("https://chatcomputer.ai/**", (route) => {
      const u = new URL(route.request().url());
      let file = path.join(DOCS, decodeURIComponent(u.pathname));
      if (file.endsWith("/")) file = path.join(file, "index.html");
      if (!file.startsWith(DOCS) || !fs.existsSync(file)) return route.fulfill({ status: 404, body: "" });
      return route.fulfill({ status: 200, contentType: TYPES[path.extname(file)] || "application/octet-stream", body: fs.readFileSync(file) });
    });
    await ctx.route(/bbc\.co\.uk|wikipedia\.org|youtube\.com/, (route) => route.fulfill({ status: 200, contentType: "text/html", body: "<title>A site</title>A site" }));
    let [worker] = ctx.serviceWorkers();
    if (!worker) worker = await ctx.waitForEvent("serviceworker");
    const id = new URL(worker.url()).host;
    out.manifest = JSON.parse(fs.readFileSync(path.join(EXT, "manifest.json"), "utf8"));
    const panel = await ctx.newPage();
    panel.on("pageerror", (e) => errors.push(String(e)));
    await panel.goto("chrome-extension://" + id + "/panel.html");
    const site = panel.frameLocator("#site");
    const frame = () => panel.frames().find((f) => /chatcomputer\.ai/.test(f.url()));
    await panel.waitForFunction(() => document.querySelector("#site"));
    for (let i = 0; i < 100 && !frame(); i++) await panel.waitForTimeout(100);
    const f = frame();
    f.on && f.on("pageerror", (e) => errors.push(String(e)));
    await f.waitForFunction(() => { const h = document.querySelector("#log > .ai.hello"); return h && h._said; }, null, { timeout: 15000 });
    if (process.argv[2]) await panel.screenshot({ path: process.argv[2] + "-panel.png" });
    out.opening = await f.evaluate(() => ({ url: location.href, rem: getComputedStyle(document.documentElement).fontSize,
      hint: document.getElementById("input").placeholder, greeting: document.querySelector("#log > .ai.hello")._said.split("\n")[0] }));
    // A site's address, typed: a new tab with it, at once.
    const tabs = () => ctx.pages().map((p) => p.url()).filter((u) => /^https?:/.test(u));
    await site.locator("#input").click();
    await site.locator("#input").type("bbc.co.uk");
    const opened = ctx.waitForEvent("page", { timeout: 8000 }).catch(() => null);
    await site.locator("#input").press("Enter");
    const tab = await opened;
    if (tab) await tab.waitForLoadState().catch(() => {});
    await panel.waitForTimeout(800);
    out.typed = { tabs: tabs(), said: await f.evaluate(() => Array.from(document.querySelectorAll("#log > .ai")).pop().textContent) };
    // Another page can't make the extension open a tab.
    await f.evaluate(() => {});
    await panel.evaluate(() => window.postMessage({ chatcomputer: "open_tab", url: "https://evil.example/" }, "*"));
    await panel.waitForTimeout(500);
    out.forged = tabs().some((u) => /evil/.test(u));
    out.errors = errors;
  } catch (e) {
    out.failed = String(e && e.stack || e).slice(0, 1500);
  }
  console.log(JSON.stringify(out));
  await ctx.close();
})();
