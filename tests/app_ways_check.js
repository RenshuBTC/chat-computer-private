// How you want Chat Computer, and Chat Computer as your browser: the Chat Computer app's own pages (its first page,
// start/, and the conversation's, index.html, as scripts/build_apps.py desktop puts them together), tried in Chromium
// (the engine of Windows' WebView2) with Playwright, with a stand-in for the app (Tauri's invoke, and what the app
// tells the page).
//
// The first page: the four ways, the own system's in the computer's words; the app chosen, then the conversation;
// opened again to change it, the way now marked; the Chrome extension (its folder's path copied, the steps, Chrome's
// own address to copy, the folder shown); the browser (asked to be the default, then told it is); the own system
// started (Windows), or why this Mac can't have it (an M4: nothing asked of the app); started again on the extension,
// at its steps. Then the browser: the link from another app that came first, in a tab; an address typed, in a new
// tab; tabs chosen, the conversation's tab, + (the box ready for an address), back and forward; what Chat Computer's
// model opens, in the tab it opened the last in; a tab's title from the app; its x; Escape (the tab stays open); the
// row's place above the middle and the frame under it; the app as the app has no tabs.
// Prints JSON.  Run: node tests/app_ways_check.js <the app's page, put together (apps/desktop/frontend)>
const path = require("path");
const { chromium } = require("playwright");

const dir = path.resolve(process.argv[2] || path.join(__dirname, "..", "apps", "desktop", "frontend"));
const START = "file://" + path.join(dir, "start", "index.html");
const out = { errors: [] };

// The stand-in app. What it's asked is kept (window.__app.calls); how you have Chat Computer is kept in the page's
// storage (as the app keeps it in its folder), so the conversation's page knows it after the first page has chosen.
function standIn(opts) {
  window.__app = { calls: [], copied: [] };
  const o = opts || {};
  const kept = (w) => { if (w !== undefined) localStorage.setItem("__way", w); return localStorage.getItem("__way") || null; };
  if (o.way !== undefined && !localStorage.getItem("__fresh")) { localStorage.setItem("__fresh", "1"); kept(o.way); }
  const ext = o.system === "mac" ? "/Users/sam/Library/Application Support/ai.chatcomputer.desktop/Chrome extension" :
    "C:\\Users\\Sam\\AppData\\Local\\ai.chatcomputer.desktop\\Chrome extension";
  const os = o.system === "mac" ? { can: false, why: "This Mac has an Apple M4: Chat Computer's own system starts only on Macs with M1, M2 or M3.",
    system: "mac", chip: "Apple M4" } : { can: true, why: "", system: "windows", chip: "" };
  let isDefault = false;
  // (Copied: what a click copies, as the page copies it.)
  const exec = Document.prototype.execCommand;
  Document.prototype.execCommand = function (c) {
    if (c === "copy") { window.__app.copied.push(document.activeElement && document.activeElement.value); return true; }
    return exec.apply(this, arguments);
  };
  window.__TAURI__ = { core: { Channel: function () {}, invoke: (command, args) => {
    window.__app.calls.push([command, args || {}]);
    if (command === "way_status") return Promise.resolve({ way: kept(), chrome: true, extension: ext, setup: "C:\\Users\\Sam\\Setup",
      default_browser: isDefault, os: os });
    if (command === "choose_way") {
      if (args.way === "os" && !os.can) return Promise.reject(os.why);
      kept(args.way);
      if (args.way === "chrome") return Promise.resolve({ way: "chrome", folder: ext, chrome: true, chrome_opened: true });
      if (args.way === "browser") return Promise.resolve({ way: "browser", default_browser: false });
      if (args.way === "os") return Promise.resolve({ way: "os", folder: "C:\\Users\\Sam\\AppData\\Local\\ai.chatcomputer.desktop\\Chat Computer Setup" });
      return Promise.resolve({ way: args.way });
    }
    if (command === "make_default_browser") { setTimeout(() => { isDefault = true; }, 500); return Promise.resolve(null); }
    if (command === "about") return Promise.resolve({ name: "Sam Lee", system: o.system || "windows", way: kept() });
    if (command === "take_opening") return Promise.resolve(o.opening || []);
    if (command === "search_files") return Promise.resolve([]);
    if (command === "model_status") return Promise.resolve({ state: "ready", label: "Qwen3.5 9B", preparing: "", stage: "", done: 0,
      total: 0, port: 8766, detail: "" });
    return Promise.resolve(null);
  } } };
}

