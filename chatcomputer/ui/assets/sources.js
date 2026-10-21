/* Reads the list of conversations on WhatsApp's, WeChat's and the social sites' own inbox pages, for Messages
   (chatcomputer/messages/web.py). It runs in a script world of its own (the page's scripts can't see or call it), only
   reads what the page shows, and never types, sends or clicks by itself; __CS.open(name) clicks a conversation in
   the list only when the person asked to see it.

   Each conversation in the list: who, the latest message (its words), when (as the page writes it), unread, and its
   address when it has one. Sent to Chat Computer whenever the list changes, at most every 1.5 seconds. These pages
   change often; "found" says how many list items were seen, for the log. */
(function () {
  "use strict";
  if (window.__CS) return;
  var host = location.hostname.replace(/^www\./, "");
  var SITES = {
    "web.whatsapp.com": { items: "#pane-side [role='listitem'], #pane-side [role='row']",
                          login: "canvas[aria-label], [data-ref], [data-testid='qrcode']" },
    "x.com": { items: "[data-testid='conversation']", login: "input[autocomplete='username'], input[name='text'], a[href='/login']" },
    "twitter.com": { items: "[data-testid='conversation']", login: "input[autocomplete='username'], a[href='/login']" },
    "instagram.com": { items: "a[href^='/direct/t/'], [role='listitem'] [role='button']", login: "input[name='username']" },
    "facebook.com": { items: "a[href*='/messages/t/'], a[href*='/messages/e2ee/t/']", login: "input[name='email'], #email" },
    "messenger.com": { items: "a[href*='/t/']", login: "input[name='email']" },
    "tiktok.com": { items: "[data-e2e='chat-list-item'], [class*='DivItemWrapper']", login: "[data-e2e='top-login-button'], [data-e2e='login-modal']" },
    "linkedin.com": { items: ".msg-conversation-listitem, .msg-conversations-container__convo-item", login: "#username, input[name='session_key']" }
  };
  // WeChat for Web (web.wechat.com, which is wx.qq.com, or wx2.qq.com for some accounts): each conversation a
  // .chat_item, its name in .nickname_text, a red dot or count when there's something unread.
  var WECHAT = { items: ".chat_item", name: ".nickname_text",
                 unread: ".web_wechat_reddot, .web_wechat_reddot_middle, .web_wechat_reddot_bbig", login: ".login_box .qrcode" };
  ["web.wechat.com", "wx.qq.com", "wx2.qq.com", "wx8.qq.com", "web2.wechat.com"].forEach(function (h) { SITES[h] = WECHAT; });
  var rule = SITES[host] || { items: "[role='listitem'], [role='row']", login: "input[type='password']" };
  var TIME = /^(\d{1,2}[:.]\d{2}(\s?[ap]\.?m\.?)?|yesterday|today|now|just now|active now|\d+\s?(s|m|min|mins|h|hr|hrs|d|w|wk|y)|(mon|tue|wed|thu|fri|sat|sun)[a-z]*|\d{1,2}[\/\-]\d{1,2}([\/\-]\d{2,4})?|[a-z]{3,9}\.? \d{1,2}(, \d{4})?|\d{1,2} [a-z]{3,9}\.?( \d{4})?)$/i;

  function post(msg) {
    try { window.webkit.messageHandlers.chatcomputerSource.postMessage(JSON.stringify(msg)); } catch (e) {}
  }
  function lines(el) {
    return (el.innerText || "").split("\n").map(function (s) { return s.replace(/\s+/g, " ").trim(); })
      .filter(function (s) { return s; });
  }
  // One conversation in the list: its lines are who, when, the latest words, sometimes a count of unread ones.
  function parse(it) {
    var ls = lines(it);
    if (ls.length < 2) return null;
    var named = rule.name && it.querySelector(rule.name), nameEl = it.querySelector("span[title], [dir='auto'][title]");
    var name = (named && named.textContent.replace(/\s+/g, " ").trim()) || (nameEl && nameEl.getAttribute("title")) || ls[0];
    var time = "", rest = [];
    ls.forEach(function (l) {
      if (l === name) return;
      var t = l.replace(/^[·•]\s*/, "");
      if (!time && TIME.test(t)) { time = t; return; }
      var m = t.match(/^(.*?)\s*[·•]\s*(\S+(?:\s\S+)?)$/);         // "You: ok · 2h"
      if (m && !time && TIME.test(m[2])) { time = m[2]; if (m[1]) rest.push(m[1]); return; }
      rest.push(t);
    });
    var badge = rest.filter(function (l) { return /^\d{1,3}$/.test(l); });
    var words = rest.filter(function (l) { return !/^\d{1,3}$/.test(l) && !/^@\S+$/.test(l); });
    var text = words.sort(function (a, b) { return b.length - a.length; })[0] || "";
    var unread = badge.length > 0 || !!it.querySelector("[aria-label*='unread' i]") || !!(rule.unread && it.querySelector(rule.unread));
    var a = it.matches("a[href]") ? it : (it.closest("a[href]") || it.querySelector("a[href]"));
    return { name: name.slice(0, 120), text: text.slice(0, 300), time: time, unread: unread, link: a ? a.href : "" };
  }
  function items() {
    return Array.prototype.slice.call(document.querySelectorAll(rule.items), 0, 60);
  }
  function read() {
    var out = [], seen = {};
    items().forEach(function (it) {
      var c = parse(it);
      if (!c || !c.text || seen[c.name]) return;
      seen[c.name] = 1;
      out.push(c);
    });
    return out;
  }
  var last = "", timer = 0;
  function report() {
    timer = 0;
    if (location.protocol !== "https:") return;
    var found = items().length, chats = read();
    var state = chats.length ? "ready" : (document.querySelector(rule.login) ? "login" : "loading");
    var key = state + JSON.stringify(chats);
    if (key === last) return;
    last = key;
    post({ type: "snapshot", state: state, chats: chats.slice(0, 40), url: location.href, found: found });
  }
  window.__CS = {
    // The person asked to see this conversation: it's opened the way a click on it would.
    open: function (name) {
      var target = null;
      items().forEach(function (it) { var c = parse(it); if (!target && c && c.name === name) target = it; });
      if (!target) return false;
      ["mousedown", "mouseup", "click"].forEach(function (t) {
        target.dispatchEvent(new MouseEvent(t, { bubbles: true, cancelable: true, view: window }));
      });
      return true;
    },
    read: read
  };
  new MutationObserver(function () { if (!timer) timer = setTimeout(report, 1500); })
    .observe(document.documentElement, { childList: true, subtree: true, characterData: true });
  setInterval(report, 15000);
  setTimeout(report, 2000);
})();
