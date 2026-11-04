// The desktop app's reader (apps/desktop/src-tauri/src/reader.js), in Chromium (the engine of Windows' WebView2), on
// pages built as each service's list is, served at each service's own address (Playwright answers for them: nothing
// goes out): what it reads (who, what, when, unread), what it hands to the app, opening one, and the times it
// understands. Prints JSON.
//
//     NODE_PATH=$(npm root -g) node tests/reader_check.js
const { chromium } = require("playwright");
const fs = require("fs");
const path = require("path");

const READER = fs.readFileSync(path.join(__dirname, "..", "apps", "desktop", "src-tauri", "src", "reader.js"), "utf8");
// Each page records a tap on a conversation in its title, as its own app would open it.
const TAP = "<script>['click', 'mousedown'].forEach(function (t) { document.addEventListener(t, function (e) {" +
  " var r = e.target.closest('[data-name]'); if (r) { e.preventDefault(); document.title = 'opened ' + r.getAttribute('data-name'); } }); });</script>";
const PAGES = {
  gmail: ["https://mail.google.com/mail/u/0/", "<title>Inbox (2) - me@example.com - Gmail</title><div role='main'><table><tbody>" +
    "<tr class='zA zE' data-name='jane'><td class='yX'><div class='yW'><span email='jane@example.com' name='Jane Lim'>Jane Lim</span></div></td>" +
    "<td><div class='xT'><span class='bog'><span data-thread-id='#thread-f:1850'>Lunch on Friday?</span></span>" +
    "<span class='y2'> - 12:30 at the usual place works for me.</span></div></td><td class='xW'><span title='Mon, Nov 2, 2026, 9:05 AM'>9:05 AM</span></td></tr>" +
    "<tr class='zA yO' data-name='team'><td class='yX'><div class='yW'><span email='a@example.com' name='Ann'>Ann</span>, <span email='b@example.com' name='Bo'>Bo</span></div></td>" +
    "<td><div class='xT'><span class='bog'><span data-legacy-thread-id='18a2'>Plans</span></span><span class='y2'> - See you there</span></div></td>" +
    "<td class='xW'><span>Oct 28</span></td></tr></tbody></table></div>"],
  outlook: ["https://outlook.live.com/mail/0/", "<title>Mail - Sam - Outlook</title><div role='listbox' aria-label='Message list'>" +
    "<div role='option' data-convid='AQQk1' aria-label='Unread Sam Lee Trip photos 8:30 AM Here they are' data-name='sam'>" +
    "<div>Sam Lee</div><div>Trip photos</div><div>8:30 AM</div><div>Here they are, all 40.</div></div>" +
    "<div role='option' data-convid='AQQk2' aria-label='City Council Bin day 2026-10-30' data-name='council'>" +
    "<div>City Council</div><div>Bin day</div><div>2026-10-30</div><div>Collections move to Friday.</div></div></div>"],
  telegram: ["https://web.telegram.org/a/", "<title>Telegram</title><div class='chat-list'>" +
    "<div class='ListItem Chat'><a class='ListItem-button' href='#7001' data-name='alex'><div class='info'><div class='title'><h3 class='fullName'>Alex</h3></div>" +
    "<div class='LastMessageMeta'><span class='time'>Yesterday</span></div><div class='subtitle'><p class='last-message'>Sent you the slides for Monday.</p>" +
    "<div class='ChatBadge unread'>2</div></div></div></a></div>" +
    "<div class='ListItem Chat'><a class='ListItem-button' href='#7002' data-name='club'><div class='info'><div class='title'><h3 class='fullName'>Chess club</h3></div>" +
    "<div class='LastMessageMeta'><span class='time'>Sat</span></div><div class='subtitle'><p class='last-message'>Lee: Game at 6?</p>" +
    "<div class='ChatBadge muted'>9</div></div></div></a></div></div>"],
  whatsapp: ["https://web.whatsapp.com/", "<title>(1) WhatsApp</title><div id='pane-side'><div aria-label='Chat list' role='grid'>" +
    "<div role='listitem' data-name='mum'><div role='gridcell'><span dir='auto' title='Mum'>Mum</span><div>10:15</div>" +
    "<span title='Dinner at 7 tonight? Bring the photos!'>Dinner at 7 tonight? Bring the photos!</span><span aria-label='1 unread message'>1</span></div></div>" +
    "<div role='listitem' data-name='dad'><div role='gridcell'><span dir='auto' title='Dad'>Dad</span><div>11/2/2026</div>" +
    "<span title='Call me back'>Call me back</span></div></div></div></div>"],
  wechat: ["https://wx2.qq.com/", "<title>微信网页版</title><div class='chat_list'>" +
    "<div class='chat_item' data-username='@abc' data-name='li'><div class='info'><h3 class='nickname'><span class='nickname_text'>李明</span></h3>" +
    "<p class='msg'><span>明天见</span></p></div><div class='ext'><p class='attr'>下午3:20</p></div>" +
    "<i class='icon web_wechat_reddot_middle'>1</i></div>" +
    "<div class='chat_item' data-username='@def' data-name='wang'><div class='info'><h3 class='nickname'><span class='nickname_text'>王芳</span></h3>" +
    "<p class='msg'><span>好的</span></p></div><div class='ext'><p class='attr'>昨天</p></div></div></div>"],
  discord: ["https://discord.com/channels/@me", "<title>(3) Discord | Friends</title><nav aria-label='Private channels'><ul>" +
    "<li><a href='/channels/@me/5551' aria-label='unread, Priya (direct message)' data-name='priya'><div>Priya</div></a></li>" +
    "<li><a href='/channels/@me/5552' aria-label='Tom (direct message)' data-name='tom'><div>Tom</div></a></li></ul></nav>"],
  // A list whose parts aren't named as expected (its service changed its page): read by its shape.
  changed: ["https://web.whatsapp.com/v2/", "<title>WhatsApp</title><main><ul class='x1'>" +
    "<li data-name='kim'><div>Kim</div><div>9:41</div><div>On my way</div></li>" +
    "<li data-name='joe'><div>Joe</div><div>Yesterday</div><div>Thanks!</div></li>" +
    "<li data-name='eve'><div>Eve</div><div>Tue</div><div>See the doc</div></li></ul></main>"],
  // Signed out: nothing listed, and the title's count alone.
  title_only: ["https://mail.google.com/mail/u/1/", "<title>Inbox (12) - you@example.com - Gmail</title><p>Loading…</p>"]
};

