// Website eval: each case is a fresh visit to chatcomputer.ai's page (docs/index.html, in Chromium with Playwright),
// the case's words typed in the box, and what the visitor then sees recorded: the answer, what opened between the
// sidebars, What's done, and how long it took. Nothing leaves the machine: requests to other sites are answered here
// (Open-Meteo's with where a few places are and one made-up forecast, the rest with an empty page). The visitor is
// in Singapore's time zone.
//
//   NODE_PATH=$(npm root -g) node evals/web/run.js OUT.json [--only id,id] [--jobs 4] [--nano stub.js]
//
// --nano: a script run in the page before its own, standing in for the browser's built-in AI (LanguageModel).
const fs = require("fs");
const path = require("path");
const { chromium } = require("playwright");

const ROOT = path.resolve(__dirname, "..", "..");
const PAGE = "file://" + path.join(ROOT, "docs", "index.html");
const args = process.argv.slice(2);
const out = args[0];
const opt = (k, d) => { const i = args.indexOf(k); return i > 0 ? args[i + 1] : d; };
const only = (opt("--only", "") || "").split(",").filter(Boolean);
const jobs = +opt("--jobs", "4");
const nano = opt("--nano", "");
const data = JSON.parse(fs.readFileSync(path.join(__dirname, "cases.json"), "utf8"));
const cases = data.cases.filter((c) => !only.length || only.includes(c.id));

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
  if (u.hostname === "api.open-meteo.com") {
    return { current: { temperature_2m: 18.4, weather_code: 3 },
             daily: { time: ["2026-10-20", "2026-10-21"], weather_code: [61, 2], temperature_2m_max: [21.2, 19.8],
                      temperature_2m_min: [12.6, 11.9], precipitation_probability_max: [70, 15] } };
  }
  return null;
}

async function visit(browser, c) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, timezoneId: "Asia/Singapore", locale: "en-SG" });
  const page = await ctx.newPage();
  const errors = [], requests = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.route(/^https?:\/\//, (route) => {
    const url = route.request().url(), om = openMeteo(url);
    requests.push(url);
    if (om) return route.fulfill({ status: 200, contentType: "application/json", headers: { "access-control-allow-origin": "*" },
                                   body: JSON.stringify(om) });
    route.fulfill({ status: 200, contentType: "text/html", body: "<!doctype html><title>elsewhere</title>" });
  });
  if (nano) await page.addInitScript({ path: nano });
  await page.goto(PAGE);
  await page.waitForFunction(() => document.getElementById("demo-tools"));
  await page.waitForTimeout(300);
  const before = await page.evaluate(() => document.querySelectorAll("#done .done-what").length);
  const n = await page.locator("#log > .ai").count();
  const t0 = Date.now();
  await page.keyboard.type(c.query, { delay: 2 });
  await page.keyboard.press("Enter");
  let first = null, last = "", text = "";
  try {
    await page.waitForFunction((k) => document.querySelectorAll("#log > .ai").length > k &&
      !document.querySelector("#log > .ai.thinking"), n, { timeout: 120000 });
    first = Date.now() - t0;
    for (let i = 0; i < 480; i++) {                      // until the answer stops growing (up to 2 minutes)
      await page.waitForTimeout(250);
      text = await page.evaluate(() => (Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).textContent || "");
      if (text && text === last) break;
      last = text;
    }
  } catch (e) { errors.push("no answer: " + String(e).slice(0, 200)); }
  const seen = await page.evaluate((k) => {
    const v = document.getElementById("demo-view");
    let title = "", body = "";
    try { title = v && v.contentDocument ? v.contentDocument.title : ""; body = v && v.contentDocument && v.contentDocument.body ? v.contentDocument.body.innerText.slice(0, 600) : ""; } catch (e) {}
    return {
      open: document.body.classList.contains("content"),
      src: v ? v.getAttribute("src") || "" : "", title: title, body: body,
      done: Array.from(document.querySelectorAll("#done .done-what")).slice(0).map((e) => e.textContent).slice(-5),
      done_added: document.querySelectorAll("#done .done-what").length - k,
    };
  }, before);
  await ctx.close();
  return { id: c.id, intent: c.intent, phrasing: c.phrasing, query: c.query, expect: c.expect, answer: text.trim(),
           first_ms: first, total_ms: Date.now() - t0, ...seen, requests: requests.slice(0, 10), errors };
}

(async () => {
  const browser = await chromium.launch();
  const results = [];
  let next = 0;
  async function worker() {
    while (next < cases.length) {
      const c = cases[next++];
      try { results.push(await visit(browser, c)); } catch (e) { results.push({ id: c.id, query: c.query, errors: [String(e)] }); }
      process.stdout.write(".");
    }
  }
  await Promise.all(Array.from({ length: jobs }, worker));
  await browser.close();
  results.sort((a, b) => cases.findIndex((c) => c.id === a.id) - cases.findIndex((c) => c.id === b.id));
  fs.writeFileSync(out, JSON.stringify(results, null, 1));
  console.log("\n" + results.length + " cases -> " + out);
})();
