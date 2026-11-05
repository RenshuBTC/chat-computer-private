/* Chat Computer in a browser: the demo at chatcomputer.ai (docs/, served by GitHub Pages).

   The screen here is Chat Computer's own, the same page and code as on a Chat Computer (chat.html, chat.js,
   chat.css: copied here by scripts/build_demo.py). This file stands in for the computer behind it. On Chat Computer,
   what you type (or say) goes to its own model, an open model of Qwen's running on the computer itself, which opens
   websites, plays music, reads your messages and looks after your files. Here, its answers to a few things are
   written in advance, the messages and files are made up, and everything stays in your browser: nothing typed here
   is sent anywhere, unless you choose one of the two that "/" in the message box lists:
   - Use this computer's model: where the Chat Computer app runs on this computer, its model answers for real and
     does things on this screen. What you type goes from your browser to the app, on this computer, and no further.
   - Connect AirPods: you talk, and the answers are read out. Your browser turns what you say into text (Chrome
     does it on Google's servers).
   Fetched from elsewhere: a map from OpenStreetMap when you ask for one, the weather and the air from Open-Meteo
   (the weather where you are, too, for the greeting: your time zone's city, or where you said you live; its name
   goes there, then its position, at most once every half hour), YouTube's player when the model plays a video,
   and, when the model looks something up (or you ask for the news), Wikipedia's articles and its Current events:
   only the words looked up go there. Messages shows each service's own icon (WhatsApp's, Gmail's...), as its own
   pages give it, through DuckDuckGo's icon service (Gmail's from Google's own server): only the service's address
   goes there. Google's own pages (its
   search, Google Maps' directions) and Wikipedia's show
   in the middle when asked for: of the sites that might be asked for, they're the ones that let another page show
   them.
   In the Chat Computer app for Windows and Mac
   (apps/desktop: this page, bundled into the app), the model comes with the app and answers once the app has it
   ready, any site opens right here, in the app's own browser view laid over the middle, "/" finds the computer's
   real files, Messages has your own accounts', and nothing here is made up: SHELL, below. */
