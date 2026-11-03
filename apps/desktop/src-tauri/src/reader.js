// Chat Computer's reader, in the view of a service you've connected (Gmail, Outlook, Telegram, WhatsApp, WeChat,
// Discord): every few seconds it reads the list of conversations on the page (who, the last message or the
// subject and its start, when, unread or not) and hands it to the app, which shows it in Messages. It reads only;
// it sends nothing anywhere but the app (messages.rs), and only from the service's own pages.
//
// Each service's list is found by what its page is made of; where that isn't found (the page has changed), by its
// shape: rows of a list, each with a time in it. Where no list is found at all, the number of unread messages the
// page's title shows is still handed over.
(function () {
  "use strict";
  if (window.top !== window || window.__chatcomputerReader) return;
  window.__chatcomputerReader = true;

  var HOSTS = {
    gmail: /^mail\.google\.com$/,
    outlook: /^outlook\.(live|office|office365)\.com$/,
    telegram: /^web\.telegram\.org$/,
    whatsapp: /^web\.whatsapp\.com$/,
    wechat: /^(web\d?\.wechat\.com|wx\d?\.qq\.com)$/,
    discord: /^discord\.com$/
  };
  var service = null;
  for (var k in HOSTS) if (HOSTS[k].test(location.hostname)) service = k;
  // (Chat Computer's own tests point a service at a page of their own, named after it.)
  var test = /^(127\.0\.0\.1|localhost)$/.test(location.hostname) && /\/(gmail|outlook|telegram|whatsapp|wechat|discord)\.html$/.exec(location.pathname);
  if (test) service = test[1];
  if (!service) return;

  var MOST = 60;
  function all(sel, root) {
    try { return Array.prototype.slice.call((root || document).querySelectorAll(sel)); } catch (e) { return []; }
  }
  function one(sel, root) { try { return (root || document).querySelector(sel); } catch (e) { return null; } }
  // The first of these that's there, in this order (a list of selectors in one finds the first in the page's order).
  function first(root, sels) {
    for (var i = 0; i < sels.length; i++) { var e = one(sels[i], root); if (e) return e; }
    return null;
  }
  function words(e) { return e ? String(e.innerText || e.textContent || "").replace(/\s+/g, " ").trim() : ""; }
  function attr(e, name) { return e ? String(e.getAttribute(name) || "").trim() : ""; }
  function lines(e) {
    var out = [];
    String((e && (e.innerText || e.textContent)) || "").split(/[\n\t]+/).forEach(function (l) {
      l = l.replace(/\s+/g, " ").trim();
      if (l && out[out.length - 1] !== l) out.push(l);
    });
    return out;
  }
  // A short name for a conversation: FNV-1a of what tells it apart, in base 36.
  function key(s) {
    var h = 0x811c9dc5;
    s = String(s);
    for (var i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 0x01000193) >>> 0; }
    return service + ":" + h.toString(36);
  }

  // ---------------------------------------------------------------- when
  var MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
  var DAYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
  var CN_DAYS = "日一二三四五六";
  var TIME = /^(上午|下午)?\s*(\d{1,2})[:.](\d{2})\s*([ap]\.?\s?m\.?)?$/i;
  function isWhen(s) {
    s = String(s || "").trim().toLowerCase();
    if (!s || s.length > 30) return false;
    return TIME.test(s) || /^(yesterday|today|now|just now|昨天|今天|前天)$/.test(s) ||
      /^\d+\s?(s|m|min|mins|h|hr|hrs|d|w)( ago)?$/.test(s) ||
      /^(星期|周)[日天一二三四五六]$/.test(s) ||
      new RegExp("^(" + DAYS.join("|") + ")[a-z]*\\.?$").test(s) ||
      /^\d{1,4}[\/.-]\d{1,2}([\/.-]\d{1,4})?$/.test(s) ||
      new RegExp("^(" + MONTHS.join("|") + ")[a-z]*\\.?\\s\\d{1,2}(,?\\s\\d{4})?$").test(s) ||
      new RegExp("^\\d{1,2}\\s(" + MONTHS.join("|") + ")[a-z]*\\.?(\\s\\d{4})?$").test(s) ||
      /^(\d{4}年)?\d{1,2}月\d{1,2}日$/.test(s);
  }
  // Seconds since 1970, from the time a list shows (or the full date in its title), or null.
  function when(s, full, now) {
    now = now || new Date();
    if (full) {
      var f = Date.parse(full.replace(/\s+at\s+/i, " ").replace(/ /g, " "));
      if (!isNaN(f)) return f / 1000;
    }
    s = String(s || "").trim();
    var low = s.toLowerCase(), d = new Date(now.getTime()), m;
    if (!s) return null;
    if (/^(now|just now)$/.test(low)) return now.getTime() / 1000;
    if ((m = /^(\d+)\s?(s|m|min|mins|h|hr|hrs|d|w)( ago)?$/.exec(low))) {
      var unit = { s: 1, m: 60, min: 60, mins: 60, h: 3600, hr: 3600, hrs: 3600, d: 86400, w: 604800 }[m[2]];
      return now.getTime() / 1000 - (+m[1]) * unit;
    }
    if ((m = TIME.exec(low))) {
      var h = +m[2] % (m[4] || m[1] ? 12 : 24), min = +m[3];
      if ((m[4] && /^p/.test(m[4])) || m[1] === "下午") h += 12;
      d.setHours(h, min, 0, 0);
      if (d.getTime() > now.getTime() + 300000) d.setDate(d.getDate() - 1);      // (a time later than now: yesterday's)
      return d.getTime() / 1000;
    }
    d.setHours(12, 0, 0, 0);
    if (/^(yesterday|昨天)$/.test(low)) { d.setDate(d.getDate() - 1); return d.getTime() / 1000; }
    if (/^(today|今天)$/.test(low)) return d.getTime() / 1000;
    if (low === "前天") { d.setDate(d.getDate() - 2); return d.getTime() / 1000; }
    var day = -1;
    DAYS.forEach(function (n, i) { if (low.indexOf(n) === 0) day = i; });
    if ((m = /^(星期|周)([日天一二三四五六])$/.exec(s))) day = m[2] === "天" ? 0 : CN_DAYS.indexOf(m[2]);
    if (day >= 0) {
      var back = (d.getDay() - day + 7) % 7 || 7;
      d.setDate(d.getDate() - back);
      return d.getTime() / 1000;
    }
    // A date: the day shown alone (no year: this year's, or last year's if that would be ahead), as the browser's own
    // parser guesses a year for those (2001).
    function date(y, mo, dd) {
      var t = new Date(y === null ? now.getFullYear() : y < 100 ? 2000 + y : y, mo, dd, 12, 0, 0, 0);
      if (y === null && t.getTime() > now.getTime() + 86400000) t.setFullYear(t.getFullYear() - 1);
      return isNaN(t.getTime()) || mo < 0 || mo > 11 || dd < 1 || dd > 31 ? null : t.getTime() / 1000;
    }
    function month(w) { return MONTHS.indexOf(String(w).slice(0, 3).toLowerCase()); }
    if ((m = /^(?:(\d{4})年)?(\d{1,2})月(\d{1,2})日$/.exec(s))) return date(m[1] ? +m[1] : null, +m[2] - 1, +m[3]);
    if ((m = /^([a-z]{3,})\.?\s(\d{1,2})(?:,?\s(\d{4}))?$/i.exec(s)) && month(m[1]) >= 0) return date(m[3] ? +m[3] : null, month(m[1]), +m[2]);
    if ((m = /^(\d{1,2})\s([a-z]{3,})\.?(?:,?\s(\d{4}))?$/i.exec(s)) && month(m[2]) >= 0) return date(m[3] ? +m[3] : null, month(m[2]), +m[1]);
    if ((m = /^(\d{4})[\/.-](\d{1,2})[\/.-](\d{1,2})$/.exec(s))) return date(+m[1], +m[2] - 1, +m[3]);
    if ((m = /^(\d{1,2})[\/.-](\d{1,2})(?:[\/.-](\d{2,4}))?$/.exec(s))) {
      // (Month first where the language is America's, the Philippines' or Canada's English; day first elsewhere.)
      var us = /^(en-(us|ph|ca)|es-us)$/i.test(navigator.language || "");
      return date(m[3] ? +m[3] : null, (us ? +m[1] : +m[2]) - 1, us ? +m[2] : +m[1]);
    }
    // Anything else with a year or a month's name in it, as the browser reads it; nothing else.
    if (/\d{4}/.test(s) || MONTHS.some(function (n) { return low.indexOf(n) >= 0; })) {
      var p = Date.parse(s.replace(/,/g, " "));
      if (!isNaN(p)) return p / 1000;
    }
    return null;
  }

  // ---------------------------------------------------------------- each service's list
  // A row, read by its lines: the time (the line that is one), who (the first other), what (the rest). An unread
  // count shown as a number alone is left out of what's said.
  function byLines(row, extra) {
    var ls = lines(row), at = "", rest = [];
    ls.forEach(function (l) {
      if (!at && isWhen(l)) at = l;
      else if (!/^\d{1,4}$/.test(l) && !/^(unread|pinned|muted|draft)$/i.test(l)) rest.push(l);
    });
    var label = attr(row, "aria-label");
    return {
      who: rest[0] || "", text: rest.slice(1).join(" – "), when: at,
      unread: /\bunread\b/i.test(label) || (extra && extra.unread) || false
    };
  }
  var READ = {
    gmail: function () {
      return all("tr.zA").map(function (tr) {
        var who = one(".yW [email], .yW [name]", tr), subject = words(one(".bog", tr));
        var start = words(one(".y2", tr)).replace(/^[-–—]\s*/, "");
        var at = first(tr, ["td.xW span[title]", ".xW span[title]", "td.xW span"]);
        var id = first(tr, ["[data-thread-id]", "[data-legacy-thread-id]"]);
        var name = all(".yW [email], .yW [name]", tr).map(function (e) { return attr(e, "name") || words(e); })
          .filter(Boolean).join(", ") || words(one(".yW", tr));
        return {
          key: id ? attr(id, "data-thread-id") || attr(id, "data-legacy-thread-id") : (who ? attr(who, "email") : "") + subject,
          who: name, text: subject + (start ? " – " + start : ""), when: words(at), full: attr(at, "title"),
          unread: tr.classList.contains("zE"), el: tr
        };
      });
    },
    outlook: function () {
      return all("[role='listbox'] [role='option'], div[data-convid]").map(function (row) {
        var r = byLines(row);
        r.key = attr(row, "data-convid") || attr(row, "id") || r.who + r.text;
        r.unread = r.unread || /^unread\b/i.test(attr(row, "aria-label"));
        r.el = row;
        return r;
      });
    },
    telegram: function () {
      var a = all(".chat-list .ListItem.Chat").map(function (row) {          // Telegram's web app, version A
        var link = one("a[href^='#']", row), who = words(first(row, [".fullName", "h3", ".title"]));
        var badge = one(".ChatBadge:not(.muted)", row);
        return {
          key: attr(link, "href") || who, who: who, text: words(first(row, [".last-message", ".subtitle"])),
          when: words(first(row, [".LastMessageMeta .time", ".time"])), unread: !!(badge && /\d/.test(words(badge))),
          el: link || row
        };
      });
      if (a.length) return a;
      return all(".chatlist-chat, a[data-peer-id]").map(function (row) {     // and version K
        var badge = first(row, [".badge-unread", ".dialog-subtitle-badge-unread"]);
        return {
          key: attr(row, "data-peer-id") || words(one(".peer-title", row)), who: words(one(".peer-title", row)),
          text: words(first(row, [".row-subtitle", ".dialog-subtitle"])), when: words(first(row, [".message-time", ".row-title-right"])),
          unread: !!(badge && /\d/.test(words(badge))), el: row
        };
      });
    },
    whatsapp: function () {
      return all("#pane-side [role='listitem'], #pane-side [role='row'], [aria-label='Chat list'] [role='listitem']").map(function (row) {
        var titled = all("span[title]", row).map(function (e) { return attr(e, "title"); }).filter(Boolean);
        var r = byLines(row);
        if (titled.length) r.who = titled[0];
        if (titled.length > 1) r.text = titled[titled.length - 1];
        r.unread = r.unread || !!one("[aria-label*='unread' i]", row);
        r.key = r.who;
        r.el = row;
        return r;
      });
    },
    wechat: function () {
      return all(".chat_list .chat_item, div.chat_item").map(function (row) {
        var dot = first(row, [".web_wechat_reddot_middle", ".web_wechat_reddot"]);
        var who = words(first(row, [".nickname_text", ".nickname"]));
        return {
          key: attr(row, "data-username") || who, who: who, text: words(first(row, [".msg span", ".msg"])),
          when: words(first(row, [".ext .attr", ".time"])), unread: !!dot && dot.offsetParent !== null, el: row
        };
      });
    },
    discord: function () {
      return all("a[href^='/channels/@me/']").map(function (a) {
        var label = attr(a, "aria-label"), name = lines(a)[0] || label.split(",")[0];
        var unread = /\bunread\b|\bmention/i.test(label) || !!one("[class*='numberBadge'], [class*='unread']", a.parentElement || a);
        return { key: attr(a, "href"), who: name, text: unread ? "New message" : "Direct message", when: "", unread: unread, el: a };
      });
    }
  };
  // The shape of a list, where the service's own wasn't found: the most rows under one parent that have a time.
  function anyList() {
    var groups = new Map(), best = [];
    all("[role='row'], [role='listitem'], [role='option'], li, tr").forEach(function (row) {
      var ls = lines(row);
      if (ls.length < 2 || ls.length > 8 || !ls.some(isWhen)) return;
      var g = groups.get(row.parentElement) || [];
      g.push(row);
      groups.set(row.parentElement, g);
    });
    groups.forEach(function (g) { if (g.length > best.length) best = g; });
    if (best.length < 2) return [];
    return best.map(function (row) { var r = byLines(row); r.key = r.who + r.text; r.el = row; return r; });
  }

  // ---------------------------------------------------------------- handing it over
  var rows = {}, last = "";
  function unreadInTitle() {
    var m = /\((\d+)\)/.exec(document.title || "");
    return m ? +m[1] : null;
  }
  function read() {
    var found = [];
    try { found = READ[service]() || []; } catch (e) { found = []; }
    if (!found.length) { try { found = anyList(); } catch (e) { found = []; } }
    var now = new Date(), seen = {}, items = [];
    rows = {};
    found.forEach(function (f) {
      if (items.length >= MOST || !(f.who || f.text)) return;
      var id = key(f.key || f.who + f.text);
      if (seen[id]) return;
      seen[id] = true;
      rows[id] = f.el;
      items.push({ id: id, who: f.who || "", text: f.text || "", when: f.when || "", ts: when(f.when, f.full, now), unread: !!f.unread });
    });
    return { items: items, unread: unreadInTitle() };
  }
  function tick() {
    var api = window.__TAURI_INTERNALS__;
    if (!api || typeof api.invoke !== "function") return;
    var now = read(), said = JSON.stringify(now);
    if (said === last) return;
    last = said;
    Promise.resolve(api.invoke("messages_found", now)).catch(function () { last = ""; });
  }
  // Opening one (the app asks, when it's tapped in Messages): as a click on it would.
  window.__chatcomputerOpen = function (id) {
    var e = rows[id];
    if (!e || !e.isConnected) return false;
    try { e.scrollIntoView({ block: "center" }); } catch (x) {}
    ["pointerdown", "mousedown", "pointerup", "mouseup"].forEach(function (t) {
      try { e.dispatchEvent(new MouseEvent(t, { bubbles: true, cancelable: true, view: window, button: 0 })); } catch (x) {}
    });
    e.click();
    return true;
  };
  // (For Chat Computer's own tests: the reading, and the times.)
  window.__chatcomputerRead = read;
  window.__chatcomputerWhen = when;
  setInterval(tick, 4000);
  setTimeout(tick, 1500);
})();
