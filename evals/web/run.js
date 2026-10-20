// Website eval: each case is a fresh visit to chatcomputer.ai's page (docs/index.html, in Chromium with Playwright),
// the case's words typed in the box (a conversation's turns one after another, each after the answer to the one
// before), and what the visitor then sees recorded: the answers, what opened between the sidebars, Actions, and
// how long it took. Nothing leaves the machine: requests to other sites are answered here (Open-Meteo's with where a
// few places are, one made-up forecast and air reading; Wikipedia's with a few made-up articles and a day's news;
// the rest with an empty page). The visitor is in Singapore's time zone.
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
function wikipedia(url) {
  const u = new URL(url);
  if (u.hostname !== "en.wikipedia.org") return null;
  if (u.searchParams.get("action") === "parse") return { parse: { title: u.searchParams.get("page"), text: EVENTS } };
  const q = u.searchParams.get("gsrsearch") || "";
  const pages = ARTICLES.filter((a) => a[0].test(q)).map((a, i) => ({ index: i + 1, title: a[1], extract: a[2],
    fullurl: "https://en.wikipedia.org/wiki/" + a[1].replace(/ /g, "_") }));
  return pages.length ? { query: { pages } } : { batchcomplete: true };
}

async function visit(browser, c) {
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, timezoneId: "Asia/Singapore", locale: "en-SG" });
  const page = await ctx.newPage();
  const errors = [], requests = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.route(/^https?:\/\//, (route) => {
    const url = route.request().url(), om = openMeteo(url) || wikipedia(url);
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
  const t0 = Date.now(), turns = c.turns || [c.query], answers = [], firsts = [];
  for (const q of turns) {
    const n = await page.locator("#log > .ai").count(), t1 = Date.now();
    await page.keyboard.type(q, { delay: 2 });
    await page.keyboard.press("Enter");
    let first = null, last = "", text = "";
    try {
      await page.waitForFunction((k) => document.querySelectorAll("#log > .ai").length > k &&
        !document.querySelector("#log > .ai.thinking"), n, { timeout: 120000 });
      first = Date.now() - t1;
      for (let i = 0; i < 480; i++) {                      // until the answer stops growing (up to 2 minutes)
        await page.waitForTimeout(250);
        text = await page.evaluate(() => (Array.from(document.querySelectorAll("#log > .ai")).pop() || {}).innerText || "");
        if (text && text === last) break;
        last = text;
      }
    } catch (e) { errors.push("no answer to " + q + ": " + String(e).slice(0, 200)); }
    answers.push(text.trim());
    firsts.push(first);
    await page.waitForTimeout(300);
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
    };
  }, before);
  await ctx.close();
  return { id: c.id, intent: c.intent, phrasing: c.phrasing, query: c.query || turns.join(" / "), turns: c.turns || null,
           expect: c.expect, answers, answer: answers[answers.length - 1] || "", first_ms: firsts[0], firsts,
           total_ms: Date.now() - t0, ...seen, requests: requests.slice(0, 10), errors };
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