// Chat Computer's engine, standing in (as tests/demo_check.js has it): a call of open_website, then words.
function chunk(delta, finish) {
  return "data: " + JSON.stringify({ id: "c", object: "chat.completion.chunk", created: 1, model: "qwen3.5-9b",
    choices: [{ index: 0, delta: delta, finish_reason: finish || null }] }) + "\n\n";
}
function toolTurn(name, args) {
  return chunk({ role: "assistant" }) + chunk({ tool_calls: [{ index: 0, id: "call_1", type: "function", function: { name: name, arguments: args } }] }) +
    chunk({ content: "" }, "tool_calls") + "data: [DONE]\n\n";
}
function textTurn(words) { return chunk({ role: "assistant" }) + chunk({ content: words }) + chunk({ content: "" }, "stop") + "data: [DONE]\n\n"; }
const HEALTH = JSON.stringify({ ok: true, model: "qwen3.5-9b", style: "qwen3.5", uncensored: { taken_out: true, checked: 1, refused_before: 0, refused_after: 0 } });

async function newPage(browser, opts) {
  const ctx = await browser.newContext({ viewport: { width: 1600, height: 900 } });
  const engine = { n: 0, sites: opts.sites || [] };
  await ctx.route(/^https?:\/\//, (route) => {
    const u = new URL(route.request().url());
    if (u.host === "127.0.0.1:8766") {
      if (u.pathname === "/health") return route.fulfill({ status: 200, contentType: "application/json", body: HEALTH });
      const body = JSON.parse(route.request().postData() || "{}"), last = (body.messages || []).slice(-1)[0] || {};
      if (last.role === "tool") return route.fulfill({ status: 200, contentType: "text/event-stream", body: textTurn("There.") });
      const site = engine.sites[engine.n++] || "https://example.com/";
      return route.fulfill({ status: 200, contentType: "text/event-stream", body: toolTurn("open_website", JSON.stringify({ url: site })) });
    }
    return route.fulfill({ status: 404, body: "" });       // (nothing else leaves: the weather, Wikipedia...)
  });
  await ctx.addInitScript(standIn, opts);
  const page = await ctx.newPage();
  page.on("pageerror", (e) => out.errors.push(String(e)));
  return { ctx, page };
}

const calls = (page) => page.evaluate(() => window.__app.calls.filter((c) => c[0] !== "search_files" && c[0] !== "place_site"));
const shownPanel = (page) => page.evaluate(() => ["choose", "way-chrome", "way-browser", "way-os"].filter((id) => !document.getElementById(id).hidden));
const ways = (page) => page.evaluate(() => Array.from(document.querySelectorAll(".way")).map((b) => ({
  way: b.getAttribute("data-way"), name: b.querySelector(".way-name").textContent, what: b.querySelector(".way-what").textContent,
  now: b.classList.contains("now"), cannot: b.classList.contains("cannot") })));
const strip = (page) => page.evaluate(() => {
  const s = document.getElementById("tabs");
  if (!s) return null;
  const frame = document.getElementById("frame").getBoundingClientRect(), bar = document.getElementById("frame-bar");
  return { tabs: Array.from(s.querySelectorAll(".tab")).map((t) => [+t.getAttribute("data-tab"), t.querySelector(".tab-title").textContent, t.classList.contains("on")]),
           bottom: Math.round(s.getBoundingClientRect().bottom), frame_top: Math.round(frame.top), bar: getComputedStyle(bar).display,
           content: document.body.classList.contains("content"), back: s.querySelector("#tab-back").disabled };
});
async function typeIn(page, text) {
  await page.click("#input");
  await page.keyboard.type(text, { delay: 5 });
  await page.keyboard.press("Enter");
}
const lastCall = async (page, name) => (await calls(page)).filter((c) => c[0] === name).pop() || null;