(async () => {
  const out = { errors: [] };
  const browser = await chromium.launch();
  try {
    const ctx = await browser.newContext({ timezoneId: "Europe/London", locale: "en-GB" });
    await ctx.route("**/*", (route) => {
      const url = route.request().url();
      const hit = Object.keys(PAGES).filter((k) => PAGES[k][0] === url)[0];
      route.fulfill(hit ? { status: 200, contentType: "text/html; charset=utf-8", body: "<!doctype html><meta charset='utf-8'>" + PAGES[hit][1] + TAP }
        : { status: 404, body: "" });
    });
    // What the app is handed (Tauri's invoke, standing in), and the reader, as the app puts it in each page.
    await ctx.addInitScript(() => {
      window.__handed = [];
      window.__TAURI_INTERNALS__ = { invoke: (cmd, args) => { window.__handed.push([cmd, args]); return Promise.resolve(null); } };
    });
    await ctx.addInitScript({ content: READER });
    for (const k of Object.keys(PAGES)) {
      const page = await ctx.newPage();
      page.on("pageerror", (e) => out.errors.push(k + ": " + e.message));
      await page.goto(PAGES[k][0]);
      await page.waitForFunction(() => window.__handed.length > 0, null, { timeout: 8000 });
      const r = await page.evaluate(() => ({ handed: window.__handed, read: window.__chatcomputerRead() }));
      const items = r.read.items;
      out[k] = {
        command: r.handed[0][0],
        unread_in_title: r.read.unread,
        items: items.map((i) => [i.who, i.text, i.when, i.unread, typeof i.ts === "number"]),
        ids: items.map((i) => i.id)
      };
      // Opening the first: as a tap on it.
      if (items.length) {
        out[k].opened = await page.evaluate((id) => window.__chatcomputerOpen(id), items[0].id);
        out[k].title_after = await page.title();
      }
      await page.close();
    }
    // The times it understands, against a fixed now: Tuesday 3 November 2026, 14:00, London.
    const page = await ctx.newPage();
    await page.goto(PAGES.whatsapp[0]);
    out.when = await page.evaluate(() => {
      const now = new Date(2026, 10, 3, 14, 0, 0);
      const iso = (s, full) => { const t = window.__chatcomputerWhen(s, full, now); return t === null ? null : new Date(t * 1000).toString().slice(0, 21); };
      return ["10:15", "9:05 AM", "3:20 PM", "16:45", "下午3:20", "上午9:00", "Yesterday", "昨天", "Sat", "星期一", "Oct 28",
              "2 Nov", "11/2/2026", "2026-10-30", "10月30日", "5m", "2h", "just now", "sometime", ""].map((s) => [s, iso(s)])
        .concat([["9:05 AM + its date", iso("9:05 AM", "Mon, Nov 2, 2026, 9:05 AM")]]);
    });
    await page.close();
  } catch (e) {
    out.failed = String(e && e.stack || e).slice(0, 1500);
  } finally {
    await browser.close();
  }
  console.log(JSON.stringify(out));
})();
