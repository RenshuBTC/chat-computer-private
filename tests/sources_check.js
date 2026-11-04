// The chat sites' page reader (chatcomputer/ui/assets/sources.js) on a stand-in for WeChat for Web's conversation
// list, in its own markup (as web.wechat.com serves it: .chat_item, .nickname_text, the red dot and count), and on
// its sign-in page (the code to scan). Prints JSON.  Run: node tests/sources_check.js
const fs = require("fs");
const path = require("path");
const { chromium } = require("playwright");

const READER = fs.readFileSync(path.join(__dirname, "..", "chatcomputer", "ui", "assets", "sources.js"), "utf8");
function item(name, time, text, count, muted) {
  return `<div class="chat_item slide-left" data-username="@${name}"><div class="ext"><p class="attr">${time}</p>` +
    (muted ? `<p class="attr"><i class="web_wechat_no-remind"></i></p>` : "") + `</div>` +
    `<div class="avatar"><img class="img" src="data:,">` +
    (count ? (muted ? `<i class="icon web_wechat_reddot"></i>` : `<i class="icon web_wechat_reddot_middle">${count}</i>`) : "") +
    `</div><div class="info"><h3 class="nickname"><span class="nickname_text">${name}</span></h3>` +
    `<p class="msg"><span>${text}</span></p></div></div>`;
}
const LIST = `<!doctype html><title>WeChat/Weixin for Web</title><div class="chat_list">` +
  item("Mum", "13:05", "Dinner at 7? Bring the photos", 3) + item("Alex Tan", "Yesterday", "[Photo]", 0) +
  item("Family group", "10/18/26", "Lin: see you Sunday", 1, true) + `</div>`;
const LOGIN = `<!doctype html><title>WeChat/Weixin for Web</title><div class="login_box"><div class="qrcode">` +
  `<img class="img" src="data:,"><p class="sub_title">Scan to log in to WeChat/Weixin</p></div></div>`;

(async () => {
  const out = {};
  const browser = await chromium.launch();
  try {
    const ctx = await browser.newContext();
    let html = LIST;
    await ctx.route("https://web.wechat.com/**", (r) => r.fulfill({ status: 200, contentType: "text/html", body: html }));
    const page = await ctx.newPage();
    await page.goto("https://web.wechat.com/?lang=en_US");
    await page.addScriptTag({ content: READER });
    out.chats = await page.evaluate(() => window.__CS.read());
    html = LOGIN;
    await page.goto("https://web.wechat.com/?lang=en_US");
    await page.addScriptTag({ content: READER });
    out.login = await page.evaluate(() => ({ chats: window.__CS.read().length, code: !!document.querySelector(".login_box .qrcode") }));
  } catch (e) {
    out.failed = String(e && e.stack || e).slice(0, 1500);
  }
  await browser.close();
  console.log(JSON.stringify(out));
})();
