/* Chat Computer in a browser: the demo at chatcomputer.ai (docs/, served by GitHub Pages).

   The screen here is Chat Computer's own, the same page and code as on a Chat Computer (chat.html, chat.js,
   chat.css: copied here by scripts/build_demo.py). This file stands in for the computer behind it. On Chat Computer,
   what you type (or say, or type in the Claude app on your phone) goes to Claude, which opens websites, plays
   music, reads your messages and looks after your files. Here, Claude's answers to a few things are written in
   advance, the messages and files are made up, and everything stays in your browser: nothing typed here is sent
   anywhere, unless you choose one of the two buttons at the right of the message box:
   - Connect Claude: with your own Claude API key, Claude answers for real and does things on this screen. What you
     type goes from your browser straight to Anthropic; the key stays in this browser.
   - Connect AirPods: you talk, and the answers are read out. Your browser turns what you say into text (Chrome
     does it on Google's servers).
   Fetched from elsewhere: YouTube's player, playing Louie Zong's "hello world" as the demo opens, and a map from
   OpenStreetMap when you ask for one. */
(function () {
  "use strict";
  var START = Date.now() / 1000;
  function now() { return Date.now() / 1000; }
  function call(name) {
    var args = Array.prototype.slice.call(arguments, 1);
    if (window.CO && typeof window.CO[name] === "function") window.CO[name].apply(null, args);
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
  var MESSAGES = [
    { id: "m1", source: "whatsapp", label: "WhatsApp", who: "Mum", text: "Dinner at 7 tonight? Bring the photos from the trip!", ts: START - 240, unread: true },
    { id: "m2", source: "email", label: "Gmail", who: "Jane Lim", text: "Re: Lunch on Friday? 12:30 at the usual place works for me.", ts: START - 3100, unread: true },
    { id: "m3", source: "telegram", label: "Telegram", who: "Alex", text: "Sent you the slides for Monday.", ts: START - 7400, unread: false },
    { id: "m4", source: "instagram", label: "Instagram", who: "sam.k", text: "Liked your photo", ts: START - 11000, unread: false },
    { id: "m5", source: "email", label: "Outlook", who: "City Library", text: "Your books are due back on Thursday.", ts: START - 90000, unread: false },
    { id: "m6", source: "x", label: "X", who: "Priya", text: "Are you coming to the meetup on Saturday?", ts: START - 96000, unread: false },
    { id: "m7", source: "linkedin", label: "LinkedIn", who: "Daniel Tan", text: "Thanks for connecting!", ts: START - 260000, unread: false }
  ];
  var ACCOUNTS = [{ id: "a1", label: "Gmail" }, { id: "a2", label: "Outlook" }, { id: "a3", label: "Telegram" },
                  { id: "a4", label: "WhatsApp" }, { id: "a5", label: "Instagram" }, { id: "a6", label: "X" },
                  { id: "a7", label: "LinkedIn" }];
  function when(ts) {
    var d = new Date(ts * 1000), today = new Date();
    var t = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
    if (d.toDateString() === today.toDateString()) return "Today, " + t;
    return d.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" }) + ", " + t;
  }
  var THREADS = {
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
  // The music playing as the demo opens: Louie Zong's "hello world", from 0:03 (before that it's silent).
  var VIDEO = "Yw6u6YkTgQ4", VIDEO_START = 3;
  var SONG = "\u201chello world\u201d by Louie Zong", SONG_DONE = "Played " + SONG + " on YouTube";
  // Earlier conversations, put back when something in What's done is tapped (the screen starts empty: the word logo).
  var EARLIER = [
    { role: "user", id: "y1", ts: START - 86400 - 3010, text: "Play some calm piano music" },
    { role: "assistant", id: "ya1", ts: START - 86400 - 3000, text: "Playing a calm piano mix on YouTube." },
    { role: "user", id: "y2", ts: START - 86400 - 2910, text: "A bit quieter please" },
    { role: "assistant", id: "ya2", ts: START - 86400 - 2900, text: "Volume is 40%." },
    { role: "user", id: "h1", ts: START - 5410, text: "How do I get to the airport on Sunday with two big suitcases?" },
    { role: "assistant", id: "ha1", ts: START - 5380, text: "The train means two changes, and one of them has stairs and no lift, so a taxi is easier: about 25 minutes. I put a packing list in your Documents too." },
    { role: "user", id: "v1", ts: START - 2, text: "Play hello world" },
    { role: "assistant", id: "va1", ts: START - 1, text: "Playing " + SONG + "." }
  ];
  var DONE = [
    { id: "d1", ts: START - 86400 - 2995, text: "Opened youtube.com", prompt_id: "y1", prompt_ts: START - 86400 - 3010 },
    { id: "d2", ts: START - 86400 - 2905, text: "Set the volume to 40%", prompt_id: "y2", prompt_ts: START - 86400 - 2910 },
    { id: "d3", ts: START - 5400, text: "Searched the web for \u201ctrains to the airport on Sunday\u201d", prompt_id: "h1", prompt_ts: START - 5410 },
    { id: "d4", ts: START - 5390, text: "Saved Packing list.md", prompt_id: "h1", prompt_ts: START - 5410 },
    { id: "d5", ts: START - 1, text: SONG_DONE, prompt_id: "v1", prompt_ts: START - 2 }
  ];
  var FILES = [
    { name: "Packing list.md", where: "~/Documents", kind: "document", ts: START - 5390, page: "packing" },
    { name: "Beach.jpg", where: "~/Pictures/Bali", kind: "image", ts: START - 40 * 86400, page: "beach" },
    { name: "Budget 2026.xlsx", where: "C:\\Users\\You\\Documents (Windows)", kind: "spreadsheet", ts: START - 17 * 86400, page: "budget" },
    { name: "Taxes 2025.pdf", where: "~/Documents/Taxes", kind: "document", ts: START - 190 * 86400, page: "taxes" },
    { name: "Recipes.txt", where: "~/Documents", kind: "document", ts: START - 100 * 86400, page: "recipes" },
    { name: "Resume.docx", where: "~/Documents/Work", kind: "document", ts: START - 60 * 86400, page: "resume" }
  ];

  // ------------------------------------------------------------------ the screen's size
  // On Chat Computer the text size comes from the screen's real size and how far away you sit (ui/layout.py); here,
  // from the window's size. Pinching the conversation (a touchscreen) changes it, as it does there.
  var size = 20, column = 600, scale = store("chatcomputer-demo-scale") || 1;
  function layout() {
    var w = window.innerWidth, h = window.innerHeight;
    size = Math.max(15, Math.min(26, Math.min(w * 0.0125, h * 0.022))) * scale;
    column = Math.round(Math.min(30 * size, w - 2 * size));
    call("layout", Math.round(size * 10) / 10, column);
    place();
    placeTools();
  }
  window.addEventListener("resize", layout);

  // ------------------------------------------------------------------ what's open between the sidebars
  // On Chat Computer a second browser view, laid over the space the conversation page leaves for it ("hole"); here a
  // frame. F1 makes it fill the window, F1 again puts it back; Escape closes it.
  var view = null, open = false, expanded = false, hole = null, drafting = false;
  function frame() {
    if (view) return view;
    view = document.createElement("iframe");
    view.id = "demo-view";
    view.title = "What's open";
    view.setAttribute("allow", "autoplay; encrypted-media; picture-in-picture; fullscreen");
    view.style.cssText = "position:fixed;border:0;background:#fff;display:none;z-index:5";
    // F1 and Escape work while a page of the demo's own has the keys too (a map's page keeps them to itself).
    view.addEventListener("load", function () {
      try { view.contentDocument.addEventListener("keydown", keys, true); } catch (e) {}
    });
    document.body.appendChild(view);
    return view;
  }
  function show(page) {
    var v = frame();
    player.token = {};                       // what was playing is gone
    player.id = page.youtube || "";
    if (page.src) { v.removeAttribute("srcdoc"); v.src = page.src; }
    else { v.removeAttribute("src"); v.srcdoc = page.html; }
    open = true;
    expanded = false;
    v.style.display = "block";
    call("mode", "content");
    place();
  }
  function place() {
    if (!view || !open) return;
    var r = expanded ? { x: 0, y: 0, w: window.innerWidth, h: window.innerHeight } : hole;
    if (!r) return;
    view.style.left = r.x + "px";
    view.style.top = r.y + "px";
    view.style.width = r.w + "px";
    view.style.height = r.h + "px";
    view.style.zIndex = expanded ? "9" : "5";
  }
  function close() {
    if (!open) return;
    open = false;
    expanded = false;
    player.token = {};
    player.id = "";
    view.style.display = "none";
    view.srcdoc = "";
    view.removeAttribute("src");
    call("mode", "chat");
  }
  function keys(e) {
    if (e.key === "F1" && open) { e.preventDefault(); expanded = !expanded; place(); }
    else if (e.key === "Escape" && open && !drafting && !document.body.classList.contains("sheet")) {
      e.preventDefault();
      close();
    }
  }
  document.addEventListener("keydown", keys, true);
  // Typing always goes into the message box. A click on the video gives YouTube's player the keys (for its own
  // shortcuts); they come back to the box as soon as the pointer leaves it.
  document.addEventListener("pointermove", function () {
    if (view && document.activeElement === view && !document.body.classList.contains("sheet")) $("input").focus({ preventScroll: true });
  }, true);
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
  var NOTE_CSS = ".demo-note{margin:0 0 1.25rem;padding:0.5rem 0.75rem;border:2px solid var(--bubble);border-radius:0.6rem}";
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
      NOTE_CSS + ".said.mail{padding:0 0 1rem;margin:0 0 1.25rem;border-bottom:2px solid var(--bubble)}.said.mail:last-child{border-bottom:0}",
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

  // ------------------------------------------------------------------ music: YouTube, playing
  // On Chat Computer, music is YouTube's own page between the sidebars. Here it's YouTube's player in the same
  // place, playing "hello world" as soon as the demo opens, and the conversation comes back when a video ends.
  // Browsers let a page play sound by itself only after you've clicked, tapped or typed on the site (or, in
  // Chrome, played its videos with sound often enough): so the video starts with sound when the browser allows it
  // and otherwise without, and then the sound comes on with the first click, tap or key anywhere here. The
  // player's own controls pause it. (The player and this page talk with YouTube's player messages.)
  var player = { token: {}, id: "", heard: false, state: -2, armed: false, volume: 100 };
  function youtube(id, start) {
    id = id || VIDEO;
    start = start == null ? (id === VIDEO ? VIDEO_START : 0) : start;
    var token = player.token = {};
    soundAllowed().then(function (sound) {
      if (player.token !== token) return;               // something else was opened meanwhile
      var origin = /^https?:$/.test(location.protocol) ? "&origin=" + encodeURIComponent(location.origin) : "";
      show({ youtube: id, src: "https://www.youtube-nocookie.com/embed/" + id + "?autoplay=1&playsinline=1&rel=0&enablejsapi=1" +
             "&start=" + start + (sound ? "" : "&mute=1") + origin });
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
  function command(func, args) {
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

  // ------------------------------------------------------------------ Claude's answers, written in advance
  var CITIES = { singapore: [103.6, 1.2, 104.1, 1.47], london: [-0.35, 51.38, 0.15, 51.62], "new york": [-74.1, 40.6, -73.8, 40.85],
                 tokyo: [139.55, 35.55, 139.95, 35.8], paris: [2.22, 48.8, 2.47, 48.92], sydney: [151.0, -33.98, 151.35, -33.78] };
  var TRY = "Try \u201cshow me a map of Tokyo\u201d, \u201cmake me a timer\u201d, \u201cwhat did Jane say?\u201d, " +
            "\u201copen my packing list\u201d or \u201cplay hello world\u201d. Type / to search the files.";
  var REAL = "Connect Claude, at the right of the message box, and Claude answers for real, with your own API key.";
  function map(b) { show({ src: "https://www.openstreetmap.org/export/embed.html?bbox=" + b.join("%2C") + "&layer=mapnik" }); }
  var INTENTS = [
    [/\bhello,? world\b|\b(music|song|songs|piano|play|playing|spotify|youtube|lofi|jazz|video)\b/i, function () {
      return { text: "Playing " + SONG + ".", done: [SONG_DONE], act: function () { youtube(); } };
    }],
    [/\b(close|stop|hide) (it|that|this|the (music|video|page|song))\b|^(close|stop)\b/i, function () {
      return { text: open ? "Closed it." : "Nothing's open.", act: close };
    }],
    [/\b(hi|hello|hey|good (morning|afternoon|evening))\b|^help\b|what can you do|how does (this|it) work|what is (this|chatcomputer)/i, function () {
      return { text: "Hi! I'm Chat Computer: Claude, running the whole computer. You talk, type or touch, and I do it: open websites, " +
        "play music, read your messages, look after your files and settings.\n\nIn this demo my answers are written in advance, " +
        "and what you type stays in your browser. " + REAL + " Chat Computer is an independent project, not made by Anthropic. " + TRY };
    }],
    [/\b(map|maps|where is|directions|how far)\b/i, function (t) {
      var name = Object.keys(CITIES).filter(function (c) { return t.toLowerCase().indexOf(c) >= 0; })[0] || "singapore", b = CITIES[name];
      var label = name.replace(/\b\w/g, function (c) { return c.toUpperCase(); });
      return { text: "Here's " + label + ".", done: ["Opened openstreetmap.org"], act: function () { map(b); } };
    }],
    [/\b(timer|countdown|stopwatch)\b/i, function (t) {
      var m = /(\d+)\s*(min|minute)/i.exec(t), mins = m ? Math.min(180, +m[1]) : 10;
      return { text: "A " + mins + "-minute timer, on the screen. I made it just now: on Chat Computer I can make any little page " +
        "or app like this.", done: ["Made a timer"], act: function () { show({ html: timer(mins) }); } };
    }],
    [/\b(weather|rain|raining|forecast|temperature|umbrella|sunny)\b/i, function () {
      return { text: "I can't look things up in this demo, so here's a made-up forecast to show where it goes: a short answer here, " +
        "the details on the screen.", act: function () {
        show({ html: doc("Weather", "| | Morning | Afternoon | Evening |\n|---|---|---|---|\n| Today | 27\u00b0, cloudy | 31\u00b0, showers | 28\u00b0, clear |\n" +
          "| Tomorrow | 26\u00b0, clear | 32\u00b0, sunny | 28\u00b0, clear |", "Made up for the demo. On Chat Computer this is today's real forecast.") });
      } };
    }],
    [/\b(jane|lunch|email|emails|e-mail|mail|inbox|gmail|outlook)\b/i, function () {
      return { text: "Jane says 12:30 at the usual place works for her, for lunch on Friday. Her emails are on the screen.", act: function () {
        openMessage("m2");
      } };
    }],
    [/\b(mum|mom|whatsapp|telegram|texts?|messages?|alex)\b/i, function (t) {
      var alex = /alex|telegram/i.test(t);
      return { text: alex ? "Alex sent you the slides for Monday." : "Your mum asks if you're coming to dinner at 7 tonight, and to bring the photos from the trip.",
        act: function () { openMessage(alex ? "m3" : "m1"); } };
    }],
    [/\b(packing|list|notes?|document|doc|write)\b/i, function () {
      return { text: "Here's your packing list for Sunday.", act: function () { show({ html: PAGES.packing() }); } };
    }],
    [/\b(volume|louder|quieter|softer|mute|unmute)\b/i, function (t) {
      var up = /louder|up|unmute/i.test(t);
      return { text: up ? "Volume is 60%." : "Volume is 40%.", done: [up ? "Turned the volume up" : "Turned the volume down"],
        act: function () { setVolume(up ? 60 : 40); } };
    }],
    [/\b(wi-?fi|internet|network)\b/i, function () {
      return { text: "You're on Home Wi-Fi, and the internet's working.", done: [] };
    }],
    [/\b(airpods|bluetooth|headphones|earbuds)\b/i, function () {
      return { text: "Connect AirPods, at the right of the message box, and you can talk to me through them.", done: [] };
    }],
    [/\b(file|files|find|search|photo|photos|pictures?|beach)\b/i, function (t) {
      if (/beach|photo|picture/i.test(t)) return { text: "Here's Beach.jpg, from your Pictures.", act: function () { show({ html: PAGES.beach() }); } };
      return { text: "Type / in the message box to search this computer's files: they show above the box as you type, and Enter opens one." };
    }],
    [/\b(bigger|smaller|text size|font|zoom)\b/i, function (t) {
      var bigger = /bigger|larger|zoom in/i.test(t);
      return { text: bigger ? "Made the text bigger." : "Made the text smaller.", done: [bigger ? "Made the text bigger" : "Made the text smaller"],
        act: function () { setScale(scale * (bigger ? 1.15 : 1 / 1.15)); } };
    }],
    [/\b(thanks|thank you|cool|great|nice|awesome)\b/i, function () { return { text: "Any time." }; }],
    [/./, function () {
      return { text: "In this demo I only know a few things; on Chat Computer, Claude answers anything, and does it. " + REAL + " " + TRY };
    }]
  ];
  function answer(text) {
    for (var i = 0; i < INTENTS.length; i++) if (INTENTS[i][0].test(text)) return INTENTS[i][1](text);
    return { text: "" };
  }
  function setScale(s) {
    scale = Math.max(0.6, Math.min(2.2, s));
    store("chatcomputer-demo-scale", scale);
    layout();
  }
  function setVolume(p) {
    player.volume = Math.max(0, Math.min(100, Math.round(p)));
    command("setVolume", [player.volume]);
    if (player.volume > 0 && !player.armed) command("unMute");
  }

  var turns = 0;
  function ask(text) {
    var you = uid("u"), uts = now(), id = "a" + (++turns);
    call("user", you, text, true);
    if (claude.key) { claudeTurn(text, you, uts, id); return; }
    var r = answer(text);
    setTimeout(function () {
      call("aiStart", id);
      setTimeout(function () {
        if (r.act) r.act();
        (r.done || []).forEach(function (d) { did(d, you, uts); });
        stream(id, r.text || "", function () { say(r.text || ""); });
      }, 600);
    }, 120);
  }
  function did(text, you, uts) { call("done", { id: uid("d"), ts: now(), text: text, prompt_id: you || "", prompt_ts: uts || now() }); }
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
    var html = thread(id);
    if (html) show({ html: html });
    return m;
  }
  function withPin(m) {
    var pins = (store("chatcomputer-demo-pins") || {}).messages || {}, out = {};
    for (var k in m) out[k] = m[k];
    if (pins[m.id]) out.pinned = pins[m.id];
    return out;
  }
  function search(q) {
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

  // ------------------------------------------------------------------ real Claude, with your own API key
  // "Connect Claude", at the right of the message box: with a Claude API key (from Claude Console), what you type
  // goes to Claude itself, from this browser straight to Anthropic, and Claude answers and does things on this
  // screen: plays YouTube, shows maps and pages, opens the made-up messages and files, searches the web. The key
  // stays in this browser (localStorage), and what's used is billed to it. (Anthropic doesn't let other apps sign
  // in with a Claude Pro or Max plan, so a key it is.)
  var API = "https://api.anthropic.com/v1/";
  var MODEL = "claude-sonnet-5-5", MODEL_NAME = "Sonnet 5.5";      // as on a Chat Computer (chatcomputer/config.py)
  var KEY = "chatcomputer-demo-claude-key";
  var claude = { key: store(KEY) || "", talk: [], search: true };
  function headers(key) {
    return { "x-api-key": key, "anthropic-version": "2023-06-01", "content-type": "application/json",
             "anthropic-dangerous-direct-browser-access": "true" };
  }
  function inbox() {
    return MESSAGES.map(function (m) {
      var t = THREADS[m.id], lines = t.mails ? t.mails.map(function (x) { return x[0] + ": " + x[2].replace(/\s+/g, " "); })
        : t.lines.map(function (x) { return x[0] + ": " + x[2]; });
      return "- " + m.id + ", " + m.label + ", from " + m.who + ", " + when(m.ts) + (m.unread ? " (unread)" : "") + ": " + lines.join(" / ");
    }).join("\n");
  }
  function systemPrompt() {
    var d = new Date(), zone = "";
    try { zone = Intl.DateTimeFormat().resolvedOptions().timeZone || ""; } catch (e) {}
    return "You are Chat Computer: Claude, running a whole computer. People talk, type or touch, and you do things for them. " +
      "Right now you're in Chat Computer's demo website (chatcomputer.ai), in the person's web browser, with their own Claude " +
      "API key. The screen is Chat Computer's: Messages at the left, What's done at the right (each thing you do is listed " +
      "there), and what you open goes between them, above their latest message, your latest reply and the message box.\n\n" +
      "Reply in one to three short, plain sentences, without markdown: replies show just above the message box, and are " +
      "read aloud when they talk to you. Anything longer (a recipe, a plan, a list, a table) goes on the screen with " +
      "show_page, and you say in a sentence what it is. Do things rather than describe them, and don't announce a tool " +
      "before using it.\n\n" +
      "Here you can: play YouTube videos (play_youtube; use web_search to find a video's address when you don't know its " +
      "ID for certain), show maps (show_map), show pages you write (show_page), give a link to a website (open_website: " +
      "other sites can't show inside the demo; on a Chat Computer they open right here and you use them), open their " +
      "messages and files (open_message, open_file), make a timer (make_timer), set the volume of what's playing " +
      "(set_volume), close what's open (close), and search the web. On a real Chat Computer you also use any website, " +
      "their real files, apps and settings, and their AirPods; when they ask for something only a Chat Computer can do, " +
      "say so in a sentence.\n\n" +
      "Their messages and files here are made up for the demo:\n" + inbox() + "\n\nFiles:\n" +
      FILES.map(function (f) { return "- " + f.name + ", in " + f.where; }).join("\n") +
      "\n\nIt's " + d.toLocaleString("en-GB", { weekday: "long", day: "numeric", month: "long", year: "numeric", hour: "numeric", minute: "2-digit" }) +
      (zone ? " (" + zone + ")" : "") + ".";
  }
  function obj(props, required) { return { type: "object", properties: props, required: required || [] }; }
  var TOOLS = [
    { name: "play_youtube", description: "Play a YouTube video between the sidebars. The demo can't search YouTube itself: find the " +
      "video's address with web_search unless you know its ID for certain. Louie Zong's \"hello world\" is Yw6u6YkTgQ4 (start it at 3: " +
      "the first seconds are silent).", input_schema: obj({ video_id: { type: "string", description: "The 11-character ID, as in youtube.com/watch?v=ID" },
      title: { type: "string", description: "The video's title, for What's done" }, start: { type: "integer", description: "Seconds in to start at" } }, ["video_id", "title"]) },
    { name: "show_map", description: "Show a map (OpenStreetMap) between the sidebars, of the area in the box given in degrees.",
      input_schema: obj({ place: { type: "string" }, west: { type: "number" }, south: { type: "number" }, east: { type: "number" },
      north: { type: "number" } }, ["place", "west", "south", "east", "north"]) },
    { name: "show_page", description: "Show a page you write between the sidebars, in Markdown (headings, lists, tables): anything " +
      "longer than a sentence or two.", input_schema: obj({ title: { type: "string" }, markdown: { type: "string" } }, ["title", "markdown"]) },
    { name: "open_website", description: "Open a website. Other sites can't show inside the demo, so this shows a link to it there.",
      input_schema: obj({ url: { type: "string", description: "The full web address, with https" }, title: { type: "string" } }, ["url"]) },
    { name: "open_message", description: "Open one of their messages between the sidebars (as the conversation it's part of).",
      input_schema: obj({ id: { type: "string", enum: MESSAGES.map(function (m) { return m.id; }) } }, ["id"]) },
    { name: "open_file", description: "Open one of their files between the sidebars.",
      input_schema: obj({ name: { type: "string", enum: FILES.map(function (f) { return f.name; }) } }, ["name"]) },
    { name: "make_timer", description: "Put a countdown timer between the sidebars.",
      input_schema: obj({ minutes: { type: "integer", minimum: 1, maximum: 180 } }, ["minutes"]) },
    { name: "set_volume", description: "Set the volume of what's playing, from 0 to 100.", input_schema: obj({ percent: { type: "integer", minimum: 0, maximum: 100 } }, ["percent"]) },
    { name: "close", description: "Close what's open between the sidebars (a video stops).", input_schema: obj({}) }
  ];
  function tools() {
    return claude.search ? TOOLS.concat([{ type: "web_search_20250305", name: "web_search", max_uses: 3 }]) : TOOLS;
  }
  // What a client tool does here, and what Claude is told about it.
  function run(u, you, uts) {
    var a = u.input || {}, n = u.name;
    if (n === "play_youtube") {
      var vid = String(a.video_id || "").trim();
      if (!/^[A-Za-z0-9_-]{11}$/.test(vid)) return "That isn't a YouTube video ID (11 letters, digits, - or _).";
      var start = Math.max(0, parseInt(a.start, 10) || (vid === VIDEO ? VIDEO_START : 0));
      youtube(vid, start);
      did(vid === VIDEO ? SONG_DONE : "Played \u201c" + clip(a.title || "a video", 60) + "\u201d on YouTube", you, uts);
      return "Playing between the sidebars. (If its owner doesn't allow it to play on other sites, YouTube says so there.)";
    }
    if (n === "show_map") {
      var b = [a.west, a.south, a.east, a.north].map(Number);
      if (b.some(isNaN) || b[0] >= b[2] || b[1] >= b[3] || Math.abs(b[1]) > 90 || Math.abs(b[3]) > 90) return "That box isn't west < east, south < north, in degrees.";
      map(b);
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
      show({ html: page(host, '<article><h1 class="title">' + esc(a.title || host) + '</h1><p class="demo-note">On Chat Computer, ' + esc(host) +
        " opens right here, and Claude uses it for you. Other sites can't show inside this demo, so here's the link.</p><p><a href=\"" + esc(url) +
        '" target="_blank" rel="noopener noreferrer">Open ' + esc(host) + " in a new tab</a></p></article>", NOTE_CSS) });
      did("Showed a link to " + host, you, uts);
      return "A link to " + host + " is on the screen (it opens in a new tab).";
    }
    if (n === "open_message") {
      var m = THREADS[a.id] ? openMessage(a.id) : null;
      if (!m) return "There's no message " + a.id + ".";
      did("Opened the message from " + m.who, you, uts);
      return "It's on the screen.";
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
      did("Made a timer", you, uts);
      return "A " + mins + "-minute timer is on the screen, with Start and Reset.";
    }
    if (n === "set_volume") {
      if (!player.id) return "Nothing is playing.";
      setVolume(Number(a.percent));
      did("Set the volume to " + player.volume + "%", you, uts);
      return "The volume is " + player.volume + "%.";
    }
    if (n === "close") { var was = open; close(); return was ? "Closed." : "Nothing was open."; }
    return "There's no tool called " + n + " here.";
  }
  // One turn: Claude's answer streams in, its tools run here, and it goes on until it's done (at most 8 rounds).
  function claudeTurn(text, you, uts, id) {
    var said = "", gap = false, rounds = 0, asked = { role: "user", content: text };
    claude.talk.push(asked);
    trimTalk();
    call("aiStart", id);
    function onText(t) {
      if (gap && said && !/\s$/.test(said)) { call("ai", id, " "); said += " "; }
      gap = false;
      said += t;
      call("ai", id, t);
    }
    (function round() {
      if (++rounds > 8) { finish(); return; }
      gap = true;
      message(onText, you, uts).then(function (msg) {
        claude.talk.push({ role: "assistant", content: msg.content });
        if (msg.stop === "pause_turn") { round(); return; }
        var uses = msg.content.filter(function (b) { return b.type === "tool_use"; });
        if (msg.stop === "tool_use" && uses.length) {
          claude.talk.push({ role: "user", content: uses.map(function (u) {
            var out;
            try { out = run(u, you, uts); } catch (e) { out = "That didn't work: " + e.message; }
            return { type: "tool_result", tool_use_id: u.id, content: out };
          }) });
          round();
          return;
        }
        finish();
      }, function (err) {
        if (err.noSearch && claude.search) { claude.search = false; rounds--; round(); return; }
        var at = claude.talk.indexOf(asked);
        if (at >= 0) claude.talk.length = at;                             // the turn didn't happen
        onText((said ? "\n\n" : "") + trouble(err));
        call("aiDone", id);
      });
    })();
    function finish() { call("aiDone", id); say(said); }
  }
  // At most 40 messages back, starting at something they said.
  function trimTalk() {
    while (claude.talk.length > 40) {
      claude.talk.shift();
      while (claude.talk.length && typeof claude.talk[0].content !== "string") claude.talk.shift();
    }
  }
  function trouble(err) {
    if (err.status === 401 || err.status === 403) {
      forget();
      return "Anthropic didn't accept your Claude API key (" + err.message + "), so I've disconnected it. Connect Claude again with a working key.";
    }
    if (err.status === 429) return "Claude's limit for your key was reached for now (" + err.message + "). Try again in a minute.";
    if (err.status === 529 || err.status === 503) return "Claude is busy right now. Try again in a moment.";
    if (!err.status) return "I couldn't reach Anthropic from this browser (" + err.message + ").";
    return "Claude said: " + err.message;
  }
  function apiError(status, raw) {
    var msg = raw, e;
    try { var j = JSON.parse(raw); msg = (j.error && j.error.message) || raw; } catch (x) {}
    e = new Error(clip(msg || ("HTTP " + status), 200));
    e.status = status;
    e.noSearch = status === 400 && /web.?search/i.test(msg || "");
    return e;
  }
  // One request, streamed: text as it comes, and the whole message (its blocks, cleaned) at the end.
  function message(onText, you, uts) {
    var body = { model: MODEL, max_tokens: 1500, stream: true,
                 system: [{ type: "text", text: systemPrompt(), cache_control: { type: "ephemeral" } }],
                 tools: tools(), messages: claude.talk };
    return fetch(API + "messages", { method: "POST", headers: headers(claude.key), body: JSON.stringify(body) }).then(function (res) {
      if (!res.ok) return res.text().then(function (t) { throw apiError(res.status, t); });
      var reader = res.body.getReader(), dec = new TextDecoder(), buf = "", blocks = [], stop = null, failed = null;
      function event(chunk) {
        var data = chunk.split("\n").filter(function (l) { return l.indexOf("data:") === 0; })
          .map(function (l) { return l.slice(5).trim(); }).join("");
        if (!data) return;
        var ev;
        try { ev = JSON.parse(data); } catch (e) { return; }
        if (ev.type === "content_block_start") {
          var b = ev.content_block || {};
          if (b.type === "text") blocks[ev.index] = { type: "text", text: b.text || "" };
          else if (b.type === "tool_use" || b.type === "server_tool_use") blocks[ev.index] = { type: b.type, id: b.id, name: b.name, input: b.input || {}, json: "" };
          else if (b.type === "thinking") blocks[ev.index] = { type: "thinking", thinking: b.thinking || "", signature: b.signature || "" };
          else blocks[ev.index] = b;          // a search's results, redacted thinking: kept whole, for the next round
          if (b.type === "text" && b.text) onText(b.text);
        } else if (ev.type === "content_block_delta") {
          var bl = blocks[ev.index], dl = ev.delta || {};
          if (!bl) return;
          if (dl.type === "text_delta") { bl.text += dl.text; onText(dl.text); }
          else if (dl.type === "input_json_delta") bl.json += dl.partial_json || "";
          else if (dl.type === "thinking_delta") bl.thinking += dl.thinking || "";
          else if (dl.type === "signature_delta") bl.signature += dl.signature || "";
        } else if (ev.type === "content_block_stop") {
          var bk = blocks[ev.index];
          if (bk && "json" in bk) {
            try { if (bk.json) bk.input = JSON.parse(bk.json); } catch (e) { bk.input = {}; }
            delete bk.json;
            if (bk.type === "server_tool_use" && bk.input && bk.input.query) did("Searched the web for \u201c" + clip(bk.input.query, 60) + "\u201d", you, uts);
          }
        } else if (ev.type === "message_delta") {
          if (ev.delta && ev.delta.stop_reason) stop = ev.delta.stop_reason;
        } else if (ev.type === "error") {
          failed = apiError(529, JSON.stringify(ev));
          if (ev.error && ev.error.type !== "overloaded_error") failed.status = 500;
        }
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
            if (failed) { reject(failed); return; }
            resolve({ content: blocks.filter(function (b) { return b && (b.type !== "text" || b.text); }), stop: stop });
          }, function (e) { var x = new Error(e && e.message || "the connection broke"); x.status = 0; reject(x); });
        })();
      });
    }, function (e) { var x = new Error(e && e.message || "no connection"); x.status = 0; throw x; });
  }
  function claudeButton() {
    if (claude.key) {
      call("accountForm", { id: "claude-off", title: "Claude is connected", ok: "Disconnect", text: [
        "What you type goes to Claude (" + MODEL_NAME + "), with your API key, from this browser straight to Anthropic.",
        "Disconnecting removes the key from this browser, and the demo's written answers come back."] });
      return;
    }
    keyForm("");
  }
  function keyForm(error, key) {
    call("accountForm", { id: "claude", title: "Connect Claude", ok: "Connect", error: error, text: [
      "Paste an API key from Claude Console (platform.claude.com), and Claude answers here for real, and does things on this " +
      "screen, instead of the demo's written answers.",
      "The key stays in this browser and goes only to Anthropic. What you use is billed to your Console account.",
      "A Claude Pro or Max plan can't be used here: Anthropic doesn't let other apps sign in with one."],
      fields: [{ name: "key", label: "API key", type: "password", value: key || "" }] });
  }
  function connectClaude(key) {
    if (/^sk-ant-oat/.test(key)) { keyForm("That's a sign-in token for a Claude plan, which other apps may not use. An API key from Claude Console starts with sk-ant-api."); return; }
    if (!/^sk-ant-/.test(key)) { keyForm("That doesn't look like a Claude API key: they start with sk-ant-.", key); return; }
    // A one-word question tells whether Anthropic takes the key (and costs a fraction of a cent).
    fetch(API + "messages", { method: "POST", headers: headers(key),
                              body: JSON.stringify({ model: MODEL, max_tokens: 1, messages: [{ role: "user", content: "Hi" }] }) }).then(function (res) {
      if (res.ok) {
        claude.key = key;
        claude.talk = [];
        claude.search = true;
        store(KEY, key);
        call("accountForm", null);
        drawTools();
        did("Connected Claude (" + MODEL_NAME + ")");
        return;
      }
      return res.text().then(function (t) { keyForm("Anthropic didn't accept that key: " + apiError(res.status, t).message, key); });
    }, function () { keyForm("I couldn't reach Anthropic from this browser. Check the connection, and try again.", key); });
  }
  function forget() {
    claude.key = "";
    claude.talk = [];
    store(KEY, null);
    drawTools();
  }

  // ------------------------------------------------------------------ AirPods: talking
  // On Chat Computer you talk through your AirPods and it answers in them. Here, "Connect AirPods": the browser
  // listens through them (or the device's own microphone, when they aren't the one it uses), turns what you say into
  // text, and the answers are read out. A website can't pair AirPods: the phone or computer does, in its Bluetooth
  // settings.
  var Recognition = window.SpeechRecognition || window.webkitSpeechRecognition;
  var ears = { on: false, rec: null, speaking: false, device: "", told: false };
  function talkButton() {
    if (ears.on) { stopTalking(); return; }
    if (!Recognition || !navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
      call("accountForm", { id: "talk-no", title: "Talking", ok: "OK", text: [
        "This browser can't turn speech into text. Chrome, Edge and Safari can."] });
      return;
    }
    if (ears.told) { startTalking(); return; }
    call("accountForm", { id: "talk", title: "Talk through your AirPods", ok: "Start talking", text: [
      "Pair your AirPods with this phone or computer first, in its Bluetooth settings: a website can't pair them.",
      "Then talk: what you say goes in the message box's place, and the answers are read out. Click the button again to stop.",
      "Your browser turns what you say into text (Chrome does it on Google's servers). It asks to use the microphone."] });
  }
  function startTalking() {
    ears.told = true;
    navigator.mediaDevices.getUserMedia({ audio: true }).then(function (stream) {
      var track = stream.getAudioTracks()[0], label = track && track.label || "";
      stream.getTracks().forEach(function (t) { t.stop(); });
      ears.device = /airpods/i.test(label) ? label : "";
      ears.on = true;
      drawTools();
      did(ears.device ? "Connected your AirPods" : "Started listening (" + (clip(label, 40) || "the microphone") + ")");
      hear();
    }, function (err) {
      call("accountForm", { id: "talk-no", title: "The microphone is off", ok: "OK", text: [
        "Your browser didn't let this page use the microphone (" + (err && err.name || "refused") + ").",
        "Allow it for chatcomputer.ai in the browser's site settings, then press Connect AirPods again."] });
    });
  }
  function hear() {
    if (!ears.on || ears.speaking || ears.rec) return;
    var r = new Recognition();
    r.lang = navigator.language || "en-US";
    r.continuous = true;
    r.interimResults = true;
    r.onresult = function (e) {
      var heard = "", final = "";
      for (var i = e.resultIndex; i < e.results.length; i++) {
        if (e.results[i].isFinal) final += e.results[i][0].transcript; else heard += e.results[i][0].transcript;
      }
      if (final.trim()) { call("partial", ""); ask(final.trim()); }
      else call("partial", heard.trim());
    };
    r.onerror = function (e) {
      if (e.error === "not-allowed" || e.error === "service-not-allowed" || e.error === "network") {
        stopTalking();
        call("accountForm", { id: "talk-no", title: "Talking stopped", ok: "OK", text: [e.error === "network" ?
          "The browser couldn't reach its speech service (it needs the internet)." :
          "The browser stopped this page from listening. Allow the microphone for chatcomputer.ai in its site settings."] });
      }
    };
    r.onend = function () { ears.rec = null; call("partial", ""); if (ears.on && !ears.speaking) setTimeout(hear, 250); };
    ears.rec = r;
    try { r.start(); } catch (x) { ears.rec = null; }
  }
  function stopTalking() {
    ears.on = false;
    if (ears.rec) { try { ears.rec.abort(); } catch (x) {} ears.rec = null; }
    try { window.speechSynthesis.cancel(); } catch (x) {}
    ears.speaking = false;
    call("partial", "");
    drawTools();
  }
  // An answer, read out while talking (not listening meanwhile, so it doesn't hear itself).
  function say(text) {
    if (!ears.on || !text || !window.speechSynthesis) return;
    var plain = String(text).replace(/[*_`#>|~]/g, "").replace(/\s+/g, " ").trim();
    if (!plain) return;
    ears.speaking = true;
    if (ears.rec) { try { ears.rec.abort(); } catch (x) {} }
    var u = new SpeechSynthesisUtterance(plain);
    u.lang = navigator.language || "en-US";
    u.onend = u.onerror = function () { ears.speaking = false; hear(); };
    window.speechSynthesis.speak(u);
  }

  // ------------------------------------------------------------------ the two buttons at the right of the box
  var KEY_ICON = '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="8" cy="15" r="4.5"/><path d="M11.2 11.8L20 3M16.5 6.5l2.5 2.5M14 9l2 2"/></svg>';
  var PODS_ICON = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 3a4 4 0 0 0-4 4 4 4 0 0 0 3 3.87V19a1.5 1.5 0 0 0 3 0V7a4 4 0 0 0-2-4z"/>' +
    '<path d="M16 3a4 4 0 0 1 4 4 4 4 0 0 1-3 3.87V19a1.5 1.5 0 0 1-3 0V7a4 4 0 0 1 2-4z"/></svg>';
  var tools_ = null;
  function drawTools() {
    if (!tools_) {
      tools_ = document.createElement("div");
      tools_.id = "demo-tools";
      tools_.innerHTML = '<button type="button" id="demo-claude" tabindex="-1">' + KEY_ICON + '<span class="label"></span></button>' +
        '<button type="button" id="demo-airpods" tabindex="-1">' + PODS_ICON + '<span class="dot"></span><span class="label"></span></button>';
      $("box").appendChild(tools_);
      tools_.addEventListener("mousedown", function (e) { e.preventDefault(); });       // typing stays in the box
      $("demo-claude").addEventListener("click", claudeButton);
      $("demo-airpods").addEventListener("click", talkButton);
      new ResizeObserver(function () { requestAnimationFrame(placeTools); }).observe($("box"));
    }
    var c = $("demo-claude"), p = $("demo-airpods");
    c.classList.toggle("on", !!claude.key);
    label(c, claude.key ? ["", "Claude", " connected"] : ["Connect ", "Claude", ""]);
    c.title = claude.key ? "Claude answers, with your API key. Click to disconnect it." : "Real Claude here, with your own API key";
    p.classList.toggle("on", ears.on);
    p.classList.toggle("listening", ears.on);
    label(p, ears.on ? (ears.device ? ["", "AirPods", " on"] : ["", "Listening", ""]) : ["Connect ", "AirPods", ""]);
    p.title = ears.on ? "Listening. Click to stop." : "Talk through your AirPods, and hear the answers";
    placeTools();
  }
  // A button's words: [before, the name, after]; with little room only the name shows (.short).
  function label(b, w) {
    var l = b.querySelector(".label");
    l.textContent = "";
    if (w[0]) l.appendChild(span("long", w[0]));
    l.appendChild(document.createTextNode(w[1]));
    if (w[2]) l.appendChild(span("long", w[2]));
    b.setAttribute("aria-label", w.join(""));
  }
  function span(cls, text) { var s = document.createElement("span"); s.className = cls; s.textContent = text; return s; }
  // Beside the box when there's room: side by side, or one above the other, or just their icons. Otherwise inside
  // it, before the arrow.
  function placeTools() {
    if (!tools_) return;
    var box = $("box"), rem = parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
    tools_.classList.remove("stack", "short", "compact", "inside");
    if (tools_.parentNode !== box) box.appendChild(tools_);
    var b = box.getBoundingClientRect(), limit = $("composer").getBoundingClientRect().right;
    var t = $("toggle-bottom"), tr = t.getBoundingClientRect();
    if (tr.width && getComputedStyle(t).display !== "none") limit = Math.min(limit, tr.left);
    var room = limit - b.right - 1.25 * rem;
    if (tools_.offsetWidth <= room) return;
    tools_.classList.add("stack");
    if (tools_.offsetWidth <= room) return;
    tools_.classList.add("short");
    if (tools_.offsetWidth <= room) return;
    tools_.classList.remove("stack", "short");
    tools_.classList.add("compact");
    if (tools_.offsetWidth <= room) return;
    tools_.classList.add("inside");
    $("row").insertBefore(tools_, $("send"));
  }

  // ------------------------------------------------------------------ what the page says
  function handle(msg) {
    switch (msg.type) {
      case "ready":
        call("openInPage", true);
        layout();
        call("settings", "", "");
        if (store("chatcomputer-demo-panels")) call("panels", store("chatcomputer-demo-panels"));
        call("doneList", DONE.map(function (e) {
          var pins = (store("chatcomputer-demo-pins") || {}).done || {}, o = {};
          for (var k in e) o[k] = e[k];
          if (pins[e.id]) o.pinned = pins[e.id];
          return o;
        }));
        call("messagesList", MESSAGES.map(withPin), ACCOUNTS);
        drawTools();
        youtube();
        break;
      case "send": ask(String(msg.text || "")); break;
      case "drafting": drafting = !!msg.on; break;
      case "hole": hole = { x: msg.x, y: msg.y, w: msg.w, h: msg.h }; place(); break;
      case "close_content": close(); break;
      case "expand": expanded = !expanded; place(); break;
      case "open_message": openMessage(msg.id); break;
      case "pin": {
        var pins = store("chatcomputer-demo-pins") || {};
        pins[msg.list] = pins[msg.list] || {};
        if (msg.on) pins[msg.list][msg.id] = now(); else delete pins[msg.list][msg.id];
        store("chatcomputer-demo-pins", pins);
        break;
      }
      case "goto": call("earlier", EARLIER, msg.id, msg.ts); break;
      case "search": call("searchResults", msg.n, msg.q || "", search(msg.q || ""),
                          search(msg.q || "").length ? "" : "Nothing in the demo's files matches \u201c" + (msg.q || "") + "\u201d.");
        break;
      case "open_file": if (PAGES[msg.path]) show({ html: PAGES[msg.path]() }); break;
      case "text_scale": setScale(scale * (+msg.factor || 1)); break;
      case "panels": store("chatcomputer-demo-panels", { left: !!msg.left, right: !!msg.right, bottom: !!msg.bottom }); break;
      case "add_account":
        call("accountForm", { id: "accounts", title: "Connect an account", ok: "OK", text: [
          "On Chat Computer, Gmail, Outlook, iCloud Mail, Telegram, WhatsApp, X, Instagram, Facebook, TikTok and LinkedIn " +
          "connect here, and their messages come into Messages, newest first. Sign-ins stay on your computer.",
          "The demo can't connect accounts."] });
        break;
      case "account": {
        var values = msg.values || {};
        if (msg.form === "claude") { connectClaude(String(values.key || "").trim()); break; }
        if (msg.form === "claude-off") { forget(); call("accountForm", null); did("Disconnected Claude"); break; }
        if (msg.form === "talk") { call("accountForm", null); startTalking(); break; }
        call("accountForm", null);
        break;
      }
      case "account_cancel":
        call("accountForm", null);
        break;
      default: break;            // keys, keyboard: nothing to do in a browser
    }
  }
})();