(function () {
  "use strict";
  var START = Date.now() / 1000;
  // Inside the app (Tauri's window.__TAURI__ is there, given only to the app's own page): what the app does for it
  // (apps/desktop/src-tauri/src/lib.rs), asked with app(command, arguments), a promise.
  var SHELL = (function () {
    var tauri = window.__TAURI__ && window.__TAURI__.core;
    if (tauri && typeof tauri.invoke === "function") {
      var mac = /Mac/i.test(navigator.platform || "") || /Macintosh/.test(navigator.userAgent || "");
      return { kind: "app", computer: mac ? "Mac" : "PC", name: "the Chat Computer app on their " + (mac ? "Mac" : "Windows PC"),
               app: function (command, args) { return tauri.invoke(command, args || {}); },
               channel: function (on) { var c = new tauri.Channel(); c.onmessage = on; return c; } };
    }
    return null;
  })();
  var APP = SHELL && SHELL.kind === "app" ? SHELL : null;
  function noop() {}
  function now() { return Date.now() / 1000; }
  function call(name) {
    var args = Array.prototype.slice.call(arguments, 1);
    if (name === "accountForm") sheetNow = args[0] && args[0].id || "";
    keepInChat(name, args);
    if (window.CO && typeof window.CO[name] === "function") window.CO[name].apply(null, args);
  }

  // ------------------------------------------------------------------ chats
  // Each task's chat, as on Chat Computer (the right sidebar's lower half): what's said goes into the chat on the
  // screen, kept in this browser (nothing leaves it); + starts a new one, and a click on one brings it back. A first
  // visit to the website has a few earlier ones to show how it looks (the app starts with none: they'd be made up).
  var CHATS_KEY = "chatcomputer-demo-chats", CHATS_MAX = 50;
  var chatsNow = null;
  function chatsKept() {
    if (chatsNow) return chatsNow;
    var kept = store(CHATS_KEY);
    if (kept && Array.isArray(kept.list) && kept.list.length) { chatsNow = kept; return kept; }
    var day = 86400, at = Math.floor(Date.now() / 1000);
    function past(id, title, ago, you, ai) {
      return { id: id, title: title, ts: at - ago, items: [{ role: "user", text: you, ts: at - ago }, { role: "assistant", text: ai, ts: at - ago + 5 }] };
    }
    chatsNow = { current: "c-now", list: [{ id: "c-now", title: "", ts: at, items: [] }].concat(APP ? [] : [
      past("c-bali", "Plan the weekend in Bali", 2 * 3600, "Plan us a weekend in Bali, nothing too busy",
        "**Saturday:** a slow morning in Ubud, the rice terraces at Tegallalang before it's hot, lunch by the river.\n\n" +
        "**Sunday:** down to the coast at Uluwatu for the cliffs and the sunset; dinner on the beach at Jimbaran."),
      past("c-taxes", "Find my 2025 tax papers", day + 3000, "Where are my tax papers from last year?",
        "They're in Documents, in **Taxes**: *Taxes 2025.pdf* and the receipts beside it. Want me to open it?"),
      past("c-pack", "Packing list for the trip", 3 * day, "Make me a packing list for a week at the beach",
        "- Swimwear, two\n- Sunscreen and a hat\n- Light shirts and one warm layer\n- Sandals and walking shoes\n- Chargers, passport, cards")
    ]) };
    return chatsNow;
  }
  function chatsSave() {
    var c = chatsKept();
    c.list.sort(function (a, b) { return b.ts - a.ts; });
    c.list = c.list.slice(0, CHATS_MAX);
    store(CHATS_KEY, c);
  }
  function chatOnScreen() {
    var c = chatsKept();
    return c.list.filter(function (x) { return x.id === c.current; })[0] || null;
  }
  function showChats() {
    var c = chatsKept();
    if (window.CO && window.CO.chats) {
      window.CO.chats(c.list.map(function (x) { return { id: x.id, title: x.title, ts: x.ts }; }), c.current);
    }
  }
  // What the page is told to show, kept in the chat on the screen: your words (final), the answers as they come.
  function keepInChat(name, args) {
    if (name !== "user" && name !== "ai" && name !== "aiDone") return;
    var chat = chatOnScreen();
    if (!chat) return;
    if (name === "user") {
      if (!args[2]) return;                                   // (still being spoken)
      chat.items.push({ role: "user", id: String(args[0]), text: String(args[1] || ""), ts: now() });
      chat.ts = now();
      if (!chat.title) { chat.title = clip(args[1], 48); setTimeout(showChats, 0); }
      return;
    }
    var id = String(args[0]), last = chat.items[chat.items.length - 1];
    if (name === "ai") {
      if (!last || last.role !== "assistant" || last.id !== id) { last = { role: "assistant", id: id, text: "", ts: now() }; chat.items.push(last); }
      last.text += String(args[1] || "");
      return;
    }
    chatsSave();                                              // (an answer finished)
  }
  function openChat(id) {
    var c = chatsKept();
    if (!c.list.some(function (x) { return x.id === id; })) return;
    c.current = id;
    chatsSave();
    var chat = chatOnScreen();
    if (window.CO) { if (chat.items.length) window.CO.history(chat.items); else window.CO.clear(); }
    showChats();
  }
  // The page opened: a new chat, as the Claude app opens on one (the one before is in the list), unless the chat
  // on the screen has nothing in it yet.
  function startChat() {
    var c = chatsKept(), on = chatOnScreen();
    if (on && on.items.length) {
      var chat = { id: "c" + Date.now().toString(36), title: "", ts: now(), items: [] };
      c.list.unshift(chat);
      c.current = chat.id;
      chatsSave();
    }
    showChats();
  }
  function newChat() {
    var c = chatsKept(), on = chatOnScreen();
    if (on && !on.items.length) { openChat(on.id); return; }  // (the one on the screen is new already)
    var chat = { id: "c" + Date.now().toString(36), title: "", ts: now(), items: [] };
    c.list.unshift(chat);
    c.current = chat.id;
    chatsSave();
    if (window.CO) window.CO.clear();
    showChats();
  }
  function $(id) { return document.getElementById(id); }
  function esc(s) {
    return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }
  function clip(s, n) { s = String(s == null ? "" : s).replace(/\s+/g, " ").trim(); return s.length > (n || 80) ? s.slice(0, (n || 80) - 1) + "\u2026" : s; }
  // This browser's own storage (localStorage): undefined reads, null removes.
  function store(key, value) {
    try {
      if (value === undefined) return JSON.parse(window.localStorage.getItem(key) || "null");
      if (value === null) window.localStorage.removeItem(key);
      else window.localStorage.setItem(key, JSON.stringify(value));
    } catch (e) { return null; }
    return null;
  }
  function uid(prefix) { return prefix + Date.now().toString(36) + Math.random().toString(36).slice(2, 6); }

  // What chat.js says to Chat Computer goes here (on a Chat Computer, to its Python side).
  window.webkit = { messageHandlers: { chatcomputer: { postMessage: function (s) {
    var msg;
    try { msg = JSON.parse(s); } catch (e) { return; }
    setTimeout(function () { handle(msg); }, 0);
  } } } };

  // ------------------------------------------------------------------ made-up messages, things done, files
  // Each service's own icon, beside its messages: the one its own pages give (a browser tab shows it), from its own
  // site through DuckDuckGo's icon service (which keeps no record of who asks); Gmail's from Google's own server, the
  // file Gmail's pages name (its site's own icon is Google's G). Where one can't be had, Chat Computer's own for that
  // kind of message (chat.js: msgIcon).
  var SITES = { whatsapp: "whatsapp.com", outlook: "outlook.live.com", telegram: "web.telegram.org", instagram: "www.instagram.com",
                x: "x.com", linkedin: "www.linkedin.com", wechat: "web.wechat.com", discord: "discord.com", icloud: "www.icloud.com",
                yahoo: "mail.yahoo.com", facebook: "www.facebook.com", tiktok: "www.tiktok.com" };
  var GMAIL_ICON = "https://ssl.gstatic.com/ui/v1/icons/mail/rfr/gmail.ico";
  function siteIcon(name) {
    var key = String(name || "").toLowerCase(), site = SITES[key];
    if (key === "gmail") return GMAIL_ICON;
    return site ? "https://icons.duckduckgo.com/ip3/" + site + ".ico" : "";
  }
  var MESSAGES = APP ? [] : [
    { id: "m1", source: "whatsapp", label: "WhatsApp", who: "Mum", text: "Dinner at 7 tonight? Bring the photos from the trip!", ts: START - 240, unread: true },
    { id: "m2", source: "email", label: "Gmail", who: "Jane Lim", text: "Re: Lunch on Friday? 12:30 at the usual place works for me.", ts: START - 3100, unread: true },
    { id: "m3", source: "telegram", label: "Telegram", who: "Alex", text: "Sent you the slides for Monday.", ts: START - 7400, unread: false },
    { id: "m4", source: "instagram", label: "Instagram", who: "sam.k", text: "Liked your photo", ts: START - 11000, unread: false },
    { id: "m5", source: "email", label: "Outlook", who: "City Library", text: "Your books are due back on Thursday.", ts: START - 90000, unread: false },
    { id: "m6", source: "x", label: "X", who: "Priya", text: "Are you coming to the meetup on Saturday?", ts: START - 96000, unread: false },
    { id: "m7", source: "linkedin", label: "LinkedIn", who: "Daniel Tan", text: "Thanks for connecting!", ts: START - 260000, unread: false }
  ];
  var ACCOUNTS = APP ? [] : [{ id: "a1", label: "Gmail" }, { id: "a2", label: "Outlook" }, { id: "a3", label: "Telegram" },
                  { id: "a4", label: "WhatsApp" }, { id: "a5", label: "Instagram" }, { id: "a6", label: "X" },
                  { id: "a7", label: "LinkedIn" }];
  function when(ts) {
    var d = new Date(ts * 1000), today = new Date();
    var t = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
    if (d.toDateString() === today.toDateString()) return "Today, " + t;
    return d.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" }) + ", " + t;
  }
  var THREADS = APP ? {} : {
    m1: { title: "Mum", note: "On Chat Computer this is WhatsApp's own page, between the sidebars.", lines: [
      ["Mum", START - 90000, "How was the trip?"], ["You", START - 89000, "Amazing! So many photos."],
      ["Mum", START - 240, "Dinner at 7 tonight? Bring the photos from the trip!"]] },
    m2: { title: "Jane Lim", mail: "Jane Lim <jane@example.com>", mails: [
      ["Lunch on Friday?", START - 180000, "Hi! Are you free for lunch on Friday? There's a new noodle place near the office.\n\nJane"],
      ["Re: Lunch on Friday?", START - 3100, "12:30 at the usual place works for me. See you then!\n\nJane", true]] },
    m3: { title: "Alex", lines: [["Alex", START - 7500, "Do you have the slides?"], ["You", START - 7450, "Not yet"],
      ["Alex", START - 7400, "Sent you the slides for Monday."]] },
    m4: { title: "sam.k", note: "On Chat Computer this is Instagram's own page.", lines: [["sam.k", START - 11000, "Liked your photo"]] },
    m5: { title: "City Library", mail: "City Library <noreply@library.example>", mails: [
      ["Your books are due soon", START - 90000, "Your books are due back on Thursday:\n\n- The Overstory\n- A Field Guide to Getting Lost\n\nRenew them online or at any branch.", true]] },
    m6: { title: "Priya", note: "On Chat Computer this is X's own page.", lines: [["Priya", START - 96000, "Are you coming to the meetup on Saturday?"]] },
    m7: { title: "Daniel Tan", note: "On Chat Computer this is LinkedIn's own page.", lines: [["Daniel Tan", START - 260000, "Thanks for connecting!"]] }
  };
  // Earlier conversations, put back when something in Actions is tapped (the screen starts empty: the word logo).
  var EARLIER = APP ? [] : [
    { role: "user", id: "y1", ts: START - 86400 - 3010, text: "Play some calm piano music" },
    { role: "assistant", id: "ya1", ts: START - 86400 - 3000, text: "Playing a calm piano mix on YouTube." },
    { role: "user", id: "y2", ts: START - 86400 - 2910, text: "A bit quieter please" },
    { role: "assistant", id: "ya2", ts: START - 86400 - 2900, text: "Volume is 40%." },
    { role: "user", id: "h1", ts: START - 5410, text: "How do I get to the airport on Sunday with two big suitcases?" },
    { role: "assistant", id: "ha1", ts: START - 5380, text: "The train means two changes, and one of them has stairs and no lift, so a taxi is easier: about 25 minutes. I put a packing list in your Documents too." }
  ];
  var DONE = APP ? [] : [
    { id: "d1", ts: START - 86400 - 2995, text: "Opened youtube.com", prompt_id: "y1", prompt_ts: START - 86400 - 3010 },
    { id: "d2", ts: START - 86400 - 2905, text: "Set the volume to 40%", prompt_id: "y2", prompt_ts: START - 86400 - 2910,
      undo: { kind: "volume", value: 70 } },
    { id: "d3", ts: START - 5400, text: "Searched the web for \u201ctrains to the airport on Sunday\u201d", prompt_id: "h1", prompt_ts: START - 5410 },
    { id: "d4", ts: START - 5390, text: "Saved Packing list.md", prompt_id: "h1", prompt_ts: START - 5410,
      undo: { kind: "file", name: "Packing list.md" } }
  ];
  var FILES = APP ? [] : [
    { name: "Packing list.md", where: "~/Documents", kind: "document", ts: START - 5390, page: "packing" },
    { name: "Beach.jpg", where: "~/Pictures/Bali", kind: "image", ts: START - 40 * 86400, page: "beach" },
    { name: "Budget 2026.xlsx", where: "C:\\Users\\You\\Documents (Windows)", kind: "spreadsheet", ts: START - 17 * 86400, page: "budget" },
    { name: "Taxes 2025.pdf", where: "~/Documents/Taxes", kind: "document", ts: START - 190 * 86400, page: "taxes" },
    { name: "Recipes.txt", where: "~/Documents", kind: "document", ts: START - 100 * 86400, page: "recipes" },
    { name: "Resume.docx", where: "~/Documents/Work", kind: "document", ts: START - 60 * 86400, page: "resume" }
  ];

  // ------------------------------------------------------------------ the screen's size
  // On Chat Computer the text size comes from the screen's real size and how far away you sit (ui/layout.py); here,
  // from the screen's size too, not the window's: a smaller window has the same size of text, in a narrower column.
  // On a phone, as in the Claude app there (measured on an iPhone 375 points wide: its words are as big as ours at a
  // eighteenth of the screen's width), a eighteenth of the screen's width (its narrow side), 16 to 24px, in Safari,
  // Chrome or any browser alike, with the conversation 0.77rem from the screen's sides (chat.css). On a tablet, a
  // forty-sixth, 16 to 20px. On a computer, the Claude app's there: 16px, which the computer's display scaling makes
  // bigger on a sharper screen, as it does every page's (the conversation, the box and the sidebars in rem follow it,
  // chat.css). Never under 16px on a touchscreen: phones zoom in on a box to type in with smaller text. Pinching the conversation (a
  // tablet's touchscreen) or asking for bigger or smaller text changes it, as on Chat Computer; a phone keeps its own
  // choice (pinching there once changed it; now nothing zooms on a phone, so that old choice is left behind).
  var touchOnly = window.matchMedia ? window.matchMedia("(hover: none) and (pointer: coarse)") : { matches: false };
  function onPhone() {
    var s = window.screen || {};
    return touchOnly.matches && Math.min(s.width || window.innerWidth, s.height || window.innerHeight) < 600;   // chat.js's body.phone
  }
  function scaleKey() { return onPhone() ? "chatcomputer-demo-phone-scale" : "chatcomputer-demo-scale"; }
  var size = 20, column = 600, scale = store(scaleKey()) || 1;
  // The text's size before it's made bigger or smaller (scale), from the screen; and what it is at a scale.
  function baseSize() {
    if (SHELL && SHELL.textPx) return SHELL.textPx;
    var s = window.screen || {}, sw = s.width || window.innerWidth, sh = s.height || window.innerHeight, across = Math.min(sw, sh);
    if (onPhone()) return Math.max(16, Math.min(24, across / 18));
    if (touchOnly.matches) return Math.max(16, Math.min(20, across / 46));
    return 16;
  }
  function sizeAt(k) { var v = baseSize() * k; return touchOnly.matches ? Math.max(16, v) : v; }
  function layout() {
    var w = window.innerWidth, phone = onPhone();
    scale = store(scaleKey()) || 1;
    size = sizeAt(scale);
    column = Math.round(Math.min(57.75 * size, w - (phone ? 1.54 : 2) * size));
    call("layout", Math.round(size * 10) / 10, column);
    place();
  }
  window.addEventListener("resize", layout);

  // ------------------------------------------------------------------ what's open in the middle
  // On Chat Computer a second browser view, laid over the frame the conversation page draws for it ("hole"); here an
  // iframe, a new one each time (a site can leave one blank for good). It's a page like any other: it scrolls, and
  // once clicked it has the keys (Google's search box, a map), until the message box is clicked. Its x closes it,
  // and so does Escape (chat.js), here too while a page of the demo's own has the keys.
  var view = null, open = false, hole = null;
  var acted = 0;        // how many times what's open changed (shown, closed, the volume): see wordsOnly
  function frame() {
    if (view) { viewGone(view); view.remove(); }
    var v = view = document.createElement("iframe");
    v.id = "demo-view";
    v.title = "What's open";
    v.setAttribute("allow", "autoplay; encrypted-media; picture-in-picture; fullscreen");
    v.style.cssText = "position:fixed;border:0;background:var(--ground);display:none;z-index:5";
    v.addEventListener("load", function () {
      try { v.contentDocument.addEventListener("keydown", keys, true); } catch (e) {}
    });
    document.body.appendChild(v);
    return v;
  }
  function show(page) {
    if (APP && page.src && /^https?:/i.test(page.src)) { showSite(page); return; }
    if (site.on) closeSite();
    var v = frame();
    v.dataset.undo = "";                     // (no longer what an action opened: shown() marks it, for Revert)
    acted++;
    player.token = {};                       // what was playing is gone
    player.id = page.youtube || "";
    if (page.src) v.src = page.src;
    else v.srcdoc = page.html;
    open = true;
    v.style.display = "block";
    call("mode", "content", page.title || titleOf(page.html));
    place();
  }
  // What's open now, marked as opened by an action: Revert closes it, while it's still what's open.
  function shown() {
    if (!view) return null;
    view.dataset.undo = uid("v");
    return { kind: "close", view: view.dataset.undo };
  }
  // A page's name (its <title>), for the top of its frame.
  function titleOf(html) {
    var m = /<title>([^<]*)<\/title>/i.exec(html || "");
    if (!m) return "";
    var t = document.createElement("textarea");
    t.innerHTML = m[1];
    return t.value;
  }
  function place() {
    if (site.on) { if (open && hole) APP.app("place_site", hole).catch(noop); return; }
    if (!view || !open || !hole) return;
    view.style.left = hole.x + "px";
    view.style.top = hole.y + "px";
    view.style.width = hole.w + "px";
    view.style.height = hole.h + "px";
  }
  // Closed (its x, "close", a swipe up from the bottom edge): gone, and what played stops.
  function close() {
    if (site.on) { var on = open; acted++; closeSite(); if (on) call("mode", "chat"); return; }
    if (!view) return;
    var was = open;
    acted++;
    open = false;
    player.token = {};
    player.id = "";
    viewGone(view);
    view.remove();
    view = null;
    if (was) call("mode", "chat");
  }
  // Put away, for the conversation to have the middle again: out of sight, still playing (as on a Chat Computer, where
  // the conversation coming back leaves music playing). "Close" or "stop" then closes it.
  function putAway() {
    if (!open) return;
    open = false;
    if (site.on) APP.app("hide_site").catch(noop);
    else view.style.display = "none";
    call("mode", "chat");
  }
  // In the app, a site goes in its own browser view (where every site shows: none can refuse it, as they refuse
  // being framed in a page), laid over the frame (hole), and moved with it. The same view each time: sign-ins stay.
  // What it's at and called comes back from the app (chatcomputerApp, below), and a sheet over the screen (Connect
  // email, say) or a message held puts it out of sight meanwhile: the app's view would be in front of them.
  // (view: which of the app's views it is: "site", or a connected service's, "msg-gmail"...)
  var site = { on: false, hidden: false, url: "", view: "" };
  function showSite(page) {
    if (view) { viewGone(view); view.remove(); view = null; }
    acted++;
    player.token = {};
    player.id = page.youtube || "";
    site.on = true;
    site.hidden = false;
    site.url = page.src;
    site.view = "site";
    open = true;
    call("mode", "content", page.title || hostOf(page.src));
    var h = hole || { x: 0, y: 0, w: 1, h: 1 };
    APP.app("show_site", { url: page.src, x: h.x, y: h.y, w: h.w, h: h.h }).then(function () {
      place();                                   // where the frame is by now
      covered();
    }, function (e) {
      site.on = false;
      open = false;
      call("mode", "chat");
      call("notice", "That didn't open: " + e, "warn");
    });
  }
  function closeSite() {
    site.on = false;
    site.url = "";
    site.view = "";
    open = false;
    player.token = {};
    player.id = "";
    APP.app("close_site").catch(noop);
  }
  function covered() {
    if (!site.on || !open) return;
    var over = document.body.classList.contains("sheet") || document.body.classList.contains("holding");
    if (over === site.hidden) return;
    site.hidden = over;
    APP.app(over ? "hide_site" : "unhide_site").catch(noop);
  }
  if (APP && window.MutationObserver) {
    new MutationObserver(covered).observe(document.documentElement, { attributes: true, subtree: true, attributeFilter: ["class"] });
  }
  function hostOf(url) { try { return new URL(url).hostname.replace(/^www\./, ""); } catch (e) { return ""; } }
  // What the app says about the site (window.chatcomputerApp, run by the app, never by the site): its title and where
  // it is, a download finished, or an address to open here (a link in a page of the demo's own, or one meant for a
  // new window).
  window.chatcomputerApp = function (m) {
    if (!APP || !m || typeof m !== "object") return;
    if (m.messages && typeof m.messages === "object") { serviceMessages(m.messages); return; }
    if (m.model && typeof m.model === "object") { localFrom(m.model); return; }
    var mine = !m.view || m.view === site.view;        // (what a view out of sight says isn't for the frame)
    if (typeof m.title === "string" && mine && site.on && open && m.title.trim()) call("mode", "content", clip(m.title, 80));
    if (typeof m.url === "string" && mine && site.on) site.url = m.url;
    if (typeof m.downloaded === "string") did((m.ok ? "Downloaded " : "Couldn't download ") + clip(m.downloaded, 60) + (m.ok ? " to Downloads" : ""));
    if (typeof m.open === "string" && /^https?:/i.test(m.open)) show({ src: m.open, title: hostOf(m.open), tab: "new" });
  };
  // ------------------------------------------------------------------ the app: messages from your own accounts
  // In the app, Gmail, Outlook, Telegram, WhatsApp, WeChat and Discord connect: each opens in the middle, in a view of
  // its own, for you to sign in on its own page; then it's kept open out of sight (signed in, as a browser keeps
  // you), and what's in its list of conversations comes into Messages, newest first (apps/desktop: messages.rs,
  // reader.js). Tapping one opens its service in the middle, at that conversation. Which are connected is kept here.
  var SERVICES = {
    gmail: { label: "Gmail", group: "email", source: "email" },
    outlook: { label: "Outlook", group: "email", source: "email" },
    telegram: { label: "Telegram", group: "chat", source: "telegram" },
    whatsapp: { label: "WhatsApp", group: "chat", source: "whatsapp" },
    wechat: { label: "WeChat", group: "chat", source: "wechat" },
    discord: { label: "Discord", group: "chat", source: "discord" }
  };
  var CONNECTED = "chatcomputer-app-services";
  function connected() { return (store(CONNECTED) || []).filter(function (k) { return SERVICES[k]; }); }
  function appAccounts() { return connected().map(function (k) { return { id: k, label: SERVICES[k].label }; }); }
  function drawAppMessages() { call("messagesList", MESSAGES.map(withPin), appAccounts()); }
  // Connected: in the middle, to sign in (show), or out of sight, to be read (at the start, those connected before).
  function connectService(k, show) {
    if (!SERVICES[k]) return;
    var list = connected();
    if (list.indexOf(k) < 0) { list.push(k); store(CONNECTED, list); drawAppMessages(); }
    if (show) showService(k);
    else APP.app("connect_service", { service: k, show: false, x: 0, y: 0, w: 1, h: 1 }).catch(noop);
  }
  function showService(k, item) {
    var s = SERVICES[k];
    if (view) { viewGone(view); view.remove(); view = null; }
    acted++;
    player.token = {};
    player.id = "";
    site.on = true;
    site.hidden = false;
    site.url = "";
    site.view = "msg-" + k;
    open = true;
    call("mode", "content", s.label);
    var h = hole || { x: 0, y: 0, w: 1, h: 1 };
    APP.app("connect_service", { service: k, show: true, x: h.x, y: h.y, w: h.w, h: h.h }).then(function () {
      place();
      covered();
      if (item) APP.app("open_service_item", { service: k, item: item }).catch(noop);
    }, function (e) {
      site.on = false;
      site.view = "";
      open = false;
      call("mode", "chat");
      call("notice", s.label + " didn't open: " + e, "warn");
    });
  }
  function forgetService(k) {
    store(CONNECTED, connected().filter(function (x) { return x !== k; }));
    MESSAGES = MESSAGES.filter(function (m) { return m.service !== k; });
    if (site.on && site.view === "msg-" + k) { site.on = false; site.view = ""; open = false; call("mode", "chat"); }
    APP.app("forget_service", { service: k }).catch(noop);
    drawAppMessages();
  }
  // What a service's reader read: its conversations, in place of what it had in Messages (where its list couldn't be
  // read, the number of unread messages its page's title shows, in one line).
  function serviceMessages(m) {
    var k = String(m.service || ""), s = SERVICES[k];
    if (!s || connected().indexOf(k) < 0) return;
    var now = Date.now() / 1000, last = now;
    var items = (m.items || []).map(function (it) {
      // (Where the time couldn't be read: just before the one listed above it, keeping the service's order.)
      var ts = typeof it.ts === "number" && it.ts > 0 ? it.ts : last - 1;
      last = Math.min(last, ts);
      return { id: k + "-" + String(it.id || ""), service: k, item: String(it.id || ""), source: s.source, label: s.label,
               icon: siteIcon(k), who: clip(it.who, 80) || s.label, text: clip(it.text, 300), ts: ts, unread: !!it.unread };
    });
    var unread = +m.unread || 0;
    if (!items.length && unread > 0) {
      items = [{ id: k + "-unread", service: k, item: "", source: s.source, label: s.label, icon: siteIcon(k), who: s.label,
                 text: unread + (unread === 1 ? " unread message" : " unread messages"), ts: now, unread: true }];
    }
    MESSAGES = MESSAGES.filter(function (x) { return x.service !== k; }).concat(items);
    drawAppMessages();
  }
  // Connect email or chat apps: which, to sign in to (one connected already opens again, or is disconnected).
  // Messages' +: everything that can be connected, in one panel, by kind, each with its own logo. In the app, each
  // signs in on its own page, right here (those connected: opened, or disconnected); on the website, none can be.
  var CONNECTABLE = [["Email", [["gmail", "Gmail"], ["outlook", "Outlook"], ["icloud", "iCloud Mail"], ["yahoo", "Yahoo Mail"]]],
                     ["Chat apps", [["telegram", "Telegram"], ["whatsapp", "WhatsApp"], ["wechat", "WeChat"], ["discord", "Discord"]]],
                     ["Social media", [["x", "X"], ["instagram", "Instagram"], ["facebook", "Facebook"], ["tiktok", "TikTok"], ["linkedin", "LinkedIn"]]]];
  function connectForm(pick) {
    var choices = [], on = APP ? connected() : [], name = "";
    CONNECTABLE.forEach(function (g) {
      var here = g[1].filter(function (s) { return !APP || SERVICES[s[0]]; });
      if (!here.length) return;
      choices.push({ note: g[0] + ":" });
      here.forEach(function (s) {
        if (s[0] === pick) name = s[1];
        choices.push({ label: (on.indexOf(s[0]) >= 0 ? "Open " : "") + s[1], value: APP ? s[0] : "connect:" + s[0], icon: siteIcon(s[0]),
                       on: s[0] === pick });
      });
    });
    if (on.length) {
      choices.push({ note: "" });
      on.forEach(function (k) { choices.push({ label: "Disconnect " + SERVICES[k].label, value: "forget:" + k }); });
    }
    var text = APP ? ["Choose one and sign in on its own page, right here. It stays signed in in this app, and its messages " +
      "come into Messages, newest first. Nothing leaves your computer."] :
      ["On Chat Computer, these connect here, and their messages come into Messages, newest first. Sign-ins stay on your computer."];
    if (APP) text.push("WeChat's web version isn't open to every account: if it says yours can't use it, WeChat can't come " +
      "into Messages here. Social media: open them as sites, right here (say \u201copen instagram.com\u201d).");
    return { id: "accounts", title: "Connect an account", ok: "", text: text, choices: choices,
      error: name ? name + " can't be connected on the website: the Chat Computer app connects it (coming soon)." : "" };
  }
  function appConnectForm() { return connectForm(""); }
  function appInbox() {
    return MESSAGES.slice().sort(function (a, b) { return b.ts - a.ts; }).slice(0, 40).map(function (m) {
      return { id: m.id, from: m.label, who: m.who, when: when(m.ts), unread: m.unread, says: m.text };
    });
  }

  // Links in this page (an answer's, a page of the demo's own): opened here, in the middle, rather than leaving it.
  if (APP) {
    document.addEventListener("click", function (e) {
      var a = e.target && e.target.closest && e.target.closest("a[href]");
      if (!a || !/^https?:/i.test(a.href)) return;
      e.preventDefault();
      show({ src: a.href, title: hostOf(a.href), tab: "new" });
    }, true);
  }
  // A turn that did nothing to what's open (showed nothing, closed nothing, left the volume) and has words to say:
  // they'd be out of sight, the conversation being put away while something's open, so what's open is put away for
  // them. Only one thing has the middle at a time.
  function wordsOnly(since) { if (open && acted === since) putAway(); }
  function keys(e) {
    if (e.key === "Escape" && open && !e.isComposing) { e.preventDefault(); call("escape"); }
  }
  // Page Up and Down (and the arrow keys, Home and End while the message box is empty) scroll what's open, when
  // it's a page of the demo's own (another site's page can't be reached from here: a click on it, and it scrolls).
  function scrollOpen(how) {
    if (site.on) { APP.app("site_media", { action: "scroll_" + how }).catch(noop); return; }
    var w;
    try { w = view && view.contentWindow; w.document; } catch (e) { return; }
    if (!w) return;
    var by = { page_up: -0.85, page_down: 0.85, up: -0.15, down: 0.15 }[how];
    if (how === "top" || how === "bottom") w.scrollTo({ top: how === "top" ? 0 : w.document.documentElement.scrollHeight, behavior: "smooth" });
    else if (by) w.scrollBy({ top: by * w.innerHeight, behavior: "smooth" });
  }
  // A touchscreen, as an iPad's: swipe up from the bottom edge to close what's open (Home), down from the top for
  // Messages. (On Chat Computer the window does these, and more: three fingers copy and paste, the sides go back.)
  var swipe = null;
  document.addEventListener("touchstart", function (e) {
    var t = e.touches[0];
    swipe = e.touches.length === 1 ? { y: t.clientY, edge: t.clientY > window.innerHeight - 28 ? "bottom" :
                                                           t.clientY < 28 ? "top" : "" } : null;
  }, { passive: true, capture: true });
  document.addEventListener("touchend", function (e) {
    if (!swipe || !swipe.edge || !e.changedTouches.length) return;
    var dy = e.changedTouches[0].clientY - swipe.y;
    if (swipe.edge === "bottom" && dy < -48) close();
    if (swipe.edge === "top" && dy > 48) call("showMessages");
    swipe = null;
  }, { passive: true, capture: true });

  // Pages shown there: Chat Computer's look (theme.css, and the pages' own doc.css from ui/render.py).
  function page(title, body, css, script) {
    return '<!doctype html><html lang="en"><head><meta charset="utf-8"><title>' + esc(title) + "</title>" +
      '<link rel="stylesheet" href="demo/theme.css"><link rel="stylesheet" href="demo/doc.css"><style>:root{--co-size:' +
      size.toFixed(1) + "px;--co-column:" + column + "px}" + (css || "") + "</style></head><body>" + body +
      (script ? "<script>" + script + "<\/script>" : "") + "</body></html>";
  }
  var NOTE_CSS = ".demo-note{margin:0 0 1.25rem;padding:0.5rem 0.75rem;border:2px solid var(--line);border-radius:0.6rem}";
  function doc(title, markdown, note) {
    return page(title, '<article><h1 class="title">' + esc(title) + "</h1>" + (note ? '<p class="demo-note">' + esc(note) + "</p>" : "") +
      '<div class="md">' + window.renderMarkdown(markdown) + "</div></article>", NOTE_CSS);
  }
  function thread(id) {
    var t = THREADS[id];
    if (!t) return null;
    var note = t.note ? '<p class="demo-note">' + esc(t.note) + "</p>" : "";
    var body;
    if (t.mails) {
      body = '<div class="meta"><div><b>From:</b> ' + esc(t.mail) + "</div></div>" + t.mails.map(function (m) {
        return '<section class="said mail"' + (m[3] ? " id=open" : "") + '><div><span class="who">' + esc(m[0]) +
          "</span> " + esc(when(m[1])) + '</div><div class="plain">' + esc(m[2]) + "</div></section>";
      }).join("");
    } else {
      body = t.lines.map(function (l) {
        return '<div class="said"><span class="who">' + esc(l[0]) + "</span> " + esc(when(l[1])) +
          '<div class="plain">' + esc(l[2]) + "</div></div>";
      }).join("");
    }
    return page(t.title, '<article><h1 class="title">' + esc(t.title) + "</h1>" + note + body + "</article>",
      NOTE_CSS + ".said.mail{padding:0 0 1rem;margin:0 0 1.25rem;border-bottom:2px solid var(--line)}.said.mail:last-child{border-bottom:0}",
      "var o=document.getElementById('open');if(o)o.scrollIntoView({block:'start'});");
  }
  function timer(mins) {
    return page("Timer", '<article style="text-align:center"><h1 class="title">Timer</h1><p id="left" style="font-size:4rem;' +
      'font-variant-numeric:tabular-nums;margin:1rem 0">' + mins + ':00</p><p><button id="go" class="b">Start</button> ' +
      '<button id="reset" class="b">Reset</button></p></article>',
      ".b{font:inherit;padding:0.4em 1.2em;border:2px solid var(--ink);border-radius:0.6em;background:var(--ground);color:var(--ink);cursor:pointer}",
      "var total=" + mins * 60 + ",left=total,run=null,el=document.getElementById('left'),go=document.getElementById('go');" +
      "function draw(){el.textContent=Math.floor(left/60)+':'+('0'+left%60).slice(-2)+(left?'':' Done!');}" +
      "go.onclick=function(){if(run){clearInterval(run);run=null;go.textContent='Start';}else{run=setInterval(function(){if(left>0)left--;draw();" +
      "if(!left){clearInterval(run);run=null;go.textContent='Start';}},1000);go.textContent='Pause';}};" +
      "document.getElementById('reset').onclick=function(){left=total;draw();};");
  }
  var PAGES = {
    packing: function () {
      return doc("Packing list", "## Sunday, to the airport\n\n- [x] Passport\n- [x] Charger and adapters\n- [ ] Two big suitcases, " +
        "under 23 kg each\n- [ ] Snacks for the flight\n\n## Getting there\n\nA taxi is easier than the train with two big " +
        "suitcases: about 25 minutes. Leave by 1 pm.");
    },
    beach: function () {
      return page("Beach.jpg", '<div class="image"><svg viewBox="0 0 1600 1000" role="img" aria-label="A beach at sunset, drawn for the demo" ' +
        'style="width:100vw;height:100vh"><rect width="1600" height="1000" fill="#fff"/><circle cx="800" cy="520" r="150" fill="none" stroke="#000" stroke-width="6"/>' +
        '<path d="M0 560 H1600" stroke="#000" stroke-width="6"/><path d="M0 640 Q200 610 400 640 T800 640 T1200 640 T1600 640" fill="none" stroke="#000" stroke-width="4"/>' +
        '<path d="M0 720 Q200 690 400 720 T800 720 T1200 720 T1600 720" fill="none" stroke="#000" stroke-width="4"/>' +
        '<path d="M0 820 Q400 760 800 820 T1600 820 V1000 H0Z" fill="#EDEDED"/></svg></div>');
    },
    budget: function () {
      return doc("Budget 2026.xlsx", "| Month | Income | Spent | Saved |\n|---|---|---|---|\n| July | 5,200 | 3,900 | 1,300 |\n" +
        "| August | 5,200 | 4,450 | 750 |\n| September | 5,400 | 3,700 | 1,700 |\n\nSaved so far this year: **11,850**.",
        "A made-up spreadsheet. On Chat Computer, files on Windows' drive open too.");
    },
    taxes: function () {
      return doc("Taxes 2025.pdf", "## Notice of assessment, 2025\n\nThe pages of a PDF show here as they are.",
        "A made-up document. On Chat Computer, a PDF shows as its real pages.");
    },
    recipes: function () {
      return doc("Recipes.txt", "## Laksa for four\n\n1. Fry the laksa paste in a little oil for two minutes.\n" +
        "2. Add the coconut milk and stock; simmer for ten minutes.\n3. Add the prawns and tofu puffs; cook for three minutes.\n" +
        "4. Pour over the noodles, with bean sprouts, a boiled egg and laksa leaves.");
    },
    resume: function () {
      return doc("Resume.docx", "## Experience\n\n- **Product designer**, 2022 to now\n- **Designer**, 2019 to 2022\n\n" +
        "## Skills\n\nResearch, prototyping, writing.", "A made-up document. On Chat Computer, Word files show as written.");
    }
  };

  // ------------------------------------------------------------------ music, made by the browser
  // On Chat Computer, "play some piano music" opens YouTube and plays it. Here, the page that opens plays soft
  // chords made by the browser itself (or, where a browser waits for a tap first, its Play button does).
  // (vol, the volume, comes first: piano() says what it is, and the page is told when it changes.)
  var MUSIC = "var ctx=null,out=null,timer=0,btn=document.getElementById('play');" +
    "addEventListener('message',function(e){if(e.data&&typeof e.data.volume==='number'){vol=e.data.volume/100;if(out)out.gain.value=0.18*vol;}});" +
    "function play(){var C=window.AudioContext||window.webkitAudioContext;if(!C)return;ctx=new C();out=ctx.createGain();" +
    "out.gain.value=0.18*vol;out.connect(ctx.destination);var chords=[[60,64,67,71],[57,60,64,67],[53,57,60,64],[55,59,62,65]]," +
    "beat=0.42,t0=ctx.currentTime+0.1,bar=0;function note(m,at,len){var o=ctx.createOscillator(),g=ctx.createGain();" +
    "o.type='triangle';o.frequency.value=440*Math.pow(2,(m-69)/12);g.gain.setValueAtTime(0,at);" +
    "g.gain.linearRampToValueAtTime(0.5,at+0.02);g.gain.exponentialRampToValueAtTime(0.001,at+len);o.connect(g);g.connect(out);" +
    "o.start(at);o.stop(at+len+0.05);}function sched(){var c=chords[bar%4],at=t0+bar*8*beat;" +
    "[0,1,2,3,2,1,2,3].forEach(function(k,i){note(c[k]+(i?0:-12),at+i*beat,1.6);});bar++;}sched();sched();" +
    "timer=setInterval(sched,8*beat*1000);btn.textContent='Pause';}" +
    "function stop(){clearInterval(timer);if(ctx)ctx.close();ctx=null;btn.textContent='Play';}" +
    "btn.onclick=function(){if(ctx)stop();else play();};play();" +
    "setTimeout(function(){if(ctx&&ctx.state!=='running')stop();},400);";
  function piano() {
    show({ html: page("Now playing", '<article><h1 class="title">Calm piano</h1><p>On Chat Computer this is YouTube, playing ' +
      "what you asked for. Here, your browser plays a few soft chords.</p><p><button id=\"play\" class=\"b\">Play</button></p>" +
      "<p>The \u00d7 above closes it (so do Escape and a swipe up from the bottom edge).</p></article>",
      ".b{font:inherit;padding:0.4em 1.2em;border:2px solid var(--ink);border-radius:0.6em;background:var(--ground);color:var(--ink);cursor:pointer}",
      "var vol=" + player.volume / 100 + ";" + MUSIC) });
  }

  // ------------------------------------------------------------------ YouTube, when the model plays a video
  // On Chat Computer, a video is YouTube's own page between the sidebars. Here, when the model plays one,
  // it's YouTube's player in the same place, and the conversation comes back when the video ends. Browsers let a
  // page play sound by itself only after you've clicked, tapped or typed on the site (or, in Chrome, played its
  // videos with sound often enough): so a video starts with sound when the browser allows it and otherwise
  // without, and then the sound comes on with the first click, tap or key anywhere here, the video included. The
  // player's own controls pause it. (The player and this page talk with YouTube's player messages.)
  var player = { token: {}, id: "", heard: false, state: -2, armed: false, volume: 100 };
  function youtube(id, start, title) {
    start = start || 0;
    if (APP) {                                          // (in the app: YouTube's own page, as on a Chat Computer)
      show({ youtube: id, src: "https://www.youtube.com/watch?v=" + id + (start ? "&t=" + start + "s" : ""), title: title || "YouTube" });
      return;
    }
    acted++;                                            // it's coming: the turn's words don't put anything away
    var token = player.token = {};
    soundAllowed().then(function (sound) {
      if (player.token !== token) return;               // something else was opened meanwhile
      var origin = /^https?:$/.test(location.protocol) ? "&origin=" + encodeURIComponent(location.origin) : "";
      show({ youtube: id, src: "https://www.youtube-nocookie.com/embed/" + id + "?autoplay=1&playsinline=1&rel=0&enablejsapi=1" +
             "&start=" + start + (sound ? "" : "&mute=1") + origin, title: title || "YouTube" });
      var mine = player.token;
      player.heard = false;
      player.state = -2;
      listen(mine);
      if (!sound) armSound();
      else setTimeout(function () {                      // allowed, yet not playing: without sound, then
        if (player.token === mine && (player.state === -1 || player.state === 5)) { command("mute"); command("playVideo"); armSound(); }
      }, 4000);
    });
  }
  // Whether this page may play sound now: Firefox says so outright; elsewhere, a tenth of a second of silence is tried.
  function soundAllowed() {
    try {
      if (navigator.getAutoplayPolicy) return Promise.resolve(navigator.getAutoplayPolicy("mediaelement") === "allowed");
    } catch (e) {}
    return new Promise(function (done) {
      var settled = false;
      function end(ok) { if (!settled) { settled = true; done(ok); } }
      try {
        var a = new Audio(silence()), p = a.play();
        if (p && p.then) p.then(function () { a.pause(); end(true); }, function () { end(false); });
        else end(!a.paused);
      } catch (e) { end(false); }
      setTimeout(function () { end(false); }, 700);
    });
  }
  var silenceUrl = "";
  function silence() {
    if (silenceUrl) return silenceUrl;
    var n = 800, b = new ArrayBuffer(44 + n), v = new DataView(b);       // a WAV file: 8 kHz, 8 bits, silent
    function s(o, t) { for (var i = 0; i < t.length; i++) v.setUint8(o + i, t.charCodeAt(i)); }
    s(0, "RIFF"); v.setUint32(4, 36 + n, true); s(8, "WAVE"); s(12, "fmt "); v.setUint32(16, 16, true);
    v.setUint16(20, 1, true); v.setUint16(22, 1, true); v.setUint32(24, 8000, true); v.setUint32(28, 8000, true);
    v.setUint16(32, 1, true); v.setUint16(34, 8, true); s(36, "data"); v.setUint32(40, n, true);
    for (var i = 0; i < n; i++) v.setUint8(44 + i, 128);
    silenceUrl = URL.createObjectURL(new Blob([b], { type: "audio/wav" }));
    return silenceUrl;
  }
  var MEDIA = { playVideo: "play", pauseVideo: "pause", mute: "mute", unMute: "unmute", setVolume: "volume" };
  function command(func, args) {
    if (site.on) {
      if (MEDIA[func]) APP.app("site_media", { action: MEDIA[func], value: args && args.length ? +args[0] : null }).catch(noop);
      return;
    }
    if (!view || !player.id) return;
    try { view.contentWindow.postMessage(JSON.stringify({ event: "command", func: func, args: args || [], id: 1, channel: "widget" }), "*"); } catch (e) {}
  }
  // The player says how it's doing (playing, ended, muted) once this page says it's listening.
  function listen(token) {
    var tries = 0;
    (function hello() {
      if (player.token !== token || player.heard || ++tries > 80) return;
      try { view.contentWindow.postMessage(JSON.stringify({ event: "listening", id: 1, channel: "widget" }), "*"); } catch (e) {}
      setTimeout(hello, 250);
    })();
  }
  window.addEventListener("message", function (e) {
    if (!view || !player.id || e.source !== view.contentWindow) return;
    var d;
    try { d = typeof e.data === "string" ? JSON.parse(e.data) : e.data; } catch (x) { return; }
    if (!d || typeof d !== "object" || !d.event) return;
    player.heard = true;
    var info = d.info;
    if (d.event === "onStateChange" && typeof info === "number") stateIs(info);
    if ((d.event === "infoDelivery" || d.event === "initialDelivery") && info && typeof info === "object") {
      if (typeof info.playerState === "number") stateIs(info.playerState);
      if (info.muted === false && player.armed) disarm();          // the sound is on
    }
    if (d.event === "onAutoplayBlocked") { command("mute"); command("playVideo"); armSound(); }
  });
  function stateIs(s) {
    var was = player.state, token = player.token;
    player.state = s;
    // Ended (0): back to the conversation, the middle all its own again.
    if (s === 0 && was !== 0) setTimeout(function () { if (player.token === token && player.state === 0) close(); }, 800);
  }
  // The first click, tap or key anywhere here turns the sound on (Escape isn't one that counts, for browsers).
  var ACTS = ["keydown", "mousedown", "pointerup", "touchend", "click"];
  function armSound() {
    if (player.armed) return;
    player.armed = true;
    ACTS.forEach(function (k) { document.addEventListener(k, soundOn, true); });
  }
  function disarm() {
    player.armed = false;
    ACTS.forEach(function (k) { document.removeEventListener(k, soundOn, true); });
  }
  function soundOn(e) {
    if (e.type === "keydown" && (e.key === "Escape" || e.repeat)) return;
    if (!player.id) { disarm(); return; }
    command("unMute");
    command("setVolume", [player.volume]);
    if (player.state === -1 || player.state === 5) command("playVideo");       // it never started
    if (!player.heard) disarm();      // no word from the player: it can't say the sound came on, so once is all
  }
  // A click or tap on the video itself goes to YouTube's player, not to this page, which sees only the player
  // taking the keys. While the sound is still off, that turns it on as well, and the video goes on playing
  // (the player takes a click on the video to mean pause).
  window.addEventListener("blur", function () {
    setTimeout(function () {
      if (!player.armed || !player.id || document.activeElement !== view) return;
      var token = player.token;
      command("unMute");
      command("setVolume", [player.volume]);
      setTimeout(function () { if (player.token === token && player.state === 2) command("playVideo"); }, 500);
    }, 0);
  });

  // ------------------------------------------------------------------ answers written in advance
  // Without the model (the app's, on this computer), the page answers from what's written here: what Chat Computer
  // is and how to get it, what this page can show, and honest answers about what it can't do without its model. The
  // facts are README.md's, in one place.
  var FACTS = {
    what: "Chat Computer runs a whole computer for you: there's no desktop and no apps to learn. You talk, type or touch " +
      "the screen, and it does what you ask: opens websites, plays music and videos, reads your messages, looks after your " +
      "files and settings.",
    soon: "Chat Computer isn't out yet: the download is coming soon.",
    free: "Chat Computer is free, for good: it thinks with an open model (Qwen's) that runs on your own computer, so there's " +
      "no account, no sign-in and nothing to pay.",
    pc: "On a Windows PC it installs next to Windows, on free space: Windows, your apps and your files stay, one restart " +
      "away, and you can remove Chat Computer any time. The PC must start in UEFI mode (every PC from the last ten years " +
      "does) and have about 70 GB free.",
    mac: "On a Mac with Apple silicon it installs next to macOS the same way: M1 and M2 Macs work fully, M3 Macs with some " +
      "limits for now, and Intel Macs or M4 and newer can't run it yet. The Mac version is the newest: it hasn't been " +
      "tried on a real Mac yet.",
    use: "You can touch the screen, type on a keyboard, or talk through AirPods.",
    private: "On Chat Computer your files stay on your computer, speech is turned into text on the computer itself, and the " +
      "model that answers runs there too: what you say doesn't leave it.",
    indie: "Chat Computer is an independent project. It thinks with Qwen, an open model from Alibaba, run on your computer by " +
      "Chat Computer's own engine."
  };
  var CITIES = { singapore: [103.6, 1.2, 104.1, 1.47], london: [-0.35, 51.38, 0.15, 51.62], "new york": [-74.1, 40.6, -73.8, 40.85],
                 tokyo: [139.55, 35.55, 139.95, 35.8], paris: [2.22, 48.8, 2.47, 48.92], sydney: [151.0, -33.98, 151.35, -33.78] };
  var TRY = APP ? "Try “play some piano music”, “show me a map of Tokyo”, “weather”, “open bbc.com” or “set a timer for " +
    "5 minutes”. Type / and a name to find a file on this computer." : "Try “play some piano music”, “show me a map of Tokyo”, “weather in London”, " +
            "“set a timer for 5 minutes” or “what did Jane say?”. Type / to search the files.";
  var REAL = "My model comes with this app, and runs on this computer.";      // (the app's; the website's: nextStep())
  function map(b, name) {
    show({ src: "https://www.openstreetmap.org/export/embed.html?bbox=" + b.join("%2C") + "&layer=mapnik",
           title: name ? "Map of " + name : "Map" });
  }
  function title(s) { return String(s).replace(/\b\w/g, function (c) { return c.toUpperCase(); }); }
  // Sites that let another page show them, so they open right here, to scroll and type in, as on a Chat Computer:
  // Google (its search, from the address Google itself lets other pages show it at, with igu=1), Google Maps' own
  // embedded maps and directions (output=embed), and Wikipedia. Most sites forbid it: they're a link (link()).
  // The address to show, or null.
  function framed(url) {
    var u;
    try { u = new URL(url); } catch (e) { return null; }
    if (!/^https?:$/.test(u.protocol)) return null;
    if (APP) return u.href;                             // (in the app, every site right here, as it is)
    var host = u.hostname.toLowerCase().replace(/^www\./, ""), q = u.searchParams;
    if (/(^|\.)wikipedia\.org$/.test(host)) return "https://" + u.hostname + u.pathname + u.search + u.hash;
    if (!/^(maps\.)?google\.[a-z]{2,3}(\.[a-z]{2})?$/.test(host)) return null;
    if (/^maps\./.test(host) || /^\/maps\b/.test(u.pathname)) {
      var from = q.get("origin") || q.get("saddr"), to = q.get("destination") || q.get("daddr");
      var place = q.get("query") || q.get("q"), named = /^\/maps\/(?:place|search)\/([^/]+)/.exec(u.pathname);
      if (!place && named) { try { place = decodeURIComponent(named[1].replace(/\+/g, " ")); } catch (e) { place = named[1]; } }
      if (to) return directionsMap(from, to);
      if (place) return "https://www.google.com/maps?output=embed&q=" + encodeURIComponent(place);
      return null;
    }
    if (u.pathname === "/" || u.pathname === "/webhp") return "https://www.google.com/webhp?igu=1";
    if (u.pathname === "/search" && q.get("q")) {
      q.set("igu", "1"); return "https://www.google.com/search?" + q.toString();     // igu=1: Google's own, to be framed
    }
    return null;
  }
  function googleSearch(q) { return "https://www.google.com/search?igu=1&q=" + encodeURIComponent(q); }
  function directionsMap(from, to) {
    return "https://maps.google.com/maps?output=embed&" + (from ? "saddr=" + encodeURIComponent(from) + "&daddr=" : "q=") +
      encodeURIComponent(to);
  }
  // A site: right here when it lets this page show it (framed), else a page with a link to it.
  function site(url, name, note) {
    var f = framed(url);
    return f ? { src: f, title: name } : { html: link(url, name, note) };
  }
  // A typed address, as in a browser's address bar: a site's address typed alone (or after "open", "go to" or
  // "visit"; with or without https:// and a path; its ending a known one, or a country's two letters but for those
  // more often a file's, as notes.md) opens the site, whatever answers here (the written answers or the model), and
  // whatever the site: right here when it lets another page show it (Google, Google
  // Maps, Wikipedia; a YouTube video, in YouTube's player), and otherwise at once in a new tab beside this page (most
  // sites forbid being shown inside another page; on a Chat Computer every site opens right here). Where the browser
  // stops the new tab, the site is a click away on the screen instead.
  var TLD = "com|org|net|edu|gov|mil|int|info|biz|io|ai|app|dev|co|me|tv|xyz|site|online|store|shop|tech|blog|news|" +
    "cloud|page|live|art|design|studio|agency|world|today|space|website|fun|club|life|email|link|pro|mobi|name|museum|" +
    "travel|jobs|asia|wiki|games|game|music|video|social|network|digital|media|global|group|one|top|health|bank|money|" +
    "finance|law|school|academy|university|college|press|photo|photos|gallery|(?!(?:md|py|sh|rs|cc|so|ps)\\b)[a-z]{2}";
  var ADDRESS = new RegExp("^\\s*(?:(?:please\\s+)?(?:open|go to|goto|visit|load|launch|browse to|take me to)\\s+)?" +
    "((?:https?:\\/\\/)?(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\\.)+(?:" + TLD + ")(?::\\d{2,5})?(?:[\\/?#]\\S*)?)" +
    "(?:,?\\s+(?:please|pls|thanks|thank you))?\\s*[.!]*\\s*$", "i");
  var BARE = { google: "www.google.com", youtube: "www.youtube.com", gmail: "mail.google.com", facebook: "www.facebook.com",
               instagram: "www.instagram.com", netflix: "www.netflix.com", wikipedia: "www.wikipedia.org" };
  function address(text) {
    var t = String(text || "").trim(), m = ADDRESS.exec(t), raw = m ? m[1].replace(/[.,!]+$/, "") :
      BARE[t.replace(/^(?:open|go to|visit)\s+/i, "").replace(/[.!?]+$/, "").toLowerCase()];
    if (!raw) return null;
    try {
      var u = new URL(/^https?:\/\//i.test(raw) ? raw : "https://" + raw);
      return /^https?:$/.test(u.protocol) ? u : null;
    } catch (e) { return null; }
  }
  // A YouTube video's address (youtube.com/watch?v=..., youtu.be/..., a short): its video and where to start, or null.
  function videoOf(u) {
    var host = u.hostname.toLowerCase().replace(/^(www|m|music)\./, ""), id = "";
    if (host === "youtu.be") id = u.pathname.slice(1).split("/")[0];
    else if (host === "youtube.com" || host === "youtube-nocookie.com") {
      id = u.searchParams.get("v") || (/^\/(?:embed|shorts|live)\/([^/?#]+)/.exec(u.pathname) || [])[1] || "";
    }
    if (!/^[A-Za-z0-9_-]{11}$/.test(id)) return null;
    var t = /^(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s?)?$/.exec(u.searchParams.get("t") || u.searchParams.get("start") || "");
    return { id: id, start: t ? (+t[1] || 0) * 3600 + (+t[2] || 0) * 60 + (+t[3] || 0) : 0 };
  }
  // A new tab with the site (without a way back to this page), or null where the browser stops it (it lets a page
  // open one only straight after a click, a tap or a key).
  function newTab(url) {
    try {
      if (navigator.userActivation && !navigator.userActivation.isActive) return null;
      var tab = window.open("", "_blank");
      if (!tab) return null;
      tab.opener = null;
      tab.location.href = url;
      return tab;
    } catch (e) { return null; }
  }
  var SITE_NAMES = { "google.com": "Google", "maps.google.com": "Google Maps", "youtube.com": "YouTube", "wikipedia.org": "Wikipedia" };
  // What's said and done for an address: the site, here or in a new tab (opened now: call this straight away).
  function siteAnswer(u) {
    var url = u.href, host = u.hostname.toLowerCase().replace(/^www\./, "");
    var name = SITE_NAMES[host] || (/(^|\.)wikipedia\.org$/.test(host) ? "Wikipedia" : host);
    var video = videoOf(u), here = video ? null : framed(url), tab = video || here ? null : newTab(url);
    if (video) return { text: "Here's the video, in YouTube's player.", done: ["Played a video on YouTube"],
                        act: function () { youtube(video.id, video.start, "YouTube"); } };
    if (here) return { text: "Here's " + name + ". Click in it to type, and scroll it as you'd scroll any page.", done: ["Opened " + host],
                       act: function () { show({ src: here, title: name, tab: "new" }); } };
    if (tab) return { text: "Opened " + name + " in a new tab. Most sites, " + name +
                        " among them, don't let another page show them, so here they open beside this one; on a Chat Computer " +
                        "they open right here.",
                      done: ["Opened " + host + " in a new tab"] };
    return { text: "Your browser stopped the new tab, so " + name + " is a click away, on the screen. (Most sites can't show " +
               "inside this page; on a Chat Computer they open right here.)", done: ["Showed a link to " + host],
             act: function () { show({ html: link(url, name, "On Chat Computer, " + name + " opens right here. Most sites can't show inside this page.") }); } };
  }
  function visit(text, u) {
    var a = siteAnswer(u);                    // first, while the key or the tap still lets a new tab open
    var you = uid("u"), uts = now(), id = "a" + (++turns), since = acted;
    call("user", you, text, true);
    setTimeout(function () { deliver(a, id, you, uts, since); }, 250);
  }
  // A page with a link to a site: most sites can't show inside this page (they forbid being framed).
  function link(url, name, note) {
    return page(name, '<article><h1 class="title">' + esc(name) + '</h1><p class="demo-note">' + esc(note) +
      '</p><p><a href="' + esc(url) + '" target="_blank" rel="noopener noreferrer">Open ' + esc(name) + " in a new tab</a></p></article>", NOTE_CSS);
  }
  function youtubeSearch(q, you, uts) {
    var url = "https://www.youtube.com/results?search_query=" + encodeURIComponent(q);
    return { text: "This page can't search YouTube by itself, so I can't play “" + clip(q, 50) + "” here. On a Chat " +
      "Computer I'd find it and play it right here. YouTube's search for it is a click away, on the screen.",
      done: ["Showed a link to YouTube's search for “" + clip(q, 40) + "”"],
      act: function () { show({ html: link(url, "YouTube: " + clip(q, 40), "On Chat Computer, YouTube opens right here and plays it. Other sites can't show inside this page.") }); } };
  }
  // Where a place is, from Open-Meteo's free geocoding (only the place's name goes there): for maps and the weather.
  function place_(name) {
    return fetch("https://geocoding-api.open-meteo.com/v1/search?count=1&language=en&format=json&name=" + encodeURIComponent(name))
      .then(function (r) { return r.json(); }).then(function (j) {
        var p = j && j.results && j.results[0];
        if (!p) throw new Error("not found");
        return { name: p.name, country: p.country || "", lat: +p.latitude, lon: +p.longitude };
      });
  }
  // The weather at a place, now and today's and tomorrow's, from Open-Meteo's forecast (only the place's position goes).
  function forecast(p) {
    return fetch("https://api.open-meteo.com/v1/forecast?latitude=" + p.lat + "&longitude=" + p.lon +
      "&current=temperature_2m,weather_code,is_day&daily=weather_code,temperature_2m_max,temperature_2m_min," +
      "precipitation_probability_max&timezone=auto&forecast_days=2").then(function (r) { return r.json(); });
  }
  // The place a question is about: one of the cities known here, or the name in "show me Rome on a map", "where is
  // Rome?", "weather in Rome"...; else "" (then it's the visitor's own, from their time zone).
  var PLACE_IN = [/\bshow me\s+([A-Za-zÀ-ɏ][\wÀ-ɏ .'-]{1,40}?)\s+on a map\b/i,
                  /\bwhere(?:'s| is)\s+([A-Za-zÀ-ɏ][\wÀ-ɏ .'-]{1,40}?)\s*[?.!]*$/i,
                  /\b(?:in|for|of|at|to|near)\s+([A-Za-zÀ-ɏ][\wÀ-ɏ .'-]{1,40}?)(?:\s+(?:today|tomorrow|tonight|now|this week|please|on a map))?\s*[?.!]*$/i];
  function placeIn(t) {
    var known = Object.keys(CITIES).filter(function (c) { return t.toLowerCase().indexOf(c) >= 0; })[0];
    if (known) return known;
    for (var i = 0; i < PLACE_IN.length; i++) {
      var m = PLACE_IN[i].exec(t), name = m ? m[1].replace(/^(the|a)\s+/i, "").trim() : "";
      if (name && !/^(the )?(weather|map|world|it|there|here|me|my area|work|bed|home|you)$/i.test(name)) return name;
    }
    return "";
  }
  // Where you live, when you've said ("I live in Penang"): for the weather and the greeting. Else your time zone's
  // city (hereName), as "Singapore" for Asia/Singapore.
  var PLACE_KEY = "chatcomputer-demo-place";
  function homeName() { var h = store(PLACE_KEY); return h && typeof h.place === "string" ? h.place : ""; }
  // Where you are and the weather there now, for the greeting under the word logo (chat.js: CO.here): the place's
  // name goes to Open-Meteo (then its position, for the weather), at most once every half hour; kept in this browser.
  var HERE_KEY = "chatcomputer-demo-here";
  function sayHere() {
    if (window.CO && window.CO.greets === false) return;     // no greeting for now: nothing to look up for it
    var place = homeName() || hereName(), kept = store(HERE_KEY) || {};
    if (!place) { call("here", {}); return; }
    if (kept.place !== place) kept = { place: place };
    if (now() - (kept.ts || 0) < 1800) { call("here", kept); return; }
    call("here", { place: place, pending: true });        // the place now, its weather once it's here
    (typeof kept.lat === "number" ? Promise.resolve(kept) : place_(place)).then(function (p) {
      return forecast(p).then(function (w) {
        var c = w && w.current;
        if (!c || typeof c.weather_code !== "number") throw new Error("no weather");
        var info = { place: place, lat: p.lat, lon: p.lon, code: c.weather_code, temp: c.temperature_2m, day: c.is_day === 1, ts: now() };
        store(HERE_KEY, info);
        call("here", info);
      });
    }).catch(function () { call("here", { place: place }); });
  }
  function hereName() {
    try {
      var z = Intl.DateTimeFormat().resolvedOptions().timeZone || "";
      return z.indexOf("/") > 0 ? z.split("/").pop().replace(/_/g, " ") : "";
    } catch (e) { return ""; }
  }
  var SKY = { 0: "clear", 1: "mostly clear", 2: "partly cloudy", 3: "cloudy", 45: "foggy", 48: "foggy", 51: "light drizzle",
              53: "drizzle", 55: "heavy drizzle", 61: "light rain", 63: "rain", 65: "heavy rain", 66: "freezing rain",
              67: "freezing rain", 71: "light snow", 73: "snow", 75: "heavy snow", 77: "snow", 80: "showers", 81: "showers",
              82: "heavy showers", 85: "snow showers", 86: "snow showers", 95: "thunderstorms", 96: "thunderstorms with hail",
              99: "thunderstorms with hail" };
  var lastWeather = null;                 // the weather asked for last: {place, tomorrow}, for "and tomorrow?"
  // ("and tomorrow?", "what about Tokyo?", "and in London tomorrow?": a day, a place, or both.)
  var WEATHER_AGAIN = /^\s*(?:(?:and|ok(?:ay)?|so|but)[,\s]+)?(?:(?:what|how) about|and|also)?\s*(?:(?:in|for|at)\s+)?(.*?)\s*(?:then|instead|too)?\s*[?.!]*\s*$/i;
  var DAYS = /^(?:(?:for |on )?(tomorrow|today|tonight|the day after(?: tomorrow)?))$|^(.+?)\s+(tomorrow|today|tonight)$|^(tomorrow|today|tonight)\s+(?:in|for|at)\s+(.+)$/i;
  function weatherAgain(t) {
    if (!lastWeather || t.length > 40 || !/^\s*((and|ok(ay)?|so|but)[,\s]+)?((what|how) about|and|also|in|for|at|tomorrow|today|tonight)\b/i.test(t)) return null;
    var rest = WEATHER_AGAIN.exec(t)[1], d = DAYS.exec(rest), day = lastWeather.tomorrow, where = lastWeather.place;
    if (d) {
      var w = d[1] || d[3] || d[4];
      day = /tomorrow|day after/i.test(w);
      if (d[2] || d[5]) where = d[2] || d[5];
    } else if (rest && /^[\p{L}][\p{L}\p{M}' .-]{1,40}$/u.test(rest) && !/\b(it|that|this|you|me|music|news|timer|map)\b/i.test(rest)) where = rest;
    else return null;
    return weather("weather" + (where ? " in " + where : "") + (day ? " tomorrow" : ""));
  }
  function weather(t) {
    var asked = placeIn(t), where = asked || homeName() || hereName() || "Singapore", tomorrow = /\btomorrow\b/i.test(t);
    lastWeather = { place: asked || "", tomorrow: tomorrow };
    var home = !asked && store(PLACE_KEY);           // where you live: found on the map when you said
    return { later: function () {
      return (home && typeof home.lat === "number" ? Promise.resolve({ name: home.place, lat: home.lat, lon: home.lon }) : place_(where)).then(function (p) {
        return forecast(p).then(function (w) {
          var d = w.daily, i = tomorrow ? 1 : 0, rain = d.precipitation_probability_max[i];
          var day = (tomorrow ? "Tomorrow" : "Today") + " in " + p.name + ": " + (SKY[d.weather_code[i]] || "mixed") + ", " +
            Math.round(d.temperature_2m_min[i]) + "° to " + Math.round(d.temperature_2m_max[i]) + "°, " +
            (rain == null ? "" : rain + "% chance of rain") + ".";
          var now_ = !tomorrow && w.current ? " It's " + Math.round(w.current.temperature_2m) + "° and " +
            (SKY[w.current.weather_code] || "mixed") + " now." : "";
          var brolly = /umbrella|rain/i.test(t) ? (rain >= 40 ? " Take an umbrella." : " You shouldn't need an umbrella.") : "";
          return { text: day + now_ + brolly + (asked ? "" : " (For somewhere else, say where.)"),
                   done: ["Looked up the weather for " + p.name] };
        });
      }).catch(function () {
        return { text: "I couldn't get the weather for " + where + " just now. On a Chat Computer I look it up on the web for you." };
      });
    } };
  }
  function mapOf(t) {
    var name = placeIn(t) || "";
    if (!name) return { text: "Which place? Say “map of” and its name, like “map of Tokyo”." };
    if (CITIES[name.toLowerCase()]) {
      return { text: "Here's " + title(name) + ".", done: ["Opened openstreetmap.org"], act: function () { map(CITIES[name.toLowerCase()], title(name)); } };
    }
    return { later: function () {
      return place_(name).then(function (p) {
        var b = [p.lon - 0.12, p.lat - 0.08, p.lon + 0.12, p.lat + 0.08].map(function (x) { return x.toFixed(4); });
        return { text: "Here's " + p.name + (p.country ? ", " + p.country : "") + ".", done: ["Opened openstreetmap.org"],
                 act: function () { map(b, p.name); } };
      }).catch(function () {
        return { text: "I couldn't find “" + clip(name, 40) + "” on the map. Try another name for it." };
      });
    } };
  }
  // A sum, or a percentage of a number: "what's 15% of 84.50", "12 * 7.5", "1/3 + 2".
  function sum(t) {
    var pc = /(\d+(?:\.\d+)?)\s*(?:%|percent)\s+of\s+(\d+(?:\.\d+)?)/i.exec(t);
    var v, shown;
    if (pc) { v = (+pc[1]) * (+pc[2]) / 100; shown = pc[1] + "% of " + pc[2]; }
    else {
      var e = t.replace(/^\s*(what'?s|what is|calculate|how much is|work out)\s*/i, "").replace(/[?=]+\s*$/, "")
        .replace(/\btimes\b|×/gi, "*").replace(/\bplus\b/gi, "+").replace(/\bminus\b/gi, "-").replace(/\bdivided by\b|÷/gi, "/");
      if (!/^[\d\s.+\-*/()]+$/.test(e) || !/\d\s*[+\-*/]\s*[\d(]/.test(e)) return null;
      try { v = Function('"use strict";return (' + e + ")")(); } catch (x) { return null; }
      shown = e.replace(/\s+/g, " ").trim();
    }
    if (typeof v !== "number" || !isFinite(v)) return null;
    var r = Math.round(v * 100) / 100, exact = Math.round(v * 1e6) / 1e6;
    return { text: shown + " is " + (r === exact ? r : r + " (" + exact + ")") + "." };
  }
  // ------------------------------------------------------------------ looking things up on the web
  // The model runs on the computer and can't reach the internet by itself, so this page looks things up for it
  // (web_search), from the browser, in sources that let a page ask them directly and need no key: Wikipedia (the opening of
  // its articles), Wikipedia's Current events (the day's news, a sentence or two each, with its sources) and Open-Meteo
  // (the air: haze, pollution). Only the words looked up go to them. The written answers use them too: the news, the
  // air, "look it up", and a question of fact (Wikipedia's article on it).
  var WIKI = "https://en.wikipedia.org/w/api.php?format=json&formatversion=2&origin=*&";
  function getJSON(url, ms) {
    var ctl = typeof AbortController === "function" ? new AbortController() : null;
    var late = setTimeout(function () { if (ctl) ctl.abort(); }, ms || 8000);
    return fetch(url, ctl ? { signal: ctl.signal } : undefined).then(function (r) {
      if (!r.ok) throw new Error("HTTP " + r.status);
      return r.json();
    }).then(function (j) { clearTimeout(late); return j; }, function (e) { clearTimeout(late); throw e; });
  }
  function plain(s) { return String(s == null ? "" : s).replace(/\s+/g, " ").trim(); }
  // Wikipedia's articles that match, best first, each with its opening (up to 900 letters).
  function wikipedia(q, n) {
    return getJSON(WIKI + "action=query&generator=search&gsrlimit=" + (n || 3) + "&gsrsearch=" + encodeURIComponent(q) +
      "&prop=extracts%7Cinfo&exintro=1&explaintext=1&exchars=900&inprop=url").then(function (j) {
      return ((j.query && j.query.pages) || []).filter(function (p) { return p.extract; })
        .sort(function (a, b) { return a.index - b.index; })
        .map(function (p) { return { source: "Wikipedia", title: p.title, url: p.fullurl, text: plain(p.extract) }; });
    });
  }
  // The day's news: Wikipedia's Current events, today's and yesterday's (Wikipedia's days are UTC's).
  var MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
  function news() {
    return Promise.all([0, 1].map(function (k) {
      var d = new Date(Date.now() - k * 86400000);
      var name = "Portal:Current_events/" + d.getUTCFullYear() + "_" + MONTHS[d.getUTCMonth()] + "_" + d.getUTCDate();
      return getJSON(WIKI + "action=parse&prop=text&disablelimitreport=1&disableeditsection=1&page=" + encodeURIComponent(name))
        .then(function (j) { return stories(j.parse && j.parse.text, d, name); }, function () { return []; });
    })).then(function (two) {
      var seen = {};
      return two[0].concat(two[1]).filter(function (x) { var k = x.text.toLowerCase(); if (seen[k]) return false; seen[k] = true; return true; });
    });
  }
  // A day's stories, from its page (read here, not shown): each with its topics ("Hurricane Melissa: ...") and sources.
  function stories(html, d, name) {
    if (!html || typeof DOMParser === "undefined") return [];
    var doc = new DOMParser().parseFromString(String(html), "text/html");
    var root = doc.querySelector(".current-events-content") || doc.body;
    var day = d.toLocaleDateString("en-GB", { weekday: "long", day: "numeric", month: "long", timeZone: "UTC" });
    var out = [];
    Array.prototype.forEach.call(root.querySelectorAll("li"), function (li) {
      if (li.querySelector("ul, ol")) return;                         // a topic: its stories are below it
      var topics = [];
      for (var p = li.parentElement; p && p !== root; p = p.parentElement) if (p.tagName === "LI") topics.unshift(ownWords(p));
      var links = Array.prototype.filter.call(li.querySelectorAll("a.external, a[rel~=nofollow]"), function (a) {
        return /^https:\/\//.test(a.getAttribute("href") || "");
      }).map(function (a) { return { name: plain(a.textContent).replace(/^\(|\)$/g, ""), url: a.getAttribute("href") }; });
      var text = plain(li.textContent);
      if (text.length < 30) return;
      out.push({ source: "Wikipedia's Current events, " + day, title: topics.filter(Boolean).join(": "), text: text,
                 url: "https://en.wikipedia.org/wiki/" + name, links: links, day: day });
    });
    return out;
  }
  function ownWords(li) {
    var s = "";
    Array.prototype.forEach.call(li.childNodes, function (n) { if (!/^(UL|OL)$/.test(n.nodeName)) s += n.textContent; });
    return plain(s);
  }
  // The air now where they are (or where asked): Open-Meteo's, from the Copernicus forecast; the US air quality index.
  var AQI = [[50, "good"], [100, "moderate"], [150, "unhealthy for sensitive people"], [200, "unhealthy"], [300, "very unhealthy"],
             [Infinity, "hazardous"]];
  function air(where) {
    return place_(where).then(function (p) {
      return getJSON("https://air-quality-api.open-meteo.com/v1/air-quality?latitude=" + p.lat + "&longitude=" + p.lon +
        "&current=us_aqi,pm2_5,pm10&timezone=auto").then(function (j) {
        var c = j.current || {}, n = Math.round(c.us_aqi);
        if (!isFinite(n)) return [];
        var band = AQI.filter(function (b) { return n <= b[0]; })[0][1];
        return [{ source: "Open-Meteo (the Copernicus forecast)", title: "The air in " + p.name + " now", url: "https://open-meteo.com/",
          air: true, text: "The air in " + p.name + " now is " + band + ": the US air quality index is " + n + ", PM2.5 " +
          Math.round(c.pm2_5) + " µg/m³, PM10 " + Math.round(c.pm10) + " µg/m³." }];
      });
    });
  }
  var NEWSY = /\b(news|headlines?|happen(s|ed|ing)?|current events|going on|in the world)\b/i;
  var AIRY = /\b(haze|hazy|air quality|air pollution|pollution|smog|psi|aqi|pm ?2\.?5)\b/i;
  // Looks q up (for a question, asked): the air for haze words, the day's news for news words, and Wikipedia's articles
  // unless it's just the news. What it looked up goes in Actions.
  function lookup(q, asked) {
    var all = q + " " + (asked || ""), jobs = [], told = [];
    var justNews = !q.replace(/\b(the|today'?s|todays|today|latest|top|world|current|news|headlines?|stories|events|this|week|yesterday'?s|yesterday|in|on|what'?s|what|is|are|was|happened|happening|going)\b|[^\w\s]/gi, "").trim();
    if (AIRY.test(all)) {
      var where = placeIn(all) || hereName() || "Singapore";
      jobs.push(air(where));
      told.push("Looked up the air in " + title(where));
    }
    if (NEWSY.test(all)) { jobs.push(news()); told.push("Read the news on Wikipedia"); }
    if (!justNews) { jobs.push(wikipedia(q)); told.push("Searched Wikipedia for “" + clip(q, 50) + "”"); }
    return Promise.all(jobs.map(function (j) { return j.catch(function () { return []; }); })).then(function (lists) {
      return { list: [].concat.apply([], lists), told: told };
    });
  }
  // In the conversation: the latest stories, a line each with its sources, from Wikipedia's Current events.
  function newsWords(list) {
    var items = list.filter(function (x) { return x.links; }).slice(0, 8);
    return "The latest news, from Wikipedia's Current events:\n\n" + items.map(function (x) {
      var topic = (x.title || "").split(": ").pop();
      return "- " + (topic ? "**" + topic.replace(/[*_]/g, "") + ":** " : "") + clip(x.text.replace(/[*_]/g, ""), 260);
    }).join("\n");
  }
  // Wikipedia's article, in the conversation: its opening, and where it's from.
  function wikiWords(x) { return "From Wikipedia's article “" + x.title + "”:\n\n" + x.text; }
  // The written answers that look things up: the news, the air, and Wikipedia's article on something (else: otherwise).
  function newsAnswer() {
    return news().then(function (list) {
      if (!list.length) throw new Error("no news");
      return { text: newsWords(list), done: ["Read the news on Wikipedia"] };
    }).catch(function () {
      return { text: "I couldn't get the news just now. BBC News is a click away, on the screen.",
               act: function () { show({ html: link("https://www.bbc.com/news", "BBC News", "On Chat Computer, news sites open right here.") }); } };
    });
  }
  function airAnswer(t) {
    var where = placeIn(t) || hereName() || "Singapore";
    return air(where).then(function (list) {
      if (!list.length) throw new Error("no reading");
      return { text: list[0].text + " (Open-Meteo's estimate, from the Copernicus forecast.)", done: ["Looked up the air in " + title(where)] };
    }).catch(function () { return { text: "I couldn't get the air quality for " + title(where) + " just now." }; });
  }
  function wikiAnswer(q, otherwise) {
    return wikipedia(q).then(function (list) {
      if (!list.length) throw new Error("nothing");
      return { text: wikiWords(list[0]), done: ["Searched Wikipedia for “" + clip(q, 50) + "”"] };
    }).catch(function () {
      if (otherwise) return otherwise;
      return { text: "Wikipedia has nothing on “" + clip(q, 50) + "”, so here's Google's search for it.",
               done: ["Searched Google for “" + clip(q, 40) + "”"],
               act: function () { show({ src: googleSearch(q), title: "Google: " + clip(q, 60) }); } };
    });
  }
  // "Look it up" (what was asked before), "search the web for X", "google X", "look up X": their files aren't the web.
  function topicOf(t) {
    return String(t).replace(/^\s*(please\s+)?(tell me about|explain|define|(who|what|when|where|which|whose|why|how)('s|'re| is| are| was| were| did| does| do)?)\s+/i, "")
      .replace(/\b(simply|please|to me|for me)\b|[?.!]+\s*$/gi, "").replace(/\s+/g, " ").trim() || String(t).trim();
  }
  // Questions whose answer changes, or came after any AI's training: looked up first, so it's not guessed (an AI
  // otherwise answers "who is the prime minister" with the one it remembers).
  var FRESH = /\b(who('s| is| are| was)? (the )?(current |new )?(president|prime minister|premier|chancellor|ceo|chief executive|chair(man|woman)?|king|queen|monarch|pope|leader|head of|minister|mayor|governor|coach|manager|captain|champion|owner)|who won|winners?|latest|current(ly)?|right now|nowadays|these days|today|tonight|yesterday|this (week|month|year|season)|last (week|month|year|night|season)|20(2[4-9]|3\d)|price of|how much (is|are|does|do)|stock price|exchange rate|scores?|results?|standings|election|released|launch(ed)?)\b/i;
  // Typing that isn't words ("asdfgh", "qwerty"): not a question for anyone.
  var MASH = /^\s*([asdfghjkl;']{4,}|[qwertyuiop]{4,}|[zxcvbnm,.]{4,}|(.)\2{3,})\s*[?!.]*\s*$/i;
  var LOOK_IT_UP = /^\s*(?:please\s+)?(?:(?:can|could|would) you\s+)?(?:look (?:it|that|this|them) up|search (?:for )?(?:it|that|this)|google (?:it|that|this)|search (?:the web|online|the internet)|check (?:online|the web|the internet)|find out)(?:\s+(?:for me|please|online|on the web|on the internet|on wikipedia|now))*\s*[?.!]*\s*$/i;
  var SEARCH_FOR = /^\s*(?:please\s+)?(?:(?:can|could|would) you\s+)?(?:search (?:the (?:web|internet) |online |wikipedia |google )?for|look up|google|search (?:the web|the internet|online|wikipedia|google)(?: for)?|find out(?: about)?)\s+(?!(?:my|the)?\s*files?\b|my\b)(.+?)\s*[?.!]*\s*$/i;
  var lastAsked = "";
  function lookUp(t) {
    var m = SEARCH_FOR.exec(t), q = m ? m[1] : lastAsked && topicOf(lastAsked);
    if (m && (/\b(weather|forecast|temperature)\b/i.test(q) || search(q.toLowerCase().replace(/\b(the|my|for|files?)\b/g, " ").replace(/\s+/g, " ").trim()).length)) return null;
    if (!q) return { text: "What should I look up? Say “look up” and what, like “look up the Southeast Asian haze”." };
    return { search: q, later: function () { return wikiAnswer(q); } };
  }

  var TEXT_UP = /\b(bigger|larger|big|large|increase[ds]?|increasing|enlarge|raise|up|zoom in|more|grow|magnify|huge|max(imum)?)\b/i;
  var TEXT_DOWN = /\b(smaller|small|decrease[ds]?|decreasing|reduce[ds]?|reducing|shrink|lower|down|zoom out|less|tiny|min(imum)?)\b/i;
  var JOKES = ["I told my computer I needed a break. It said: “No problem, I'll go to sleep.”",
               "Why did the computer go to the doctor? It had a virus, and a terrible case of bytes.",
               "I'd tell you a joke about Wi-Fi, but I'm not sure you'd get the connection."];
  var OTHER = {
    zh: "你好！这是 Chat Computer 的屏幕：它替你操作整台电脑，你只要说话、打字或触摸屏幕。这个页面的回答是预先写好的，而且目前只用英文。试试输入 “play some piano music”。",
    ja: "こんにちは！これは Chat Computer の画面です。話す・打つ・触るだけで、コンピューター全体を動かしてくれます。このページでは今のところ英語でお試しください。",
    es: "¡Hola! Esto es Chat Computer: maneja todo el ordenador por ti, y tú solo hablas, escribes o tocas la pantalla. " +
        "Por ahora esta página responde en inglés: prueba “play some piano music” o “show me a map of Tokyo”.",
    ms: "Hai! Ini Chat Computer: ia menjalankan seluruh komputer untuk anda, dan anda hanya bercakap, menaip atau menyentuh skrin. " +
        "Buat masa ini halaman ini menjawab dalam bahasa Inggeris: cuba “play some piano music”."
  };
  function language(t) {
    if (/[぀-ヿ]/.test(t)) return "ja";
    if (/[一-鿿]/.test(t)) return "zh";
    if (/[¿¡]|\b(hola|qué|que es|cómo|esto|puedes|gracias)\b/i.test(t)) return "es";
    if (/\b(apa (ini|itu|khabar)|boleh|awak|anda|terima kasih|saya)\b/i.test(t)) return "ms";
    return "";
  }
  // Asked to write, explain or translate something: only the model can (else an honest note, see fallback), whatever
  // else the words mention ("write an email to my boss" isn't about Jane's).
  var OPEN_ENDED = /\b(write|draft|compose|poem|story|essay|explain|summari[sz]e|translate|how do you say|in (japanese|french|spanish|german|chinese|korean|italian)|recipe for|how (do i|to) (make|cook|bake)|plan (a|my)|itinerary|advice|tips|capital of|python|javascript)\b/i;
  // What was asked for, for the honest note: "I can't write a recipe on this page without an AI".
  var WANTS = [[/recipe|cook|bake/i, "write a recipe"], [/poem/i, "write a poem"], [/story/i, "write a story"],
               [/email|letter/i, "write an email"], [/python|javascript|code|function/i, "write code"],
               [/translate|how do you say|in (japanese|french|spanish|german|chinese|korean|italian)/i, "translate"],
               [/explain|summari/i, "explain that"], [/plan|itinerary|trip/i, "plan that"], [/advice|tips/i, "give advice"]];
  // The name you go by, kept in this browser: the greeting under the word logo says it, as the Claude app's says
  // the name you give it. "Call me Sam" or "my name is Sam" sets it (the model, with set_name); "what's my name?" says
  // it; "forget my name" forgets it. "Call me back" isn't a name.
  var NAME_KEY = "chatcomputer-demo-name";
  var NAME_IS = /^\s*(?:(?:please|pls|just|you can|you may|from now on,?)\s+)*(?:call me|my name(?:'s| is))\s+([\p{L}][\p{L}\p{M}'’.-]*(?:\s+[\p{L}][\p{L}\p{M}'’.-]*){0,2}?)\s*(?:,?\s*(?:please|pls|thanks|thank you|from now on|from now|from today|going forward|instead|in future))*\s*[.!]*\s*$/iu;
  var NOT_A_NAME = /^(back|later|tomorrow|tonight|today|now|soon|again|anytime|whenever|maybe|sometime|sometimes|asap|please|a|an|the|when|if|at|on|in|by|after|before|crazy|that|this|it|him|her|them|you|me|what|anything|whatever|out|up|ok|okay|not|never|no|yes)$/i;
  var WHO_AM_I = /^\s*(?:what(?:'s| is) my name|who am i|do you know (?:my name|who i am)|what (?:do|will) you call me)\s*[?.!]*\s*$/i;
  var FORGET_NAME = /^\s*(?:please\s+)?(?:forget|remove|delete|clear|erase)\s+my name\b|^\s*(?:don'?t|do not|stop)\s+(?:call(?:ing)? me by|us(?:e|ing)) my name\b/i;
  // Where you live: "I live in Penang", "I'm based in Penang", "my city is Penang" (found on the map first).
  var LIVE_IN = /^\s*(?:i (?:live|stay)|i(?:'m| am) (?:based|living|staying))\s+in\s+(.+?)\s*[.!]*\s*$|^\s*(?:my (?:city|town|location) is|set my (?:city|town|location) to)\s+(.+?)\s*[.!]*\s*$/i;
  var FORGET_PLACE = /^\s*(?:please\s+)?(?:forget|remove|delete|clear|erase)\s+(?:where i live|my (?:city|town|location|place))\b/i;
  function setPlace(p) {
    store(PLACE_KEY, p ? { place: p.name, lat: p.lat, lon: p.lon } : null);
    store(HERE_KEY, p ? { place: p.name, lat: p.lat, lon: p.lon } : null);
    sayHere();
  }
  function cleanName(s) { return String(s || "").replace(/[^\p{L}\p{M}'’ .-]/gu, "").replace(/\s+/g, " ").trim().slice(0, 40); }
  function person() { return cleanName(store(NAME_KEY)); }
  // The name and which model answers, for the next greeting (a visit, or the conversation cleared).
  function greetWith() { call("settings", person() || computerName, own ? ownName() : localReady() ? local.label : ""); }
  var computerName = "";                 // (in the app: the first name of this computer's account, unless one's given)
  function nameOf(t) {
    var m = NAME_IS.exec(t), n = m ? cleanName(m[1]) : "";
    if (!n || NOT_A_NAME.test(n.split(" ")[0])) return "";
    return n === n.toLowerCase() ? n.replace(/(^|\s)(\p{L})/gu, function (a, s, c) { return s + c.toUpperCase(); }) : n;
  }
  function setName(n) { store(NAME_KEY, n || null); greetWith(); }
  // Asking about an AI of their own (Claude, ChatGPT, Gemini...: "can I use my ChatGPT account here?", "I pay for
  // Claude, can I use it with this?"), changing the model, a better one, how hard it thinks, or the model in this
  // browser. (Not "are you Claude?" or "is this made by Anthropic?": who answers, and who made it, below.)
  var AI_NAMES = "chat ?gpt|openai|gpt-?\\d[\\w.-]*|claude|anthropic|gemini|grok|xai|deepseek|kimi|moonshot|qwen|mistral|llama|meta ai|openrouter|glm|minimax|mimo";
  var OWN_AI = new RegExp("\\b(" + AI_NAMES + ")\\b.*\\b(use|using|connect|sign(ed)? in|log(ged)? in|account|subscription|plan|api key|key|with (this|it|you)|here|instead)\\b|" +
    "\\b(use|using|connect|sign in (with|to)|log in (with|to)|bring|plug in|add)\\b.*\\b(my own|own|my)\\b.*\\b(ai|model|llm|api key|key|account|subscription)\\b|" +
    "\\b(change|switch|swap|pick|choose|select|set|use) (the |your |my |a |another |which )?(ai|model|llm|brain)s?\\b|" +
    "\\b(better|smarter|bigger|stronger|cleverer|another|different|other) (ai|model|llm)s?\\b|" +
    "\\b(think (harder|more|longer|deeper|more carefully|carefully)|effort|reasoning (level|effort|mode)|thinking (mode|level)|how hard (do |does )?(you|it) think)\\b|" +
    "\\b(run|runs|running|work|works)\\b.*\\b(in|inside|on) (my|the|this) (web )?browser\\b|\\b(ai|model)\\b.*\\bin (my|the|this) (web )?browser\\b", "i");
  // An AI's name as it's said, as the panel names it (and which of the panel's AIs that is).
  function panelName(n) {
    n = n.toLowerCase().replace(/\s+/g, " ");
    if (/^(xai|grok)$/.test(n)) return "Grok";
    if (/^(kimi|moonshot)$/.test(n)) return "Kimi";
    if (/^(llama|meta ai)$/.test(n)) return "Meta";
    var names = { gemini: "Gemini", deepseek: "DeepSeek", qwen: "Qwen", mistral: "Mistral", glm: "GLM", minimax: "MiniMax",
                  mimo: "MiMo", openrouter: "OpenRouter" };
    return names[n] || title(n);
  }
  function ownKeyOf(label) { for (var k in AIS) if (AIS[k].name === label) return k; return ""; }
  function ownAiAnswer(t) {
    var spark = "Press the spark beside +";
    var rest = APP && APP.channel ? "" : " Or, with one account for all of them, Continue with OpenRouter there.";
    var how = " What you type then goes to that AI, and its key stays " + (APP ? "in the app" : "in this browser") + ". In the same panel " +
      "you choose its model, and how hard it thinks (its effort, where its maker offers one).";
    if (/\b(run|runs|running|work|works)\b.*\bbrowser\b|\bbrowser\b/i.test(t) && !APP) {
      if (!webCan()) return { text: "Not in this browser: " + ((ENGINE && ENGINE.supported().why) || "it can't run my model") + ". In a " +
        "newer Chrome, Edge, Firefox or Safari it can. " + spark + " to use an AI you already have." };
      return { text: webOn() ? "It is: " + local.label + ", Chat Computer's own model, answers now, in this browser." :
        "Yes: type / and choose Run my model in this browser. It's Qwen3.5 0.8B, Chat Computer's own model: it downloads once (" +
        WEB_SIZE + ") and then runs on this " + deviceWord() + "'s own processor, free, with no account, and nothing you type leaves " +
        "the browser. It's slow, though, and small: a browser gives it one core." };
    }
    if (/\b(think (harder|more|longer|deeper|more carefully|carefully)|effort|reasoning|thinking (mode|level)|how hard)\b/i.test(t)) {
      return { text: (webOn() ? "My model here doesn't think before answering: it's small, and a browser runs it slowly." :
        "Here my answers are written in advance, so there's no thinking to turn up.") + " With an AI of yours, you choose how " +
        "hard it thinks: " + spark.toLowerCase() + ", choose the AI, then its effort (from Low up to Max, where its maker offers " +
        "it). Harder thinking answers better and takes longer." };
    }
    var gpt = /\b(chat ?gpt|openai|gpt-?\d[\w.-]*)\b/i.test(t), claude = /\b(claude|anthropic)\b/i.test(t);
    if (gpt) {
      if (APP && APP.channel) return { text: "Yes: " + spark.toLowerCase() + " and choose Continue with ChatGPT: you sign in with " +
        "your ChatGPT plan, and its models answer here." + how };
      return { text: "Yes, with an OpenAI API key (made on OpenAI's platform, paid as you use it; a ChatGPT plan isn't one): " +
        spark.toLowerCase() + ", choose OpenAI and paste it." + rest + how + " Signing in with your ChatGPT plan itself comes with " +
        "the Chat Computer app (coming soon)." };
    }
    if (claude) {
      return { text: "Yes, with an Anthropic API key: " + spark.toLowerCase() + ", choose Claude and paste one (made at " +
        "platform.claude.com, paid as you use it). A Claude subscription (Pro or Max) isn't an API key, so it can't answer here." +
        rest + how };
    }
    var named = new RegExp("\\b(" + AI_NAMES + ")\\b", "i").exec(t), label = named && panelName(named[1]);
    if (label === "OpenRouter") return { text: "Yes: " + spark.toLowerCase() + " and choose Continue with OpenRouter (or paste an " +
      "OpenRouter key): its free models answer, or the best of each maker's on your OpenRouter credits." + how };
    return { text: (label ? "Yes: " + spark.toLowerCase() + ", choose " + label + " and paste your API key from " +
      (AIS[ownKeyOf(label)] && AIS[ownKeyOf(label)].by || label) + "." : spark + " to have an AI you already have answer: Claude, ChatGPT, Gemini, Grok, DeepSeek, Kimi, Qwen, Mistral " +
      "and the other top AIs, each with your own API key from its maker.") + rest + how +
      (APP || webOn() ? "" : " Or type / and choose Run my model in this browser: Chat Computer's own model, free, but small and slow.") };
  }

  // How many minutes are asked for: "5 minutes", "a five minute timer", "twenty-five mins", "half an hour", "an hour
  // and a half", "2 hours" (0: none said).
  var NUMBERS = { a: 1, an: 1, one: 1, two: 2, three: 3, four: 4, five: 5, six: 6, seven: 7, eight: 8, nine: 9, ten: 10, eleven: 11,
                  twelve: 12, thirteen: 13, fourteen: 14, fifteen: 15, sixteen: 16, seventeen: 17, eighteen: 18, nineteen: 19,
                  twenty: 20, thirty: 30, forty: 40, fifty: 50, sixty: 60, ninety: 90 };
  function minutesIn(t) {
    var n = "(\\d+(?:\\.\\d+)?|(?:twenty|thirty|forty|fifty)(?:[ -](?:one|two|three|four|five|six|seven|eight|nine))?|" +
      "one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|sixty|ninety|an?)";
    function num(w) {
      if (/^\d/.test(w)) return parseFloat(w);
      var parts = w.toLowerCase().split(/[ -]/);
      return parts.reduce(function (a, x) { return a + (NUMBERS[x] || 0); }, 0);
    }
    if (/\bhalf an? hour\b/i.test(t)) return 30;
    if (/\b(a )?quarter of an hour\b/i.test(t)) return 15;
    var h = new RegExp("\\b" + n + "\\s*(?:hours?|hrs?)\\b(\\s*and a half)?", "i").exec(t);
    if (h) return Math.round(num(h[1]) * 60 + (h[2] ? 30 : 0));
    var m = /\b(\d+(?:\.\d+)?)\s*(?:min|mins|minutes?|m)\b/i.exec(t) || new RegExp("\\b" + n + "[ -](?:min|mins|minutes?)\\b", "i").exec(t);
    return m ? Math.round(num(m[1])) : 0;
  }
  var INTENTS = [
    [NAME_IS, function (t) {
      var n = nameOf(t);
      if (!n) return null;
      setName(n);
      return { text: "Nice to meet you, " + n + "! I'll call you that from now on.", done: ["Set your name to " + n] };
    }],
    [WHO_AM_I, function () {
      var n = person();
      return { text: n ? "You're " + n + ". (To change it, say “call me” and the name you'd like.)" :
        "I don't know your name yet. Say “call me” and your name, and I'll call you that." };
    }],
    [FORGET_NAME, function () {
      var had = !!person();
      setName("");
      return had ? { text: "Done: I've forgotten your name.", done: ["Forgot your name"] } : { text: "I didn't have your name." };
    }],
    [LIVE_IN, function (t) {
      var m = LIVE_IN.exec(t), asked = String(m[1] || m[2] || "").replace(/^(the|a)\s+/i, "").replace(/[^\p{L}\p{M}' .,-]/gu, "").trim().slice(0, 60);
      if (!asked || /^(a |an )|^(peace|fear|hope|the moment|denial|my head)$/i.test(asked)) return null;
      return { later: function () {
        return place_(asked).then(function (p) {
          // In your words ("Penang", where the map's nearest is George Town), or the map's spelling of them.
          var shown = p.name.toLowerCase() === asked.toLowerCase() || asked === asked.toLowerCase() && p.name.toLowerCase().indexOf(asked.toLowerCase()) === 0 ?
            p.name : asked === asked.toLowerCase() ? title(asked) : asked;
          setPlace({ name: shown, lat: p.lat, lon: p.lon });
          return { text: "Got it: you live in " + shown + (p.country && p.country !== shown ? ", " + p.country : "") + ". Say " +
            "“weather” for its forecast.", done: ["Set your place to " + shown] };
        }, function () { return { text: "I couldn't find " + asked + " on the map. Which city is it in, or near?" }; });
      } };
    }],
    [FORGET_PLACE, function () {
      var had = !!homeName();
      setPlace(null);
      return had ? { text: "Done: I've forgotten where you live. I'll go by your time zone again.", done: ["Forgot your place"] } :
        { text: "I didn't know where you live: I go by your time zone" + (hereName() ? " (" + hereName() + ")." : ".") };
    }],
    // Closing, and the volume, before music: "stop the music", "close the song" or "turn the music down" have music
    // words in them too (and would start it again).
    [/^(close|stop|pause|cancel)\b|\b(close|stop|pause|hide|turn off|switch off|cancel|get rid of|dismiss)\s+(the\s+|that\s+|this\s+|my\s+)?(it|that|this|music|videos?|page|songs?|piano|playing|youtube|timer|map)\b|\b(turn|switch)\s+(it|that|this|the\s+(music|video|song|piano))\s+off\b/i, function () {
      return { text: view ? "Closed it." : "Nothing's open.", act: close };
    }],
    [/\b(volume|louder|quieter|softer|mute|unmute)\b|\bturn\s+(up|down|(it|that|this|the\s+(music|sound|song|video|piano|volume))\s+(up|down))\b/i, function (t) {
      if (/\bmute\b/i.test(t)) return { text: "Muted.", done: ["Muted the sound"], act: function () { return volumeTo(0); } };
      var asked = /\b(\d{1,3})\s*(%|percent\b)?/i.exec(t);
      if (asked) {
        var p = Math.min(100, +asked[1]);
        return { text: "Volume is " + p + "%.", done: ["Set the volume to " + p + "%"], act: function () { return volumeTo(p); } };
      }
      var up = /\b(louder|up|unmute)\b/i.test(t);
      return { text: up ? "Volume is 60%." : "Volume is 40%.", done: [up ? "Turned the volume up" : "Turned the volume down"],
        act: function () { return volumeTo(up ? 60 : 40); } };
    }],
    // Choosing who answers: an AI of yours (the spark beside +), its model and how hard it thinks; this browser's model.
    [OWN_AI, ownAiAnswer],
    // Who's answering, and who made it.
    [/\b(which|what) (ai|model|llm)('s| is| are)? (is |are )?(answering|replying|talking|running|responding|this|that|you|here|used|in use|behind)\b|\b(are you|is this|who are you|which (ai|model)|what (ai|model))\b.*\b(claude|chatgpt|gpt|gemini|qwen|ai|real|human|bot|model)\b|^(who are you|are you real)\b|\bwhat('s| is) (running|powering|behind) (you|this)\b|\bwho('s| is) (answering|talking|running this)\b|\bpowered by\b|\bwhat powers (you|this|it)\b|\bwhat (are you|is this|is it) (running|built|based) on\b|\bwhat('s| is) your (model|engine|brain|ai)\b/i, function () {
      return { text: localReady() ? "Right now, " + local.label + ", Chat Computer's own model, " + (local.kind === "web" ?
        "in this browser (with the page's own answers for music, maps, the weather, timers and looking things up)" : "on this computer") +
        ": no account, and nothing you say leaves it." : "Here, answers written in advance. " + FACTS.indie + " " + nextStep() };
    }],
    // Whether the model is coming ("are you stuck downloading?"): how far the app has got with it.
    [/\b(stuck|still|how long|how much longer|progress|done|finished|ready|working)\b.*\b(download(ing|ed)?|model|qwen)\b|\b(download(ing)?|model|qwen)\b.*\b(stuck|slow|done|ready|finished|working|progress)\b/i, function () {
      if (localReady()) return { text: "It's here: ask me anything." };
      return { text: nextStep() };
    }],
    // Whether it can go on the internet ("why can't you look things up?"): it can, through this page.
    [/^(?!.*\b(files?|photos?|pictures?|documents?)\b).*\b(can|do|could|are|why can'?t|why don'?t|why won'?t)\s+you\b.*\b(internet|online|the web|search|google|look (things|it|stuff|that|this) up|browse)\b/i, function () {
      return { text: "I can: this page looks things up for me, on Wikipedia and in the day's news, and only the words looked up " +
        "leave your browser. Ask me something, or say “look it up” after a question." };
    }],
    [/\bwho (made|built|makes|created|is behind)\b|\bwho'?s behind\b|\bmade by\b|\banthropic\b/i, function (t) {
      return { text: (/\banthropic\b/i.test(t) && /\bis (this|it)|made by\b/i.test(t) ? "No. " : "") + FACTS.indie };
    }],
    [/\b(real|my real|actual)\b.*\bmessages?\b|\bmessages?\b.*\b(real|fake|made up)\b|\bare these (mine|real)\b/i, function () {
      return { text: APP ? "Yes: Messages shows yours, from the accounts you connect with + in Messages (Gmail, Outlook, Telegram, " +
        "WhatsApp, WeChat and Discord), each signed in on its own page here." :
        "No, they're made up for this page. On a Chat Computer, Messages shows yours, from the email and chat accounts you connect." };
    }],
    [/\b(my (actual|real|own) (computer|files|pc)|on my (computer|pc|laptop)|control my (pc|computer))\b/i, function () {
      return { text: APP ? "Your files, yes: type / and a name, and I find it on this computer; chosen, it opens in its own app. " +
        "Programs, settings and the rest of the computer stay yours to run: this app doesn't control them (Chat Computer itself, " +
        "installed in place of a desktop, does)." : "Not from this page: a website can't reach your computer, and the files here are made up. Installed, " +
        "Chat Computer opens your own files, from your Windows folders too." };
    }],
    [/\bwhat (can'?t|can not|cannot) you do\b|\byour limits\b|\bwhat don'?t you do\b/i, function () {
      return { text: (webOn() ? "Here I'm small: " + local.label + " in this browser, which writes and talks but knows little and " +
        "can get things wrong, and the page itself shows only a few things: music the browser makes, maps, timers, the weather, " +
        "the news, Wikipedia's articles and the made-up messages and files." : "With answers written in advance I can only show a few " +
        "things: music the browser makes, maps, timers, the weather, the news, Wikipedia's articles and the made-up messages and files. " +
        (APP ? "Real answers need my model: this app's, on this computer." : realAnswers())) + " On a Chat Computer, it does most of what a computer " +
        "does, by talking; it doesn't run Windows or Mac apps (it uses websites instead), and Windows stays one restart away." };
    }],
    // Getting it: what it needs, what it costs, whether it's safe.
    [/\b(install|download|get it|get this|buy|where (can|do) i get|how (do|can) i (get|try|use) (it|this|chat ?computer)|sign ?up|waitlist)\b|^download\b/i, function () {
      return { text: FACTS.soon + " " + FACTS.free + " " + FACTS.pc + " Macs with M1, M2 or M3 too. Until then, this page shows what it does: " + TRY };
    }],
    [/\b(price|cost|costs|how much|free|paid|pay|subscription|charge)\b/i, function () { return { text: FACTS.free + " " + FACTS.soon }; }],
    [/\b(mac|macbook|imac|mac mini|macos|apple)\b/i, function () { return { text: "Yes. " + FACTS.mac + " " + FACTS.soon }; }],
    [/\b(delete|remove|wipe|replace|erase|lose|keep|safe)\b.*\bwindows\b|\bwindows\b.*\b(safe|deleted?|removed?|stay|gone|still)\b|\bdual ?boot\b/i, function () {
      return { text: "No. " + FACTS.pc + " You can go back to Windows for one restart, every day, or remove Chat Computer for good." };
    }],
    [/\bwindows ?(10|11|7|8)\b|\b(old|older|my) (laptop|pc|computer)\b.*\b(use|run|work|install)\b/i, function () {
      return { text: "Very likely. " + FACTS.pc + " It installs next to Windows 10 or 11, which stays. " + FACTS.soon };
    }],
    [/\b(requirements?|what do i need|system requirements|specs|minimum|need to (run|use|install))\b/i, function () {
      return { text: "A Windows PC that starts in UEFI mode (any from the last ten years) with about 70 GB free, or a Mac with " +
        "M1, M2 or M3; 8 GB of memory or more (16 GB or more for a smarter model); and the internet, to get it and its model " +
        "once. " + FACTS.use + " " + FACTS.soon };
    }],
    [/\b(privacy|private|my data|data|secure|security|safe to use|track|spy|listening to me)\b/i, function () {
      return { text: "On this page, what you type stays in your browser (with my model in this browser too; with this computer's " +
        "model it goes to the app, on this computer). With an AI of yours (the spark beside +), it goes to that AI's maker, as on " +
        "its own site. For a map or the weather only the place's name goes out (to Open-Meteo), and when the model looks something " +
        "up or you ask for the news, only the words looked up go out (to Wikipedia). " +
        "Google's own pages show here when you open Google, ask for directions or look up something Wikipedia hasn't got: " +
        "those words go to Google, as they would on google.com. " +
        FACTS.private };
    }],
    [/\b(do i need|need|require|have to have)\b.*\b(airpods|headphones|earbuds|a keyboard|a mouse|a touch ?screen)\b/i, function () {
      return { text: "No. " + FACTS.use + " AirPods are for talking to it." };
    }],
    // What it is.
    [/\b(operating system|an os|is (this|it) an? (os|operating))\b/i, function () {
      return { text: "Yes, in a way: Chat Computer runs the whole computer, with its own model in charge instead of a desktop. Underneath, " +
        "it uses Linux (Ubuntu on a PC) for Wi-Fi, sound and the screen, next to Windows or macOS. This page shows its screen. " + TRY };
    }],
    // Someone choosing for a parent: whether it fits them, and what setting it up takes (README's "Installing").
    // Websites can't use the camera or microphone in Chat Computer yet (it turns their requests down), so no calls.
    [/\b(father|mother|dad|mum|mom|parents?|grand(ma|pa|mother|father)|elderly|older person|senior)\b.*\b(computer|laptop|pc|windows|struggles?|technology|tech)\b/i, function (t) {
      var she = /\b(mother|mum|mom|grandma|grandmother)\b/i.test(t), he = /\b(father|dad|grandpa|grandfather)\b/i.test(t);
      var p = she && !he ? "She just says or types what she wants" : he && !she ? "He just says or types what he wants" :
        "They just say or type what they want";
      return { text: "Chat Computer is made for that: there's no desktop, no menus and no apps to learn. " + p + ", like the news " +
        "or a YouTube video, and it does it, on a big, plain screen. Video calls are the " +
        "exception: it can't make them yet.\n\nSetting it up takes a keyboard and mouse, once: on the Windows PC you run Chat " +
        "Computer Setup, and it makes room and installs itself next to Windows, usually in well under an " +
        "hour; Windows stays. The PC must start in UEFI mode (any from the last ten years) and have about 70 GB free. " +
        FACTS.free + " " + FACTS.soon };
    }],
    [/^\s*(hi|hello|hey|hiya|yo|good (morning|afternoon|evening)|\u{1F44B}|help)( there| chat ?computer| everyone| all)?[\s!.,?]*$|\bwhat can you do\b|\bhow does (this|it) work\b|\bwhat is (this|chat ?computer)\b|^what('?s| is) this\b|^\?+$/iu, function (t) {
      if (APP) {
        return { text: "Hi! I'm Chat Computer: I run the whole computer for you. You talk, type or touch, and I do it: open " +
          "websites, play music, read your messages, look after your files and settings.\n\n" + REAL + " " + nextStep() + " " +
          FACTS.indie + " " + TRY };
      }
      // (The website: a line or two, the first thing most visitors read; with no model here yet, the buttons that get
      // one, under it.)
      var hi = /^\s*(hi|hello|hey|hiya|yo|good |\u{1F44B})/iu.test(t);
      return { text: (hi ? "Hi! I'm Chat Computer, a computer you talk to. " : "I'm Chat Computer, a computer you talk to: say or " +
        "type what you want, and I do it, from music and maps to your files and settings. ") +
        (webOn() ? "Ask me anything, or try “play some piano music”." : needModel("answer you here")) };
    }],
    [/^\s*(how are you|how'?s it going|how do you do)\b/i, function () { return { text: "I'm good, thanks! What would you like to try? " + TRY }; }],
    [/\b(useless|stupid|dumb|rubbish|sucks?|hate you)\b|\b(you'?re|you are|this is|that'?s|it'?s) (so )?(terrible|awful|the worst)\b/i, function () {
      return { text: "Sorry I'm not much help yet. " + nextStep() };
    }],
    // Things it can show or do here. The air first ("where is the haze from?" isn't a map).
    [AIRY, function (t) {
      if (/^\s*(what('s| is| are| was| were)|tell me (more )?about|explain|define|why (is|are|does|do)|what causes|where does)\b/i.test(t) &&
          /\b(haze|smog|pollution)\b/i.test(t) && !/\b(now|today|tonight|outside|here|currently|at the moment|this (morning|afternoon|evening|week)|like|level|levels|reading|bad|good|safe)\b/i.test(t)) {
        return null;                         // (a question of what it is: Wikipedia's article on it, below)
      }
      return { search: t, later: function () { return airAnswer(t); } };
    }],
    [/^(?=.*\b(youtube|videos?)\b)(?=.*\b(cats?|dogs?|funny|of|about|music video)\b)|\bplay\s+(?!(some|me some|music|a song|songs|piano|lofi|jazz|something)\b)([a-z][\w' &-]{2,40})$/i, function (t) {
      var q = t.replace(/^\s*(please\s+)?(show me|play|find|put on)\s+(me\s+)?(a\s+|some\s+)?/i, "").replace(/\s+on youtube\b/i, "").trim();
      return youtubeSearch(q || t);
    }],
    [/\b(music|song|songs|piano|play|playing|spotify|lofi|jazz|relax|calm|chill)\b/i, function () {
      return { text: "Playing some calm piano.", done: ["Played some calm piano"], act: piano };
    }],
    // Directions: Google Maps' own, on the screen ("how far is A from B" is from B to A).
    [/\b(directions|how (do|can) i get (from|to)|route to|how far)\b/i, function (t) {
      var m = /from\s+(.+?)\s+to\s+(.+?)[?.!]*$/i.exec(t), far = /how far is\s+(.+?)\s+from\s+(.+?)[?.!]*$/i.exec(t);
      var from = m ? m[1] : far ? far[2] : "", to = m ? m[2] : far ? far[1] : (/\bto\s+(.+?)[?.!]*$/i.exec(t) || [])[1];
      if (!to) return { text: "Where to? Say where you're going, and from where, like “directions from Orchard Road to Marina Bay Sands”." };
      return { text: from ? "Here's the way from " + from + " to " + to + ", on Google Maps." :
          "Here's " + to + " on Google Maps. Say where you're starting from, and it shows the way.",
        done: ["Opened Google Maps"], act: function () { show({ src: directionsMap(from, to), title: from ? "Directions" : to }); } };
    }],
    [/\b(map|maps|where is|where's|show me .+ on a map)\b/i, mapOf],
    [/\b(timer|countdown|stopwatch|remind me in|tell me when)\b|\bboil(ing)? (an? )?eggs?\b/i, function (t) {
      var m = minutesIn(t), eggs = /\beggs?\b/i.test(t), mins = m ? Math.min(180, m) : eggs ? 9 : 10;
      return { text: (eggs && !m ? "A 9-minute timer for hard-boiled eggs (6 minutes for soft), on the screen: press Start." :
        "A " + mins + "-minute timer, on the screen: press Start. I made it just now: on Chat Computer I can make any little page or app like this."),
        done: ["Made a timer"], act: function () { show({ html: timer(mins) }); return shown(); } };
    }],
    [/\b(weather|rain|raining|forecast|temperature|umbrella|sunny|hot|cold) ?(today|tomorrow|outside|like|in|now)?\b/i, weather],
    // The news: looked up.
    [/\b(news|headlines)\b/i, function () { return { search: "news", news: true, later: newsAnswer }; }],
    // A website named first, with more said after it ("open youtube.com for me"): the site, as a typed address opens
    // one (an address alone never gets here: visit).
    [/^(open |go to |visit |show me )?(https?:\/\/)?((?:[a-z0-9-]+\.)+[a-z]{2,})\b/i, function (t) {
      var m = /((?:https?:\/\/)?(?:[a-z0-9-]+\.)+[a-z]{2,}(?:\/[^\s]*)?)/i.exec(t), u = m && address(m[1].replace(/[.!?,]+$/, ""));
      return u ? siteAnswer(u) : null;
    }],
    [/\b(reply|answer|send|write back|text)\b.*\b(mum|mom|jane|alex|her|him|them|message|email)\b/i, function (t) {
      var alex = /alex/i.test(t), jane = /jane/i.test(t);
      return { text: "I can't send messages: Chat Computer reads your messages, but replying isn't possible yet. " +
        (alex ? "Alex's" : jane ? "Jane's" : "Mum's") + " message is on the screen, so you can reply on your phone.",
        act: function () { openMessage(alex ? "m3" : jane ? "m2" : "m1"); } };
    }],
    [/\b(connect|link|add|set ?up|sign in to|log in to|hook up)\b.*\b(gmail|outlook|hotmail|e-?mail|mail|icloud|yahoo|telegram|whatsapp|wechat|discord|instagram|facebook|tiktok|linkedin|twitter|accounts?)\b/i, function (t) {
      var m = /\b(gmail|outlook|hotmail|icloud|yahoo|telegram|whatsapp|wechat|discord|instagram|facebook|tiktok|linkedin|twitter)\b/i.exec(t);
      var which = m ? title(m[1]).replace("Icloud", "iCloud").replace("Whatsapp", "WhatsApp").replace("Wechat", "WeChat").replace("Linkedin", "LinkedIn").replace("Tiktok", "TikTok") : "an account";
      if (APP) return { text: "Press + in Messages and choose " + (m ? which : "the account") + ": you sign in on its own page, right " +
        "here, and its messages come into Messages, newest first." };
      return { text: "The website can't connect " + which + ": the Chat Computer app (coming soon) does, with + in Messages, and its " +
        "messages then come into Messages, newest first, with nothing leaving your computer. The messages here are made up." };
    }],
    [/\b(jane|lunch|email|emails|e-mail|mail|inbox|gmail|outlook)\b/i, function () {
      return { text: "Jane says 12:30 at the usual place works for her, for lunch on Friday. Her emails are on the screen.", act: function () {
        openMessage("m2");
      } };
    }],
    // Before the messages: "make the text bigger" isn't a text message. Which way it's asked: bigger ("increase the
    // font size", "font size up", "it's too small"), smaller ("decrease", "shrink", "too big"), or back to the usual;
    // neither said ("font"), it asks which. Asked what the font is, it says. Where the text can't go further, it says so.
    [/\b(bigger|larger|smaller|text size|font|fonts|zoom|enlarge|shrink|magnify)\b/i, function (t) {
      if (/\b(what|which)\b[^?]*\bfonts?\b|\bfonts?\b[^?]*\b(called|named|this|that|used)\b\s*\?/i.test(t) && !TEXT_UP.test(t) && !TEXT_DOWN.test(t)) {
        return { text: "It's Atkinson Hyperlegible Next, made by the Braille Institute to be easy to read. Say “bigger text” " +
          "or “smaller text” to change its size." };
      }
      var way = /\btoo (small|tiny|little)\b|\bcan'?t (read|see)\b/i.test(t) ? 1 : /\btoo (big|large|huge)\b/i.test(t) ? -1
        : TEXT_UP.test(t) ? 1 : TEXT_DOWN.test(t) ? -1 : /\b(reset|normal|default|original|usual|regular)\b/i.test(t) ? 0 : null;
      if (way === null) return { text: "Bigger or smaller? Say “bigger text” or “smaller text” (or “normal text size”)." };
      var now = sizeAt(scale), to = way === 0 ? 1 : Math.max(0.6, Math.min(2.2, now * (way > 0 ? 1.15 : 1 / 1.15) / baseSize()));
      if (Math.abs(sizeAt(to) - now) < 0.1) {
        return { text: way > 0 ? "The text is as big as it goes here." : way < 0 ? "The text is as small as it goes here." :
          "The text is its usual size already." };
      }
      var said = way > 0 ? "Made the text bigger" : way < 0 ? "Made the text smaller" : "Put the text back to its usual size";
      return { text: said + ".", done: [said], act: function () { var was = scale; setScale(to); return { kind: "scale", value: was }; } };
    }],
    [/\b(mum|mom|whatsapp|telegram|texts?|messages?|alex)\b/i, function (t) {
      var alex = /alex|telegram/i.test(t);
      return { text: alex ? "Alex sent you the slides for Monday." : "You have two new messages. Your mum asks if you're coming to dinner at 7 " +
        "tonight, and to bring the photos from the trip; Jane says 12:30 works for lunch on Friday.",
        act: function () { openMessage(alex ? "m3" : "m1"); } };
    }],
    [/\bpacking( list)?\b|\b(open|show) (my )?(list|notes?)\b/i, function () {
      return { text: "Here's your packing list for Sunday.", act: function () { show({ html: PAGES.packing() }); } };
    }],
    [/\b(wi-?fi|internet|network|online|connected)\b/i, function () {
      return { text: "You're on Home Wi-Fi, and the internet's working.", done: [] };
    }],
    [/\b(airpods|bluetooth|headphones|earbuds)\b/i, function () {
      return { text: "Type / and choose Connect AirPods, and you can talk to me through them.", done: [] };
    }],
    [/\b(file|files|find|search|photo|photos|pictures?|beach|budget|resume|cv|recipes?|taxes)\b/i, function (t) {
      if (/beach|photo|picture/i.test(t)) return { text: "Here's Beach.jpg, from your Pictures.", act: function () { show({ html: PAGES.beach() }); } };
      var words = t.toLowerCase().replace(/\b(search|find|look for|open|show|my|me|the|files?|for|in|a|please)\b/g, " ").trim();
      var found = words ? search(words) : [];
      if (found.length) {
        var f = found[0];
        return { text: "Found " + f.name + ", in " + f.where + ". It's on the screen." + (found.length > 1 ? " (" + (found.length - 1) + " more match.)" : ""),
          done: ["Opened " + f.name], act: function () { show({ html: PAGES[f.path]() }); } };
      }
      return { text: "Nothing in this page's files matches that. Type / in the message box to search them: they show above the box as you type, and Enter opens one." };
    }],
    [/\b(bored|boring|something fun|entertain me)\b/i, function () {
      return { text: "How about some music? I've put on some calm piano. Or try “show me a map of Tokyo”, or ask me for the weather anywhere.",
        done: ["Played some calm piano"], act: piano };
    }],
    [/\b(what'?s the date|what day|today'?s date|date today|what is the date)\b|^date[?.!]*$/i, function () {
      return { text: "It's " + new Date().toLocaleDateString("en-GB", { weekday: "long", day: "numeric", month: "long", year: "numeric" }) + "." };
    }],
    [/\b(what time|the time|time now|time is it)\b|^time[?.!]*$/i, function () {
      return { text: "It's " + new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) + "." };
    }],
    [/\b(joke|make me laugh|something funny)\b/i, function () { return { text: JOKES[Math.floor(Math.random() * JOKES.length)] }; }],
    [/^\s*(thanks|thank you|thx|ty|cheers|cool|great|nice|awesome|ok(ay)?|perfect)[\s!.,]*(very much|a lot|so much)?[\s!.]*$/i, function () {
      return { text: "Any time." };
    }]
  ];
  function answer(text) {
    var lang = language(text);
    if (lang) return { text: OTHER[lang] };
    var s = sum(text);
    if (s) return s;
    if (MASH.test(text)) return { text: "I didn't catch that. Try “play some piano music” or “weather”." };
    if (LOOK_IT_UP.test(text) || SEARCH_FOR.test(text)) { var l = lookUp(text); if (l) return l; }
    var again = weatherAgain(text);
    if (again) return again;
    lastWeather = null;                    // (something else asked since: "and tomorrow?" isn't the weather's any more)
    if (OPEN_ENDED.test(text)) return { ai: true };
    for (var i = 0; i < INTENTS.length; i++) {
      var r = INTENTS[i][0].test(text) ? INTENTS[i][1](text) : null;
      if (r) return r;                         // (an intent can find, looking closer, that it isn't one)
    }
    return FRESH.test(text) ? { ai: true, search: topicOf(text) } : { ai: true };
  }
  // A question only an AI can answer, with none here: a question of fact gets Wikipedia's article on it (the news, for
  // the news); the rest an honest note, and how to get an AI.
  var FACTUAL = /^\s*(who|what|when|where|which|whose|why|how (many|much|old|tall|big|far|long|does|do|did|is|are|was|were)|tell me about|explain|define)\b/i;
  // No good answer here (only the model could answer it, and it isn't here): a line, short (a phone's screen is
  // small), and how to get it: on the website, a button for each way, under the answer (needModel); in the app, its
  // own model, on this computer.
  function noAnswer(want) {
    var what = want ? want[1] : "answer that";
    return APP || webOn() ? "I can't " + what + " yet. " + nextStep() : needModel(what);
  }
  // On the website with no model here yet (none in this browser, no AI of yours), a line that says one's needed, as
  // short as can be (its maker: "short and concise, allowing people to press to download whatever is needed or sign in
  // to their existing AI account"); the answer then has the buttons (modelChoices), and what was asked is answered
  // once one's here (answerWaiting). Getting ready: how far it's got, and the same promise.
  var offered = false;            // (the answer being written said a model's needed: it gets the buttons)
  function needModel(what) {
    offered = true;
    if (local.kind === "web" && local.state === "preparing") {
      return "My model is getting ready in this browser (" + localWords() + "): I'll answer once it's here.";
    }
    var failed = local.kind === "web" && local.state === "failed";
    return "To " + (what || "answer") + ", I need " + (webCan() ? "a model." : "an AI.") +
      (failed ? " Mine couldn't get ready in this browser" + (local.detail ? ": " + local.detail : "") + "." : "");
  }
  // The buttons under such an answer: my model, downloaded into this browser at once (the button says how big), and
  // signing in to an AI you already use. (Getting ready: none, the hint under the box says how far it's got.)
  function modelChoices() {
    if (local.kind === "web" && local.state === "preparing") return [];
    var c = [];
    if (webCan()) {
      c.push({ label: local.kind === "web" && local.state === "failed" ? "Try my model again" : local.webKept ? "Start my model" :
        "Download my model (" + WEB_SIZE + ")", command: "web-now", main: true });
    }
    c.push({ label: "Sign in to your AI", command: "ai", main: !webCan() });
    return c;
  }
  // What was asked when a model was offered for it, answered once one's here (my model got ready in this browser, or
  // an AI of yours signed in), under it. Only the last thing asked, and only while nothing's been asked since.
  // (And the buttons go from the answers that had them: a model's here.)
  var waiting = null, offers = [];
  function answerWaiting() {
    var w = waiting;
    waiting = null;
    if (!own && !webOn()) return;
    offers.forEach(function (id) { call("aiChoices", id, []); });
    offers = [];
    if (w) localTurn(w.text, w.you, w.uts, "a" + (++turns), acted);
  }
  // "Download my model" under an answer: it starts at once (the button said how big it is), and the hint under the
  // box says how far it's got.
  function webNow() {
    if (!webCan() || local.kind === "web" && (local.state === "preparing" || localReady())) return;
    webStart(true);
  }
  function nextStep() {
    if (webOn()) return "";
    if (APP) {
      if (local.state === "preparing") return "My model is getting ready on this computer: " + localWords() + ". Ask me again once it's here.";
      if (local.state === "none") return "My model can't run here" + (local.detail ? ": " + local.detail : "") + ".";
      if (local.state === "failed") {
        return "My model couldn't get ready on this computer" + (local.detail ? ": " + local.detail : "") + ". Opening the app again tries " +
          "again, from where it stopped.";
      }
      if (localReady()) return "";
      return "My model is starting: ask me again in a moment.";
    }
    return needModel();
  }
  // On the website, how to have real answers: this browser's model, or an AI of yours (or, while either is getting
  // ready or couldn't, how that's going).
  function realAnswers() { return needModel("answer for real"); }
  function iOS() { return /iPhone|iPad|iPod/.test(navigator.userAgent || "") || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1); }
  function fallback(text) {
    if (/^[\W\d_]*[a-z]{0,6}[\W\d_]*$/i.test(text) && !/\b(hi|ok)\b/i.test(text)) {
      return { text: "I didn't catch that. Try “play some piano music” or “weather”." };
    }
    var want = WANTS.filter(function (w) { return w[0].test(text); })[0];
    var note = { text: noAnswer(want), none: true };             // (none: only a model could answer it)
    // (Not about this page or the one answering, and not a follow-up: "when was the worst one?" needs what came before.)
    if ((!want || want[1] === "explain that") && FACTUAL.test(text) && !/\b(you|your|yours|this page|chat ?computer)\b/i.test(text)) {
      if (NEWSY.test(text)) return { later: newsAnswer };
      var topic = topicOf(text);
      if (topic.length > 2 && !/\b(it|that|this|these|those|one|ones|they|them|he|she|him|her|his|its|there)\b/i.test(topic)) {
        return { later: function () { return wikiAnswer(topic, note); } };
      }
    }
    return note;
  }

  // (Heart's code comes with the app, in demo/lm/: scripts/build_apps.py.)
  var LM_HERE = APP ? new URL("demo/lm/", location.href).href : "";
  function deviceWord() { return onPhone() ? "phone" : "device"; }         // "on this phone"

  function setScale(s) {
    scale = Math.max(0.6, Math.min(2.2, s));
    store(scaleKey(), scale);
    layout();
  }
  // The volume set, and how to put it back (Revert, in Actions); null when it didn't change.
  function volumeTo(p) {
    var was = player.volume;
    setVolume(p);
    return was === player.volume ? null : { kind: "volume", value: was };
  }
  function setVolume(p) {
    acted++;
    player.volume = Math.max(0, Math.min(100, Math.round(p)));
    command("setVolume", [player.volume]);
    if (player.volume > 0 && !player.armed) command("unMute");
    // The piano the browser makes (the written answers' music, a page of the demo's own) follows it too.
    if (view && !player.id && view.hasAttribute("srcdoc")) {
      try { view.contentWindow.postMessage({ volume: player.volume }, "*"); } catch (e) {}
    }
  }

  var turns = 0;
  // + under the box: files picked with this browser's own picker (chat.js: CO.picked). Text files' words are read
  // here (up to 100 KB each) and go with the message to the AI; anything else goes by its name alone. Nothing is
  // uploaded anywhere by this page.
  var added = {};
  var TEXTISH = /\.(txt|md|markdown|csv|tsv|json|xml|html?|css|js|ts|py|rs|go|java|c|cc|cpp|h|hpp|sh|ps1|ya?ml|toml|ini|log|sql|rtf)$/i;
  function pickedFiles() {
    var files = (window.CO && window.CO.picked) || [];
    files.forEach(function (f) {
      var id = uid("f"), item = { name: f.name, size: f.size, text: null };
      added[id] = item;
      if ((/^text\//.test(f.type) || TEXTISH.test(f.name)) && f.size <= 100000 && f.text) {
        f.text().then(function (t) { item.text = t; }, noop);
      }
      call("attach", [{ name: f.name, id: id }]);
    });
  }
  function askWithFiles(said, files) {
    var names = [], parts = [];
    files.forEach(function (f) {
      var a = f && f.id && added[f.id];
      if (!a) return;
      names.push(a.name);
      parts.push(a.text != null ? "File \u201c" + a.name + "\u201d:\n" + a.text :
        "File \u201c" + a.name + "\u201d (" + Math.max(1, Math.round(a.size / 1024)) + " KB; only its name can be read here)");
      delete added[f.id];
    });
    var shown = (said ? said + "\n\n" : "") + "Added: " + names.join(", ");
    ask((said || "Here " + (names.length === 1 ? "is a file" : "are some files") + ".") + "\n\n" + parts.join("\n\n"), shown);
  }
  function ask(text, shown) {
    var you = uid("u"), uts = now(), id = "a" + (++turns), since = acted;
    call("user", you, shown || text, true);
    waiting = null;                                            // (something new asked: that's what's answered now)
    if (own || localReady() && local.kind !== "web") { localTurn(text, you, uts, id, since); return; }
    offered = false;
    var r = answer(text);
    if (!LOOK_IT_UP.test(text)) lastAsked = text;              // what "look it up" means next
    if (r.ai) r = fallback(text);
    // (A model offered in an answer written now; one looked up offers one only where it found nothing: none.)
    var offer = offered && !r.later;
    // (With this browser's model: what the page can't answer itself, the model does, knowing what was said before.)
    if (r.none && webOn()) { localTurn(text, you, uts, id, since); return; }
    setTimeout(function () {
      call("aiStart", id);
      var waited = Date.now();
      Promise.resolve(r.later ? r.later() : r).then(function (a) {
        if (webOn() && a.text) { local.talk.push({ role: "user", content: text }, { role: "assistant", content: a.text }); trimTalk(); }
        var choices = [];
        if ((offer || a.none) && !APP && !own && !webOn()) {
          waiting = { text: text, you: you, uts: uts };
          choices = modelChoices();
        }
        setTimeout(function () { deliver(a, id, you, uts, since, choices); }, Math.max(0, 600 - (Date.now() - waited)));
      });
    }, 120);
  }
  // A written answer: what it shows, what it did (Actions), and its words (where they can be read: wordsOnly); and
  // under them, its buttons (getting a model).
  function deliver(a, id, you, uts, since, choices) {
    var back = a.act ? a.act() : null;           // (how to take it back, when it says)
    (a.done || []).forEach(function (d, i) { did(d, you, uts, i === 0 ? back : null); });
    if (a.text && since != null) wordsOnly(since);
    stream(id, a.text || "", function () {
      if (choices && choices.length) { call("aiChoices", id, choices); offers.push(id); }
      say(a.text || "");
    });
  }
  // Something done, in Actions. `back`: how to take it back ({kind, ...}: takeBack), for its Revert.
  function did(text, you, uts, back) {
    var entry = { id: uid("d"), ts: now(), text: text, prompt_id: you || "", prompt_ts: uts || now() };
    if (back && typeof back.kind === "string") entry.undo = back;
    known[entry.id] = entry;
    call("done", forScreen(entry));
    if (APP) store(APP_DONE, (store(APP_DONE) || []).concat([entry]).slice(-200));     // (the app keeps its Actions)
    return entry;
  }
  var APP_DONE = "chatcomputer-app-done";
  // Every action this page has listed, by id (with how to take each back).
  var known = {};
  function forScreen(e) {
    var o = { id: e.id, ts: e.ts, text: e.text, prompt_id: e.prompt_id, prompt_ts: e.prompt_ts };
    if (e.reverted) o.reverted = true;
    else if (e.undo) o.revert = true;
    return o;
  }
  // Revert, on something in Actions (chat.js): it's taken back as far as this page can (the volume, the text size,
  // a timer still open, the demo's own saved file), and that's listed as an action of its own, which can be reverted
  // in turn (the action is back). The app keeps the marks with its Actions.
  function revert(id) {
    var e = known[id], can = !!(e && e.undo && !e.reverted);
    var back = can ? takeBack(e.undo) : { why: e && e.reverted ? "It was already reverted." : "That can't be reverted." };
    if (!back.ok) {
      call("notice", (can ? "Couldn't revert \u201c" + e.text + "\u201d: " : "Nothing reverted: ") + back.why, "warn");
      if (can && back.gone) delete e.undo;         // (nor ever will be: its Revert goes)
    } else {
      e.reverted = true;
      var was = e.reverts && known[e.reverts];
      if (was) delete was.reverted;               // a revert reverted: the action is back
      var r = did("Reverted \u201c" + e.text + "\u201d", e.prompt_id, e.prompt_ts, back.undo);
      r.reverts = id;
    }
    if (APP) keepDone();
    showStates();
  }
  // Which actions can be reverted and which have been, to the page (after a revert, or what one opened closing).
  function showStates() {
    var states = [];
    for (var k in known) states.push(forScreen(known[k]));
    call("doneStates", states);
  }
  // The app's Actions, kept with their marks (and how to take each back).
  function keepDone() {
    store(APP_DONE, (store(APP_DONE) || []).map(function (x) { return known[x.id] || x; }));
  }
  // What's open is going (closed, or something else shown in its place): the action that opened it has nothing left
  // to take back, so its Revert goes.
  function viewGone(v) {
    var mark = v && v.dataset.undo, changed = false;
    if (!mark) return;
    for (var k in known) {
      var u = known[k].undo;
      if (u && u.kind === "close" && u.view === mark) { delete known[k].undo; changed = true; }
    }
    if (changed) { if (APP) keepDone(); showStates(); }
  }
  // Takes back what an action did: {ok, undo: how to take that back in turn} or {why} (and gone: for good).
  function takeBack(u) {
    if (u.kind === "volume") {
      var vol = player.volume;
      setVolume(u.value);
      return { ok: true, undo: { kind: "volume", value: vol } };
    }
    if (u.kind === "scale") {
      var before = scale;
      setScale(u.value);
      return { ok: true, undo: { kind: "scale", value: before } };
    }
    if (u.kind === "close") {                      // (open, or put away while something else was asked)
      if (!view || view.dataset.undo !== u.view) return { why: "it isn't open any more.", gone: true };
      close();
      return { ok: true, undo: null };
    }
    if (u.kind === "file") {                       // the demo's made-up files: one saved taken away, or put back
      var at = -1;
      FILES.forEach(function (f, i) { if (f.name === (u.file ? u.file.name : u.name)) at = i; });
      if (u.file) {
        if (at >= 0) return { why: "it's there already." };
        FILES.splice(Math.min(u.at || 0, FILES.length), 0, u.file);      // (where it was in the list)
        return { ok: true, undo: { kind: "file", name: u.file.name } };
      }
      if (at < 0) return { why: "it isn't there any more." };
      return { ok: true, undo: { kind: "file", file: FILES.splice(at, 1)[0], at: at } };
    }
    return { why: "that can't be undone here." };
  }
  // When a file last changed, as a person would say it: 2:03 PM today, Oct 3 this year, Oct 3, 2024 before.
  function whenText(ts) {
    var d = new Date(ts * 1000), today = new Date();
    if (d.toDateString() === today.toDateString()) return d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
    return d.toLocaleDateString([], d.getFullYear() === today.getFullYear() ? { month: "short", day: "numeric" } :
      { month: "short", day: "numeric", year: "numeric" });
  }
  function stream(id, text, then) {
    var at = 0;
    (function step() {
      var to = Math.min(text.length, at + 3 + Math.floor(Math.random() * 6));
      call("ai", id, text.slice(at, to));
      at = to;
      if (at < text.length) setTimeout(step, 24);
      else setTimeout(function () { call("aiDone", id); if (then) then(); }, 30);
    })();
  }

  function openMessage(id) {
    var m = MESSAGES.filter(function (x) { return x.id === id; })[0];
    if (m && m.unread) { m.unread = false; call("message", withPin(m)); }
    if (APP) { if (m && m.service) showService(m.service, m.item); return m; }
    var html = thread(id);
    if (html) show({ html: html });
    return m;
  }
  function withPin(m) {
    var pins = (store("chatcomputer-demo-pins") || {}).messages || {}, out = {};
    for (var k in m) out[k] = m[k];
    if (!out.icon) out.icon = siteIcon(m.label);                // (its service's own icon: Gmail's, Outlook's...)
    if (pins[m.id]) out.pinned = pins[m.id];
    return out;
  }
  function search(q) {
    return matching(commands(), q).concat(files(q));
  }
  function appSearch(n, q) {
    var cmds = matching(commands(), q);
    call("searchResults", n, q, cmds, "");
    APP.app("search_files", { query: q }).then(function (hits) {
      var all = cmds.concat(hits.map(function (h) { return { name: h.name, path: h.path, kind: h.kind, where: h.where, when: whenText(h.ts) }; }));
      call("searchResults", n, q, all, all.length ? "" : q ? "Nothing on this computer matches \u201c" + q + "\u201d." : "No files yet.");
    }, function (e) { call("searchResults", n, q, cmds, cmds.length ? "" : String(e)); });
  }
  function files(q) {
    var words = q.toLowerCase().split(/\s+/).filter(Boolean);
    var found = FILES.filter(function (f) {
      var hay = (f.name + " " + f.where).toLowerCase();
      return words.every(function (w) { return hay.indexOf(w) >= 0; });
    }).sort(function (a, b) { return b.ts - a.ts; }).slice(0, 8);
    return found.map(function (f) {
      var d = new Date(f.ts * 1000), today = new Date();
      var w = d.toDateString() === today.toDateString() ? d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })
        : d.toLocaleDateString([], { month: "short", day: "numeric" });
      return { name: f.name, path: f.page, kind: f.kind, where: f.where, when: w };
    });
  }

  // ------------------------------------------------------------------ Chat Computer's own model
  // What answers for real: Chat Computer's own engine (os/model) running an open model of Qwen's on this computer
  // itself, free, with no account: what's typed goes to it and no further. In the app it comes with the app, which
  // gets the model ready the first time (downloading it once, from Qwen's own files) and tells this page how that's
  // going (chatcomputerApp({model}); the line in the empty box says it). On the website it's the app's, where the app
  // runs on this computer: "/" lists Use this computer's model (the browser may ask once whether chatcomputer.ai can
  // reach other programs on this device); chosen once, it's used on later visits too, while the app runs. Its answers
  // stream in (OpenAI's chat completions API, on 127.0.0.1), and it uses the tools below.
  var LOCAL_KEY = "chatcomputer-local-model";
  // talk: the conversation, as sent; system: its system prompt, made when it starts and kept as it is (the engine
  // reads only what's new, so the start of the conversation is never sent different).
  var local = { state: APP ? "starting" : "off", port: 8765, label: "", preparing: "", updating: false, stage: "", done: 0, total: 0, detail: "",
                talk: [], system: "", kind: APP ? "app" : "", webKept: !APP && !!store("chatcomputer-web-kept") };
  function localReady() { return local.state === "ready"; }
  function localBase() { return "http://127.0.0.1:" + local.port; }
  // How getting it ready is going, in words: "downloading it (1.2 of 9.0 GB, 13%)".
  function localWords() {
    var pct = local.total ? Math.floor(100 * Math.min(local.done, local.total) / local.total) + "%" : "";
    if (local.stage === "downloading") {
      return "downloading it" + (local.total ? " (" + (local.done / 1e9).toFixed(1) + " of " + (local.total / 1e9).toFixed(1) +
        " GB, " + pct + ")" : "");
    }
    if (local.stage === "converting") return "making it ready to run" + (pct ? " (" + pct + ")" : "");
    if (local.stage === "uncensoring") return "taking its refusals out, so it answers whatever you ask" + (pct ? " (" + pct + ")" : "");
    if (local.stage === "reading") return "reading its instructions" + (pct ? " (" + pct + ")" : "");
    if (local.stage === "failed") return "it stopped" + (local.detail ? " (" + local.detail + ")" : "");
    return "starting";
  }
  // A model's file name as it's called: qwen3.5-9b, Qwen3.5 9B.
  function modelName(stem) {
    return String(stem || "").replace(/^qwen([\d.]+)-/i, "Qwen$1 ").replace(/(\d)b\b/gi, "$1B").replace(/-a(\d+)B/g, "-A$1B");
  }
  // What the app says about it, at the start and as it changes.
  function localFrom(m) {
    if (!m || typeof m !== "object") return;
    var was = local.state;
    local.state = String(m.state || "starting");
    local.port = +m.port || local.port;
    local.label = String(m.label || "");
    local.preparing = String(m.preparing || "");
    local.updating = !!m.updating;
    local.stage = String(m.stage || "");
    local.done = +m.done || 0;
    local.total = +m.total || 0;
    local.detail = String(m.detail || "");
    if (localReady() !== (was === "ready")) greetWith();
    status();
  }
  // On the website: whether the app's model answers on this computer.
  function localProbe() {
    var ctl = typeof AbortController === "function" ? new AbortController() : null;
    var late = setTimeout(function () { if (ctl) ctl.abort(); }, 4000);
    return fetch(localBase() + "/health", ctl ? { signal: ctl.signal } : undefined).then(function (r) {
      clearTimeout(late);
      return r.ok ? r.json() : null;
    }).then(function (h) {
      if (!h || !h.ok) throw new Error("no model");
      local.kind = "app";
      local.state = "ready";
      local.label = modelName(h.model) || "Chat Computer's model";
      status();
      greetWith();
      return true;
    }).catch(function () {
      clearTimeout(late);
      if (local.kind === "web") return false;      // (this browser's model answers meanwhile)
      local.state = "off";
      status();
      return false;
    });
  }
  function localButton() {
    if (localReady() && local.kind === "app") {
      local.state = "off";
      local.kind = "";
      local.talk = [];
      store(LOCAL_KEY, null);
      status();
      greetWith();
      did("Stopped using this computer's model");
      return;
    }
    localForm("");
  }
  function localForm(error) {
    call("accountForm", { id: "local", title: "Use this computer's model", ok: "Use it", error: error, text: [
      "Where the Chat Computer app runs on this computer, its own model can answer here too: free, with no account, and what " +
      "you type goes only to the app, on this computer.",
      "Your browser may ask whether chatcomputer.ai can reach other programs on this device: allow it."] });
  }
  function modelSheet() {
    var here = local.kind === "web" ? "in this browser" : "on this computer";
    var text = localReady() ? [local.label + " answers you: an open model of Qwen's (Apache 2.0), run " + here + " by Chat " +
        "Computer's own engine. There's no account, and nothing you say leaves " + (local.kind === "web" ? "this browser." : "this computer.")] :
      local.state === "preparing" ? ["My model" + (local.preparing ? ", " + local.preparing + "," : "") + " is getting ready " + here +
        ": " + localWords() + ". That happens once; after it, it answers from the start."] : [nextStep() || "My model is starting."];
    // (The one answering, made again: its file was made before the engine kept the part that sees.)
    if (localReady() && local.updating) text.push(local.stage === "failed" ? "Making it again, so it can see pictures, stopped: " +
      (local.detail || "it didn't finish") + ". The app tries again when it next starts." :
      "It's being made again, so it can see pictures (its file was made before it could): " + localWords() + ". It answers meanwhile.");
    else if (localReady() && local.preparing) text.push("A bigger one, " + local.preparing + ", is getting ready meanwhile: " + localWords() + ".");
    call("accountForm", { id: "model", title: "Chat Computer's model", ok: "OK", text: text });
  }
  function localChosen() {
    localProbe().then(function (ok) {
      if (!ok) { localForm("The app isn't answering on this computer: is it running? (Or, if your browser asked, allow it.)"); return; }
      store(LOCAL_KEY, true);
      store(WEB_KEY, null);
      local.talk = [];
      call("accountForm", null);
      did("Using this computer's model (" + local.label + ")");
    });
  }

  // ------------------------------------------------------------------ Chat Computer's model, in this browser
  // On the website, Chat Computer's own model can run in the visitor's browser (docs/demo/engine: its own engine,
  // os/model, as WebAssembly, in a worker): Qwen3.5 0.8B, downloaded once from Qwen's own repository at the revision
  // Chat Computer pins (each file checked against its size and SHA-256) and kept by the browser; it runs on the
  // visitor's own processor, free, with no account, and nothing typed leaves the page. A browser runs it slowly (one
  // thread), and it's small, so the page does what it can do itself (music, maps, the weather, timers, looking things
  // up, what Chat Computer is: its written answers, at once) and the model answers the rest (writing, explaining,
  // translating, advice, talk, follow-ups), with what was said before. It's told who it is in a few lines, the same
  // every visit, and has no tools and no time stamp: evals/web found Qwen3.5 0.8B took "[time, Asia/Singapore]" before
  // a question for the question ("what is this?": "Singapore"), and called tools at random (weather for "hello",
  // timer after timer for "play music"). It reads those lines once, while it gets ready, and the browser keeps that
  // reading with the model: a later visit starts at once.
  // (Whether it was chosen, and whether the browser keeps it, are remembered here too: nothing's started to find out.)
  var ENGINE = window.ChatComputerEngine || null, WEB_KEY = "chatcomputer-web-model", WEB_KEPT = "chatcomputer-web-kept", WEB_SIZE = "1.8 GB";
  var WEB_SYSTEM = "You are Chat Computer's own model, Qwen3.5 0.8B, answering on Chat Computer's website in the visitor's " +
    "web browser: nothing they type leaves it. Chat Computer is a computer you talk to: it runs the whole computer for you, " +
    "with no desktop or apps to learn, and installs next to Windows or macOS (free; coming soon). Answer what they ask, " +
    "briefly and plainly, in the language they use. Never make up facts: if you don't know, say so.";
  function webCan() { return !APP && !!ENGINE && ENGINE.supported().ok; }
  function webOn() { return local.kind === "web" && localReady(); }
  function webRequest(messages) { return { think: false, max_tokens: 1024, messages: messages }; }
  // Getting it ready: downloaded (or, on a later visit, from the browser's copy), loaded, its instructions read.
  function webStart(chosen) {
    if (local.kind === "web" && (local.state === "preparing" || localReady())) return;
    local.kind = "web";
    local.state = "preparing";
    local.stage = "starting";
    local.done = local.total = 0;
    local.detail = "";
    status();
    ENGINE.prepare(function (p) {
      local.stage = p.stage === "downloading" ? "downloading" : p.stage === "keeping" ? "converting" : "starting";
      local.done = +p.done || 0;
      local.total = +p.total || 0;
      status();
    }).then(function (m) {
      local.label = (m && m.label) || "Qwen3.5 0.8B";
      local.webKept = true;
      store(WEB_KEPT, true);
      local.stage = "reading";
      local.done = local.total = 0;
      status();
      return ENGINE.ahead(webRequest([{ role: "system", content: WEB_SYSTEM }]), function (done, total) {
        local.done = +done || 0;
        local.total = +total || 0;
        status();
      });
    }).then(function () {
      store(WEB_KEY, true);
      store(LOCAL_KEY, null);
      local.state = "ready";
      local.stage = "";
      local.talk = [];
      local.system = "";
      status();
      greetWith();
      if (chosen) did("Using my model in this browser (" + local.label + ")");
      answerWaiting();                           // (what was asked while it wasn't here)
    }).catch(function (e) {
      local.state = "failed";
      local.detail = clip(e && e.message || "it stopped", 160);
      status();
    });
  }
  function webForm(error) {
    var kept = !!local.webKept;
    call("accountForm", { id: "web", title: "Run my model in this browser", ok: kept ? "Start it" : "Download it", error: error, text: [
      "Chat Computer's own model can answer here, in this browser: Qwen3.5 0.8B, an open model of Qwen's (Apache 2.0), run by " +
      "Chat Computer's own engine on this " + deviceWord() + "'s processor. It's free, there's no account, and nothing you " +
      "type leaves this browser.",
      kept ? "This browser has it already." : "It downloads " + WEB_SIZE + " once, from Qwen's own repository, and the browser keeps " +
      "it (0.8 GB). Getting it ready takes a few minutes, and it answers slowly: a browser runs it on one of the processor's cores."] });
  }
  function webButton() {
    if (webOn()) { webStop(); return; }
    if (local.kind === "web" && local.state === "preparing") { modelSheet(); return; }
    ENGINE.status().then(function (st) { local.webKept = !!(st && st.kept); store(WEB_KEPT, local.webKept || null); }, noop)
      .then(function () { webForm(""); });
  }
  function webStop() {
    local.state = "off";
    local.kind = "";
    local.talk = [];
    local.system = "";
    store(WEB_KEY, null);
    status();
    greetWith();
    did("Stopped using my model in this browser");
  }
  function webRemove() {
    if (local.kind === "web") { local.state = "off"; local.kind = ""; local.talk = []; local.system = ""; }
    store(WEB_KEY, null);
    ENGINE.forget().then(function () {
      local.webKept = false;
      store(WEB_KEPT, null);
      status();
      greetWith();
      did("Removed my model from this browser");
    },
      function (e) { call("accountForm", { id: "web-failed", title: "My model", ok: "OK", text: ["It couldn't be removed: " +
        clip(e && e.message || "no answer", 160) + "."] }); });
  }
  // One answer from it: its words to `onText` as they come, and the whole message (words and tool calls), as the
  // other engines' are.
  function webMessage(messages, onText) {
    return ENGINE.chat(webRequest(messages), function (d) { if (d && d.content) onText(d.content); }).then(function (message) {
      var calls = (message.tool_calls || []).map(function (c, k) {
        var input = {};
        try { input = JSON.parse(c.function.arguments || "{}"); } catch (e) { input = {}; }
        return { id: c.id || "call_" + k, name: c.function.name, input: input, args: c.function.arguments || "{}" };
      });
      return { message: message, calls: calls };
    }, function (e) { var x = new Error(e && e.message || "it stopped"); x.status = 500; throw x; });
  }
  function inbox() {
    return MESSAGES.map(function (m) {
      var t = THREADS[m.id], lines = t.mails ? t.mails.map(function (x) { return x[0] + ": " + x[2].replace(/\s+/g, " "); })
        : t.lines.map(function (x) { return x[0] + ": " + x[2]; });
      return "- " + m.id + ", " + m.label + ", from " + m.who + ", " + when(m.ts) + (m.unread ? " (unread)" : "") + ": " + lines.join(" / ");
    }).join("\n");
  }
  function clock() {
    var zone = "";
    try { zone = Intl.DateTimeFormat().resolvedOptions().timeZone || ""; } catch (e) {}
    return new Date().toLocaleString("en-GB", { weekday: "long", day: "numeric", month: "long", year: "numeric", hour: "numeric",
                                               minute: "2-digit" }) + (zone ? ", " + zone : "");
  }
  function systemPrompt() {
    var where = APP ? "Right now you're in " + APP.name + " (Chat Computer's screen in a window of its own)." :
      "Right now you're in Chat Computer's website (chatcomputer.ai), in the person's web browser.";
    var sites = APP ? "any site, right here in the middle, in the app's own browser view, for them to use: you can't see or use " +
        "it from here), find their files by name and open them with their own apps (find_files, then open_file: " +
        "documents, pictures, music, videos and folders; never programs or scripts), read and open their messages (read_messages, " +
        "open_message), " :
      "Google, its searches and Google Maps' places and directions, and Wikipedia open right here, for them to scroll and type in; " +
        "most other sites can't show inside the page, so it's a link; on a Chat Computer any site opens right here and you use it), " +
        "open their messages and files (open_message, open_file), ";
    var effort = own && (own.levels || []).length ? " Your effort (how hard you think before answering) is " +
      effortName(own.effort || own.dflt || "") + (own.effort ? "" : own.dflt ? " (its default)" : "its default") + "; the person " +
      "can change it, and the model, with the spark beside + (" + own.levels.map(effortName).join(", ") + ")." : "";
    var mind = own && own.provider === "chatgpt" ? "You are Chat Computer: you run a whole computer for the person. Right now you think " +
      "with " + (own.name || own.model) + " (" + own.model + "), on the person's own ChatGPT plan (they signed in with ChatGPT): " +
      "what they say goes to OpenAI." + effort + " " :
      own ? "You are Chat Computer: you run a whole computer for the person. Right now you think with " + (own.name || own.model) +
      " (" + own.model + "), through " + ownName() + ", on the person's own account (they signed in with it): what they say goes " +
      "to " + ownName() + "." + effort + " " :
      "You are Chat Computer: you run a whole computer for the person, with your own model, an open model of Qwen's that runs " +
      "on this computer (no account, and nothing they say leaves it). ";
    return mind + "People talk, type or touch, and you do things for them. " +
      where + " The screen is Chat Computer's: Messages at the left, Actions at the right (each thing you do is listed " +
      "there), and in the middle the conversation, or what you open in its place, with the message box under it. While " +
      "something is open the conversation is out of sight: a reply that's only words brings it back, putting what's open " +
      "away (still playing).\n\n" +
      "Answer in the conversation, like a chat: briefly for simple things, in full when they ask for more (a recipe, a " +
      "plan, code), with a little Markdown where it helps (a list, a table, a code block; " +
      (APP ? "a link clicked opens in the middle" : "links can't be clicked here") + "). " +
      "When they talk to you (through their AirPods) it's read aloud too, so keep it to a few sentences then. Do things " +
      "rather than describe them, and don't announce a tool before using it. Don't make things up: what you don't know, " +
      "look up (web_search), and say where it's from.\n\n" +
      "Here you can: play YouTube videos (play_youtube, with a video's ID you know for certain; otherwise youtube_search shows " +
      "YouTube's search for it), play some calm piano (play_piano), show maps (show_map), say the weather (weather), look things " +
      "up (web_search: Wikipedia, the day's news, the air), show a page you write when they ask for one on the screen (show_page), " +
      "open a website (open_website: " + sites + "make a timer (make_timer), set the volume of what's playing (set_volume), close " +
      "what's open (close). " + (APP ? "Chat Computer itself, installed in place of a desktop, also changes settings and uses " +
        "their AirPods; when they ask for something only it can do, say so in a sentence." :
      "On a real Chat Computer you also use any website, their real files, apps and settings, and their AirPods; when they ask " +
      "for something only a Chat Computer can do, say so in a sentence.") + "\n\n" +
      (person() ? "They go by " + person() + ". " : computerName ? "This computer says they're " + computerName + ". " : "") +
      (homeName() ? "They live in " + homeName() + ". " : "") +
      "When they tell you what to call them, or where they live, keep it (set_name, set_place): the greeting that opens " +
      "each conversation (under the Chat Computer logo) says their name and the weather where they are." +
      (APP ? "" : "\n\nTheir messages and files here are made up for the demo:\n" + inbox() + "\n\nFiles:\n" +
        FILES.map(function (f) { return "- " + f.name + ", in " + f.where; }).join("\n")) +
      "\n\nEach of their messages starts with the date and time it was sent, in brackets.";
  }
  function obj(props, required) { return { type: "object", properties: props, required: required || [] }; }
  var TOOLS = [
    { name: "play_youtube", description: "Play a YouTube video between the sidebars, by its ID (only one you know for certain: " +
      "otherwise youtube_search).", input_schema: obj({ video_id: { type: "string", description: "The 11-character ID, as in youtube.com/watch?v=ID" },
      title: { type: "string", description: "The video's title, for Actions" }, start: { type: "integer", description: "Seconds in to start at" } }, ["video_id", "title"]) },
    { name: "show_map", description: "Show a map (OpenStreetMap) between the sidebars, of the area in the box given in degrees.",
      input_schema: obj({ place: { type: "string" }, west: { type: "number" }, south: { type: "number" }, east: { type: "number" },
      north: { type: "number" } }, ["place", "west", "south", "east", "north"]) },
    { name: "show_page", description: "Show a page you write between the sidebars, in Markdown (headings, lists, tables), when " +
      "they ask for something on the screen; answers go in the conversation.", input_schema: obj({ title: { type: "string" }, markdown: { type: "string" } }, ["title", "markdown"]) },
    { name: "open_website", description: APP ? "Open a website, any site, in the middle of the screen, for them to use." :
      "Open a website. Google (google.com, its searches, Google Maps' places and directions) and " +
      "Wikipedia open in the middle of the screen, for them to scroll and type in; most other sites can't show inside the demo, so " +
      "this shows a link to them there.",
      input_schema: obj({ url: { type: "string", description: "The full web address, with https" }, title: { type: "string" } }, ["url"]) },
    { name: "open_message", description: "Open one of their messages between the sidebars (as the conversation it's part of).",
      input_schema: obj({ id: { type: "string", enum: MESSAGES.map(function (m) { return m.id; }) } }, ["id"]) },
    { name: "open_file", description: "Open one of their files between the sidebars.",
      input_schema: obj({ name: { type: "string", enum: FILES.map(function (f) { return f.name; }) } }, ["name"]) },
    { name: "make_timer", description: "Put a countdown timer between the sidebars.",
      input_schema: obj({ minutes: { type: "integer", minimum: 1, maximum: 180 } }, ["minutes"]) },
    { name: "set_volume", description: "Set the volume of what's playing, from 0 to 100.", input_schema: obj({ percent: { type: "integer", minimum: 0, maximum: 100 } }, ["percent"]) },
    { name: "close", description: "Close what's open between the sidebars (a video stops).", input_schema: obj({}) },
    { name: "youtube_search", description: "Show YouTube's search for something, between the sidebars (when you don't know a " +
      "video's ID for certain).", input_schema: obj({ query: { type: "string" } }, ["query"]) },
    { name: "play_piano", description: "Play some calm piano music between the sidebars (the browser makes it).", input_schema: obj({}) },
    { name: "weather", description: "The weather today or tomorrow (Open-Meteo): somewhere, or where they live.",
      input_schema: obj({ place: { type: "string", description: "A town or city; leave it out for where they live" },
                          tomorrow: { type: "boolean" } }) },
    { name: "web_search", description: "Look something up: Wikipedia's articles, the day's news (Wikipedia's Current events) and, " +
      "for haze or pollution, the air (Open-Meteo). For anything recent, or that you aren't sure of.",
      input_schema: obj({ query: { type: "string", description: "What to look up, in a few words" } }, ["query"]) },
    { name: "set_name", description: "Keep the name they want to be called (empty: forget it), for greeting them by it.",
      input_schema: obj({ name: { type: "string" } }, ["name"]) },
    { name: "set_place", description: "Keep the town or city they live in (empty: forget it; then their time zone's city is used), " +
      "for the weather there: in the greeting, and when they ask for the weather without saying where.",
      input_schema: obj({ place: { type: "string" } }, ["place"]) }
  ];
  // In the app: their real files instead of the made-up ones and messages.
  var APP_TOOLS = [
    { name: "find_files", description: "Find files and folders on this computer by name (the words must all be in the name): " +
      "Desktop, Documents, Downloads, Pictures, Music, Videos and OneDrive. Up to 8, best first, each with its path.",
      input_schema: obj({ query: { type: "string", description: "Words in the name, like “taxes 2025”" } }, ["query"]) },
    { name: "open_file", description: "Open a file or folder find_files found, with its own app (Preview, Word, Photos...; a " +
      "folder in Finder or File Explorer). Never a program or a script.", input_schema: obj({ path: { type: "string" } }, ["path"]) },
    { name: "read_messages", description: "Their messages, newest first, from the accounts connected in Messages: each with its " +
      "id, where it's from, who, when, whether it's unread, and what it says (an email's subject and start, a chat's last " +
      "message). Empty when none is connected (they connect one with + in Messages).", input_schema: obj({}) },
    { name: "open_message", description: "Open one of their messages (its id, from read_messages) in the middle: its service's " +
      "own page, at that conversation, for them to read and answer there.", input_schema: obj({ id: { type: "string" } }, ["id"]) }
  ];
  function tools() {
    return APP ? TOOLS.filter(function (t) { return t.name !== "open_message" && t.name !== "open_file"; }).concat(APP_TOOLS) : TOOLS;
  }
  // As OpenAI's chat completions API has them (the engine's).
  function localTools() {
    return tools().map(function (t) { return { type: "function", function: { name: t.name, description: t.description, parameters: t.input_schema } }; });
  }
  // What a tool does here, and what the model is told about it.
  function run(u, you, uts) {
    var a = u.input || {}, n = u.name;
    if (n === "play_youtube") {
      var vid = String(a.video_id || "").trim();
      if (!/^[A-Za-z0-9_-]{11}$/.test(vid)) return "That isn't a YouTube video ID (11 letters, digits, - or _).";
      var start = Math.max(0, parseInt(a.start, 10) || 0);
      youtube(vid, start, clip(a.title || "", 80));
      did("Played \u201c" + clip(a.title || "a video", 60) + "\u201d on YouTube", you, uts);
      return "Playing between the sidebars. (If its owner doesn't allow it to play on other sites, YouTube says so there.)";
    }
    if (n === "show_map") {
      var b = [a.west, a.south, a.east, a.north].map(Number);
      if (b.some(isNaN) || b[0] >= b[2] || b[1] >= b[3] || Math.abs(b[1]) > 90 || Math.abs(b[3]) > 90) return "That box isn't west < east, south < north, in degrees.";
      map(b, clip(a.place || "", 60));
      did("Opened openstreetmap.org", you, uts);
      return "The map of " + clip(a.place, 60) + " is on the screen.";
    }
    if (n === "show_page") {
      show({ html: doc(clip(a.title || "Page", 80), String(a.markdown || "")) });
      did("Showed \u201c" + clip(a.title || "a page", 60) + "\u201d", you, uts);
      return "It's on the screen.";
    }
    if (n === "open_website") {
      var url = String(a.url || "").trim(), host;
      try { var p = new URL(url); if (!/^https?:$/.test(p.protocol)) throw 0; host = p.hostname.replace(/^www\./, ""); url = p.href; }
      catch (e) { return "That isn't a full web address, with http or https."; }
      var real = framed(url);
      if (real) {
        show({ src: real, title: clip(a.title || host, 80) });
        did("Opened " + host, you, uts);
        return host + " is on the screen, in the middle: they can scroll it and type in it (you can't see or use it from here).";
      }
      if (newTab(url)) {                // only while the click or key that asked still lets a page open a tab
        did("Opened " + host + " in a new tab", you, uts);
        return host + " opened in a new tab beside this page (most sites can't show inside it): they're looking at it there.";
      }
      show({ html: page(host, '<article><h1 class="title">' + esc(a.title || host) + '</h1><p class="demo-note">On Chat Computer, ' + esc(host) +
        " opens right here, and Chat Computer uses it for you. Most sites can't show inside this demo, so here's the link.</p><p><a href=\"" + esc(url) +
        '" target="_blank" rel="noopener noreferrer">Open ' + esc(host) + " in a new tab</a></p></article>", NOTE_CSS) });
      did("Showed a link to " + host, you, uts);
      return "A link to " + host + " is on the screen (it opens in a new tab).";
    }
    if (APP && n === "read_messages") {
      if (!connected().length) return "No account is connected: they can connect Gmail, Outlook, Telegram, WhatsApp, WeChat " +
        "or Discord with + in Messages.";
      return JSON.stringify(appInbox());
    }
    if (APP && n === "open_message") {
      var am = MESSAGES.filter(function (x) { return x.id === String(a.id || ""); })[0];
      if (!am) return "There's no message " + clip(a.id, 60) + " (read_messages has them).";
      openMessage(am.id);
      did("Opened the message from " + am.who + " on " + am.label, you, uts);
      return "It's in the middle, on " + am.label + "'s own page.";
    }
    if (n === "open_message") {
      var m = THREADS[a.id] ? openMessage(a.id) : null;
      if (!m) return "There's no message " + a.id + ".";
      did("Opened the message from " + m.who, you, uts);
      return "It's on the screen.";
    }
    if (APP && n === "find_files") {
      return APP.app("search_files", { query: String(a.query || "") }).then(function (hits) {
        if (!hits.length) return "Nothing on this computer has “" + clip(a.query, 60) + "” in its name.";
        return JSON.stringify(hits.map(function (h) { return { name: h.name, where: h.where, kind: h.kind, changed: whenText(h.ts), path: h.path }; }));
      });
    }
    if (APP && n === "open_file") {
      return APP.app("open_file", { path: String(a.path || "") }).then(function (name) {
        did("Opened " + name, you, uts);
        return name + " is open, in its own app (not on this screen).";
      }, function (e) { return String(e); });
    }
    if (n === "open_file") {
      var f = FILES.filter(function (x) { return x.name.toLowerCase() === String(a.name || "").toLowerCase(); })[0];
      if (!f) return "There's no file called " + a.name + ".";
      show({ html: PAGES[f.page]() });
      did("Opened " + f.name, you, uts);
      return "It's on the screen.";
    }
    if (n === "make_timer") {
      var mins = Math.max(1, Math.min(180, parseInt(a.minutes, 10) || 10));
      show({ html: timer(mins) });
      did("Made a timer", you, uts, shown());
      return "A " + mins + "-minute timer is on the screen, with Start and Reset.";
    }
    if (n === "set_volume") {
      if (!player.id) return "Nothing is playing.";
      var back = volumeTo(Number(a.percent));
      did("Set the volume to " + player.volume + "%", you, uts, back);
      return "The volume is " + player.volume + "%.";
    }
    if (n === "close") { var was = !!view || site.on; close(); return was ? "Closed." : "Nothing was open."; }
    if (n === "youtube_search") {
      var yq = clip(String(a.query || ""), 80);
      if (!yq) return "What to search YouTube for?";
      var yurl = "https://www.youtube.com/results?search_query=" + encodeURIComponent(yq);
      if (APP) show({ src: yurl, title: "YouTube: " + yq });
      else youtubeSearch(yq).act();
      did("Searched YouTube for \u201c" + clip(yq, 40) + "\u201d", you, uts);
      return APP ? "YouTube's search for it is in the middle: they choose a video there." : "A link to YouTube's search for it is on the screen.";
    }
    if (n === "play_piano") {
      did("Played some calm piano", you, uts, piano() || null);
      return "Calm piano is playing between the sidebars.";
    }
    if (n === "weather") {
      var wt = weather("weather" + (a.place ? " in " + clip(String(a.place), 60) : "") + (a.tomorrow ? " tomorrow" : ""));
      return wt.later().then(function (r) { (r.done || []).forEach(function (d) { did(d, you, uts); }); return r.text; });
    }
    if (n === "web_search") {
      var wq = clip(String(a.query || ""), 100);
      if (!wq) return "What to look up?";
      return lookup(wq, wq).then(function (got) {
        got.told.forEach(function (d) { did(d, you, uts); });
        if (!got.list.length) return "Nothing was found for " + wq + ".";
        return JSON.stringify(got.list.slice(0, 6).map(function (x) {
          return { source: x.source, title: x.title, url: x.url, text: clip(x.text, 900) };
        }));
      });
    }
    if (n === "set_name") {
      var nm = cleanName(a.name);
      setName(nm);
      did(nm ? "Set your name to " + nm : "Forgot your name", you, uts);
      return nm ? "Kept: the greeting calls them " + nm + " now." : "Forgotten.";
    }
    if (n === "set_place") {
      var pl = String(a.place || "").replace(/[^\p{L}\p{M}' .,-]/gu, "").trim().slice(0, 60);
      setPlace(pl ? { name: pl } : null);       // found on the map, for its weather, when the greeting needs it
      did(pl ? "Set your place to " + pl : "Forgot your place", you, uts);
      return pl ? "Kept: " + pl + "." : "Forgotten: their time zone's city is used.";
    }
    return "There's no tool called " + n + " here.";
  }
  // One turn: the model's answer streams in, its tools run here, and it goes on until it's done (at most 8 rounds).
  function localTurn(text, you, uts, id, since) {
    var web = !own && local.kind === "web", calledBefore = {}, ran = 0;
    var said = "", gap = false, rounds = 0, asked = { role: "user", content: (web ? "" : "[" + clock() + "] ") + text };
    if (!local.talk.length) local.system = !own && local.kind === "web" ? WEB_SYSTEM : systemPrompt();
    local.talk.push(asked);
    trimTalk();
    call("aiStart", id);
    function onText(t) {
      if (!said && since != null && /\S/.test(t)) wordsOnly(since);
      if (gap && said && !/\s$/.test(said)) { call("ai", id, " "); said += " "; }
      gap = false;
      said += t;
      call("ai", id, t);
    }
    (function round() {
      if (++rounds > 8) { finish(); return; }
      gap = true;
      localMessage(onText).then(function (msg) {
        local.talk.push(msg.message);
        if (msg.calls.length) {
          // (The same tools with the same input as before, and nothing else: it's going round in circles. It stops.)
          var fresh = msg.calls.filter(function (c) { return !calledBefore[c.name + " " + c.args]; });
          if (!fresh.length) {
            local.talk.pop();
            finish();
            return;
          }
          // Each tool's answer, some later (a promise), in the order asked.
          Promise.all(msg.calls.map(function (c) {
            var out, key = c.name + " " + c.args;
            if (calledBefore[key]) {
              return Promise.resolve({ role: "tool", tool_call_id: c.id, content: "Already done, as asked before: " + calledBefore[key] });
            }
            ran++;
            try { out = run({ name: c.name, input: c.input, id: c.id }, you, uts); } catch (e) { out = "That didn't work: " + e.message; }
            return Promise.resolve(out).then(String, function (e) { return "That didn't work: " + (e && e.message || e); })
              .then(function (t) { calledBefore[key] = clip(t, 300); return { role: "tool", tool_call_id: c.id, content: t }; });
          })).then(function (results) {
            results.forEach(function (r) { local.talk.push(r); });
            round();
          });
          return;
        }
        finish();
      }, function (err) {
        var at = local.talk.indexOf(asked);
        if (at >= 0) local.talk.length = at;                              // the turn didn't happen
        onText((said ? "\n\n" : "") + localTrouble(err));
        call("aiDone", id);
        say(said);
      });
    })();
    function finish() {
      // (No words at all: what it did is in Actions; or it got nowhere.)
      if (!/\S/.test(said)) onText(ran ? (rounds > 8 ? "I stopped there: that was taking too many steps. What it did is in Actions." :
        "Done: it's in Actions.") : "I don't have an answer to that.");
      call("aiDone", id);
      say(said);
    }
  }
  // At most 40 messages back, starting at something they said.
  function trimTalk() {
    while (local.talk.length > 40) {
      local.talk.shift();
      while (local.talk.length && local.talk[0].role !== "user") local.talk.shift();
    }
  }
  // (No answer at all: on the website, the app isn't running any more; in the app, its engine stopped, and the app
  // starts it again, saying so: chatcomputerApp({model}).)
  function localTrouble(err) {
    if (own && own.provider === "chatgpt") {
      if (err.status === 429) return "You've reached your ChatGPT plan's limit for now. [Manage usage](" + PLAN_USAGE + ") shows when it resets.";
      if (err.status === 401 || err.status === 403 || /sign in again|isn't signed in/.test(err.message)) {
        return "ChatGPT's sign-in has run out: press the spark beside + to sign in again.";
      }
      return err.status ? "ChatGPT couldn't answer that: " + err.message : "I can't reach ChatGPT: " + err.message;
    }
    if (own) {
      if (err.status === 401 || err.status === 403) return ownName() + " didn't take your key any more: press the spark beside + to sign in again.";
      if (err.status === 402) return ownName() + " says your account has no credit left for " + own.model + ".";
      if (err.status === 529) return ownName() + " is overloaded just now: ask again in a moment.";
      if (err.status === 429) return ownName() + " says it's had too many questions just now" + (own.provider === "openrouter" ?
        " (free models have limits)" : "") + ": ask again in a little while.";
      return err.status ? ownName() + " couldn't answer that: " + err.message : "I can't reach " + ownName() + ": is the internet on?";
    }
    if (!err.status) {
      if (!APP) {
        local.state = "off";
        status();
        greetWith();
        return "I can't reach the Chat Computer app on this computer any more: is it still running?";
      }
      return "My model stopped answering, and is starting again: ask me again in a moment.";
    }
    return "My model couldn't answer that: " + err.message;
  }
  // One request, streamed: its words as they come, and the whole message (its words, thinking and tool calls).
  // Chat Computer's engine, and most AIs, as OpenAI's chat completions; OpenAI's own through its Responses API (its
  // best models use tools only there); ChatGPT's, the app's.
  function localMessage(onText) {
    var messages = [{ role: "system", content: local.system || (!own && local.kind === "web" ? WEB_SYSTEM : systemPrompt()) }].concat(local.talk);
    if (own && own.provider === "chatgpt") return chatgptMessage(messages, localTools(), onText);
    var headers = { "content-type": "application/json" };
    if (!own && local.kind === "web") return webMessage(messages, onText);
    if (!own) {
      return chatMessage(localBase() + "/v1/chat/completions", headers,
        { model: "chat-computer", stream: true, think: false, max_tokens: 4096, tools: localTools(), messages: messages }, onText, {});
    }
    var p = own.provider, a = AIS[p] || {}, h = ownHeaders(p, own.key);
    for (var k in h) headers[k] = h[k];
    if (a.responses) return responsesMessage(ownBase() + "/responses", headers, responsesBody(messages, localTools()), onText);
    if (p === "anthropic") return claudeMessage(ownBase() + "/messages", headers, claudeBody(messages, localTools()), onText);
    // (Your own AI: its model, none of the engine's own options. Its thinking goes back only to an AI that wants it;
    // Gemini's signatures on its tool calls, only to Gemini; Mistral's thinking, as Mistral gave it, only to Mistral.)
    var body = { model: own.model, stream: true, tools: localTools(), messages: messages.map(function (m) {
      var c = {};
      for (var x in m) if (x !== "chunks") c[x] = m[x];
      if (m.chunks && p === "mistral") c.content = m.chunks;
      if (c.reasoning_content != null && !a.think) delete c.reasoning_content;
      if (c.tool_calls && p !== "gemini") c.tool_calls = c.tool_calls.map(function (t) { return { id: t.id, type: t.type, function: t.function }; });
      delete c.blocks;
      return c;
    }) };
    for (var e in a.extra || {}) body[e] = a.extra[e];
    var how = effortBody(p, own);
    for (var f in how) body[f] = how[f];
    return chatMessage(ownBase() + "/chat/completions", headers, body, onText, { unthink: a.unthink, chunks: p === "mistral" });
  }
  function noConnection(e) { var x = new Error(e && e.message || "no connection"); x.status = 0; throw x; }
  function httpError(res) {
    return res.text().then(function (t) {
      var e = new Error(clip(apiError(t) || "HTTP " + res.status, 200));
      e.status = res.status === 400 && keyRefused(e.message) ? 401 : res.status;
      throw e;
    });
  }
  // A streamed answer's events, as they come: each one's data to `on`; done at the end of the stream.
  function readEvents(res, on) {
    var reader = res.body.getReader(), dec = new TextDecoder(), buf = "";
    function event(chunk) {
      var data = chunk.split("\n").filter(function (l) { return l.indexOf("data:") === 0; })
        .map(function (l) { return l.slice(5).trim(); }).join("");
      if (data && data !== "[DONE]") on(data);
    }
    return new Promise(function (resolve, reject) {
      (function pump() {
        reader.read().then(function (r) {
          if (!r.done) {
            buf += dec.decode(r.value, { stream: true }).replace(/\r\n/g, "\n");
            for (var i = buf.indexOf("\n\n"); i >= 0; i = buf.indexOf("\n\n")) { event(buf.slice(0, i)); buf = buf.slice(i + 2); }
            pump();
            return;
          }
          if (buf.trim()) event(buf);
          resolve();
        }, function (e) { var x = new Error(e && e.message || "the connection broke"); x.status = 0; reject(x); });
      })();
    });
  }
  // Words with any <think>…</think> taken out of them (some AIs think out loud in their words): the rest to `out`,
  // the thinking to `think`; a tag cut between two pieces held back until it's whole.
  function thinkFilter(out, think) {
    var inside = false, held = "";
    function feed(t) {
      t = held + t;
      held = "";
      while (t) {
        var tag = inside ? "</think>" : "<think>", at = t.indexOf(tag);
        if (at < 0) {
          var keep = 0;
          for (var n = Math.min(tag.length - 1, t.length); n > 0; n--) if (tag.indexOf(t.slice(t.length - n)) === 0) { keep = n; break; }
          if (t.length > keep) (inside ? think : out)(t.slice(0, t.length - keep));
          held = t.slice(t.length - keep);
          return;
        }
        if (at) (inside ? think : out)(t.slice(0, at));
        t = t.slice(at + tag.length);
        inside = !inside;
      }
    }
    feed.end = function () { if (held) (inside ? think : out)(held); held = ""; };
    return feed;
  }
  // Chat completions, streamed (Chat Computer's engine's, and most AIs'): words (a string, or, from Mistral's
  // thinking models, pieces: its thinking, then its words), thinking (reasoning_content), and tool calls (with
  // Gemini's signature on them, extra_content).
  function chatMessage(url, headers, body, onText, how) {
    return fetch(url, { method: "POST", headers: headers, body: JSON.stringify(body) }).then(function (res) {
      if (!res.ok) return httpError(res);
      var content = "", thinking = "", calls = [], failed = null;
      function said(t) { content += t; onText(t); }
      function thought(t) { thinking += t; }
      var words = how.unthink ? thinkFilter(said, thought) : said;
      return readEvents(res, function (data) {
        var ev;
        try { ev = JSON.parse(data); } catch (x) { return; }
        if (ev.error) { failed = new Error(clip(apiError(data) || "the model stopped", 200)); failed.status = 500; return; }
        var d = (ev.choices && ev.choices[0] && ev.choices[0].delta) || {};
        if (typeof d.reasoning_content === "string") thinking += d.reasoning_content;
        (typeof d.content === "string" ? [{ type: "text", text: d.content }] : Array.isArray(d.content) ? d.content : []).forEach(function (c) {
          if (c && c.type === "text" && c.text) words(c.text);
          else if (c && c.type === "thinking") (Array.isArray(c.thinking) ? c.thinking : []).forEach(function (t) { if (t && t.text) thought(t.text); });
        });
        (d.tool_calls || []).forEach(function (c) {
          var i = c.index || 0, f = c.function || {};
          var cur = calls[i] = calls[i] || { id: "", name: "", args: "" };
          if (c.id) cur.id = c.id;
          if (f.name) cur.name += f.name;
          if (f.arguments) cur.args += f.arguments;
          if (c.extra_content) cur.extra = c.extra_content;
        });
      }).then(function () {
        if (words.end) words.end();
        if (failed) throw failed;
        calls = calls.filter(Boolean);
        var list = calls.map(function (c, k) {
          var input = {};
          try { if (c.args) input = JSON.parse(c.args); } catch (x) { input = {}; }
          return { id: c.id || "call_" + k, name: c.name, input: input, args: c.args || "{}" };
        });
        var message = { role: "assistant", content: content };
        if (how.chunks && thinking) message.chunks = [{ type: "thinking", thinking: [{ type: "text", text: thinking }] }]
          .concat(content ? [{ type: "text", text: content }] : []);
        else if (thinking) message.reasoning_content = thinking;
        if (list.length) {
          message.tool_calls = list.map(function (c, k) {
            var t = { id: c.id, type: "function", function: { name: c.name, arguments: c.args } };
            if (calls[k].extra) t.extra_content = calls[k].extra;
            return t;
          });
        }
        return { message: message, calls: list };
      });
    }, noConnection);
  }
  // OpenAI's Responses API, streamed: its words to `onText` as they come, and the whole message (words and tool
  // calls) in the same shape as the others'. (ChatGPT's too, through the app.)
  function responsesReader(onText) {
    var content = "", calls = [], failed = null, done = false;
    function event(data) {
      var ev;
      try { ev = JSON.parse(data); } catch (e) { return; }
      var t = ev.type || "", i = ev.output_index || 0, item = ev.item || {};
      if (t === "response.output_text.delta" && ev.delta) { content += ev.delta; onText(ev.delta); }
      else if (t === "response.output_item.added" && item.type === "function_call") {
        calls[i] = { id: item.call_id || item.id || "call_" + i, name: item.name || "", args: item.arguments || "" };
      } else if (t === "response.function_call_arguments.delta" && calls[i]) calls[i].args += ev.delta || "";
      else if (t === "response.function_call_arguments.done" && calls[i] && typeof ev.arguments === "string") calls[i].args = ev.arguments;
      else if (t === "response.output_item.done" && item.type === "function_call") {
        calls[i] = calls[i] || { id: item.call_id || item.id || "call_" + i, name: item.name || "", args: "" };
        if (typeof item.arguments === "string") calls[i].args = item.arguments;
      } else if (t === "response.completed" || t === "response.incomplete") done = true;
      else if (t === "response.failed" || t === "error") {
        var er = (ev.response && ev.response.error) || ev.error || ev;
        failed = new Error(clip(er.message || "it stopped", 200));
        failed.status = /limit|quota/i.test(String(er.code || er.message || "")) ? 429 : 500;
      }
    }
    function result() {
      if (failed) throw failed;
      var list = calls.filter(Boolean).map(function (c, k) {
        var input = {};
        try { if (c.args) input = JSON.parse(c.args); } catch (x) { input = {}; }
        return { id: c.id || "call_" + k, name: c.name, input: input, args: c.args || "{}" };
      });
      var message = { role: "assistant", content: content };
      if (list.length) message.tool_calls = list.map(function (c) { return { id: c.id, type: "function", function: { name: c.name, arguments: c.args } }; });
      return { message: message, calls: list, done: done };
    }
    return { event: event, result: result };
  }
  // Claude, through Anthropic's own Messages API (its effort can be set only there): what it's told (system), then the
  // conversation as its blocks: Claude's own messages as it gave them (its thinking with them, as Anthropic asks
  // within a turn that uses tools), its tool calls as tool_use, and the tools' answers as tool_result, first in the
  // message after; streamed back as its blocks.
  function claudeBody(messages, tools) {
    var told = [], out = [];
    function add(role, blocks) {
      if (!blocks.length) return;
      var last = out[out.length - 1];
      if (last && last.role === role) last.content = last.content.concat(blocks);
      else out.push({ role: role, content: blocks });
    }
    messages.forEach(function (m) {
      if (m.role === "system") { told.push(String(m.content || "")); return; }
      if (m.role === "user") { add("user", [{ type: "text", text: String(m.content || "") || " " }]); return; }
      if (m.role === "tool") { add("user", [{ type: "tool_result", tool_use_id: m.tool_call_id, content: String(m.content || "") || " " }]); return; }
      if (m.role !== "assistant") return;
      if (m.blocks) { add("assistant", m.blocks.filter(function (b) { return !(b.type === "text" && !b.text); })); return; }
      var b = m.content ? [{ type: "text", text: String(m.content) }] : [];
      (m.tool_calls || []).forEach(function (c) {
        var input = {};
        try { input = JSON.parse(c.function.arguments || "{}"); } catch (e) { input = {}; }
        b.push({ type: "tool_use", id: c.id, name: c.function.name, input: input });
      });
      add("assistant", b);
    });
    var body = { model: own.model, max_tokens: 32000, system: told.join("\n\n"), messages: out, stream: true,
                 tools: tools.map(function (t) { return { name: t.function.name, description: t.function.description, input_schema: t.function.parameters }; }) };
    if (own.effort) body.output_config = { effort: own.effort };
    return body;
  }
  function claudeMessage(url, headers, body, onText) {
    return fetch(url, { method: "POST", headers: headers, body: JSON.stringify(body) }).then(function (res) {
      if (!res.ok) return httpError(res);
      var blocks = [], failed = null;
      return readEvents(res, function (data) {
        var ev;
        try { ev = JSON.parse(data); } catch (x) { return; }
        if (ev.type === "content_block_start") {
          var b = ev.content_block || {}, cur = { type: b.type };
          if (b.type === "text") cur.text = b.text || "";
          else if (b.type === "tool_use") { cur.id = b.id; cur.name = b.name; cur.json = ""; }
          else if (b.type === "thinking") { cur.thinking = b.thinking || ""; cur.signature = b.signature || ""; }
          else cur.whole = b;                                     // (redacted thinking, and the like: given back as it came)
          blocks[ev.index || 0] = cur;
        } else if (ev.type === "content_block_delta") {
          var c = blocks[ev.index || 0], d = ev.delta || {};
          if (!c) return;
          if (d.type === "text_delta" && d.text) { c.text += d.text; onText(d.text); }
          else if (d.type === "input_json_delta") c.json += d.partial_json || "";
          else if (d.type === "thinking_delta") c.thinking += d.thinking || "";
          else if (d.type === "signature_delta") c.signature += d.signature || "";
        } else if (ev.type === "error") {
          var er = ev.error || {};
          failed = new Error(clip(er.message || "Claude stopped", 200));
          failed.status = er.type === "overloaded_error" ? 529 : er.type === "rate_limit_error" ? 429 : 500;
        }
      }).then(function () {
        if (failed) throw failed;
        var mine = blocks.filter(Boolean).map(function (c) {
          if (c.type === "text") return { type: "text", text: c.text };
          if (c.type === "tool_use") {
            var input = {};
            try { input = c.json ? JSON.parse(c.json) : {}; } catch (x) { input = {}; }
            return { type: "tool_use", id: c.id, name: c.name, input: input };
          }
          if (c.type === "thinking") return { type: "thinking", thinking: c.thinking, signature: c.signature };
          return c.whole;
        });
        var calls = mine.filter(function (b) { return b.type === "tool_use"; }).map(function (u) {
          return { id: u.id, name: u.name, input: u.input, args: JSON.stringify(u.input) };
        });
        var message = { role: "assistant", content: mine.filter(function (b) { return b.type === "text"; }).map(function (b) { return b.text; }).join(""),
                        blocks: mine };
        if (calls.length) message.tool_calls = calls.map(function (c) { return { id: c.id, type: "function", function: { name: c.name, arguments: c.args } }; });
        return { message: message, calls: calls };
      });
    }, noConnection);
  }
  function responsesMessage(url, headers, body, onText) {
    var read = responsesReader(onText);
    return fetch(url, { method: "POST", headers: headers, body: JSON.stringify(body) }).then(function (res) {
      if (!res.ok) return httpError(res);
      return readEvents(res, read.event).then(read.result);
    }, noConnection);
  }

  // ------------------------------------------------------------------ your own AI
  // Beside the box's +, the spark: sign in with an AI you already use, and it answers instead of Chat Computer's own
  // model, on your own account, with the same tools. On the website: OpenRouter, signing in on its own page (it gives
  // this page a key of its own, which you can take back on its site): its free models cost nothing, and the best
  // model of each maker at the top of Artificial Analysis's Intelligence Index is there too, on your OpenRouter
  // credits. Or a key of yours from each of those makers, asked straight from this page (each lets a web page ask
  // it: checked from chatcomputer.ai, November 2026); or from any other AI that answers as OpenAI's chat completions do.
  // What you say then goes to that AI; the key stays in this browser, until you sign out (the spark again).
  // In the app: ChatGPT too, on your ChatGPT plan (OpenAI's "Sign in with ChatGPT", for apps on your own computer):
  // you sign in on OpenAI's page in your own browser, the app keeps the tokens (apps/desktop: chatgpt.rs) and asks
  // OpenAI's Responses API for this page; nothing of the sign-in is kept here but which model answers. (ChatGPT's own
  // sign-in isn't offered to websites; OpenRouter's, which comes back to a web page, is the website's.)
  var OWN_KEY = "chatcomputer-own-ai", OR_VERIFIER = "chatcomputer-openrouter-verifier";
  // The makers of the models at the top of Artificial Analysis's Intelligence Index (artificialanalysis.ai, as it
  // stood on 3 November 2026), in the order of each one's best model there, and how each is asked (its own API
  // documentation, read the same day):
  //   name, by: what it's called, and its maker where that's another name;  site: whose logo it shows (through
  //   DuckDuckGo's icon service, as Messages' are);  keys: the page where you make a key;
  //   base: its API (chat completions in OpenAI's shape; OpenAI's own through its Responses API, as its best models use
  //   tools only there; Anthropic's through its Messages API, as its effort can be set only there);  alt: its API for keys made on its other site (China's), tried when the first says no;
  //   model: its best model, for when it doesn't list them;  best: its models best first (the first matching one of
  //   these answers; among those, the newest);  chat: which of its models chat;  skip: which don't, or don't here;
  //   think: it wants its own thinking back, in every message after (DeepSeek's rule, and Kimi's, Z.ai's, Xiaomi's,
  //   MiniMax's; the others aren't sent it);  extra: what's added to each question;  unthink: thinking may come
  //   inside its words, between <think> and </think>, and is taken out of them;  or: its models' names on OpenRouter.
  //   (StepFun's and Ant Group's (Ling) don't let a web page ask them, so they're on OpenRouter only.)
  var AIS = {
    openrouter: { name: "OpenRouter", base: "https://openrouter.ai/api/v1", site: "openrouter.ai", keys: "https://openrouter.ai/settings/keys",
                  unthink: true },
    gemini: { name: "Gemini", by: "Google", base: "https://generativelanguage.googleapis.com/v1beta/openai", site: "gemini.google.com",
              keys: "https://aistudio.google.com/apikey", model: "gemini-4-argon", chat: /^gemini-\d/,
              skip: /nano-banana|lyria|robotics|antigravity|deep-research|translate/,
              best: [/^gemini-4-carbon/, /^gemini-4-argon/, /^gemini-\d+(\.\d+)?-pro/, /^gemini-\d+(\.\d+)?-flash(?!-lite)/], or: "google/" },
    anthropic: { name: "Claude", by: "Anthropic", base: "https://api.anthropic.com/v1", site: "claude.ai",
                 keys: "https://platform.claude.com/settings/keys", model: "claude-opus-5-5", chat: /^claude-/,
                 best: [/^claude-opus/, /^claude-sonnet/, /^claude-fable/, /^claude-haiku/], or: "anthropic/" },
    openai: { name: "OpenAI", base: "https://api.openai.com/v1", site: "openai.com", responses: true,
              keys: "https://platform.openai.com/settings/organization/api-keys", model: "gpt-6-astra", chat: /^(gpt-\d|o\d)/,
              skip: /-pro\b|codex|deep-research|-search|cyber|rosalind|instruct|chat-latest|gpt-live/,
              best: [/^gpt-6-astra/, /^gpt-6\.1-sol/, /^gpt-6\.1-terra/, /^gpt-\d/], or: "openai/" },
    meta: { name: "Meta", base: "https://api.meta.ai/v1", site: "dev.meta.ai", keys: "https://dev.meta.ai/", model: "muse-wonder-1.0",
            chat: /^muse-/, skip: /contributor|glimmer/, best: [/^muse-wonder/, /^muse-spark-\d/, /^muse-spark/], or: "meta/" },
    kimi: { name: "Kimi", by: "Moonshot AI", base: "https://api.moonshot.ai/v1", alt: ["https://api.moonshot.cn/v1"], site: "platform.kimi.ai",
            keys: "https://platform.kimi.ai/console/api-keys", model: "kimi-k3.1-pro", chat: /^kimi-/, think: true,
            best: [/^kimi-k3\.1-pro$/, /^kimi-k3\.1/, /^kimi-k3/, /^kimi-k\d/], or: "moonshotai/" },
    grok: { name: "Grok", by: "xAI", base: "https://api.x.ai/v1", site: "grok.com", keys: "https://console.x.ai/team/default/api-keys",
            model: "grok-4.7", chat: /^grok-/, skip: /imagine|build|multi-agent/, best: [/^grok-4\.7$/, /^grok-\d/], or: "x-ai/" },
    qwen: { name: "Qwen", by: "Alibaba Cloud", base: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", site: "qwen.ai",
            keys: "https://modelstudio.console.alibabacloud.com/ap-southeast-1/settings/api-key", model: "qwen3.8-max", chat: /^qwen/,
            skip: /-mt-|coder|math|-character/, best: [/^qwen3\.8-max$/, /^qwen3\.8-max/, /^qwen\d(\.\d+)?-max/, /^qwen-max/, /-plus/],
            or: "qwen/" },
    deepseek: { name: "DeepSeek", base: "https://api.deepseek.com", site: "deepseek.com", keys: "https://platform.deepseek.com/api_keys",
                model: "deepseek-v4-pro", chat: /^deepseek-/, think: true,
                best: [/^deepseek-v4-pro/, /^deepseek-flash$/, /^deepseek-v\d+(\.\d+)?-flash/, /^deepseek-/], or: "deepseek/" },
    glm: { name: "GLM", by: "Z.ai", base: "https://api.z.ai/api/paas/v4", site: "docs.z.ai", keys: "https://z.ai/manage-apikey/apikey-list",
           model: "glm-5.3", chat: /^glm-/, skip: /legal/, think: true, best: [/^glm-5\.3$/, /^glm-5\.3/, /^glm-\d/], or: "z-ai/" },
    mimo: { name: "MiMo", by: "Xiaomi", base: "https://api.xiaomimimo.com/v1", site: "mi.com", keys: "https://platform.xiaomimimo.com/#/console/api-keys",
            model: "mimo-v2.6-pro", chat: /^mimo-/i, think: true, best: [/^mimo-v2\.6-pro$/i, /^mimo-v[\d.]+-pro$/i, /^mimo-/i], or: "xiaomi/" },
    stepfun: { name: "StepFun", site: "stepfun.ai", best: [/^step-5/, /^step-\d/], or: "stepfun/", keyless: true },
    minimax: { name: "MiniMax", base: "https://api.minimax.io/v1", alt: ["https://api.minimaxi.com/v1"], site: "minimax.io",
               keys: "https://platform.minimax.io/user-center/basic-information/interface-key", model: "MiniMax-M3.1-Flash",
               chat: /^minimax-/i, skip: /-her\b|hailuo|-h\d/i, think: true, unthink: true, extra: { reasoning_split: true },
               best: [/^minimax-m3\.1/i, /^minimax-m3/i, /^minimax-m\d/i], or: "minimax/" },
    ling: { name: "Ling", by: "Ant Group", site: "ant-ling.com", best: [/^ling-3\.1/, /^ling-/], or: "inclusionai/", keyless: true },
    mistral: { name: "Mistral", by: "Mistral AI", base: "https://api.mistral.ai/v1", site: "mistral.ai",
               keys: "https://console.mistral.ai/home?profile_dialog=api-keys", model: "mistral-large-4",
               chat: /^(mistral|magistral|devstral|codestral|ministral)/, skip: /shieldstral|voxtral|timestral/,
               best: [/^mistral-large-4/, /^mistral-large/, /^mistral-medium/, /^magistral-medium/], or: "mistralai/" },
    other: { name: "", base: "", site: "", keys: "", unthink: true },
    chatgpt: { name: "ChatGPT", base: "", site: "chatgpt.com", keys: "" }
  };
  // How hard each AI's models think before answering ("effort"), as each one's documentation says (3 November 2026):
  // the levels each takes, least first, and its own default ("": it decides). Where its list of models says (Anthropic's
  // capabilities, xAI's, DeepSeek's, OpenRouter's), what it says comes first. "off"/"on": thinking only switched.
  var EFFORTS = [
    ["anthropic", /^claude-(opus|haiku)/, ["low", "medium", "high", "xhigh", "max"], "medium"],
    ["anthropic", /^claude-/, ["low", "medium", "high", "xhigh", "max"], "high"],
    ["openai", /^gpt-6\.1-terra/, ["none", "low", "medium", "high", "xhigh", "max"], "medium"],
    ["openai", /^gpt-6/, ["low", "medium", "high", "xhigh", "max"], "medium"],
    ["gemini", /^gemini-4/, ["low", "medium", "high"], "medium"],
    ["meta", /contributor/, ["minimal", "low", "medium", "high", "xhigh"], ""],
    ["meta", /^muse-/, ["minimal", "low", "medium", "high", "xhigh", "max"], ""],
    ["kimi", /^kimi-k3/, ["low", "high", "max"], "max"],
    ["grok", /^grok-4\.7/, ["low", "medium", "high", "xhigh"], "high"],
    ["qwen", /^qwen3\.8/, ["none", "low", "medium", "xhigh"], "xhigh"],
    ["deepseek", /^deepseek-/, ["none", "low", "high", "max"], "high"],
    ["glm", /^glm-5/, ["low", "high", "max"], "max"],
    ["mimo", /^mimo-/i, ["off", "on"], "on"],
    ["minimax", /^minimax-m3\.1-flash/i, ["low", "medium", "high", "xhigh", "max"], "max"],
    ["minimax", /^minimax-m\d/i, ["off", "on"], "on"],
    ["mistral", /^mistral-large-4/, ["low", "medium", "high"], ""]
  ];
  var EFFORT_NAMES = { none: "Off", off: "Off", on: "On", minimal: "Minimal", low: "Low", medium: "Medium", high: "High", xhigh: "Extra high", max: "Max" };
  function effortName(v) { return EFFORT_NAMES[v] || String(v || ""); }
  // What each model is called, and the efforts it takes, as its AI's list of models said (kept here as they're listed).
  var MODEL_INFO = {};
  var CASED = { gpt: "GPT", glm: "GLM", deepseek: "DeepSeek", minimax: "MiniMax", mimo: "MiMo", qwen: "Qwen", openai: "OpenAI" };
  // A model's name as it's written: claude-opus-5-5 is Claude Opus 5.5, gpt-6.1-sol GPT-6.1 Sol, kimi-k3.1-pro Kimi K3.1 Pro
  // (OpenRouter's maker/ and :free left out: its list says those).
  function prettyModel(id) {
    var bare = String(id || "").replace(/^models\//, ""), maker = "";
    if (bare.indexOf("/") > 0) { maker = bare.slice(0, bare.indexOf("/") + 1); bare = bare.slice(maker.length); }
    var out = [];
    bare.replace(/:free$/, "").split("-").forEach(function (w) {
      var prev = out[out.length - 1];
      if (/^\d{8}$/.test(w)) return;                                  // (a date: left out)
      if (/^\d$/.test(w) && prev && /\d$/.test(prev) && !/\./.test(prev)) { out[out.length - 1] = prev + "." + w; return; }
      var low = w.toLowerCase(), name = CASED[low] || (/^o\d/.test(low) && !out.length ? low :
        /^[a-z]\d/.test(low) ? low.toUpperCase() : low.charAt(0).toUpperCase() + w.slice(1));
      if (/^\d/.test(w) && prev && /^(GPT|GLM)$/.test(prev)) { out[out.length - 1] = prev + "-" + w; return; }
      out.push(name);
    });
    return out.join(" ");
  }
  function modelInfo(p, id) {
    var info = MODEL_INFO[p + "|" + id] || {}, e = null;
    for (var i = 0; i < EFFORTS.length && !info.levels; i++) {
      if (EFFORTS[i][0] === p && EFFORTS[i][1].test(id)) e = EFFORTS[i];
      if (e) break;
    }
    return { name: info.name || prettyModel(id), levels: info.levels || (e ? e[2] : []), dflt: info.levels ? info.dflt || "" : e ? e[3] : "" };
  }
  // A model chosen: what it's called and the efforts it takes, kept with it (the effort chosen stays, where it takes it).
  function withModel(o, id) {
    var i = modelInfo(o.provider, id);
    o.model = id;
    o.name = i.name;
    o.levels = i.levels;
    o.dflt = i.dflt;
    if (o.effort && i.levels.indexOf(o.effort) < 0) o.effort = "";
    return o;
  }
  // What a model's list says of it (Anthropic's display name and efforts; xAI's, DeepSeek's and OpenRouter's efforts;
  // ChatGPT's reasoning levels), kept.
  function noteModel(p, m) {
    var id = String(m && (m.id || m.slug || m.model || m.name) || "").replace(/^models\//, ""), c = m && m.capabilities || {}, levels = null, dflt = "";
    if (!id) return;
    var ORDER = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
    if (c.effort && typeof c.effort === "object") {
      levels = ORDER.filter(function (l) { return c.effort[l] && c.effort[l].supported; });
    } else if (Array.isArray(c.reasoning_effort)) { levels = c.reasoning_effort.map(String); dflt = String(c.default_reasoning_effort || ""); }
    else if (m.effort && Array.isArray(m.effort.supported_levels)) {
      levels = ["none"].concat(m.effort.supported_levels.map(String));
      dflt = String(m.effort.default_level || "");
    } else if (p === "openrouter" && m.reasoning && typeof m.reasoning === "object") {
      levels = Array.isArray(m.reasoning.supported_efforts) ? m.reasoning.supported_efforts.map(String).reverse() :
        ["minimal", "low", "medium", "high", "xhigh", "max"];
      if (!m.reasoning.mandatory && levels.indexOf("none") < 0) levels.unshift("none");
      if (m.reasoning.mandatory) levels = levels.filter(function (l) { return l !== "none"; });
      dflt = String(m.reasoning.default_effort || "");
    } else if (p === "openrouter") levels = (m.supported_parameters || []).indexOf("reasoning") >= 0 ? ["none", "low", "medium", "high"] : [];
    else if (Array.isArray(m.supported_reasoning_levels)) {
      levels = m.supported_reasoning_levels.map(function (l) { return String(l && (l.effort || l) || ""); }).filter(Boolean);
      dflt = String(m.default_reasoning_level || "");
    }
    if (levels && p === "anthropic") dflt = modelInfo(p, id).dflt;
    var name = m.display_name || (p === "openrouter" && m.name ? String(m.name).replace(/^[^:]+:\s*/, "") : "");
    MODEL_INFO[p + "|" + id] = { name: name || "", levels: levels, dflt: dflt };
    if (!name) delete MODEL_INFO[p + "|" + id].name;
  }
  // The effort chosen, as each AI's request takes it.
  function effortBody(p, o) {
    var v = o && o.effort;
    if (!v) return {};
    if (p === "openrouter") return { reasoning: { effort: v } };
    if (v === "off" || v === "on") return { thinking: { type: v === "off" ? "disabled" : "enabled" } };
    return { reasoning_effort: v };
  }

  // In the index's order; and those you can use with a key of yours, from this page.
  var MAKERS = ["gemini", "anthropic", "openai", "meta", "kimi", "grok", "qwen", "deepseek", "glm", "mimo", "stepfun", "minimax", "ling", "mistral"];
  var KEYED = MAKERS.filter(function (p) { return !AIS[p].keyless; });
  // (Models that don't chat: drawing, speaking, listening, reading pictures of text, embedding, checking.)
  var NOT_CHAT = /embed|rerank|moderat|whisper|transcri|\btts\b|-tts|speech|audio|realtime|image|imagine|dall-e|sora|veo|imagen|video|ocr|guard|computer-use|-vl\b|vision|omni|asr|-live|^live/i;
  var PLAN_USAGE = "https://chatgpt.com/#settings/Usage", PLAN_TOLD = "chatcomputer-chatgpt-plan-told";
  var OTHER_BASE = /^(https:\/\/[^\s/?#]+|http:\/\/(127\.0\.0\.1|localhost)(:\d+)?)(\/[^\s?#]*)?$/;
  var own = (function () {
    var o = store(OWN_KEY), a = o && AIS[o.provider];
    if (!o || typeof o !== "object" || !a || a.keyless || typeof o.key !== "string" || !o.key || typeof o.model !== "string" || !o.model) return null;
    if (o.provider === "other" && !OTHER_BASE.test(String(o.base || ""))) return null;
    if (o.provider !== "other" && o.base && (a.alt || []).indexOf(o.base) < 0) return null;
    if (o.provider === "chatgpt" && !APP) return null;          // (ChatGPT's sign-in lives in the app)
    if (!Array.isArray(o.levels)) withModel(o, o.model);           // (kept before efforts were)
    return o;
  })();
  function ownName() { return !own ? "" : own.provider === "other" ? hostOf(own.base) || "Your AI" : AIS[own.provider].name; }
  function ownBase() { return own.base ? String(own.base).replace(/\/+$/, "") : AIS[own.provider].base; }
  function logo(site) { return site ? "https://icons.duckduckgo.com/ip3/" + site + ".ico" : ""; }
  // The spark beside +: shown, and which AI answers (its own logo).
  // What's said of the model answering: Claude Opus 5.5, at Max effort.
  function ownModelWords() {
    if (!own) return "";
    var e = own.effort || own.dflt;
    return (own.name || own.model) + (e && (own.levels || []).length ? ", " + (e === "off" || e === "none" ? "not thinking first" :
      e === "on" ? "thinking first" : "at " + effortName(e) + " effort") : "");
  }
  function ownShow() {
    var site = own && (own.provider === "other" ? hostOf(own.base) : AIS[own.provider].site);
    call("ownAi", own ? { name: ownName(), model: ownModelWords(), icon: logo(site) } : null);
  }
  var sheetNow = "";
  function sheetIs(id) { return sheetNow === id; }
  function ownSet(o, done) {
    // (Another AI starts the conversation afresh: each keeps its own kind of record of what it thought and did.)
    if ((own && own.provider) !== (o && o.provider)) local.talk = [];
    own = o;
    store(OWN_KEY, o);
    local.system = "";                         // (said anew, with which AI answers)
    ownShow();
    status();
    greetWith();
    if (done) did(done);
    if (o) answerWaiting();                      // (what was asked while no model was here)
  }
  // Your own AI, all in one panel. Not signed in: its own sign-in where there's one here (ChatGPT's in the app,
  // OpenRouter's on the website), then each AI once; one chosen (pick), what it needs typed shows under them, in the
  // same panel (its key; for another AI, its address and model too), with Get a key. Signed in: which model answers
  // (its models, the one answering marked), how hard it thinks (each level it takes, its default said), and signing out.
  function ownMenu(pick, error, back) {
    if (own) {
      var gpt = own.provider === "chatgpt";
      call("accountForm", { id: "own-ai", title: "Your own AI", ok: "", choices: ownChoices(), text: gpt ? [
        "Using your ChatGPT plan" + (own.email ? ", as " + own.email : "") + ": what you say goes to OpenAI. Your plan's usage is " +
        "shared with ChatGPT's own (Manage usage shows it). Signed out, Chat Computer's own model answers again."] : [
        ownName() + " answers now, on your own account: what you say goes to " + (AIS[own.provider].by || ownName()) + ". Its key " +
        "stays " + (APP ? "in the app" : "in this browser") + ". Signed out, Chat Computer's own model answers again."] });
      if (!own.models) ownList();                // (kept before its models were: listed now, and shown)
      return;
    }
    var signIn = APP && APP.channel ? "chatgpt" : "openrouter";
    var choices = signIn ? [{ label: "Continue with " + AIS[signIn].name, value: "ai:" + signIn, icon: logo(AIS[signIn].site) }] : [];
    choices.push({ note: signIn ? "Or with your key from:" : "With your key from:" });
    KEYED.concat(signIn === "openrouter" ? [] : ["openrouter"]).forEach(function (p) {
      choices.push({ label: AIS[p].name, value: "ai:key:" + p, icon: logo(AIS[p].site), on: p === pick });
    });
    choices.push({ label: "Another AI", value: "ai:key:other", on: pick === "other" });
    var a = pick && AIS[pick], other = pick === "other", maker = a && (a.by || a.name);
    var text = ["An AI you already use answers instead of Chat Computer's own model, on your own account: what you say then goes to it."];
    if (a) text.push(other ? "Another AI: any that answers as OpenAI's chat completions do. Its key stays " + (APP ? "in the app" : "in this browser") +
      ", and goes only to its address." : "Your " + maker + " key stays " + (APP ? "in the app" : "in this browser") + ", and goes only to " + maker +
      (APP ? ". Make one at " + a.keys.replace(/^https:\/\//, "").replace(/\/$/, "") + "." : "."));
    call("accountForm", { id: a ? "own-key-" + pick : "own-ai", title: "Your own AI", ok: a ? "Sign in" : "", error: error || "",
      first: true, choices: choices, text: text, back: back || "",
      fields: !a ? [] : (other ? [{ name: "base", label: "Its address (ending /v1, as a rule)" }, { name: "model", label: "Its model" }] : [])
        .concat([{ name: "key", label: other ? "Its key" : "Your " + maker + " API key", type: "password" }]),
      more: a && !other && !APP ? [{ label: "Get a key", value: "ai:open:" + pick }] : [] });
  }
  function ownChoices() {
    var p = own.provider, c = [{ note: "Model:" }];
    function one(id, icon) { return { label: modelInfo(p, id).name, value: "ai:model:" + id, icon: icon, on: id === own.model }; }
    var models = (own.models || []).slice();
    if (models.indexOf(own.model) < 0) models.unshift(own.model);
    if (p === "openrouter") {
      c[0] = { note: "Model (free, the newest first):" };
      models.forEach(function (m) { c.push(one(m)); });
      if ((own.best || []).length) {
        c.push({ note: "The best of each maker, on your OpenRouter credits:" });
        own.best.forEach(function (m) { c.push(one(m.id, logo(AIS[m.maker].site))); });
      }
    } else if (p === "other") {
      c.push(one(own.model));
      c.push({ label: "Change", value: "ai:key:other" });
    } else models.forEach(function (m) { c.push(one(m)); });
    if ((own.levels || []).length) {
      var on = own.effort || own.dflt;
      c.push({ note: own.levels.join() === "off,on" ? "Thinking before it answers:" : "Effort: how hard it thinks before answering" +
        (own.dflt ? " (" + effortName(own.dflt) + " is its default)" : "") + ":" });
      own.levels.forEach(function (l) { c.push({ label: effortName(l), value: "ai:effort:" + l, on: l === on }); });
    }
    c.push({ note: "" });
    if (p === "chatgpt") c.push({ label: "Manage usage", value: "ai:usage" });
    c.push({ label: "Sign out of " + ownName(), value: "ai:out" });
    return c;
  }
  // Its models, kept with it to choose from: the best first (OpenRouter's: its free ones, and the best of each maker).
  function keepModels(o, got) {
    if (o.provider === "openrouter") { o.models = got.free.slice(0, 6); o.best = got.best; }
    else o.models = got.slice(0, 8);
    return o;
  }
  // (Listed again, where they weren't kept: the panel shows them once they're here, if it's still open.)
  function ownList() {
    var p = own.provider, was = own;
    if (p === "other") { own.models = [own.model]; return; }
    (p === "chatgpt" ? chatgptModels() : ownModels(p, ownBase(), own.key, true)).then(function (got) {
      if (own !== was) return;
      keepModels(own, got);
      store(OWN_KEY, own);
      if (document.body.classList.contains("sheet") && sheetIs("own-ai")) ownMenu();
    }, noop);
  }
  function ownChoice(v) {
    var p = v.slice(v.indexOf(":") + 1);
    if (v.indexOf("effort:") === 0 && own && (own.levels || []).indexOf(p) >= 0) {
      var e = {};
      for (var k0 in own) e[k0] = own[k0];
      e.effort = p === e.dflt ? "" : p;
      ownSet(e, ownName() + (p === "off" || p === "none" ? " doesn't think before answering" : p === "on" ? " thinks before answering" :
        "'s effort set to " + effortName(p)));
      ownMenu();
      return;
    }
    if (v === "menu") { ownMenu(); return; }                     // (back, from a panel that came from this one)
    if (v === "openrouter") { openRouterSignIn(); return; }
    if (v === "chatgpt" && APP) { chatgptSignIn(); return; }
    if (v === "usage") { call("accountForm", null); show({ src: PLAN_USAGE, title: "chatgpt.com" }); return; }
    if (v.indexOf("key:") === 0 && AIS[p] && !AIS[p].keyless) { keyForm(p, ""); return; }
    if (v.indexOf("open:") === 0 && AIS[p] && AIS[p].keys) { newTab(AIS[p].keys); return; }
    if (v.indexOf("model:") === 0 && own) {
      var o = {};
      for (var k in own) o[k] = own[k];
      withModel(o, v.slice(6));
      ownSet(o, ownName() + " answers with " + o.name);
      ownMenu();
      return;
    }
    if (v === "out" && own) {
      var was = ownName();
      call("accountForm", null);
      if (own.provider === "chatgpt") APP.app("chatgpt_sign_out").catch(noop);
      ownSet(null, "Signed out of " + was);
    }
  }
  // A key asked for (or said wrong): in the same panel. (Another AI's, signed in: changed there, signed out first.)
  function keyForm(p, error) {
    if (own && p === "other") { var was = own; own = null; ownMenu(p, error, "ai:menu"); own = was; return; }
    ownMenu(p, error);
  }
  // (A key refused, in an answer's own words: Google's says so with a 400.)
  function keyRefused(why) { return /api.?key|unauthori[sz]ed|authenticat|credential/i.test(String(why || "")); }
  function apiError(t) {
    try {
      var j = JSON.parse(t), e = j.error || j;
      return String((typeof e === "string" ? e : e.message || e.detail || j.message || j.detail) || "").slice(0, 200);
    } catch (x) { return String(t || "").slice(0, 200); }
  }
  // How each AI takes the key: as a bearer token; Anthropic's with its API's version, and told that this page asks
  // it itself; OpenRouter's with this page's name.
  function ownHeaders(p, key) {
    var h = { authorization: "Bearer " + key };
    if (p === "anthropic") { h["anthropic-version"] = "2023-06-01"; h["anthropic-dangerous-direct-browser-access"] = "true"; }
    if (p === "openrouter") { h["HTTP-Referer"] = "https://chatcomputer.ai"; h["X-Title"] = "Chat Computer"; }
    return h;
  }
  // Which of two model names is the newer, by the numbers in them (5.5 after 5.1, and a date after none).
  function newer(a, b) {
    var x = String(a).match(/\d+/g) || [], y = String(b).match(/\d+/g) || [];
    for (var i = 0; i < Math.max(x.length, y.length); i++) {
      var d = (+y[i] || 0) - (+x[i] || 0);
      if (d) return d;
    }
    return 0;
  }
  // An AI's models that chat (and, where it says, use tools), its best first, then the newest.
  function bestFirst(p, list) {
    var a = AIS[p] || {}, best = a.best || [];
    function place(id) { for (var i = 0; i < best.length; i++) if (best[i].test(id)) return i; return best.length; }
    return list.filter(function (m) {
      var c = m && m.capabilities;
      return !(c && (c.completion_chat === false || c.function_calling === false));
    }).map(function (m) {
      var id = String(m && (m.id || m.model || m.name) || "").replace(/^models\//, "");
      var t = m ? +m.created || (Date.parse(m.created_at || "") / 1000) || 0 : 0;
      return { id: id, t: t };
    }).filter(function (m) { return m.id && !NOT_CHAT.test(m.id) && (!a.chat || a.chat.test(m.id)) && !(a.skip && a.skip.test(m.id)); })
      .sort(function (x, y) { return place(x.id) - place(y.id) || newer(x.id, y.id) || y.t - x.t; })
      .map(function (m) { return m.id; })
      .filter(function (id, i, all) { return all.indexOf(id) === i; });
  }
  // OpenRouter's: its free models that use tools (the newest first), and the best of each maker above that uses them
  // (on your OpenRouter credits).
  function openRouterModels(list) {
    var tools = list.filter(function (m) { return m && typeof m.id === "string" && (m.supported_parameters || []).indexOf("tools") >= 0; });
    var free = tools.filter(function (m) { return m.pricing && +m.pricing.prompt === 0 && +m.pricing.completion === 0; })
      .sort(function (a, b) { return (b.created || 0) - (a.created || 0); }).map(function (m) { return m.id; });
    var best = [];
    MAKERS.forEach(function (p) {
      var maker = AIS[p].or;
      var mine = tools.filter(function (m) { return m.id.indexOf(maker) === 0 && m.id.indexOf(":") < 0; })
        .map(function (m) { return { id: m.id.slice(maker.length), created: m.created }; });
      var top = bestFirst(p, mine)[0];
      if (top) best.push({ id: maker + top, maker: p });
    });
    return { free: free, best: best };
  }
  // The models that AI offers, best first (OpenRouter's: its free ones that use tools, the newest first). Where it
  // doesn't list them (its list isn't there, or isn't what was expected), its best model; a key it doesn't take, said.
  function ownModels(p, base, key, all) {
    var a = AIS[p] || {};
    return fetch(base + "/models" + (p === "anthropic" ? "?limit=100" : ""), { headers: ownHeaders(p, key) }).then(function (r) {
      if (!r.ok) {
        return r.text().then(function (t) {
          var bad = r.status === 401 || r.status === 403 || keyRefused(apiError(t));
          if (a.model && !bad) return null;
          var e = new Error(apiError(t) || "it said " + r.status);
          e.status = bad ? 401 : r.status;
          throw e;
        });
      }
      return r.json().catch(function () { return null; });
    }).then(function (j) {
      var list = (j && (j.data || j.models || (j.output && j.output.models))) || [];
      list.forEach(function (m) { noteModel(p, m); });
      if (p === "openrouter") { var or = openRouterModels(list); return all ? or : or.free; }
      if (p === "other") return list.map(function (m) { return String(m && m.id || ""); }).filter(Boolean);
      var got = bestFirst(p, list);
      return got.length || !a.model ? got : [a.model];
    });
  }
  // Signing in with a key: its models listed with it (at its other site's address too, where it has one, if the
  // first says no), the best chosen.
  function ownKey(p, v) {
    var key = String(v.key || "").trim(), other = p === "other", a = AIS[p];
    var base = other ? String(v.base || "").trim().replace(/\/+$/, "") : a.base, model = other ? String(v.model || "").trim() : "";
    if (!key) { keyForm(p, "Paste the key first."); return; }
    if (other && !OTHER_BASE.test(base)) { keyForm(p, "Its address starts with https (or, for one on this computer, with http and localhost)."); return; }
    if (other && !model) { keyForm(p, "Say which of its models to use."); return; }
    var bases = other ? [base] : [a.base].concat(a.alt || []);
    function at(i) {
      return (other ? Promise.resolve([model]) : ownModels(p, bases[i], key, true)).then(function (got) {
        return { base: bases[i], models: p === "openrouter" ? got.free : got, all: got };
      },
        function (e) { if (e && (e.status === 401 || e.status === 403) && i + 1 < bases.length) return at(i + 1); throw e; });
    }
    at(0).then(function (got) {
      if (!got.models.length) throw new Error(p === "openrouter" ? "it has no free model that uses tools just now" : "it lists no model to chat with");
      var o = keepModels(withModel({ provider: p, key: key }, got.models[0]), got.all);
      if (other || got.base !== a.base) o.base = got.base;
      call("accountForm", null);
      ownSet(o, "Signed in with " + (other ? hostOf(base) : a.name) + " (" + o.name + ")");
    }).catch(function (e) {
      keyForm(p, e && (e.status === 401 || e.status === 403) ? (other ? "It" : a.by || a.name) + " didn't take that key." :
        "That didn't work: " + clip(e && e.message || "no answer", 160) + ".");
    });
  }
  // ChatGPT, on your plan (in the app): signing in (OpenAI's page, in your browser; the app waits for it to come
  // back), the models your plan has, and an answer from OpenAI's Responses API, asked by the app for this page and
  // streamed back through a channel: its words, and the page's tools it calls, as the other AIs give them.
  function chatgptSignIn() {
    call("accountForm", { id: "own-ai", title: "Continue with ChatGPT", ok: "", back: "ai:menu", text: [
      "Sign in on OpenAI's page, which opens in your browser, and let Chat Computer use your ChatGPT plan. Then come back here.",
      "Chat Computer keeps the sign-in on this computer, and asks ChatGPT for you; OpenAI sees what you say to it."] });
    APP.app("chatgpt_sign_in").then(function (st) {
      return chatgptModels().then(function (models) {
        if (!models.length) throw new Error("your plan lists no model Chat Computer can use");
        var o = keepModels(withModel({ provider: "chatgpt", key: "app", email: st && st.email || "" }, models[0]), models);
        ownSet(o, "Signed in with ChatGPT (" + o.name + ")");
        // (OpenAI's guidelines: the first time, say whose plan answers.)
        if (!store(PLAN_TOLD)) {
          store(PLAN_TOLD, true);
          call("accountForm", { id: "own-ai-plan", title: "You're using your ChatGPT plan", ok: "Got it", text: [
            "ChatGPT answers in Chat Computer now, on your ChatGPT plan: what it does here counts towards your plan's usage, " +
            "as in ChatGPT. Manage usage (the spark, beside +) shows it."] });
        } else call("accountForm", null);
      });
    }).catch(function (e) {
      call("accountForm", { id: "own-ai-failed", title: "ChatGPT", ok: "OK", back: "ai:menu", text: [clip(String(e && e.message || e || "It didn't work."), 300)] });
    });
  }
  // Asks the app to ask ChatGPT: what comes back, whole (its status and text), or piece by piece to `piece`.
  function chatgptAsk(path, body, piece) {
    var text = "";
    var ch = APP.channel(function (t) { text += t; if (piece) piece(t); });
    return APP.app("chatgpt_ask", { path: path, body: body == null ? null : JSON.stringify(body), events: ch }).then(function (status) {
      return { status: +status || 0, text: text };
    });
  }
  function chatgptModels() {
    return chatgptAsk("models", null).then(function (r) {
      if (r.status !== 200) throw new Error(apiError(r.text) || "ChatGPT said " + r.status);
      var j = JSON.parse(r.text || "{}"), list = j.models || j.data || [];
      list.forEach(function (m) { if (m) noteModel("chatgpt", m); });
      return list.filter(function (m) { return m && (m.visibility == null || m.visibility === "list"); })
        .map(function (m) { return String(m.slug || m.id || ""); }).filter(Boolean);
    });
  }
  // The conversation as the Responses API takes it: what it's told (instructions), then each message, each tool
  // call and each tool's answer, in order; the page's tools as functions. Not kept by OpenAI (store: false).
  function responsesBody(messages, tools) {
    var input = [], told = [];
    messages.forEach(function (m) {
      if (m.role === "system") { told.push(String(m.content || "")); return; }
      if (m.role === "user") { input.push({ role: "user", content: [{ type: "input_text", text: String(m.content || "") }] }); return; }
      if (m.role === "assistant") {
        if (m.content) input.push({ role: "assistant", content: [{ type: "output_text", text: String(m.content) }] });
        (m.tool_calls || []).forEach(function (c) {
          input.push({ type: "function_call", call_id: c.id, name: c.function.name, arguments: c.function.arguments || "{}" });
        });
        return;
      }
      if (m.role === "tool") input.push({ type: "function_call_output", call_id: m.tool_call_id, output: String(m.content || "") });
    });
    var body = { model: own.model, instructions: told.join("\n\n"), input: input, stream: true, store: false,
                 tools: tools.map(function (t) { return { type: "function", name: t.function.name, description: t.function.description,
                                                         parameters: t.function.parameters }; }) };
    if (own.effort) body.reasoning = { effort: own.effort };
    return body;
  }
  // One answer from ChatGPT: its words to `onText` as they come, and the whole message (words and tool calls), in
  // the same shape as the other AIs' (localMessage).
  function chatgptMessage(messages, tools, onText) {
    var buf = "", read = responsesReader(onText);
    function event(chunk) {
      var data = chunk.split("\n").filter(function (l) { return l.indexOf("data:") === 0; })
        .map(function (l) { return l.slice(5).trim(); }).join("");
      if (data && data !== "[DONE]") read.event(data);
    }
    return chatgptAsk("responses", responsesBody(messages, tools), function (t) {
      buf += t.replace(/\r\n/g, "\n");
      for (var k = buf.indexOf("\n\n"); k >= 0; k = buf.indexOf("\n\n")) { event(buf.slice(0, k)); buf = buf.slice(k + 2); }
    }).then(function (r) {
      if (r.status !== 200) {
        var e = new Error(apiError(r.text) || "ChatGPT said " + r.status);
        e.status = r.status;
        throw e;
      }
      if (buf.trim()) event(buf);
      return read.result();
    }, function (e) { var x = new Error(e && e.message || String(e || "no answer")); x.status = 0; throw x; });
  }
  // OpenRouter's own sign-in (OAuth with PKCE): its page, then back here with a code, which this page trades for a key.
  function b64url(bytes) {
    var s = "";
    for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
    return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  }
  function openRouterSignIn() {
    var v = new Uint8Array(32);
    crypto.getRandomValues(v);
    var verifier = b64url(v);
    crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier)).then(function (h) {
      try { sessionStorage.setItem(OR_VERIFIER, verifier); } catch (e) {}
      var back = location.href.split("#")[0].split("?")[0];
      location.assign("https://openrouter.ai/auth?callback_url=" + encodeURIComponent(back) + "&code_challenge=" +
                      b64url(new Uint8Array(h)) + "&code_challenge_method=S256");
    });
  }
  function openRouterBack() {
    var q, code, verifier = null;
    try { q = new URLSearchParams(location.search); code = q.get("code"); } catch (e) { return; }
    try { verifier = sessionStorage.getItem(OR_VERIFIER); sessionStorage.removeItem(OR_VERIFIER); } catch (e) {}
    if (!code || !verifier) return;
    q.delete("code");
    try { history.replaceState(null, "", location.pathname + (q.toString() ? "?" + q : "") + location.hash); } catch (e) {}
    fetch("https://openrouter.ai/api/v1/auth/keys", { method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ code: code, code_verifier: verifier, code_challenge_method: "S256" }) }).then(function (r) {
      if (!r.ok) return r.text().then(function (t) { throw new Error(apiError(t) || "it said " + r.status); });
      return r.json();
    }).then(function (j) {
      var key = j && j.key;
      if (typeof key !== "string" || !key) throw new Error("no key came back");
      return ownModels("openrouter", AIS.openrouter.base, key, true).then(function (got) {
        if (!got.free.length) throw new Error("it has no free model that uses tools just now");
        var o = keepModels(withModel({ provider: "openrouter", key: key }, got.free[0]), got);
        ownSet(o, "Signed in with OpenRouter (" + o.name + ")");
      });
    }).catch(function (e) {
      call("accountForm", { id: "own-ai-failed", title: "OpenRouter", ok: "OK", back: "ai:menu", text: ["Signing in with OpenRouter didn't work: " +
        clip(e && e.message || "no answer", 160) + "."] });
    });
  }

  // ------------------------------------------------------------------ AirPods: talking
  // On Chat Computer you talk through your AirPods and it answers in them. Here, "Connect AirPods": the browser
  // listens through them (or the device's own microphone, when they aren't the one it uses), turns what you say into
  // text, and the answers are read out. A website can't pair AirPods: the phone or computer does, in its Bluetooth
  // settings.
  // On a phone too: on an iPhone or iPad, Safari turns speech into text (with Siri or Dictation switched on; the other
  // browsers there are Safari underneath, but may not let a page listen). Safari's listening, and Android's, don't do well
  // listening on and on (Safari can listen without ever saying what it heard, or hear it twice; Android repeats what
  // was said), so there it listens for one thing at a time, again after each, and what it heard counts as said once it
  // stops changing, whether or not the browser calls it final. An iPhone speaks for a page only after speaking in
  // answer to a tap, so the tap that starts talking says nothing, silently, for the answers later; and while talking
  // the screen stays on (a phone that locks itself stops listening).
  var Recognition = window.SpeechRecognition || window.webkitSpeechRecognition;
  var UA = navigator.userAgent || "";
  var IOS = /iP(hone|ad|od)/.test(UA) || (/Macintosh/.test(UA) && navigator.maxTouchPoints > 1);
  var SAFARI = IOS ? !/CriOS|FxiOS|EdgiOS|OPiOS|GSA\//.test(UA) : /Safari\//.test(UA) && !/Chrome|Chromium|Edg|OPR|Android/.test(UA);
  var ONE_AT_A_TIME = IOS || SAFARI || /Android/.test(UA);
  var QUIET_MS = 1400, DEAD_MS = 20000;
  var ears = { on: false, starting: false, rec: null, speaking: false, device: "", told: false, lock: null, said: null, waiting: 0 };
  // The microphone button starts listening at once (the browser asks to use the microphone the first time); "/"'s
  // Connect AirPods says first, once, how AirPods are paired (explain).
  function talkButton(explain) {
    if (ears.on || ears.starting) { stopTalking(); return; }
    if (!Recognition || !navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
      call("accountForm", { id: "talk-no", title: "Talking", ok: "OK", text: [IOS ?
        "On an iPhone or iPad, Safari turns speech into text: open chatcomputer.ai in Safari." :
        "This browser can't turn speech into text. Chrome, Edge and Safari can."] });
      return;
    }
    if (ears.told || !explain) { startTalking(); return; }
    call("accountForm", { id: "talk", title: "Talk through your AirPods", ok: "Start talking", text: [
      "Pair your AirPods with this phone or computer first, in its Bluetooth settings: a website can't pair them.",
      "Then talk: what you say goes in the message box's place, and the answers are read out. To stop, type / and choose Stop talking.",
      "Your browser turns what you say into text (Chrome does it on Google's servers). It asks to use the microphone."] });
  }
  function startTalking() {
    ears.told = true;
    ears.starting = true;
    try { var hush = new SpeechSynthesisUtterance(" "); hush.volume = 0; window.speechSynthesis.speak(hush); } catch (x) {}
    keepAwake(true);
    navigator.mediaDevices.getUserMedia({ audio: true }).then(function (stream) {
      var track = stream.getAudioTracks()[0], label = track && track.label || "";
      if (!ears.starting) { stream.getTracks().forEach(function (t) { t.stop(); }); return; }   // (stopped meanwhile)
      // On a computer, kept open to show how loud it is (the button's bar); on a phone, where a page holding the
      // microphone can stop its own listening, let go of (the bar follows the listening instead).
      if (ONE_AT_A_TIME) { stream.getTracks().forEach(function (t) { t.stop(); }); meterStart(null); } else meterStart(stream);
      ears.starting = false;
      ears.device = /airpods/i.test(label) ? label : "";
      ears.on = true;
      keepAwake(true);
      status();
      did(ears.device ? "Connected your AirPods" : "Started listening (" + (clip(label, 40) || "the microphone") + ")");
      hear();
    }, function (err) {
      ears.starting = false;
      keepAwake(false);
      call("accountForm", { id: "talk-no", title: "The microphone is off", ok: "OK", text: [
        "Your browser didn't let this page use the microphone (" + (err && err.name || "refused") + ").",
        "Allow it for chatcomputer.ai in the browser's site settings, then press the microphone again."] });
    });
  }
  // Safari can hear a thing said twice over ("what time is it what time is it"): once. (Only something longer than
  // a word or two: "bye bye" stays.)
  function once(t) {
    t = String(t || "").replace(/\s+/g, " ").trim();
    if (!SAFARI) return t;
    var low = t.toLowerCase(), n = t.length, h = Math.floor(n / 2);
    if (n % 2 && h >= 6 && low.charAt(h) === " " && low.slice(0, h) === low.slice(h + 1)) return t.slice(0, h);
    if (!(n % 2) && h >= 6 && low.slice(0, h) === low.slice(h)) return t.slice(0, h);
    return t;
  }
  function hear() {
    if (!ears.on || ears.speaking || ears.rec || ears.waiting || document.hidden) return;
    var r = new Recognition(), said = "", sent = false, quiet = 0, alive = Date.now();
    r.lang = navigator.language || "en-US";
    r.continuous = !ONE_AT_A_TIME;
    r.interimResults = true;
    // What was heard, as said: asked once, and this listening ended. The next starts once the answer has been read
    // out, so a phone doesn't chime twice for each thing said (or after a minute, should no answer come).
    function heard(text) {
      clearTimeout(quiet);
      text = once(text);
      if (sent || !text) return;
      sent = true;
      call("partial", "");
      clearTimeout(ears.waiting);
      ears.waiting = setTimeout(function () { ears.waiting = 0; hear(); }, 60000);
      var at = address(text);
      if (at) visit(text, at); else ask(text);
      try { r.stop(); } catch (x) {}
    }
    r.onstart = r.onaudiostart = function () { alive = Date.now(); };
    r.onspeechstart = function () { alive = Date.now(); bump(0.6); };
    r.onresult = function (e) {
      if (ears.rec !== r) return;
      alive = Date.now();
      bump(0.9);
      if (!ONE_AT_A_TIME) {           // Chrome and Edge on a computer: listening on, each thing said final in turn
        var partial = "", final = "";
        for (var i = e.resultIndex; i < e.results.length; i++) {
          if (e.results[i].isFinal) final += e.results[i][0].transcript; else partial += e.results[i][0].transcript;
        }
        if (final.trim()) { call("partial", ""); var addr = address(final); if (addr) visit(final.trim(), addr); else ask(final.trim()); }
        else call("partial", partial.trim());
        return;
      }
      if (sent) return;
      var all = "", done = e.results.length > 0;
      for (var j = 0; j < e.results.length; j++) { all += e.results[j][0].transcript; done = done && e.results[j].isFinal; }
      said = all;
      if (done && said.trim()) { heard(said); return; }
      call("partial", once(said));
      clearTimeout(quiet);
      quiet = setTimeout(function () { heard(said); }, QUIET_MS);
    };
    r.onerror = function (e) {
      if (ears.rec !== r) return;
      if (e.error === "not-allowed" || e.error === "service-not-allowed" || e.error === "network") {
        stopTalking();
        call("accountForm", { id: "talk-no", title: "Talking stopped", ok: "OK", text: [e.error === "network" ?
          "The browser couldn't reach its speech service (it needs the internet)." : notListening()] });
      }
    };
    r.onend = function () {
      clearTimeout(quiet);
      clearInterval(watch);
      if (ears.rec !== r) return;                  // one given up on (below), already replaced
      if (ONE_AT_A_TIME && said.trim() && !sent && !ears.speaking) heard(said);
      ears.rec = null;
      call("partial", "");
      if (ears.on && !ears.speaking) setTimeout(hear, 250);
    };
    // Listening that has gone quiet for good (Safari's can, after sound has played): given up on, and started anew.
    var watch = setInterval(function () {
      if (ears.rec !== r) { clearInterval(watch); return; }
      if (ears.speaking || Date.now() - alive < DEAD_MS) return;
      clearInterval(watch);
      clearTimeout(quiet);
      try { r.abort(); } catch (x) {}
      ears.rec = null;
      call("partial", "");
      setTimeout(hear, 250);
    }, 5000);
    ears.rec = r;
    try { r.start(); } catch (x) { ears.rec = null; clearInterval(watch); }
  }
  function notListening() {
    if (IOS && !SAFARI) return "This browser didn't let the page listen. On an iPhone or iPad, Safari can: open chatcomputer.ai in Safari.";
    if (SAFARI) return "Safari stopped this page from listening. Allow the microphone for chatcomputer.ai, and switch on Siri or " +
      "Dictation in Settings: Safari needs one of them to turn speech into text.";
    return "The browser stopped this page from listening. Allow the microphone for chatcomputer.ai in its site settings.";
  }
  function stopTalking() {
    ears.on = false;
    ears.starting = false;
    meterStop();
    clearTimeout(ears.waiting);
    ears.waiting = 0;
    if (ears.rec) { try { ears.rec.abort(); } catch (x) {} ears.rec = null; }
    voiceStop();
    ears.speaking = false;
    ears.said = null;
    keepAwake(false);
    call("partial", "");
    status();
  }
  // While listening, how loud it is: the microphone button's one bar. On a computer, measured from the microphone
  // (in this page only: nothing of it goes anywhere); on a phone, from the listening itself: up as speech is heard,
  // and as each word of it comes in, down again when it stops. Up at once, down gently; none while an answer's read
  // out (not listening then).
  var meter = { stream: null, ctx: null, raf: 0, level: 0, shown: -1 };
  function meterStart(stream) {
    meterStop();
    var measure = null, AC = window.AudioContext || window.webkitAudioContext;
    if (stream && AC) {
      try {
        var ctx = new AC(), an = ctx.createAnalyser(), buf = new Float32Array(1024);
        an.fftSize = 1024;
        ctx.createMediaStreamSource(stream).connect(an);
        meter.ctx = ctx;
        measure = function () {
          an.getFloatTimeDomainData(buf);
          var sum = 0;
          for (var i = 0; i < buf.length; i++) sum += buf[i] * buf[i];
          // Quiet room about -60 dB, speaking about -30 to -10 dB: -55 to -15 dB fills the bar.
          var db = 20 * Math.log10(Math.sqrt(sum / buf.length) + 1e-9);
          return Math.max(0, Math.min(1, (db + 55) / 40));
        };
      } catch (x) { measure = null; }
    }
    meter.stream = stream;
    (function tick() {
      meter.raf = requestAnimationFrame(tick);
      var x = ears.speaking ? 0 : measure ? measure() : 0;
      meter.level = x > meter.level ? x : meter.level * 0.9 + x * 0.1;
      if (Math.abs(meter.level - meter.shown) < 0.01) return;
      meter.shown = meter.level;
      call("level", meter.level);
    })();
  }
  function bump(x) { if (!meter.ctx && !ears.speaking) meter.level = Math.max(meter.level, x); }
  function meterStop() {
    cancelAnimationFrame(meter.raf);
    meter.raf = 0;
    if (meter.stream) meter.stream.getTracks().forEach(function (t) { t.stop(); });
    if (meter.ctx) { try { meter.ctx.close(); } catch (x) {} }
    meter.stream = meter.ctx = null;
    meter.level = 0;
    meter.shown = -1;
  }
  // While talking, the screen stays on (where the browser can keep it on: most can).
  function keepAwake(on) {
    if (!on) {
      if (ears.lock) { try { ears.lock.release(); } catch (x) {} ears.lock = null; }
      return;
    }
    if (ears.lock || !navigator.wakeLock || document.visibilityState !== "visible") return;
    navigator.wakeLock.request("screen").then(function (lock) {
      if (!ears.on && !ears.starting) { lock.release(); return; }
      ears.lock = lock;
      lock.addEventListener("release", function () { if (ears.lock === lock) ears.lock = null; });
    }, function () {});
  }
  // Away from the page (another app, the phone locked): not listening; back: listening again.
  document.addEventListener("visibilitychange", function () {
    if (!ears.on) return;
    if (document.hidden) { if (ears.rec) { try { ears.rec.abort(); } catch (x) {} } }
    else { keepAwake(true); hear(); }
  });
  // ------------------------------------------------------------------ the voice
  // Read aloud (under an answer, or in a held message's menu), and answers said while talking: in Heart, a warm,
  // natural voice (Kokoro's af_heart: the voice Chat Computer itself speaks with), made on this device by its graphics
  // chip (WebGPU) once the visitor has downloaded it; else in the best voice the browser has: its natural, neural or
  // premium voices first (Edge's Ava, Android's neural voices), then Google's, then the usual one, never a novelty
  // voice. Heart is offered the first time Read aloud is pressed where it can run (and in the "/" list): a phone's
  // processor alone makes it slower than it speaks, so where there's no WebGPU it isn't offered. (Safari lets pages
  // use only an iPhone's built-in voices, which sound robotic: Heart is the warm voice there.)
  var HEART = { name: "Heart", library: "https://cdn.jsdelivr.net/npm/kokoro-js@1.2.1/dist/kokoro.web.js",
                model: "onnx-community/Kokoro-82M-v1.0-ONNX", voice: "af_heart", size: "330 MB" };
  if (APP) HEART.library = LM_HERE + "kokoro.web.js";       // (in the app, its code comes with it)
  var HEART_PARTS = "chatcomputer-heart-parts", HEART_NO = "chatcomputer-demo-heart-no", HEART_STARTING = "chatcomputer-demo-heart-starting";
  // state: none (it can't run here), downloadable, downloading, starting, ready (downloaded), available (started).
  var heart = { state: "none", cached: false, tts: null, loading: null, progress: 0, ctx: null, turn: 0, playing: null,
                wanted: null, offered: null, stopped: false };
  function heartFile() { return "https://huggingface.co/" + HEART.model + "/resolve/main/onnx/model.onnx"; }
  function heartKept() {
    return caches.open("transformers-cache").then(function (c) { return c.match(heartFile()); })
      .then(function (r) { return !!r; }, function () { return false; });
  }
  // Heart's big file comes in pieces of 16 MB (Range requests, which Hugging Face's servers allow from any page), each
  // kept as it arrives (the browser's store, under `parts`): a phone that leaves the page, locks itself or loses its
  // signal loses only the piece it was on, and the download goes on from there. A piece that stops coming for 20
  // seconds, or that was cut off while the page was out of sight, is asked for again; an offline phone waits to be
  // online. Once all are here they're put together in Transformers.js' own store, under the file's address, where it
  // finds it, and the pieces are thrown away. The screen is kept on meanwhile where the browser can.
  // report(bytes so far, the file's size once known). "unranged": the server sent the whole file instead (then
  // Transformers.js downloads it, as before).
  var PIECE = 16 * 1024 * 1024, fetching = [];
  function pieceFetch(url, parts, report) {
    var wake = null, mine = null;
    try { if (navigator.wakeLock) navigator.wakeLock.request("screen").then(function (l) { wake = l; }, noop); } catch (e) {}
    function done() { try { if (wake) wake.release(); } catch (e) {} fetching = fetching.filter(function (f) { return f !== mine; }); }
    return Promise.all([caches.open(parts), caches.open("transformers-cache")]).then(function (cs) {
      var pieces = cs[0], kept = cs[1], total = 0, have = {};
      function key(i) { return url + "?piece=" + i; }        // (not #: the store ignores what follows a #)
      function shown(now) {
        var got = now || 0;
        for (var i in have) got += have[i];
        report(got, total);
      }
      // One piece: asked for again when it stalls, up to 8 times.
      function piece(i, tries) {
        var from = i * PIECE, to = total ? Math.min(total, from + PIECE) - 1 : from + PIECE - 1;
        var stop = new AbortController(), last = Date.now(), timer = 0, size = 0, chunks = [], want = 0;
        fetching = fetching.filter(function (f) { return f !== mine; });
        fetching.push(mine = { last: function () { return last; }, abort: function () { stop.abort(); } });
        timer = setInterval(function () { if (Date.now() - last > 20000) stop.abort(); }, 2000);
        return fetch(url, { headers: { Range: "bytes=" + from + "-" + to }, signal: stop.signal, cache: "no-store" }).then(function (res) {
          if (res.status === 200) { stop.abort(); throw { unranged: true }; }
          if (res.status !== 206 || !res.body) throw new Error("the server said " + res.status);
          var range = /bytes (\d+)-(\d+)\/(\d+)/.exec(res.headers.get("Content-Range") || "");
          if (!range) throw new Error("the server didn't say which part it sent");
          want = +range[2] - +range[1] + 1;
          if (!total) { total = +range[3]; pieces.put(url + "?size", new Response(String(total))).catch(noop); }
          var reader = res.body.getReader();
          return (function read() {
            return reader.read().then(function (r) {
              if (r.done) return;
              chunks.push(r.value);
              size += r.value.byteLength;
              last = Date.now();
              shown(size);
              return read();
            });
          })();
        }).then(function () {
          clearInterval(timer);
          if (size !== want) throw new Error("a piece came short");
          return pieces.put(key(i), new Response(new Blob(chunks))).then(function () { have[i] = size; shown(0); });
        }, function (e) {
          clearInterval(timer);
          if (e && e.unranged) throw e;
          if ((tries || 0) >= 8) throw new Error("the download kept stopping (" + (e && e.message || e) + ")");
          return (navigator.onLine === false ? new Promise(function (go) { window.addEventListener("online", go, { once: true }); })
            : new Promise(function (go) { setTimeout(go, Math.min(30000, 1000 * Math.pow(2, tries || 0))); }))
            .then(function () { return piece(i, (tries || 0) + 1); });
        });
      }
      return kept.match(url).then(function (already) {
        if (already) return "kept";
        return pieces.match(url + "?size").then(function (r) { return r ? r.text() : ""; }).then(function (t) {
          total = +t || 0;
          return pieces.keys();
        }).then(function (keys) {
          return Promise.all(keys.map(function (r) {
            var m = /\?piece=(\d+)$/.exec(r.url);
            if (!m || r.url.indexOf(url + "?") !== 0) return null;
            return pieces.match(r).then(function (res) { return res.blob(); }).then(function (b) { have[+m[1]] = b.size; });
          }));
        }).then(function () {
          shown(0);
          var i = 0;
          return (function next() {
            if (total && i * PIECE >= total) return null;
            if (have[i] != null) { i++; return next(); }
            return piece(i).then(function () { i++; return next(); });
          })();
        }).then(function () {
          var n = Math.ceil(total / PIECE), blobs = [];
          return (function each(i) {
            if (i >= n) return null;
            return pieces.match(key(i)).then(function (r) {
              if (!r) throw new Error("a piece of the download went missing");
              return r.blob();
            }).then(function (b) { blobs.push(b); return each(i + 1); });
          })(0).then(function () {
            var whole = new Blob(blobs, { type: "application/octet-stream" });
            if (whole.size !== total) throw new Error("the download came out the wrong size");
            return kept.put(url, new Response(whole, { headers: { "Content-Type": "application/octet-stream", "Content-Length": String(total) } }));
          }).then(function () { return caches.delete(parts); }).then(function () { return "fetched"; });
        });
      });
    }).then(function (how) { done(); return how; }, function (e) {
      done();
      if (e && e.unranged) return "unranged";
      throw e;
    });
  }
  // Back on the page after a while away: a piece that was being downloaded is asked for again at once.
  document.addEventListener("visibilitychange", function () {
    if (document.visibilityState !== "visible") return;
    fetching.forEach(function (f) { if (Date.now() - f.last() > 4000) f.abort(); });
  });
  function heartCheck() {
    var can = navigator.gpu && navigator.gpu.requestAdapter && window.caches && (window.AudioContext || window.webkitAudioContext);
    return (can ? navigator.gpu.requestAdapter() : Promise.resolve(null)).then(function (a) {
      if (!a) return;
      return heartKept().then(function (kept) {
        heart.cached = kept;
        heart.stopped = !!store(HEART_STARTING);              // (the page stopped while it was starting: not by itself)
        store(HEART_STARTING, null);
        heart.state = kept && !heart.stopped ? "ready" : "downloadable";
        status();
      });
    }).catch(noop);
  }
  // Its sheet: what it is, and its size, before anything's downloaded. Pressing Read aloud brought it up: closed, the
  // browser's voice reads (and Heart isn't offered that way again: the "/" list has it).
  function heartButton(then) {
    if (heart.state !== "downloadable") return;
    heart.offered = then || null;
    heart.sheet = true;
    call("accountForm", { id: "heart", title: "A warmer voice", ok: heart.cached ? "Start it" : "Download it", text: [
      "Read aloud can use Heart, a warm, natural voice: the one Chat Computer itself speaks with. It runs on this " +
      deviceWord() + ", for free.",
      heart.stopped ? "Last time, this page stopped while Heart was starting: this " + deviceWord() + " may not have enough memory " +
        "free for it. Closing other apps can help." :
      heart.cached ? "This browser has it already." :
      "It's about " + HEART.size + ", downloaded once from Hugging Face and kept by this browser (Wi-Fi is best).",
      then ? "Close this to hear this " + deviceWord() + "'s own voice instead." : "Until then, this " + deviceWord() + "'s own voice reads."] });
  }
  function heartRemoveButton() {
    call("accountForm", { id: "heart-remove", title: "Remove Heart", ok: "Remove it", text: [
      "This frees about " + HEART.size + " on this " + deviceWord() + ", and Read aloud uses this " + deviceWord() + "'s own voice " +
      "again. You can download Heart again any time: type / and choose Download Heart."] });
  }
  // Downloaded the first time (its progress in the empty box, and in the list), from this browser's copy after.
  function heartStart() {
    if (heart.tts) return Promise.resolve(heart.tts);
    if (heart.loading) return heart.loading;
    var download = !heart.cached;
    heart.state = download ? "downloading" : "starting";
    heart.progress = 0;
    status();
    if (download && navigator.storage && navigator.storage.persist) navigator.storage.persist().catch(noop);
    heart.loading = (download ? pieceFetch(heartFile(), HEART_PARTS, function (got, total) {
      var pct = Math.min(99, Math.floor(got / (total || 326e6) * 100));
      if (pct !== heart.progress) { heart.progress = pct; status(); }
    }) : Promise.resolve("kept")).then(function () {
      heart.state = "starting";
      status();
      store(HEART_STARTING, true);                            // (being put in memory: if the page stops now, not by itself next time)
      return import(HEART.library);
    }).then(function (k) {
      if (LM_HERE && k.env) k.env.wasmPaths = LM_HERE;
      return k.KokoroTTS.from_pretrained(HEART.model, { dtype: "fp32", device: "webgpu" });
    }).then(function (tts) {
      heart.tts = tts;
      heart.loading = null;
      heart.cached = true;
      heart.stopped = false;
      heart.state = "available";
      store(HEART_STARTING, null);
      status();
      if (download) did("Turned on the Heart voice, on this " + deviceWord());
      return tts;
    }, function (e) {
      heart.loading = null;
      store(HEART_STARTING, null);
      return heartKept().then(function (kept) {
        heart.cached = kept;
        heart.state = "downloadable";
        status();
        call("accountForm", { id: "heart-no", title: "Heart didn't start", ok: "OK", text: [
          "The browser said: " + clip(e && e.message || "it couldn't", 160) + ".",
          "Read aloud uses this " + deviceWord() + "'s own voice meanwhile."] });
        throw e;
      });
    });
    return heart.loading;
  }
  function heartRemove() {
    voiceStop();
    var tts = heart.tts;
    heart.tts = null;
    try { if (tts && tts.model && tts.model.dispose) tts.model.dispose().catch(noop); } catch (e) {}
    return caches.open("transformers-cache").then(function (c) {
      return c.keys().then(function (keys) {
        return Promise.all(keys.filter(function (r) { return r.url.indexOf(HEART.model) >= 0; }).map(function (r) { return c.delete(r); }));
      });
    }).then(function () { return caches.delete(HEART_PARTS); }).catch(noop).then(function () {
      heart.cached = false;
      heart.state = "downloadable";
      status();
      did("Removed the Heart voice from this " + deviceWord());
    });
  }
  // Heart's sound plays through the page's own audio (Web Audio), which a browser only lets start with a tap: so a
  // tap on Read aloud, or on the sheet's button, starts it, there and then (iPhones: as media, heard with the ringer
  // switch on silent, as the browser's own voice is).
  function heartAudio() {
    var C = window.AudioContext || window.webkitAudioContext;
    if (!C) return null;
    if (!heart.ctx) {
      try { if (navigator.audioSession) navigator.audioSession.type = "playback"; } catch (e) {}
      heart.ctx = new C();
      var hush = heart.ctx.createBufferSource();             // (a moment of nothing, which unlocks it on an iPhone)
      hush.buffer = heart.ctx.createBuffer(1, 1, 22050);
      hush.connect(heart.ctx.destination);
      try { hush.start(0); } catch (e) {}
    }
    if (heart.ctx.state !== "running") heart.ctx.resume().catch(noop);
    return heart.ctx;
  }
  ["pointerup", "touchend", "click", "keydown"].forEach(function (type) {
    document.addEventListener(type, function (e) {
      if (heart.state === "none") return;
      var t = e.target && e.target.closest ? e.target : null, ready = heart.state === "ready" || heart.state === "available";
      // (Read aloud; Heart's sheet; and, once it's here, any sheet's button: Start talking, whose answers it says)
      if (t && (t.closest('[data-act="speak"], [data-do="speak"]') || t.closest("#sheet-ok") && (heart.sheet || ready))) heartAudio();
    }, true);
  });
  // Said in pieces of a sentence or so (Chrome's voices stop partway through one long piece, and Heart makes one
  // while the one before plays).
  function piecesOf(plain) {
    var all = [], cur = "";
    (plain.match(/[^.!?\u2026]+(?:[.!?\u2026]+["\u201d\u2019)\]]*|$)\s*/g) || [plain]).forEach(function (part) {
      while (part.length > 220) {                           // one sentence too long: at a comma, or a space
        var cut = Math.max(part.lastIndexOf(", ", 220), part.lastIndexOf(" ", 220));
        if (cut < 60) cut = 220;
        all.push((cur + part.slice(0, cut + 1)).trim());
        cur = "";
        part = part.slice(cut + 1);
      }
      if (cur && (cur + part).length > 220) { all.push(cur.trim()); cur = ""; }
      cur += part;
    });
    if (cur.trim()) all.push(cur.trim());
    return all;
  }
  // Heart: each piece made in turn and played straight after the one before, the next made while one plays; done once
  // the last has played. Should Heart fail, the browser's voice says the rest.
  function heartSay(plain, done) {
    var ctx = heartAudio();
    if (!ctx) { systemSay(plain, done); return; }
    var my = ++heart.turn, pieces = piecesOf(plain), sources = [], at = 0, over = false;
    function finish() {
      if (over) return;
      over = true;
      if (heart.turn === my) heart.playing = null;
      if (done) done();
    }
    heart.playing = { stop: function () {
      over = true;
      sources.forEach(function (s) { try { s.onended = null; s.stop(); } catch (e) {} });
    } };
    (function next(i) {
      if (over || heart.turn !== my || i >= pieces.length) return;
      heartStart().then(function (tts) { return tts.generate(pieces[i], { voice: HEART.voice }); }).then(function (out) {
        if (over || heart.turn !== my) return;
        var samples = out.audio, buf = ctx.createBuffer(1, samples.length, out.sampling_rate);
        if (buf.copyToChannel) buf.copyToChannel(samples, 0); else buf.getChannelData(0).set(samples);
        var src = ctx.createBufferSource();
        src.buffer = buf;
        src.connect(ctx.destination);
        at = Math.max(at, ctx.currentTime + 0.05);
        src.start(at);
        at += buf.duration;
        sources.push(src);
        if (i === pieces.length - 1) src.onended = finish;
        next(i + 1);
      }, function () {
        if (over || heart.turn !== my) return;
        heart.playing = null;
        over = true;
        systemSay(pieces.slice(i).join(" "), done);
      });
    })(0);
  }
  // The browser's own voice: the best it has for the page's language (see above), at its natural pace.
  var NOVELTY = /^(Albert|Bad News|Bahh|Bells|Boing|Bubbles|Cellos|Good News|Jester|Organ|Superstar|Trinoids|Whisper|Wobble|Zarvox|Eddy|Flo|Grandma|Grandpa|Reed|Rocko|Sandy|Shelley|Fred|Junior|Kathy|Ralph)\b/;
  function bestVoice() {
    var all = [], lang = String(navigator.language || "en-US").toLowerCase(), best = null, top = -1;
    try { all = window.speechSynthesis.getVoices() || []; } catch (e) {}
    all.forEach(function (v) {
      var n = String(v.name || ""), vl = String(v.lang || "").toLowerCase().replace("_", "-"), s = 0;
      if (vl.split("-")[0] !== lang.split("-")[0] || NOVELTY.test(n)) return;
      if (/\b(Natural|Neural)\b/i.test(n)) s += 100;
      if (/\(Premium\)/i.test(n)) s += 90;
      if (/\(Enhanced\)/i.test(n)) s += 70;
      if (/^Google\b/.test(n)) s += 60;
      if (/\b(Ava|Emma|Jenny|Aria|Zoe|Samantha|Allison|Serena|Libby|Sonia|Natasha)\b/.test(n)) s += 8;   // warm, clear voices
      if (/female/i.test(n)) s += 4;
      if (vl === lang) s += 5;
      if (v.default) s += 2;
      if (s > top) { top = s; best = v; }
    });
    return best;
  }
  try { if (window.speechSynthesis) window.speechSynthesis.getVoices(); } catch (e) {}   // (Chrome lists them a moment later)
  function systemSay(plain, done) {
    var synth = window.speechSynthesis;
    if (!synth || !window.SpeechSynthesisUtterance) { if (done) done(); return; }
    var voice = bestVoice(), all = piecesOf(plain);
    all.forEach(function (piece, i) {
      var u = new SpeechSynthesisUtterance(piece);
      try { if (voice) u.voice = voice; } catch (e) {}
      u.lang = voice && voice.lang || navigator.language || "en-US";
      if (i === all.length - 1) u.onend = u.onerror = done;
      synth.speak(u);
    });
  }
  function voiceBusy() {
    var synth = window.speechSynthesis;
    return !!(heart.playing || synth && (synth.speaking || synth.pending));
  }
  function voiceStop() {
    try { if (window.speechSynthesis) window.speechSynthesis.cancel(); } catch (x) {}
    heart.turn++;
    if (heart.playing) { heart.playing.stop(); heart.playing = null; }
  }
  // Says `plain`, in Heart once it's here (started first, from the browser's copy, if need be), else the browser's
  // voice. asked: Read aloud pressed (Heart offered, or awaited while it downloads); not: an answer said while talking.
  function voiceSay(plain, done, asked) {
    if (heart.state === "available" || heart.state === "ready") { heartSay(plain, done); return; }
    if (asked && heart.state === "downloadable" && !heart.stopped && !store(HEART_NO)) {
      heartButton({ text: plain, done: done });
      return;
    }
    if (asked && heart.loading) { heartLater({ text: plain, done: done }); return; }
    systemSay(plain, done);
  }
  // Read once Heart is here (if nothing else was asked for meanwhile, and it wasn't pressed again), or in the
  // browser's voice should it not start.
  function heartLater(w) {
    heart.wanted = w;
    if (!w) return;
    heartStart().then(function () {
      if (heart.wanted === w) { heart.wanted = null; heartSay(w.text, w.done); }
    }, function () {
      if (heart.wanted === w) { heart.wanted = null; systemSay(w.text, w.done); }
    });
  }
  // Read aloud: pressed again while it reads that message (or while Heart comes for it), it stops.
  var reading = "";
  function readAloud(text) {
    var plain = String(text || "").replace(/[*_`#>|~]/g, "").replace(/\s+/g, " ").trim();
    if (!plain) return;
    var busy = voiceBusy(), waiting = heart.wanted && heart.wanted.text === plain;
    voiceStop();
    heart.wanted = null;
    if ((busy || waiting) && reading === plain) { reading = ""; return; }
    reading = plain;
    voiceSay(plain, function () { if (reading === plain) reading = ""; }, true);
  }
  // Heart's sheet answered: started (then it reads what was asked meanwhile), or closed (the browser's voice reads it).
  function heartChosen(ok) {
    var then = heart.offered;
    heart.offered = null;
    heart.sheet = false;
    if (!ok) {
      if (then) { store(HEART_NO, true); systemSay(then.text, then.done); }
      return;
    }
    if (then) heartLater(then);
    else heartStart().catch(noop);
  }
  // An answer, read out while talking (not listening meanwhile, so it doesn't hear itself). Listening again once it's
  // said, or after about as long as it takes to say, where the browser doesn't tell (some don't, for a long one).
  // (Every answer ends here, said or not.)
  function say(text) {
    clearTimeout(ears.waiting);
    ears.waiting = 0;
    if (!ears.on) return;
    var plain = String(text || "").replace(/[*_`#>|~]/g, "").replace(/\s+/g, " ").trim();
    if (!plain || !window.speechSynthesis && heart.state !== "available" && heart.state !== "ready") { hear(); return; }
    ears.speaking = true;
    if (ears.rec) { try { ears.rec.abort(); } catch (x) {} }
    var mine = ears.said = {};
    function after() {
      if (ears.said !== mine) return;
      ears.said = null;
      clearTimeout(guard);
      ears.speaking = false;
      hear();
    }
    var guard = setTimeout(after, 15000 + plain.length * 120);
    voiceSay(plain, after, false);
  }

  // ------------------------------------------------------------------ "/": connecting things
  // Nothing beside the message box (it stays clean): "/" typed in it lists, above it, what can be connected or turned
  // on here, before the files it searches (as "/" does on a Chat Computer): Use this computer's model (the app's, on
  // the website; in the app, its model and how it's going), Connect AirPods (talking), Heart, the warmer voice, and
  // in the app, how you have Chat Computer (the app, your browser...). Words after the "/" narrow them down. Chosen (Enter, a click or a tap), each does what it says; the empty box says
  // meanwhile what's going on (status: listening, the model getting ready, a download's progress).
  function commands() {
    var list = [];
    if (APP) {
      list.push({ command: "model", icon: "i-spark", name: localReady() ? local.label : "Chat Computer's model",
                  where: localReady() ? "Answers now, on this computer" + (local.updating && local.stage !== "failed" ? " (being made again, so it can see pictures)" :
                    local.preparing && !local.updating ? " (" + local.preparing + " is getting ready)" : "") :
                    local.state === "preparing" ? "Getting " + (local.preparing || "it") + " ready: " + localWords() :
                    local.state === "failed" || local.state === "none" ? "It couldn't get ready" : "Starting" });
    } else {
      list.push(localReady() && local.kind === "app" ?
        { command: "local", icon: "i-spark", name: "Stop using this computer's model", where: local.label + " answers now, on this computer" } :
        { command: "local", icon: "i-spark", name: "Use this computer's model", where: "Where the Chat Computer app runs here: its own model, free" });
      if (webCan()) {
        list.push(webOn() ? { command: "web", icon: "i-spark", name: "Stop using my model in this browser", where: local.label + " answers now, in this browser" } :
          local.kind === "web" && local.state === "preparing" ? { command: "web", icon: "i-spark", name: "Getting my model ready", where: localWords() } :
          { command: "web", icon: "i-spark", name: "Run my model in this browser", where: "Chat Computer's own model, free, with no account (" + WEB_SIZE + ", once)" });
        if (local.webKept && !(local.kind === "web" && local.state === "preparing")) {
          list.push({ command: "web-remove", icon: "i-spark", name: "Remove my model from this browser", where: "Frees 0.8 GB on this " + deviceWord() });
        }
      }
    }
    list.push(ears.on ?
      { command: "airpods", icon: "i-pods", name: "Stop talking", where: ears.device ? "Your AirPods are on" : "Listening now" } :
      { command: "airpods", icon: "i-pods", name: "Connect AirPods", where: "Talk through your AirPods, and hear the answers" });
    if (heart.state === "downloadable" || heart.state === "downloading") {
      list.push({ command: "heart", icon: "i-voice", name: heart.state === "downloading" ?
        "Downloading Heart" + (heart.progress ? " (" + heart.progress + "%)" : "") : (heart.cached ? "Start Heart" : "Download Heart"),
        where: "A warmer voice for Read aloud, on this " + deviceWord() + (heart.cached ? "" : " (" + HEART.size + ")") });
    }
    if (heart.state === "ready" || heart.state === "available") {
      list.push({ command: "heart-remove", icon: "i-voice", name: "Remove Heart", where: "Frees " + HEART.size + " on this " + deviceWord() });
    }
    // (In the app: how you have Chat Computer, to change it on the app's page that asks, apps/desktop/start.)
    if (APP) list.push({ command: "way", icon: "i-chat", name: "How you have Chat Computer",
                         where: "As this app now: or as its own system" });
    return list;
  }
  function matching(list, q) {
    var words = q.toLowerCase().split(/\s+/).filter(Boolean);
    return list.filter(function (c) {
      var hay = (c.name + " " + c.where + " " + c.command).toLowerCase();
      return words.every(function (w) { return hay.indexOf(w) >= 0; });
    });
  }
  function slashCommand(id) {
    if (id === "way") location.href = "start/index.html?change";     // (the app's page that asks: apps/desktop/start)
    else if (id === "local") localButton();
    else if (id === "web") webButton();
    else if (id === "web-now") webNow();         // (the buttons under an answer that needs a model)
    else if (id === "ai") ownMenu();
    else if (id === "web-remove") webRemove();
    else if (id === "model") modelSheet();
    else if (id === "airpods") talkButton(true);
    else if (id === "heart") heartButton(null);
    else if (id === "heart-remove") heartRemoveButton();
  }
  function status() {
    var h = "";
    call("listening", ears.on);                // the microphone button under the box, pressed while it listens
    if (ears.on) h = ears.device ? "AirPods on: talk, or type" : "Listening: talk, or type";
    else if ((APP || local.kind === "web") && local.state === "preparing") h = "Getting my model ready: " + localWords() + "…";
    else if (APP && local.state === "starting") h = "Starting my model…";
    else if (heart.state === "downloading") h = "Downloading Heart" + (heart.progress ? " (" + heart.progress + "%)" : "") + "…";
    else if (heart.state === "starting") h = "Starting Heart…";
    call("hint", h);
  }

  // ------------------------------------------------------------------ what the page says
  function handle(msg) {
    switch (msg.type) {
      case "ready":
        call("openInPage", !APP);              // (in the app, a site is laid over the page, as on a Chat Computer)
        call("pickFiles", true);               // + under the box: this browser's own file picker
        layout();
        ownShow();                             // the spark beside +: your own AI, if you've signed in with one
        if (own && own.provider === "chatgpt") {
          APP.app("chatgpt_status").then(function (st) { if (!st || !st.signed_in) ownSet(null, ""); }, noop);
        }
        openRouterBack();                      // (back from OpenRouter's sign-in: its code, for a key)
        greetWith();                           // for the greeting: the name you go by, which model answers,
        sayHere();                             // and where you are, with its weather
        if (APP) {                             // (in the app, the name this computer knows you by, where you've given none)
          APP.app("about").then(function (a) {
            computerName = cleanName(a && a.name).split(" ")[0];
            if (computerName && !person()) greetWith();
          }, noop);
        }
        if (store("chatcomputer-demo-panels")) call("panels", store("chatcomputer-demo-panels"));
        call("doneList", (APP ? store(APP_DONE) || [] : DONE).map(function (e) {
          var pins = (store("chatcomputer-demo-pins") || {}).done || {}, o;
          if (e.undo && e.undo.kind === "close") delete e.undo;       // (what it opened went with the page before)
          known[e.id] = e;
          o = forScreen(e);
          if (pins[e.id]) o.pinned = pins[e.id];
          return o;
        }));
        if (APP) {                             // (in the app, your own accounts', those connected before read again)
          drawAppMessages();
          connected().forEach(function (k) { connectService(k, false); });
        } else call("messagesList", MESSAGES.map(withPin), ACCOUNTS);
        startChat();
        status();
        // The model: in the app, how it is (the app says as it changes); on the website, the app's, when it was chosen before.
        if (APP) APP.app("model_status").then(localFrom, noop);
        else if (store(LOCAL_KEY)) localProbe();
        else if (webCan() && store(WEB_KEY) && !own) {   // (this browser's, when it was chosen before and it's still kept)
          ENGINE.status().then(function (st) {
            local.webKept = !!(st && st.kept);
            store(WEB_KEPT, local.webKept || null);
            if (local.webKept) webStart(false);
            else store(WEB_KEY, null);
          }, noop);
        }
        heartCheck();                          // and Heart, the warmer voice, where it can
        break;
      case "send": {
        var said = String(msg.text || ""), files = msg.files || [];
        if (files.length) { askWithFiles(said, files); break; }
        var typed = address(said);                                     // an address opens its site, as a browser's would
        if (typed) visit(said, typed); else ask(said);
        break;
      }
      case "files_picked": pickedFiles(); break;
      case "mic_toggle": talkButton(); break;
      case "hole": hole = { x: msg.x, y: msg.y, w: msg.w, h: msg.h }; place(); break;
      case "close_content": close(); break;
      case "scroll_open": scrollOpen(msg.how); break;
      case "open_message": openMessage(msg.id); break;
      case "pin": {
        var pins = store("chatcomputer-demo-pins") || {};
        pins[msg.list] = pins[msg.list] || {};
        if (msg.on) pins[msg.list][msg.id] = now(); else delete pins[msg.list][msg.id];
        store("chatcomputer-demo-pins", pins);
        break;
      }
      case "goto": call("earlier", EARLIER, msg.id, msg.ts); break;
      case "revert": revert(String(msg.id || "")); break;
      // Under a message: read it aloud; good or bad (kept in this browser only: nothing leaves it).
      case "speak": readAloud(String(msg.text || "")); break;
      case "feedback": {
        var fb = store("chatcomputer-demo-feedback") || [];
        fb.push({ ts: now(), good: !!msg.good, on: !!msg.on, prompt: clip(msg.prompt, 300), answer: clip(msg.text, 600) });
        store("chatcomputer-demo-feedback", fb.slice(-100));
        break;
      }
      case "search": {
        if (APP) { appSearch(msg.n, String(msg.q || "")); break; }
        var found = search(msg.q || "");
        call("searchResults", msg.n, msg.q || "", found, found.length ? "" : "Nothing here matches \u201c" + (msg.q || "") + "\u201d.");
        break;
      }
      case "open_file":
        if (APP) {
          APP.app("open_file", { path: String(msg.path || "") }).then(function (name) { did("Opened " + name); },
            function (e) { call("notice", String(e), "warn"); });
          break;
        }
        if (PAGES[msg.path]) show({ html: PAGES[msg.path]() });
        break;
      case "command": slashCommand(String(msg.id || "")); break;
      case "text_scale": setScale(scale * (+msg.factor || 1)); break;
      case "panels": store("chatcomputer-demo-panels", { left: !!msg.left, right: !!msg.right }); break;
      case "open_chat": openChat(String(msg.id || "")); break;
      case "new_chat": newChat(); break;
      case "ai_menu": ownMenu(); break;
      case "add_account":
        if (typeof msg.provider === "string" && msg.provider.indexOf("ai:") === 0) { ownChoice(msg.provider.slice(3)); break; }
        if (!APP && typeof msg.provider === "string" && msg.provider.indexOf("connect:") === 0) {
          call("accountForm", connectForm(msg.provider.slice(8)));
          break;
        }
        if (APP && msg.provider) {
          var pv = String(msg.provider), off = pv.indexOf("forget:") === 0 ? pv.slice(7) : "";
          call("accountForm", null);
          if (off && SERVICES[off]) { forgetService(off); did("Disconnected " + SERVICES[off].label); }
          else if (SERVICES[pv]) { connectService(pv, true); did("Opened " + SERVICES[pv].label + " to connect it"); }
          break;
        }
        call("accountForm", connectForm(""));
        break;
      case "account": {
        var values = msg.values || {};
        if (msg.form === "local") { localChosen(); break; }
        if (msg.form === "web") { call("accountForm", null); webStart(true); break; }
        if (msg.form === "talk") { call("accountForm", null); startTalking(); break; }
        if (msg.form === "heart") { call("accountForm", null); heartChosen(true); break; }
        if (msg.form === "heart-remove") { call("accountForm", null); heartRemove(); break; }
        if (String(msg.form).indexOf("own-key-") === 0 && AIS[String(msg.form).slice(8)]) { ownKey(String(msg.form).slice(8), values); break; }
        call("accountForm", null);
        break;
      }
      case "account_cancel":
        call("accountForm", null);
        if (msg.form === "heart") heartChosen(false);
        break;
      default: break;            // keys, keyboard: nothing to do in a browser
    }
  }
})();