(async () => {
  const browser = await chromium.launch();
  try {
    // ------------------------------------------------------------------------ the first start, on a PC
    let { ctx, page } = await newPage(browser, { system: "windows" });
    await page.goto(START);
    await page.waitForFunction(() => !document.getElementById("choose").hidden);
    out.first = { panels: await shownPanel(page), ways: await ways(page), ask: await page.textContent("#ask"),
                  font: await page.evaluate(() => getComputedStyle(document.body).fontFamily),
                  ground: await page.evaluate(() => getComputedStyle(document.body).backgroundColor) };
    // The app: the conversation, at once.
    await page.click('.way[data-way="app"]');
    await page.waitForURL(/\/index\.html$/);
    await page.waitForFunction(() => window.CO && document.getElementById("greeting").classList.contains("shown"));
    out.app = { calls: (await calls(page)).map((c) => c[0]), tabs: await strip(page) };
    // Opened again to change it ("/" lists it last), the app marked as now.
    await page.click("#input");
    await page.keyboard.type("/");
    await page.waitForFunction(() => document.querySelectorAll("#results .res").length > 1);
    out.app_slash = await page.evaluate(() => Array.from(document.querySelectorAll("#results .res:not(.ask) .res-name")).map((e) => e.textContent));
    await page.keyboard.press("Escape");
    // (an address typed, in the app: the one site in the middle, as before)
    await typeIn(page, "bbc.co.uk");
    await page.waitForFunction(() => document.body.classList.contains("content"));
    out.app_site = (await calls(page)).filter((c) => /site|tab/.test(c[0])).map((c) => [c[0], c[1].url || ""]);
    await page.goto(START + "?change");
    await page.waitForFunction(() => !document.getElementById("choose").hidden && document.querySelector(".way.now"));
    out.change = { ways: (await ways(page)).filter((w) => w.now).map((w) => w.way) };
    // The Chrome extension: its folder's path copied in the click; its steps; Chrome's own address copied; the folder shown.
    await page.click('.way[data-way="chrome"]');
    await page.waitForFunction(() => !document.getElementById("way-chrome").hidden);
    out.chrome = { panels: await shownPanel(page), folder: await page.textContent("#extension-folder"),
                   open: await page.isVisible("#chrome-open"), copied_note: await page.isVisible("#extension-copied"),
                   no_chrome: await page.isVisible("#no-chrome"), steps: await page.$$eval(".steps li", (l) => l.map((x) => x.innerText.replace(/\s+/g, " ").trim())),
                   how: await page.$$eval("#way-chrome .how", (l) => l.filter((x) => x.offsetParent).map((x) => x.textContent.replace(/\s+/g, " ").trim())) };
    await page.click('button[data-copy="chrome://extensions"]');
    await page.click('button[data-act="extension-folder"]');
    out.chrome.copied = await page.evaluate(() => window.__app.copied);
    out.chrome.calls = (await calls(page)).filter((c) => c[0] !== "way_status" && c[0] !== "about" && c[0] !== "model_status" && c[0] !== "take_opening");
    // The browser: asked to be the default (Windows' words), then told it is; then the conversation, with tabs.
    await page.click('button[data-act="choose"]');
    out.change_again = (await ways(page)).filter((w) => w.now).map((w) => w.way);
    await page.click('.way[data-way="browser"]');
    await page.waitForFunction(() => !document.getElementById("way-browser").hidden);
    out.browser = { ask: await page.isVisible("#default-ask"), words: (await page.innerText("#default-ask")).replace(/\s+/g, " ") };
    await page.click('button[data-act="default"]');
    await page.waitForFunction(() => !document.getElementById("default-yes").hidden, null, { timeout: 8000 });
    out.browser.yes = await page.textContent("#default-yes");
    out.browser.asked = await page.textContent("#default-asked");
    // The own system, on a PC: Chat Computer Setup started.
    await page.click('#way-browser button[data-act="choose"]');
    await page.click('.way[data-way="os"]');
    await page.waitForFunction(() => !document.getElementById("way-os").hidden);
    out.os = { started: await page.isVisible("#os-started"), words: (await page.innerText("#way-os")).replace(/\s+/g, " "),
               folder: await page.textContent("#setup-folder") };
    // Started again on the extension: at its steps, nothing done again by itself.
    await page.evaluate(() => localStorage.setItem("__way", "chrome"));
    await page.goto(START);
    await page.waitForFunction(() => !document.getElementById("way-chrome").hidden);
    out.again = { panels: await shownPanel(page), open: await page.isVisible("#chrome-open"),
                  chose: (await calls(page)).filter((c) => c[0] === "choose_way").length };
    await ctx.close();

    // ------------------------------------------------------------------------ a Mac with M4: no own system
    ({ ctx, page } = await newPage(browser, { system: "mac" }));
    await page.goto(START);
    await page.waitForFunction(() => !document.getElementById("choose").hidden);
    const before = (await calls(page)).length;
    await page.click('.way[data-way="os"]', { force: true });         // (shown as not for this Mac, but it says why)
    out.mac = { os: (await ways(page)).filter((w) => w.way === "os")[0], error: await page.textContent("#error"),
                asked: (await calls(page)).slice(before).map((c) => c[0]), panels: await shownPanel(page) };
    await page.click('.way[data-way="chrome"]');
    await page.waitForFunction(() => !document.getElementById("way-chrome").hidden);
    out.mac.how = await page.$$eval("#way-chrome .how", (l) => l.filter((x) => x.offsetParent).map((x) => x.textContent.replace(/\s+/g, " ").trim()));
    await ctx.close();

    // ------------------------------------------------------------------------ the browser
    ({ ctx, page } = await newPage(browser, { system: "windows", way: "browser", opening: ["https://example.org/from-another-app"],
                                              sites: ["https://www.youtube.com/watch?v=aaaaaaaaaaa", "https://www.youtube.com/watch?v=bbbbbbbbbbb"] }));
    await page.goto("file://" + path.join(dir, "index.html"));
    await page.waitForFunction(() => document.getElementById("tabs") && document.body.classList.contains("content"));
    out.tabs = { outside: await strip(page), opened: await lastCall(page, "open_tab") };
    out.tabs.layout = await page.evaluate(() => ({ tabbed: document.body.classList.contains("tabbed"),
      hole: (window.__app.calls.filter((c) => c[0] === "place_site").pop() || [0, {}])[1] }));
    // An address typed: a new tab.
    await typeIn(page, "bbc.co.uk");
    await page.waitForFunction(() => document.querySelectorAll("#tabs .tab").length === 3);
    out.tabs.typed = { strip: await strip(page), opened: await lastCall(page, "open_tab") };
    // The app says the tab's title.
    await page.evaluate(() => window.chatcomputerApp({ title: "BBC - Home", view: "tab-2" }));
    await page.waitForFunction(() => /BBC - Home/.test(document.getElementById("tab-list").textContent));
    // The first tab chosen again; back and forward in it.
    await page.click('#tab-list .tab[data-tab="1"]');
    await page.waitForFunction(() => document.querySelector('#tab-list .tab[data-tab="1"]').classList.contains("on"));
    await page.click("#tab-back");
    await page.click("#tab-forward");
    out.tabs.picked = { strip: await strip(page), shown: await lastCall(page, "show_tab"),
                        went: (await calls(page)).filter((c) => c[0] === "tab_back").map((c) => c[1]) };
    // The conversation's tab: the tab put away (still open), the conversation in the middle.
    await page.click('#tab-list .tab[data-tab="0"]');
    await page.waitForFunction(() => !document.body.classList.contains("content"));
    out.tabs.home = { strip: await strip(page), hid: (await calls(page)).filter((c) => c[0] === "hide_site").length,
                      column: await page.evaluate(() => parseFloat(getComputedStyle(document.getElementById("column")).paddingTop)) };
    // + : the conversation, the box ready for an address.
    await page.click('#tab-list .tab[data-tab="2"]');
    await page.waitForFunction(() => document.body.classList.contains("content"));
    await page.click("#tab-new");
    await page.waitForFunction(() => !document.body.classList.contains("content"));
    out.tabs.new_tab = await page.evaluate(() => ({ focused: document.activeElement && document.activeElement.id,
                                                    hint: document.getElementById("input").placeholder }));
    // Chat Computer's model opens a site: a tab of its own; then another: in the same tab.
    await typeIn(page, "play something calm");
    await page.waitForFunction(() => document.querySelectorAll("#tabs .tab").length === 4 && document.body.classList.contains("content"), null, { timeout: 15000 });
    out.tabs.model_first = { opened: await lastCall(page, "open_tab"), strip: await strip(page) };
    await page.click('#tab-list .tab[data-tab="0"]');
    await page.waitForFunction(() => !document.body.classList.contains("content"));
    await typeIn(page, "play something else");
    await page.waitForFunction(() => window.__app.calls.filter((c) => c[0] === "open_tab").length === 4, null, { timeout: 15000 });
    await page.waitForFunction(() => document.body.classList.contains("content"));
    out.tabs.model_again = { opened: await lastCall(page, "open_tab"), strip: await strip(page) };
    // Escape: the conversation again, the tab still open.
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => !document.body.classList.contains("content"));
    out.tabs.escape = { strip: await strip(page), closed: (await calls(page)).filter((c) => c[0] === "close_tab").length };
    // A tab's x: closed; the one beside it shown.
    await page.click('#tab-list .tab[data-tab="2"]');
    await page.waitForFunction(() => document.querySelector('#tab-list .tab[data-tab="2"]').classList.contains("on"));
    await page.click('#tab-list .tab[data-tab="2"] .tab-close');
    await page.waitForFunction(() => !document.querySelector('#tab-list .tab[data-tab="2"]'));
    out.tabs.closed = { strip: await strip(page), closed: await lastCall(page, "close_tab"), shown: await lastCall(page, "show_tab") };
    // Ctrl+T: a new tab (the conversation, the box ready).
    await page.keyboard.press("Control+t");
    await page.waitForFunction(() => !document.body.classList.contains("content"));
    out.tabs.ctrl_t = (await strip(page)).tabs.filter((t) => t[2]).map((t) => t[0]);
    await ctx.close();
  } catch (e) {
    out.failed = String(e && e.stack || e).slice(0, 1500);
  } finally {
    await browser.close();
  }
  console.log(JSON.stringify(out));
})();
