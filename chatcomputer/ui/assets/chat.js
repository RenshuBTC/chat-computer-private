/* The conversation screen: the messages (yours on the right, Chat Computer's on the left), the message box at the bottom,
   Messages at the left and Actions at the right. Python calls window.CO.*; the page tells Python
   (postMessage) what was typed, what was tapped, and where it leaves room for what Chat Computer opens. */
(function () {
  "use strict";
  function $(id) { return document.getElementById(id); }
  var log = $("log"), live = $("live"), liveText = $("live-text"), body = document.body;
  var input = $("input"), composer = $("composer"), column = $("column");
  var doneList = $("done-list"), donePinned = $("done-pinned");
  var msgList = $("messages-list"), msgPinned = $("messages-pinned");
  var page = document.scrollingElement || document.documentElement;
  var rootStyle = document.documentElement.style;
  var MAX_ITEMS = 400;                // messages, and the row under each
  var ai = {};          // id -> {el, md, text, raf, cut, tail}
  var mode = "chat";    // "content" while something is open in the middle (a website, a video, a message...)
  var full = false;     // no sidebars, the middle filling the screen (Escape); to start with, Messages and Actions show
  var shown = { left: true, right: true };    // the sidebars (see "the panels' switches")
  // A touchscreen alone (a phone or a tablet, no mouse): its keyboard comes up only for the box, and goes down after
  // sending. A phone (a screen under 600px across, either way round) is laid out as the Claude app is there
  // (chat.css: body.phone): nothing zooms, and holding a message brings up what can be done with it.
  var touchQuery = window.matchMedia ? window.matchMedia("(hover: none) and (pointer: coarse)") : { matches: false };
  var phone = false;
  function touchOnly() { return touchQuery.matches; }
  function fitPhone() {
    var s = window.screen || {}, across = Math.min(s.width || window.innerWidth, s.height || window.innerHeight);
    phone = touchOnly() && across < 600;
    body.classList.toggle("phone", phone);
  }
  fitPhone();
  function post(msg) {
    try { window.webkit.messageHandlers.chatcomputer.postMessage(JSON.stringify(msg)); } catch (e) {}
  }
  function now() { return Date.now() / 1000; }
  function remPx() { return parseFloat(getComputedStyle(document.documentElement).fontSize) || 24; }

  // As in the Claude app: the conversation starts at the top of the screen, and what you send goes to the top, with
  // the answer coming in under it, so it's read without scrolling down; earlier messages are above (scroll up).
  // Under the latest exchange there's room for that (#room: the screen's height less what's under your message), so
  // the page's end is your message at the top. While you're at the end, new words keep it there: an answer longer
  // than the screen is followed down as it comes. Scrolled up to read something earlier, it stays where you are
  // until you scroll back down, say something or type.
  var stick = true, pinning = 0, room = $("room");
  function atBottom() { return page.scrollHeight - page.scrollTop - page.clientHeight < 4; }
  window.addEventListener("scroll", function () { if (mode === "chat" && Date.now() > pinning) stick = atBottom(); }, { passive: true });
  window.addEventListener("resize", function () { fitPhone(); closeHold(); fitPanel(); follow(); reportHole(); });
  function latestYou() { var y = log.querySelectorAll(":scope > .you"); return y.length ? y[y.length - 1] : null; }
  // Where something is laid out on the page (offsets: not moved by a message's rising into place).
  function pageTop(e) { var t = 0; for (; e; e = e.offsetParent) t += e.offsetTop; return t; }
  // The page scrolled to your latest message: a little above it (its scroll-margin-top), but never so far up that
  // any of what came before shows at the top of the screen (the answer before it, or that answer's row of buttons).
  function pinTop(you) {
    var at = pageTop(you), top = at - (parseFloat(getComputedStyle(you).scrollMarginTop) || 0);
    for (var b = you.previousElementSibling; b; b = b.previousElementSibling) {
      if (!b.getClientRects().length) continue;                               // (not shown)
      return Math.max(top, Math.min(at, pageTop(b) + b.offsetHeight));
    }
    return top;
  }
  function makeRoom() {
    var you = latestYou(), was = room.offsetHeight, need = 0;
    if (you && mode === "chat") {
      var top = pinTop(you);                                                     // the page scrolled to it
      var end = column.offsetTop + column.offsetHeight - was;                    // where the conversation ends
      need = Math.max(0, Math.ceil(top + page.clientHeight - end));
    }
    if (need !== was) room.style.height = need + "px";
  }
  function follow() {
    makeRoom();
    // (Not while a message is held: it stays where it is, under what can be done with it.)
    if (stick && mode === "chat" && Date.now() > pinning && !holdMsg) page.scrollTop = page.scrollHeight;
  }
  function jumpDown() { stick = true; pinning = 0; follow(); }
  // What you've just sent, to the top of the screen (gliding there, unless the system asks for less motion).
  function pin() {
    stick = true;
    makeRoom();
    var glide = !(window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches);
    pinning = glide ? Date.now() + 600 : 0;
    page.scrollTo({ top: page.scrollHeight - page.clientHeight, behavior: glide ? "smooth" : "auto" });
  }

  // At most MAX_ITEMS messages on the page; the oldest go, but only while you're at the bottom (not while you
  // read back through earlier ones).
  function trim() {
    if (stick) while (log.children.length > MAX_ITEMS) log.removeChild(log.firstChild);
    while (log.firstElementChild && log.firstElementChild.classList.contains("acts")) log.removeChild(log.firstElementChild);
    marks();
  }
  // Whether anything is said, and whether you've said anything (the word logo shows until you have).
  function marks() {
    body.classList.toggle("has-log", log.children.length > 0);
    body.classList.toggle("has-you", !!log.querySelector(":scope > .you"));
  }
  function el(cls, text) {
    var d = document.createElement("div");
    d.className = cls;
    if (text != null) d.textContent = text;
    return d;
  }
  function add(parent, html) {
    var t = document.createElement("template");
    t.innerHTML = html;
    var nodes = Array.prototype.slice.call(t.content.childNodes);
    parent.appendChild(t.content);
    return nodes;
  }
  function ensureAI(id) {
    var m = ai[id];
    if (m) return m;
    var wrap = el("ai thinking enter");
    wrap.dataset.id = id;
    wrap.dataset.ts = String(now());
    var md = el("md");
    wrap.appendChild(md);
    log.appendChild(wrap);
    trim();
    m = ai[id] = { el: wrap, md: md, text: "", raf: 0, cut: 0, tail: [], done: 0, drawn: 0, pieces: [], base: 0 };
    return m;
  }
  // ------------------------------------------------------------------ the opening
  // A conversation starts with the word logo in the middle of the screen (body.intro), and it stays there until
  // something's said (body.has-log). Under it, one line: a greeting, as warm and personal as the Claude app's, a
  // different one each time. Theirs (as claude.ai's code has them) are by the time of day (from 6 in the morning, 12,
  // 5 in the evening, 9 at night), some by the day of the week, with the name you go by ("Good evening, Sam"); here
  // the same. The name comes from CO.settings (on a Chat Computer the one it was set up with; "call me ..." changes
  // it). [with the name, without one (null: not without), from, until (hours), days (0 Sunday) or null, only where a
  // model answers]
  var GREETING = true;
  var GREETINGS = [
    ["Good morning, {name}", "Good morning", 6, 12],
    ["Welcome, {name}", "Welcome", 6, 12],
    ["Hey there, {name}", "Hey there", 6, 12],
    ["Happy Monday, {name}", "Happy Monday", 6, 12, [1]],
    ["Happy Tuesday, {name}", "Happy Tuesday", 6, 12, [2]],
    ["Happy Wednesday, {name}", "Happy Wednesday", 6, 12, [3]],
    ["Happy Thursday, {name}", "Happy Thursday", 6, 12, [4]],
    ["Happy Friday, {name}", "Happy Friday", 6, 12, [5]],
    ["Hello to that Friday feeling, {name}", "Hello to that Friday feeling", 6, 12, [5]],
    ["Welcome to the weekend, {name}", "Welcome to the weekend", 6, 12, [6, 0]],
    ["Happy Saturday, {name}", "Happy Saturday", 6, 12, [6]],
    ["What's on your mind, {name}?", "What's on your mind?", 6, 12, [6, 0]],
    ["Sunday session, {name}?", "Sunday session?", 6, 12, [0]],
    ["Happy Sunday, {name}", "Happy Sunday", 6, 12, [0]],
    ["Coffee and computer time, {name}?", "Coffee and computer time?", 6, 12, null, true],
    ["Good afternoon, {name}", "Good afternoon", 12, 17],
    ["Hi {name}, how are you?", "Hi, how are you?", 12, 17],
    ["What's new, {name}?", "What's new?", 12, 17],
    ["Back at it, {name}?", "Back at it?", 12, 17],
    ["Look who's back: {name}!", null, 17, 21],
    ["Good evening, {name}", "Good evening", 17, 21],
    ["How was your day, {name}?", "How was your day?", 17, 21],
    ["How's it going, {name}?", "How's it going?", 21, 6],
    ["What's on your mind tonight, {name}?", "What's on your mind tonight?", 21, 6],
    ["Hello, night owl", "Hello, night owl", 21, 6]
  ];
  var who = "", modelAnswers = false;             // the name you go by, and whether a model answers (CO.settings)
  // The greetings: for this hour and this day, with the name or without it (and, without, one of their own:
  // "Greetings, whoever you are").
  function greetings(at) {
    var h = at.getHours(), day = at.getDay(), out = [];
    GREETINGS.forEach(function (g) {
      if (!(g[2] < g[3] ? h >= g[2] && h < g[3] : h >= g[2] || h < g[3])) return;
      if ((g[4] && g[4].indexOf(day) < 0) || (g[5] && !modelAnswers)) return;
      out.push(named(g));
    });
    if (!who) out.push("Greetings, whoever you are");
    return out.filter(Boolean);
  }
  // A greeting with the name in it, or as it's said without one ("" where it can't be).
  function named(g) { return who ? g[0].replace("{name}", who) : g[1] || ""; }
  // And, as theirs are for the time of day, greetings for where you are and its weather now (CO.here: the place,
  // your time zone's city or where you said you live; the weather there from Open-Meteo, as a WMO weather code, °C
  // and whether it's light there), from the world to your city: "Hello, Singapore! How was your day, Sam?".
  // {part}: morning (from 6), afternoon (12), evening (5 pm) or night (9 pm).
  var here = { place: "", code: null, temp: null, day: null, pending: false };
  var AROUND = {
    morning: [["Good morning in {place}, {name}", "Good morning in {place}"],
              ["Hello, {place}! How's your morning, {name}?", "Hello, {place}! How's your morning?"]],
    afternoon: [["Good afternoon in {place}, {name}", "Good afternoon in {place}"],
                ["Hello, {place}! How's your day going, {name}?", "Hello, {place}! How's your day going?"]],
    evening: [["Good evening in {place}, {name}", "Good evening in {place}"],
              ["Hello, {place}! How was your day, {name}?", "Hello, {place}! How was your day?"]],
    night: [["Hello, {place}! Still up, {name}?", "Hello, {place}! Still up?"],
            ["How's the night in {place}, {name}?", "How's the night in {place}?"]]
  };
  // [with the name, without, "day" or "night" for only then]: by the weather (weatherOf), and the heat or cold.
  var WEATHER = {
    storm: [["Stay safe in stormy {place}, {name}", "Stay safe in stormy {place}"],
            ["Hello, {place}! Quite the storm out there, {name}", "Hello, {place}! Quite the storm out there"]],
    snow: [["Stay warm in snowy {place}, {name}", "Stay warm in snowy {place}"],
           ["Hello, snowy {place}! Snow day, {name}?", "Hello, snowy {place}! Snow day?"]],
    rain: [["Stay dry in rainy {place}, {name}", "Stay dry in rainy {place}"],
           ["Hello, rainy {place}! A cozy {part} in, {name}?", "Hello, rainy {place}! A cozy {part} in?"]],
    fog: [["Hello, foggy {place}! Mind how you go, {name}", "Hello, foggy {place}! Mind how you go"]],
    clear: [["Hello, sunny {place}! Lovely {part}, {name}", "Hello, sunny {place}! Lovely {part}", "day"],
            ["Blue skies in {place} today, {name}", "Blue skies in {place} today", "day"],
            ["Hello, {place}! Clear skies tonight, {name}", "Hello, {place}! Clear skies tonight", "night"]],
    cloud: [["Hello, cloudy {place}! How's your {part}, {name}?", "Hello, cloudy {place}! How's your {part}?"],
            ["Grey skies in {place} today, {name}", "Grey skies in {place} today", "day"]],
    hot: [["Stay cool in the heat of {place}, {name}", "Stay cool in the heat of {place}"],
          ["Hello, {place}! A hot one today, {name}", "Hello, {place}! A hot one today", "day"]],
    warm: [["Hello, {place}! A warm night, {name}", "Hello, {place}! A warm night", "night"]],
    cold: [["Bundle up in chilly {place}, {name}", "Bundle up in chilly {place}"],
           ["Hello, {place}! A chilly {part}, {name}", "Hello, {place}! A chilly {part}"]]
  };
  function partOf(h) { return h >= 6 && h < 12 ? "morning" : h >= 12 && h < 17 ? "afternoon" : h >= 17 && h < 21 ? "evening" : "night"; }
  function weatherOf(code) {
    return code >= 95 ? "storm" : (code >= 71 && code <= 77) || code === 85 || code === 86 ? "snow" :
      (code >= 51 && code <= 67) || (code >= 80 && code <= 82) ? "rain" : code === 45 || code === 48 ? "fog" :
      code === 0 || code === 1 ? "clear" : code === 2 || code === 3 ? "cloud" : "";
  }
  function hereGreetings(at) {
    if (!here.place) return [];
    var part = partOf(at.getHours()), lines = AROUND[part].slice(), w = here.code;
    if (typeof w === "number") {
      var light = here.day !== false, t = here.temp;
      lines = lines.concat(WEATHER[weatherOf(w)] || []);
      if (typeof t === "number") lines = lines.concat(t >= 32 ? WEATHER.hot : t <= 5 ? WEATHER.cold : t >= 26 && !light ? WEATHER.warm : []);
      // "day" while it's light; "night" once the light's gone in the evening or at night (not before dawn).
      var late = part === "evening" || part === "night";
      lines = lines.filter(function (l) { return !l[2] || (l[2] === "day" ? light : !light && late); });
    }
    return lines.map(function (l) {
      return named(l).replace("{place}", here.place).replace("{part}", part);
    }).filter(Boolean);
  }
  // One of them, never the one shown last time (kept in this browser, where it can be): half the time one for where
  // you are, where there's one.
  function greeting() {
    var at = new Date(), time = greetings(at), local = hereGreetings(at), last = "";
    try { last = localStorage.getItem("chatcomputer-greeting") || ""; } catch (e) {}
    var from = local.length && Math.random() < 0.5 ? local : time;
    var fresh = from.filter(function (g) { return g !== last; });
    if (fresh.length) from = fresh;
    var pick = from[Math.floor(Math.random() * from.length)];
    try { localStorage.setItem("chatcomputer-greeting", pick); } catch (e) {}
    return pick;
  }
  // The logo, and its greeting chosen once the name and where you are have come (or a moment's gone by: while the
  // weather is being looked up, CO.here's pending, up to a second and a half), fading in. Chosen again if the name
  // comes later (CO.settings), or where you are, while it's still showing.
  var intro = [], greeted = { who: null, place: null };
  function introLater(ms, f) { intro.push(setTimeout(f, ms)); }
  function showGreeting() {
    var g = $("greeting");
    if (!g || !GREETING || body.classList.contains("has-log")) return;
    greeted = { who: who, place: here.place };
    g.textContent = greeting();
    g.classList.add("shown");
  }
  function greet() {
    intro.forEach(clearTimeout);
    intro = [];
    var g = $("greeting");
    if (g) { g.textContent = ""; g.classList.remove("shown"); }
    if (log.children.length) { body.classList.remove("intro"); return; }
    body.classList.add("intro");
    var waited = 0;
    introLater(250, function pick() {
      if (here.pending && waited < 1500) { waited += 100; introLater(100, pick); return; }
      showGreeting();
    });
  }
  function regreet() {
    var g = $("greeting");
    if (g && g.classList.contains("shown") && body.classList.contains("intro") && (greeted.who !== who || greeted.place !== here.place) &&
        !here.pending) showGreeting();
  }
  // A message from the transcript (the start, or an earlier day put back).
  function message(it) {
    var m;
    if (it.role === "user") m = el("you", it.text);
    else if (it.role === "assistant") {
      m = el("ai");
      var md = el("md");
      md.innerHTML = window.renderMarkdown(it.text);
      m.appendChild(md);
    } else m = el("gap", it.text);
    if (it.id) m.dataset.id = it.id;
    if (it.ts) m.dataset.ts = String(it.ts);
    m._said = it.text;
    return m;
  }

  // Where the finished part of Chat Computer's words ends: after the last blank line that isn't inside a code block.
  // Every block md.js draws (a paragraph, a list, a table, a quote) ends at a blank line, so what comes before it
  // can't change any more: it's drawn once, and only the part still being written is drawn again as words stream
  // in. (Drawing the whole message again on every word cost more the longer it got.)
  var OPEN = /^\s*```(\w*)\s*$/, CLOSE = /^\s*```\s*$/, BLANK = /^\s*$/;
  function finishedUpTo(text, from) {
    var i = from, end = from, fence = false;
    while (i < text.length) {
      var nl = text.indexOf("\n", i);
      if (nl < 0) break;                   // the last line may still be growing
      var line = text.slice(i, nl);
      if (fence) { if (CLOSE.test(line)) fence = false; }
      else if (OPEN.test(line)) fence = true;
      else if (BLANK.test(line)) end = nl + 1;
      i = nl + 1;
    }
    return end;
  }
  // Words fade in as they come, as in the Claude app (measured there: each new piece of an answer goes from nothing
  // to black in 400 ms, at an even pace). paint() keeps how much text it had drawn and when each piece came
  // (m.pieces: [where it ends in the drawn text, when]); what it draws anew (a block just finished, or the part still
  // being written, drawn again each time) has its young text in spans that fade, carried on from where they were (a
  // negative delay), so nothing flickers. Not when the system asks for less motion.
  var FADE = 400, calm = !!(window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  function textOf(nodes) { var n = 0; for (var i = 0; i < nodes.length; i++) n += nodes[i].textContent.length; return n; }
  function fadeIn(m, nodes, start, at) {
    var young = [], prev = m.base;
    m.pieces.forEach(function (p) { if (at - p[1] < FADE) young.push([prev, p[0], p[1]]); prev = p[0]; });
    if (calm || !young.length) return;
    var pos = start;
    nodes.forEach(function (n) {
      var texts = [], w, t;
      if (n.nodeType === 3) texts.push(n);
      else { w = document.createTreeWalker(n, NodeFilter.SHOW_TEXT); while ((t = w.nextNode())) texts.push(t); }
      texts.forEach(function (tn) {
        var a = pos, b = pos + tn.data.length, cuts = [];
        pos = b;
        young.forEach(function (y) { var s0 = Math.max(a, y[0]), e0 = Math.min(b, y[1]); if (s0 < e0) cuts.push([s0 - a, e0 - a, y[2]]); });
        for (var k = cuts.length - 1; k >= 0; k--) {      // from the end, so tn keeps the text before each cut
          var c = cuts[k];
          if (c[1] < tn.data.length) tn.splitText(c[1]);
          var part = c[0] > 0 ? tn.splitText(c[0]) : tn, span = document.createElement("span");
          span.className = "fresh";
          span.style.animationDelay = -Math.round(at - c[2]) + "ms";
          part.parentNode.insertBefore(span, part);
          span.appendChild(part);
        }
      });
    });
  }
  function paint(m) {
    m.raf = 0;
    for (var i = 0; i < m.tail.length; i++) if (m.tail[i].parentNode === m.md) m.md.removeChild(m.tail[i]);
    m.tail = [];
    var fin = [], cut = finishedUpTo(m.text, m.cut);
    if (cut > m.cut) { fin = add(m.md, window.renderMarkdown(m.text.slice(m.cut, cut))); m.cut = cut; }
    var rest = m.text.slice(m.cut);
    if (rest) m.tail = add(m.md, window.renderMarkdown(rest));
    var at = performance.now(), finLen = textOf(fin), drawn = m.done + finLen + textOf(m.tail);
    if (drawn > m.drawn) m.pieces.push([drawn, at]);
    else { m.pieces.forEach(function (p) { p[0] = Math.min(p[0], drawn); }); m.base = Math.min(m.base, drawn); }
    m.drawn = drawn;
    while (m.pieces.length && at - m.pieces[0][1] >= FADE) m.base = m.pieces.shift()[0];      // faded in: plain text
    fadeIn(m, fin, m.done, at);
    fadeIn(m, m.tail, m.done + finLen, at);
    m.done += finLen;
    if (m.text) m.el.classList.remove("thinking");
    follow();
  }
  // ------------------------------------------------------------------ under each message: what can be done with it
  // As in the Claude app, and different under each kind: under Chat Computer's words, copy, read them out, good, bad
  // and ask again, then when (shown under the pointer); under yours, at the right, when, ask again, edit and copy. A
  // row shows under the pointer, under the latest answer, and under a message tapped on a touchscreen.
  var ACT_ICON = {
    copy: '<svg class="i-copy" viewBox="0 0 24 24" aria-hidden="true"><rect x="8.5" y="8.5" width="11.5" height="11.5" rx="2.5"/>' +
          '<path d="M15.5 8.5V6.5A2.5 2.5 0 0 0 13 4H6.5A2.5 2.5 0 0 0 4 6.5V13a2.5 2.5 0 0 0 2.5 2.5h2"/></svg>' +
          '<svg class="i-done" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg>',
    speak: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 9.5h3.5L12 6v12l-4.5-3.5H4z"/><path d="M15.5 9.5a3.5 3.5 0 0 1 0 5M18 7a7 7 0 0 1 0 10"/></svg>',
    good: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7.5 11v9H4v-9zM7.5 11l3.6-7a2.2 2.2 0 0 1 2.3 2.6L12.8 10h5.4a2 2 0 0 1 2 2.4l-1.3 6A2 2 0 0 1 16.9 20H7.5"/></svg>',
    bad: '<svg viewBox="0 0 24 24" aria-hidden="true"><g transform="rotate(180 12 12)"><path d="M7.5 11v9H4v-9zM7.5 11l3.6-7a2.2 2.2 0 0 1 2.3 2.6L12.8 10h5.4a2 2 0 0 1 2 2.4l-1.3 6A2 2 0 0 1 16.9 20H7.5"/></g></svg>',
    retry: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3"/><path d="M19.5 4.5v4.2h-4.2"/></svg>',
    edit: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 19h3.5L18.7 8.8a2.1 2.1 0 0 0-3-3L5.5 16v3z"/><path d="M14.5 7l2.5 2.5"/></svg>'
  };
  var ACT_NAME = { copy: "Copy", speak: "Read aloud", good: "Good answer", bad: "Bad answer", retry: "Ask again", edit: "Edit" };
  function ago(ts) {
    var s = Math.max(0, now() - ts), m = Math.floor(s / 60), h = Math.floor(m / 60);
    if (m < 1) return "just now";
    if (m < 60) return m === 1 ? "1 minute ago" : m + " minutes ago";
    if (new Date(ts * 1000).toDateString() === new Date().toDateString()) return h === 1 ? "1 hour ago" : h + " hours ago";
    var day = dayLabel(ts);
    return day === "Yesterday" ? "yesterday" : day;
  }
  function addActs(msg) {
    if (!msg || !msg.parentNode || !/\b(you|ai)\b/.test(msg.className)) return null;
    var next = msg.nextElementSibling;
    if (next && next.classList.contains("acts")) return next;
    var mine = msg.classList.contains("you"), row = el("acts " + (mine ? "mine" : "theirs"));
    row.setAttribute("aria-label", mine ? "Your message" : "The answer");
    (mine ? ["when", "retry", "edit", "copy"] : ["copy", "speak", "good", "bad", "retry", "when"]).forEach(function (k) {
      if (k === "when") {
        var w = el("when");
        w.dataset.ts = msg.dataset.ts || String(now());
        w.textContent = ago(+w.dataset.ts);
        row.appendChild(w);
        return;
      }
      var b = document.createElement("button");
      b.type = "button";
      b.className = "act";
      b.dataset.act = k;
      b.tabIndex = -1;
      b.title = ACT_NAME[k];
      b.setAttribute("aria-label", ACT_NAME[k]);
      b.innerHTML = ACT_ICON[k];
      row.appendChild(b);
    });
    msg.parentNode.insertBefore(row, msg.nextSibling);
    return row;
  }
  // The latest answer's row stays shown.
  function lastActs(row) {
    Array.prototype.forEach.call(log.querySelectorAll(".acts.last"), function (r) { r.classList.remove("last"); });
    if (row) row.classList.add("last");
  }
  function saidIn(msg) {
    if (msg._said != null) return String(msg._said).trim();
    return (msg.classList.contains("you") ? msg.textContent : (msg.querySelector(".md") || msg).innerText).trim();
  }
  function promptFor(msg) {
    for (var p = msg.previousElementSibling; p; p = p.previousElementSibling) if (p.classList.contains("you")) return saidIn(p);
    return "";
  }
  function copyText(text, b) {
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(text).catch(function () { copyOld(text); });
      else copyOld(text);
    } catch (e) { copyOld(text); }
    post({ type: "copy", text: text });            // on a Chat Computer the window puts it on the clipboard too
    if (!b) return;
    b.classList.add("did");
    setTimeout(function () { b.classList.remove("did"); }, 1500);
  }
  function copyOld(text) {
    var t = document.createElement("textarea");
    t.value = text;
    t.setAttribute("readonly", "");
    t.style.cssText = "position:fixed;left:-9999px;top:0";
    document.body.appendChild(t);
    t.select();
    try { t.setSelectionRange(0, text.length); } catch (e) {}      // (Safari on an iPhone selects only this way)
    try { document.execCommand("copy"); } catch (e) {}
    t.remove();
    focusBox();
  }
  function act(b) {
    var row = b.parentNode, msg = row.previousElementSibling, k = b.dataset.act;
    if (!msg) return;
    var text = saidIn(msg);
    if (k === "copy") copyText(text, b);
    else if (k === "speak") post({ type: "speak", text: text });
    else if (k === "good" || k === "bad") {
      var on = !b.classList.contains("on");
      Array.prototype.forEach.call(row.querySelectorAll(".act.on"), function (x) { x.classList.remove("on"); });
      b.classList.toggle("on", on);
      post({ type: "feedback", id: msg.dataset.id || "", good: k === "good", on: on, text: text, prompt: promptFor(msg) });
    } else if (k === "retry") {
      var again = msg.classList.contains("you") ? text : promptFor(msg);
      if (again) { stick = true; post({ type: "send", text: again }); }
    } else if (k === "edit") editText(text);
  }
  // Your words back in the box, to change and send again (a touchscreen's keyboard comes up for them).
  function editText(text) {
    input.value = text;
    fitInput();
    focusBox(true);
    try { input.setSelectionRange(text.length, text.length); } catch (e) {}
  }
  var coarse = !!(window.matchMedia && window.matchMedia("(any-pointer: coarse)").matches);
  log.addEventListener("click", function (e) {
    if (holdMsg) { closeHold(); return; }          // a tap on the held message puts its menu away, as one elsewhere does
    var b = e.target.closest ? e.target.closest(".acts .act") : null;
    if (b) { act(b); return; }
    var msg = coarse && e.target.closest ? e.target.closest("#log > .you, #log > .ai") : null;
    var row = msg && msg.nextElementSibling;
    if (row && row.classList.contains("acts")) {
      var on = !row.classList.contains("shown");
      Array.prototype.forEach.call(log.querySelectorAll(".acts.shown"), function (r) { r.classList.remove("shown"); });
      row.classList.toggle("shown", on);
    }
  });

  function showLive(text) {
    liveText.textContent = text || "";
    body.classList.toggle("hearing", !!text);
  }

  // ------------------------------------------------------------------ something open in the middle
  // A website, a video, a map, a conversation from Messages or a file fills the middle, from the top of the screen
  // down to the message box, in a frame with its name and a close button (x) at the top. The conversation with Chat
  // Computer is put away until it's closed (with the x, or by asking). The window lays what's open in the frame,
  // under its bar: this says where ("hole"), whenever it changes.
  var lastHole = "";
  function reportHole() {
    if (mode !== "content") return;
    var rem = remPx(), gap = Math.round(0.75 * rem);
    // Between the sidebars, that much from each (from where each one ends when it shows: switched off, what's open
    // stays where it was). Without them (a narrow window, or the middle filling the screen), the width of the
    // screen, less that space.
    var sides = body.classList.contains("panel") && !full, cs = getComputedStyle(body);
    var left = sides ? Math.round(parseFloat(cs.paddingLeft) || 0) + gap : gap;
    var right = window.innerWidth - (sides ? Math.round(parseFloat(cs.paddingRight) || 0) + gap : gap);
    // Down to the message box (or the line above it while asleep), with that space again between them.
    var bottom = Math.round(composer.getBoundingClientRect().top + (parseFloat(getComputedStyle(composer).paddingTop) || 0)) - gap;
    // "/" searching: its list goes in front of what's open. Where the window lays what's open over this page (on
    // a Chat Computer), nothing of the page can show over it, so it ends above the list instead.
    var list = $("results").getBoundingClientRect();
    if (searching && !openInPage && list.height) bottom = Math.min(bottom, Math.round(list.top) - gap);
    // From the top of the screen.
    var top = gap;
    var f = $("frame").style, bar = $("frame-bar").offsetHeight;
    f.left = left + "px"; f.top = top + "px";
    f.width = Math.max(0, right - left) + "px"; f.height = Math.max(0, bottom - top) + "px";
    // What's open: under the bar, inside the frame's border and clear of its bottom corners' curve.
    var inset = 2 + Math.round(0.4 * rem);
    var r = { x: left + inset, y: top + 2 + bar, w: Math.max(0, right - left - 2 * inset), h: Math.max(0, bottom - top - 2 - bar - inset) };
    var key = r.x + "," + r.y + "," + r.w + "," + r.h;
    if (key !== lastHole) { lastHole = key; post({ type: "hole", x: r.x, y: r.y, w: r.w, h: r.h }); }
  }
  function setMode(m, title) {
    mode = m;
    body.classList.toggle("content", m === "content");
    if (m === "content") { $("frame-title").textContent = title || ""; reportHole(); }
    else { lastHole = ""; jumpDown(); }
  }
  $("frame-close").addEventListener("click", function () { post({ type: "close_content", pause: true }); });
  // To start with, Messages and Actions show at the middle's sides (where there's room for them), as the person
  // asked; only their switches put them away and bring them back (not Escape: that's for what's in the middle).
  // toggleFull puts them both away, the middle filling the screen, for a page's host that asks.
  function toggleFull(on) {
    full = on == null ? !full : !!on;
    body.classList.toggle("full", full);
    follow();
    reportHole();
  }
  // Whether what's open is drawn in this same page (the browser demo's frame), so the page's own "/" list can go
  // over it; on a Chat Computer the window lays it over the page.
  var openInPage = false;

  // ------------------------------------------------------------------ the message box
  // A real text box, like the Claude app's: the cursor, arrow keys, selecting, pasting and input methods all work as
  // anywhere else. Enter sends; Shift+Enter starts a new line. Page Up and Down scroll the conversation, and so do
  // the arrow keys, Home and End while the box is empty. It grows with what's typed.
  function fitInput() {
    input.style.height = "auto";
    input.style.height = input.scrollHeight + "px";
    body.classList.toggle("drafting", input.value.trim() !== "" || !!(attached && attached.length));
    body.classList.toggle("typed", input.value !== "");
    checkSearch();
  }
  function sendNow() {
    if (searching) { openResult(); return; }        // "/": Enter opens what's chosen
    var text = input.value.trim();
    if (!text && !attached.length) return;
    input.value = "";
    fitInput();
    post(attached.length ? { type: "send", text: text, files: attached } : { type: "send", text: text });
    attached = [];
    drawAttached();
    stick = true;                    // it goes to the top of the screen when it's shown (CO.user)
    doneTyping();
  }
  // Sent: on a touchscreen the keyboard goes down, as in ChatGPT's app, for the answer to be seen; touching the box
  // brings it back.
  function doneTyping() {
    if (!touchOnly()) return;
    if (document.activeElement === input) input.blur();
    post({ type: "keyboard", show: false });
  }
  var SCROLL_ALWAYS = { PageUp: "page_up", PageDown: "page_down" };
  var SCROLL_EMPTY = { ArrowUp: "up", ArrowDown: "down", Home: "top", End: "bottom" };
  input.addEventListener("input", fitInput);
  input.addEventListener("keydown", function (e) {
    if (e.isComposing || e.keyCode === 229) return;        // an input method is still putting a character together
    var k = e.key;
    if (searchKeys(e)) return;
    if (k === "Enter" && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) { e.preventDefault(); sendNow(); return; }
    var how = SCROLL_ALWAYS[k] || (input.value === "" ? SCROLL_EMPTY[k] : null);
    if (how) { e.preventDefault(); window.CO.scroll(how); return; }
    if (k.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey) measure(e.timeStamp);
  });
  $("send").addEventListener("click", sendNow);
  // ------------------------------------------------------------------ + and the microphone
  // Under what's typed, as in the Claude app, each doing just its one thing (no menus): + adds files to the message
  // (the computer behind the page shows its own file picker; where that's this page's browser, CO.pickFiles, the
  // browser's), and the microphone starts or stops listening. Neither takes the keys from the box.
  var attached = [], pickHere = false, picker = null;
  ["add", "ai", "mic"].forEach(function (id) { $(id).addEventListener("mousedown", function (e) { e.preventDefault(); }); });
  // Beside +: your own AI. Pressed, the computer behind the page lists the AIs to sign in with instead of Chat
  // Computer's own model, and which answers now (CO.ownAi says which: its name, model and own icon).
  $("ai").addEventListener("click", function () { post({ type: "ai_menu" }); });
  $("add").addEventListener("click", function () {
    if (!pickHere) { post({ type: "add_files" }); return; }
    if (!picker) {
      picker = document.createElement("input");
      picker.type = "file";
      picker.multiple = true;
      picker.hidden = true;
      picker.addEventListener("change", function () {
        var files = Array.prototype.slice.call(picker.files || []);
        picker.value = "";
        if (!files.length) return;
        window.CO.picked = files;          // (for the page's host, which reads them: it's in this same page)
        post({ type: "files_picked", files: files.map(function (f) { return { name: f.name, size: f.size, kind: f.type }; }) });
      });
      document.body.appendChild(picker);
    }
    picker.click();
  });
  $("mic").addEventListener("click", function () { post({ type: "mic_toggle" }); });
  function micPressed(on) {
    var m = $("mic");
    m.setAttribute("aria-pressed", on ? "true" : "false");
    m.title = on ? "Stop listening" : "Talk";
    m.setAttribute("aria-label", m.title);
    if (!on) m.style.removeProperty("--level");
  }
  // The files added to the message: a chip each, above what's typed, with an x to take it off again.
  function drawAttached() {
    var box = $("attached");
    box.textContent = "";
    attached.forEach(function (f, i) {
      var chip = el("chip"), name = document.createElement("span"), x = document.createElement("button");
      chip.title = f.path || f.name;
      name.textContent = f.name;
      x.type = "button";
      x.tabIndex = -1;
      x.setAttribute("aria-label", "Take " + f.name + " off");
      x.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/></svg>';
      x.addEventListener("mousedown", function (e) { e.preventDefault(); });
      x.addEventListener("click", function () { attached.splice(i, 1); drawAttached(); });
      chip.appendChild(name);
      chip.appendChild(x);
      box.appendChild(chip);
    });
    body.classList.toggle("attaching", attached.length > 0);
    body.classList.toggle("drafting", input.value.trim() !== "" || attached.length > 0);
  }
  // Anywhere in the box (around the words, beside its buttons) is somewhere to type, as in the Claude app.
  $("box").addEventListener("click", function (e) {
    if (!(e.target.closest && e.target.closest("button, #results, #input"))) focusBox(true);
  });
  // Typing always goes into the box, even after a click elsewhere on the page or selecting words to copy them (but
  // not while a sign-in form is open: that has boxes of its own). On a touchscreen alone, only when it's asked for
  // (force: the box touched, Edit, or a key on a real keyboard): there it would bring up the onscreen keyboard.
  function focusBox(force) {
    if (locked) { lockFocus(); return; }
    if (touchOnly() && !force) return;
    if (!body.classList.contains("sheet") && document.activeElement !== input) input.focus({ preventScroll: true });
  }
  document.addEventListener("keydown", function (e) {
    if (locked) { if (e.target !== $("lock-pin")) lockFocus(); return; }   // keys are for the PIN, nothing else
    if (body.classList.contains("sheet") || body.classList.contains("reading")) return;
    var paste = (e.ctrlKey || e.metaKey) && (e.key === "v" || e.key === "V");
    if (!(paste || (!e.ctrlKey && !e.altKey && !e.metaKey && (e.key.length === 1 || e.key === "Backspace" || e.key === "Enter")))) return;
    if (holdMsg) closeHold();
    if (e.target !== input) focusBox(true);
  }, true);
  // Escape: a sheet cancelled (one with nothing to type in has no keys of its own; a sign-in form's Escape is its
  // own: Cancel), or what can be done with a held message put away, or Select text, or Messages (where it comes over
  // the conversation), or the "/" list; or else what's in the middle (a message, a site, a file, what something in
  // Actions opened) closed, back to the conversation, as its x does; or, with the conversation scrolled back (to
  // where something in Actions was asked), back down to its latest. The sidebars stay: their switches are for them.
  function escape() {
    if (body.classList.contains("sheet")) cancelForm();
    else if (holdMsg) closeHold();
    else if (body.classList.contains("reading")) closeReader();
    else if (body.classList.contains("show-messages")) hideMessages();
    else if (searching) { input.value = ""; fitInput(); }
    else if (mode === "content") post({ type: "close_content", pause: true });
    else if (!stick) jumpDown();
  }
  document.addEventListener("keydown", function (e) {
    if (e.key !== "Escape" || e.isComposing || e.defaultPrevented || locked) return;
    e.preventDefault();
    escape();
  });
  document.addEventListener("mouseup", function () {
    var sel = window.getSelection();
    if (!sel || sel.isCollapsed) focusBox();
  });
  window.addEventListener("focus", focusBox);
  // The space the box takes at the bottom (it grows with what's typed): the conversation ends above it.
  new ResizeObserver(function () {
    rootStyle.setProperty("--composer-h", composer.offsetHeight + "px");
    follow();
    reportHole();
  }).observe(composer);
  // A phone's keyboard comes up over the bottom of the page, and Safari on an iPhone doesn't make the page smaller
  // for it (Chrome on Android does: the viewport's interactive-widget=resizes-content), so the box, fixed at the
  // page's bottom, could be left under the keyboard. Never: what's on the screen (the visual viewport) is watched, and
  // however far the box's bottom is below it (--kb), the box is lifted, to sit just above the keyboard as in the
  // Claude app, wherever the browser has scrolled what's on the screen to. The conversation gets that much more room
  // at its end, so its last words can be read above the box.
  var onScreen = window.visualViewport, kb = 0;
  function fitKeyboard() {
    var bottom = composer.getBoundingClientRect().bottom + kb;       // where the box's bottom would be, not lifted
    // (Only a keyboard lifts the box, and one is up only while something's being typed in. The conversation pulled
    // down past its top, as a phone's browser lets it be to reload, puts what's on the screen above the page's top
    // (offsetTop under 0): that's not a keyboard, and the box stays at the bottom.)
    var at = document.activeElement, typing = !!at && (at.tagName === "TEXTAREA" || at.tagName === "INPUT" || at.isContentEditable);
    var lift = Math.max(0, Math.round(bottom - (Math.max(0, onScreen.offsetTop) + onScreen.height)));
    if (!typing || lift < 2 || Math.abs((onScreen.scale || 1) - 1) > 0.01) lift = 0;     // (a part of a pixel; or zoomed in, not a keyboard)
    if (lift === kb) return;
    kb = lift;
    rootStyle.setProperty("--kb", kb + "px");
    body.classList.toggle("keyboard", kb > 0);
    follow();
    reportHole();
  }
  if (onScreen) {
    onScreen.addEventListener("resize", fitKeyboard);
    onScreen.addEventListener("scroll", fitKeyboard);
  }
  // The empty box's hint (its placeholder): "Type anything", as the person asked (no more about "/" or Escape).
  // Or, for a while, what's going on (CO.hint: "Listening", a download's progress), as much as fits.
  var HINTS = [input.placeholder];
  var ruler = document.createElement("canvas").getContext("2d"), hintNow = "";
  function fitHint() {
    var cs = getComputedStyle(input), room = input.clientWidth - 2;
    ruler.font = cs.fontStyle + " " + cs.fontWeight + " " + cs.fontSize + " " + cs.fontFamily;
    var options = hintNow ? [hintNow] : HINTS;
    var want = options[options.length - 1];
    for (var i = 0; i < options.length; i++) if (ruler.measureText(options[i]).width <= room) { want = options[i]; break; }
    if (input.placeholder !== want) input.placeholder = want;
  }
  if (document.fonts && document.fonts.ready) document.fonts.ready.then(fitHint);
  new ResizeObserver(function () { requestAnimationFrame(fitHint); }).observe(input);

  // How long typed letters take to reach the screen (two frames after the key), sent every few seconds and logged
  // once a minute (GtkScreen._watch_speed): typing on a real PC once felt slow, with nothing in the log to say why.
  var typedMs = [], typedTimer = 0;
  function measure(t0) {
    requestAnimationFrame(function () {
      requestAnimationFrame(function () {
        typedMs.push(Math.max(0, Math.round(performance.now() - t0)));
        if (!typedTimer) typedTimer = setTimeout(function () { typedTimer = 0; post({ type: "keys", ms: typedMs }); typedMs = []; }, 3000);
      });
    });
  }


  // ------------------------------------------------------------------ the sidebars
  // Shown when there's room for them (17.5rem each, as the Claude app's sidebar) beside more than half the
  // conversation's width (most screens, and a laptop's at 150%: 1280 points across; not a narrow window).
  function fitPanel() {
    var rem = remPx();
    var col = parseFloat(rootStyle.getPropertyValue("--co-column")) || 57.75 * rem;
    var w = window.innerWidth;
    body.classList.toggle("panel", w - 35 * rem - 3 * rem >= 0.55 * col);
    fitHint();                                   // (the box's width, and so its hint's room, changes with them)
  }
  // The sidebars' switches: Messages (left) and Actions (right), each switched off and on again with its switch,
  // or by asking Chat Computer (the settings left_panel and right_panel). Chat Computer keeps the choice: the page tells
  // it, and it tells the page at the start.
  var PANEL_NAMES = { left: "Messages", right: "Actions" };
  function applyPanels() {
    ["left", "right"].forEach(function (k) {
      body.classList.toggle("hide-" + k, !shown[k]);
      var b = $("toggle-" + k), label = (shown[k] ? "Hide " : "Show ") + PANEL_NAMES[k];
      b.setAttribute("aria-pressed", String(shown[k]));
      b.setAttribute("aria-label", label);
      b.title = label;
    });
    follow();
    reportHole();
  }
  function setPanel(k, on) {
    if (shown[k] === on) return;
    shown[k] = on;
    applyPanels();
    post({ type: "panels", left: shown.left, right: shown.right });
  }
  ["left", "right"].forEach(function (k) {
    $("toggle-" + k).addEventListener("click", function () { setPanel(k, !shown[k]); });
  });
  function dayLabel(ts) {
    var d = new Date(ts * 1000), t = new Date(), y = new Date(t.getFullYear(), t.getMonth(), t.getDate() - 1);
    if (d.toDateString() === t.toDateString()) return "Today";
    if (d.toDateString() === y.toDateString()) return "Yesterday";
    return d.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" });
  }
  function clock(ts) { return new Date(ts * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }); }
  function dayItem(ts) {
    var h = document.createElement("li");
    h.className = "side-day";
    h.textContent = dayLabel(ts);
    return h;
  }
  function pinButton(pinned) {
    var b = document.createElement("button");
    b.className = "pin";
    b.type = "button";
    b.tabIndex = -1;
    b.title = pinned ? "Unpin" : "Pin to the top";
    b.dataset.label = pinned ? "Unpin" : "Pin";          // what it says when an item is swiped (touchscreen)
    b.setAttribute("aria-label", b.title);
    b.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><use href="#i-pin"/></svg>';
    return b;
  }
  // Revert, on something in Actions that changed something which can be taken back (a file, a setting): the computer
  // takes it back, lists that as an action of its own, and marks this one reverted (CO.doneStates).
  function revertButton() {
    var b = document.createElement("button");
    b.className = "revert";
    b.type = "button";
    b.tabIndex = -1;
    b.title = "Revert: undo this";
    b.dataset.label = "Revert";                          // what it says when an item is swiped (touchscreen)
    b.setAttribute("aria-label", b.title);
    b.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><use href="#i-revert"/></svg>';
    return b;
  }
  // Pins: a pinned item stays at the top of its list, newest pin first, until it's unpinned. The page shows it at
  // once; Chat Computer keeps it (pins.json) for next time.
  function togglePin(kind, items, id) {
    var it = items[id];
    if (!it) return;
    if (it.pinned) delete it.pinned; else it.pinned = now();
    post({ type: "pin", list: kind, id: id, on: !!it.pinned });
  }
  function onSide(listEls, handler) {
    listEls.forEach(function (ul) {
      ul.addEventListener("mousedown", function (e) { e.preventDefault(); });     // typing stays in the box
      ul.addEventListener("click", function (e) {
        var li = e.target.closest ? e.target.closest(".item") : null;
        var tool = e.target.closest ? e.target.closest(".pin, .revert") : null;
        if (li) handler(li, tool ? tool.className.split(" ")[0] : "");
      });
    });
  }

  // Actions: oldest at the top, grouped by day; pinned ones above.
  var dones = [], doneById = {}, lastDay = "", labelsFor = "", DONE_MAX = 300;
  function doneItem(e) {
    var li = document.createElement("li");
    li.className = "item done-item" + (e.pinned ? " pinned" : "") + (e.reverted ? " reverted" : "") +
      (e.reverting ? " reverting" : "") + (e.revert && !e.reverted && !e.reverting ? " revertable" : "");
    li.dataset.id = e.id;
    li.dataset.prompt = e.prompt_id || "";
    li.dataset.ts = String(e.prompt_ts || e.ts);
    // What, and its time at the right of it (on its first line), as a message's time is. A pinned one from another
    // day (pinned ones aren't under a day) says its day there instead.
    var pinnedDay = e.pinned && new Date(e.ts * 1000).toDateString() !== new Date().toDateString();
    li.appendChild(el("done-what", e.text));
    li.appendChild(el("done-when", e.reverting ? "Reverting\u2026" : pinnedDay ? dayLabel(e.ts) : clock(e.ts)));
    if (e.revert && !e.reverted && !e.reverting) li.appendChild(revertButton());
    li.appendChild(pinButton(e.pinned));
    return li;
  }
  // One item's marks changed (Revert, reverted): only what differs is changed, where it is (not the whole list again:
  // it can hold 300, and the one changed is often under the pointer).
  function redrawDone(e) {
    var li = null;
    [doneList, donePinned].forEach(function (list) {
      Array.prototype.forEach.call(list.children, function (x) { if (x.dataset.id === e.id) li = x; });
    });
    if (!li) return;
    var fresh = doneItem(e), when = li.querySelector(".done-when"), button = li.querySelector(".revert");
    var freshWhen = fresh.querySelector(".done-when").textContent;
    li.className = fresh.className;
    if (when.textContent !== freshWhen) when.textContent = freshWhen;
    if (button && !fresh.querySelector(".revert")) li.removeChild(button);
    else if (!button && fresh.querySelector(".revert")) li.insertBefore(revertButton(), li.querySelector(".pin"));
  }
  function addDone(e) {
    if (e.pinned) return;
    var day = new Date(e.ts * 1000).toDateString();
    if (day !== lastDay) { doneList.appendChild(dayItem(e.ts)); lastDay = day; }
    doneList.appendChild(doneItem(e));
  }
  function drawDone(keepScroll) {
    var at = [doneList.scrollTop, donePinned.scrollTop];
    doneList.textContent = "";
    donePinned.textContent = "";
    lastDay = "";
    doneById = {};
    dones.forEach(function (e) { doneById[e.id] = e; });
    dones.filter(function (e) { return e.pinned; }).sort(function (a, b) { return b.pinned - a.pinned; })
      .forEach(function (e) { donePinned.appendChild(doneItem(e)); });
    dones.forEach(addDone);
    labelsFor = new Date().toDateString();
    body.classList.toggle("has-done", dones.length > 0);
    if (keepScroll) { doneList.scrollTop = at[0]; donePinned.scrollTop = at[1]; }
    else doneList.scrollTop = doneList.scrollHeight;
  }
  // Today's date and the time, above Actions: changed on the minute (and the days' names at midnight).
  function tick() {
    var d = new Date();
    $("clock-date").textContent = d.toLocaleDateString([], { weekday: "long", day: "numeric", month: "long", year: "numeric" });
    $("clock-time").textContent = clock(d.getTime() / 1000);
    if (labelsFor && labelsFor !== d.toDateString()) { drawDone(); drawMessages(); }
    Array.prototype.forEach.call(log.querySelectorAll(".acts .when"), function (w) { w.textContent = ago(+w.dataset.ts); });
    setTimeout(tick, 60000 - (d.getSeconds() * 1000 + d.getMilliseconds()) + 50);
  }
  onSide([doneList, donePinned], function (li, tool) {
    if (tool === "pin") { togglePin("done", doneById, li.dataset.id); drawDone(true); return; }
    if (tool === "revert") {
      var it = doneById[li.dataset.id];
      if (it && it.revert && !it.reverting) {
        it.reverting = true;                 // until the computer says how it went (CO.doneStates)
        post({ type: "revert", id: it.id });
        redrawDone(it);
      }
      return;
    }
    var ts = parseFloat(li.dataset.ts) || 0, id = li.dataset.prompt;
    if (mode === "content") {
      post({ type: "close_content", pause: false });     // back to the conversation, to show where it was asked
      setMode("chat");
      requestAnimationFrame(function () { goTo(id, ts); });
    } else goTo(id, ts);
  });
  // The message a done thing was asked in: by its id, else the last message you sent by then.
  function findMessage(id, ts) {
    if (id) {
      var all = log.querySelectorAll(".you[data-id]");
      for (var i = 0; i < all.length; i++) if (all[i].dataset.id === id) return all[i];
    }
    var best = null, list = log.querySelectorAll(".you[data-ts]");
    for (var j = 0; j < list.length; j++) if (parseFloat(list[j].dataset.ts) <= ts + 1) best = list[j];
    var first = log.querySelector("[data-ts]");
    return best && first && parseFloat(first.dataset.ts) <= ts + 1 ? best : null;
  }
  function firstTs() {
    var f = log.querySelector("[data-ts]");
    return f ? parseFloat(f.dataset.ts) : null;
  }
  var foundTimer = 0;
  function reveal(target) {
    stick = false;
    target.scrollIntoView({ block: "start", behavior: "smooth" });
    var old = log.querySelector(".found");
    if (old) old.classList.remove("found");
    target.classList.add("found");
    clearTimeout(foundTimer);
    foundTimer = setTimeout(function () { target.classList.remove("found"); }, 2500);
  }
  function goTo(id, ts) {
    var target = findMessage(id, ts);
    if (target) reveal(target);
    else post({ type: "goto", id: id || null, ts: ts, before: firstTs() });     // an earlier day: CO.earlier
  }

  // Messages: everything sent to you (email, Telegram, WhatsApp, social media), newest first, grouped by day;
  // pinned ones above. Unread: who in bold. Tapping one opens it between the sidebars.
  var msgs = {}, accounts = [], MSG_MAX = 400;
  var ICON = { email: "i-email", telegram: "i-telegram", whatsapp: "i-chat", wechat: "i-chat", discord: "i-chat" };
  // Beside each message, its service's own icon (m.icon, a link to it from the computer behind the page: the icon
  // the service itself gives its pages, as a browser tab shows), or, where there's none or it can't be had, Chat
  // Computer's own for that kind of message.
  var iconFailed = {};
  function ownIcon(m) {
    var s = document.createElement("span");
    s.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><use href="#' + (ICON[m.source] || "i-social") + '"/></svg>';
    return s.firstChild;
  }
  function msgIcon(m) {
    var url = typeof m.icon === "string" && /^(https:\/\/|data:image\/)/.test(m.icon) ? m.icon : "";
    if (!url || iconFailed[url]) return ownIcon(m);
    var img = document.createElement("img");
    img.className = "msg-icon";
    img.alt = "";
    img.decoding = "async";
    img.referrerPolicy = "no-referrer";
    img.addEventListener("error", function () {
      iconFailed[url] = true;
      if (img.parentNode) img.parentNode.replaceChild(ownIcon(m), img);
    });
    img.src = url;
    return img;
  }
  function msgItem(m) {
    var li = document.createElement("li");
    li.className = "item msg" + (m.unread ? " unread" : "") + (m.pinned ? " pinned" : "");
    li.dataset.id = m.id;
    var top = el("msg-top");
    top.appendChild(msgIcon(m));
    top.appendChild(el("msg-who", m.who || ""));
    top.appendChild(el("msg-when", clock(m.ts)));
    var text = el("msg-text");
    var src = document.createElement("span");
    src.className = "msg-src";
    src.textContent = (m.label || m.source) + ": ";
    text.appendChild(src);
    text.appendChild(document.createTextNode(m.text || ""));
    li.appendChild(top);
    li.appendChild(text);
    li.appendChild(pinButton(m.pinned));
    return li;
  }
  function drawMessages() {
    var atTop = msgList.scrollTop < 4;
    msgList.textContent = "";
    msgPinned.textContent = "";
    var all = Object.keys(msgs).map(function (k) { return msgs[k]; });
    all.filter(function (m) { return m.pinned; }).sort(function (a, b) { return b.pinned - a.pinned; })
      .forEach(function (m) { msgPinned.appendChild(msgItem(m)); });
    var day = "";
    all.filter(function (m) { return !m.pinned; }).sort(function (a, b) { return b.ts - a.ts; }).forEach(function (m) {
      var d = new Date(m.ts * 1000).toDateString();
      if (d !== day) { msgList.appendChild(dayItem(m.ts)); day = d; }
      msgList.appendChild(msgItem(m));
    });
    body.classList.toggle("has-messages", all.length > 0 || accounts.length > 0);
    if (atTop) msgList.scrollTop = 0;
  }
  function trimMessages() {
    var all = Object.keys(msgs).map(function (k) { return msgs[k]; });
    if (all.length <= MSG_MAX) return;
    all.filter(function (m) { return !m.pinned; }).sort(function (a, b) { return a.ts - b.ts; })
      .slice(0, all.length - MSG_MAX).forEach(function (m) { delete msgs[m.id]; });
  }
  onSide([msgList, msgPinned], function (li, tool) {
    var pin = tool === "pin";
    var m = msgs[li.dataset.id];
    if (!m) return;
    if (pin) togglePin("messages", msgs, m.id);
    else { m.unread = false; hideMessages(); post({ type: "open_message", id: m.id }); }
    drawMessages();
  });
  // + beside Messages: what can be connected (email, chat apps, social media), in a panel of its own, as the one for
  // your own AI is (the computer behind the page shows it: CO.accountForm).
  $("add-account").addEventListener("click", function () { post({ type: "add_account" }); });

  // ------------------------------------------------------------------ chats
  // Each task's chat, in the right sidebar's lower half: newest first, the one on the screen marked; a click opens
  // one, + starts a new one (as the Claude app's sidebar does). Chat Computer keeps them, and says which (CO.chats).
  var chatsList = $("chats-list"), chats = [], chatNow = "";
  function drawChats() {
    chatsList.innerHTML = "";
    chats.forEach(function (c) {
      var li = document.createElement("li");
      li.className = "item chat-item" + (c.id === chatNow ? " current" : "");
      li.textContent = c.title || "New chat";
      li.title = c.title || "New chat";
      li.setAttribute("data-id", c.id);
      li.addEventListener("click", function () { if (c.id !== chatNow) post({ type: "open_chat", id: c.id }); });
      chatsList.appendChild(li);
    });
    body.classList.toggle("has-chats", chats.length > 0);
  }
  $("new-chat").addEventListener("click", function () { post({ type: "new_chat" }); });

  // ------------------------------------------------------------------ connecting an account
  // A form Chat Computer shows (CO.accountForm): what's typed in it goes back to Chat Computer's own sign-in store, never to
  // the model. Choices (which kind of account) are buttons.
  var form = null;
  function showForm(f) {
    form = f;
    body.classList.toggle("sheet", !!f);
    body.classList.remove("sheet-busy");
    if (!f) { focusBox(); return; }
    $("sheet-title").textContent = f.title || "";
    var text = $("sheet-text");
    text.textContent = "";
    (f.text || []).forEach(function (p) { var e = document.createElement("p"); e.textContent = p; text.appendChild(e); });
    var fields = $("sheet-fields");
    fields.textContent = "";
    var firstInput = null;
    // (Choices: buttons, each with its own logo where it has one (c.icon, as Messages' are), the one chosen now
    // marked (c.on); a note between them (c.note) starts a new row. They come after the fields, or before them (f.first:
    // a choice, then what it needs typed); f.more, after the fields too.)
    function choices(list) {
      if (!list || !list.length) return;
      var row = el("choices");
      list.forEach(function (c) {
        if (typeof c.note === "string") { var n = document.createElement("p"); n.className = "choices-note"; n.textContent = c.note; row.appendChild(n); return; }
        var b = document.createElement("button");
        b.type = "button";
        var url = typeof c.icon === "string" && /^https:\/\//.test(c.icon) && !iconFailed[c.icon] ? c.icon : "";
        if (url) {
          var img = document.createElement("img");
          img.className = "choice-icon";
          img.alt = "";
          img.decoding = "async";
          img.referrerPolicy = "no-referrer";
          img.addEventListener("error", function () { iconFailed[url] = true; if (img.parentNode) img.parentNode.removeChild(img); });
          img.src = url;
          b.appendChild(img);
        }
        b.appendChild(document.createTextNode(c.label));
        if (c.on) { b.classList.add("on"); b.setAttribute("aria-pressed", "true"); }
        b.addEventListener("click", function () { post({ type: "add_account", provider: c.value }); });
        row.appendChild(b);
      });
      fields.appendChild(row);
    }
    if (f.first) choices(f.choices);
    (f.fields || []).forEach(function (fd) {
      var label = document.createElement("label");
      label.textContent = fd.label || fd.name;
      var inp = document.createElement("input");
      inp.name = fd.name;
      inp.type = fd.type || "text";
      inp.value = fd.value || "";
      inp.autocomplete = "off";
      inp.spellcheck = false;
      if (fd.placeholder) inp.setAttribute("aria-describedby", "sheet-text");
      label.appendChild(inp);
      fields.appendChild(label);
      if (!firstInput && !fd.value) firstInput = inp;
    });
    if (!f.first) choices(f.choices);
    choices(f.more);
    $("sheet-error").textContent = f.error || "";
    $("sheet-ok").textContent = f.ok || "Connect";
    $("sheet-buttons").style.display = f.ok === "" ? "none" : "";
    body.classList.toggle("sheet-back", !!f.back);
    if (f.busy) body.classList.add("sheet-busy");
    var focusOn = firstInput || fields.querySelector("input");
    if (focusOn) focusOn.focus({ preventScroll: true });
  }
  $("sheet-form").addEventListener("submit", function (e) {
    e.preventDefault();
    if (!form || body.classList.contains("sheet-busy")) return;
    var values = {};
    Array.prototype.forEach.call($("sheet-fields").querySelectorAll("input"), function (i) { values[i.name] = i.value; });
    body.classList.add("sheet-busy");
    $("sheet-error").textContent = "";
    post({ type: "account", form: form.id, values: values });
  });
  function cancelForm() {
    if (form) post({ type: "account_cancel", form: form.id });
    showForm(null);
  }
  $("sheet-cancel").addEventListener("click", cancelForm);
  // (Back: to the panel this one came from, as the computer behind the page says, f.back.)
  $("sheet-back").addEventListener("click", function () { if (form && form.back) post({ type: "add_account", provider: form.back }); });
  // (A click beside the panel, on the dimmed screen: Cancel.)
  $("sheet").addEventListener("mousedown", function (e) { if (e.target === $("sheet")) cancelForm(); });
  $("sheet").addEventListener("keydown", function (e) { if (e.key === "Escape") { e.preventDefault(); cancelForm(); } });

  // ------------------------------------------------------------------ the lock screen
  // CO.lock({error}) covers everything with the time and a keypad; the PIN typed (on the keypad, or a keyboard) goes
  // back to Chat Computer to check, never to the model. CO.lock(null) takes it away.
  var locked = false;
  function lockFocus() { var p = $("lock-pin"); if (document.activeElement !== p) p.focus({ preventScroll: true }); }
  function lockClock() {
    if (!locked) return;
    var d = new Date();
    $("lock-time").textContent = clock(d.getTime() / 1000);
    $("lock-date").textContent = d.toLocaleDateString([], { weekday: "long", day: "numeric", month: "long" });
    setTimeout(lockClock, 1000 - d.getMilliseconds() + 20);
  }
  function showLock(info) {
    var was = locked;
    locked = !!info;
    body.classList.toggle("locked", locked);
    $("lock-pin").value = "";
    $("lock-error").textContent = (info && info.error) || "";
    if (info && info.note) $("lock-note").textContent = info.note;
    if (locked) { if (!was) lockClock(); lockFocus(); } else { focusBox(); }
  }
  function sendPin() {
    var pin = $("lock-pin").value;
    $("lock-pin").value = "";
    if (pin) post({ type: "unlock", pin: pin });
  }
  $("lock-form").addEventListener("submit", function (e) { e.preventDefault(); sendPin(); });
  $("lock-pin").addEventListener("input", function () {
    var p = $("lock-pin");
    p.value = p.value.replace(/\D+/g, "").slice(0, 12);
  });
  Array.prototype.forEach.call(document.querySelectorAll("#lock-pad button"), function (b) {
    b.addEventListener("click", function (e) {
      var k = b.dataset.key, p = $("lock-pin");
      if (k === "ok") return;                    // the form's submit
      e.preventDefault();
      if (k === "del") p.value = p.value.slice(0, -1);
      else if (p.value.length < 12) p.value += k;
      $("lock-error").textContent = "";
    });
  });

  // ------------------------------------------------------------------ "/": searching this computer
  // A "/" first in the box makes it search this computer's files (Chat Computer's index of them) for the words after
  // it, as they're typed: the best matches in a list above the box. The arrow keys choose, Enter or a click opens
  // one between the sidebars; the last line asks Chat Computer to look instead. Escape (clearing the box) ends it. The list
  // can also hold things to connect or turn on (an item with a `command`: the website's Connect AirPods...), done with Enter or a click as a file is opened.
  var searching = false, searchN = 0, searchQ = "", results = [], resSel = 0, searchTimer = 0;
  function useIcon(id) { return '<svg viewBox="0 0 24 24" aria-hidden="true"><use href="#' + id + '"/></svg>'; }
  function checkSearch() {
    var on = input.value.charAt(0) === "/" && input.value.indexOf("\n") < 0;
    if (on !== searching) {
      searching = on;
      body.classList.toggle("searching", on);
      if (!on) { results = []; $("results").textContent = ""; clearTimeout(searchTimer); }
      reportHole();
    }
    if (!on) return;
    var q = input.value.slice(1).trim();
    if (q === searchQ && results.length) return;
    clearTimeout(searchTimer);
    searchTimer = setTimeout(function () { searchN++; post({ type: "search", q: q, n: searchN }); }, 120);
  }
  function bold(target, text, q) {
    var words = q.toLowerCase().split(/\s+/).filter(Boolean), low = text.toLowerCase(), marks = [];
    words.forEach(function (w) {
      for (var i = low.indexOf(w); i >= 0; i = low.indexOf(w, i + w.length)) marks.push([i, i + w.length]);
    });
    marks.sort(function (a, b) { return a[0] - b[0]; });
    var at = 0;
    marks.forEach(function (m) {
      if (m[0] < at) return;
      target.appendChild(document.createTextNode(text.slice(at, m[0])));
      var b = document.createElement("b");
      b.textContent = text.slice(m[0], m[1]);
      target.appendChild(b);
      at = m[1];
    });
    target.appendChild(document.createTextNode(text.slice(at)));
  }
  function drawResults(q, items, note) {
    var list = $("results");
    list.textContent = "";
    results = items || [];
    searchQ = q;
    results.forEach(function (r, i) {
      var row = el("res");
      row.setAttribute("role", "option");
      row.innerHTML = useIcon(r.icon || "i-file");          // a file; or something to connect or turn on (r.command)
      var t = el("res-text"), name = document.createElement("span"), where = document.createElement("span");
      name.className = "res-name";
      bold(name, r.name || "", q);
      where.className = "res-where";
      where.textContent = r.where || "";
      t.appendChild(name);
      t.appendChild(where);
      row.appendChild(t);
      if (r.when) row.appendChild(el("res-when", r.when));
      row.addEventListener("mousedown", function (e) { e.preventDefault(); });
      row.addEventListener("click", function () { resSel = i; openResult(); });
      list.appendChild(row);
    });
    if (!results.length && note) list.appendChild(el("res-note", note));
    var ask = el("res ask");
    ask.setAttribute("role", "option");
    ask.innerHTML = useIcon("i-chat");
    ask.appendChild(el("res-text", q ? "Ask Chat Computer to find “" + q + "”" : "Ask Chat Computer to find something"));
    ask.addEventListener("mousedown", function (e) { e.preventDefault(); });
    ask.addEventListener("click", function () { resSel = results.length; openResult(); });
    list.appendChild(ask);
    markResult(0);
  }
  new ResizeObserver(function () { reportHole(); }).observe($("results"));    // the list grows: what's open makes room
  function markResult(i) {
    var rows = $("results").querySelectorAll(".res");
    resSel = Math.max(0, Math.min(i, rows.length - 1));
    Array.prototype.forEach.call(rows, function (r, j) { r.classList.toggle("sel", j === resSel); });
    if (rows[resSel]) rows[resSel].scrollIntoView({ block: "nearest" });
  }
  function openResult() {
    var q = input.value.slice(1).trim(), r = results[resSel];
    if (!r && !q) return;
    input.value = "";
    fitInput();
    if (r && r.command) post({ type: "command", id: r.command });
    else if (r) post({ type: "open_file", path: r.path });
    else post({ type: "send", text: "Find “" + q + "” on my computer." });
    jumpDown();
    doneTyping();
  }
  function searchKeys(e) {
    if (!searching) return false;
    var n = $("results").querySelectorAll(".res").length;
    if (e.key === "ArrowDown") markResult((resSel + 1) % Math.max(1, n));
    else if (e.key === "ArrowUp") markResult((resSel - 1 + n) % Math.max(1, n));
    else return false;
    e.preventDefault();
    return true;
  }

  // ------------------------------------------------------------------ the touchscreen
  // As on an iPad. The window takes the system's gestures (ui/touch.py: Home from the bottom edge, back and
  // forward, three fingers to copy and paste...). Here, the conversation's own: a finger on the message box brings
  // up the onscreen keyboard and a finger anywhere else puts it away; two fingers pinched make the text bigger or
  // smaller; holding a message selects it; a double tap selects a word and a triple tap a paragraph; holding the
  // message box and moving moves where typing goes; a sidebar item swiped left shows Pin (and Revert), as Mail shows its
  // buttons.
  // On a phone, as in the Claude app there: two fingers do nothing (nothing zooms), holding a message brings up what
  // can be done with it (see "holding a message"), and in the box the phone's own magnifier and handles work.
  var HOLD_MS = 500, MOVE_PX = 10, TAP_MS = 350;
  var touch = null, pinch = null, taps = { n: 0, t: 0, x: 0, y: 0 }, holdTimer = 0;
  function spread(ts) { return Math.hypot(ts[0].clientX - ts[1].clientX, ts[0].clientY - ts[1].clientY) || 1; }
  function typeBox(el) { return el && el.closest ? el.closest("#box, #sheet input") : null; }
  function unswipe(except) {
    Array.prototype.forEach.call(document.querySelectorAll(".item.swiped"), function (li) {
      if (li !== except) li.classList.remove("swiped");
    });
  }
  document.addEventListener("touchstart", function (e) {
    clearTimeout(holdTimer);
    if (e.touches.length === 2 && !pinch) {
      touch = null;
      if (!phone) pinch = { d: spread(e.touches), size: remPx(), f: 1 };
      return;
    }
    if (e.touches.length !== 1) { if (pinch) endPinch(false); touch = null; return; }
    var t = e.touches[0], target = e.target;
    touch = { x: t.clientX, y: t.clientY, target: target, moved: false, held: false, horiz: null,
              item: target.closest ? target.closest(".side-list .item, .side-pinned .item") : null,
              msg: target.closest ? target.closest("#log > .you, #log > .ai") : null, caret: null };
    // The keyboard comes up for the box. For anything else it goes down, but only once the finger lifts or moves:
    // gone at once, the page would move under the finger (the keyboard's room given back) and a tap on a button (one
    // under a message) land on something else.
    var inBox = typeBox(target);
    if (inBox) post({ type: "keyboard", show: true });
    touch.away = !inBox;
    if (!touch.item || !touch.item.classList.contains("swiped")) unswipe(null);
    if (body.classList.contains("show-messages") && !(target.closest && target.closest("#messages"))) hideMessages();
    holdTimer = setTimeout(function () { held(touch); }, HOLD_MS);
  }, { passive: true, capture: true });
  function keyboardAway() {
    post({ type: "keyboard", show: false });
    if (touchOnly() && document.activeElement === input) input.blur();
  }
  function held(tc) {
    if (!tc || tc.moved || (phone && tc.target === input)) return;
    tc.held = true;
    if (tc.away && tc.msg) { tc.away = false; keyboardAway(); }
    if (tc.target === input) {                    // holding the box: moving now moves where typing goes
      tc.caret = { pos: input.selectionStart, line: parseFloat(getComputedStyle(input).lineHeight) || 36,
                   ch: (parseFloat(getComputedStyle(input).fontSize) || 24) * 0.55 };
    } else if (tc.msg) {                          // holding a message: all of it selected (three fingers copy it)
      if (phone) openHold(tc.msg, tc.y);          // on a phone: what can be done with it
      else selectContents(tc.msg);
    }
  }
  function selectNode(n) {
    var sel = window.getSelection(), r = document.createRange();
    r.selectNodeContents(n);
    sel.removeAllRanges();
    sel.addRange(r);
  }
  function selectContents(m) { selectNode(m.querySelector(".md") || m); }
  document.addEventListener("touchmove", function (e) {
    if (phone && e.touches.length > 1) { e.preventDefault(); return; }            // a phone: nothing zooms
    if (pinch && e.touches.length === 2) {
      e.preventDefault();                          // the text's size, not WebKit's zoom of the whole page
      pinch.f = Math.max(0.5, Math.min(2.5, spread(e.touches) / pinch.d));
      rootStyle.setProperty("--co-size", (pinch.size * pinch.f).toFixed(1) + "px");
      return;
    }
    if (!touch || e.touches.length !== 1) return;
    var t = e.touches[0], dx = t.clientX - touch.x, dy = t.clientY - touch.y;
    if (!touch.moved && Math.abs(dx) + Math.abs(dy) > MOVE_PX) {
      touch.moved = true;
      if (!touch.held) clearTimeout(holdTimer);
      touch.horiz = Math.abs(dx) > Math.abs(dy);
      if (touch.away) { touch.away = false; keyboardAway(); }          // scrolling puts the keyboard away
    }
    if (touch.caret) {                             // moving the insertion point, a character or a line at a time
      e.preventDefault();
      var lines = input.value.split("\n"), pos = touch.caret.pos, before = input.value.slice(0, pos);
      var row = before.split("\n").length - 1, col = pos - before.lastIndexOf("\n") - 1;
      row = Math.max(0, Math.min(lines.length - 1, row + Math.round(dy / touch.caret.line)));
      col = Math.max(0, Math.min(lines[row].length, col + Math.round(dx / touch.caret.ch)));
      var at = 0;
      for (var i = 0; i < row; i++) at += lines[i].length + 1;
      input.setSelectionRange(at + col, at + col);
      return;
    }
    if (touch.item && touch.horiz) {
      e.preventDefault();
      touch.item.classList.toggle("swiped", dx < -40 || (touch.item.classList.contains("swiped") && dx < 40));
    }
  }, { passive: false, capture: true });
  document.addEventListener("touchend", function (e) {
    clearTimeout(holdTimer);
    if (pinch) { if (e.touches.length < 2) endPinch(true); return; }
    var tc = touch;
    touch = null;
    if (!tc || e.touches.length) return;
    if (tc.held && (tc.caret || tc.msg)) { e.preventDefault(); return; }       // not a tap as well
    if (tc.moved) { taps.n = 0; return; }
    if (tc.away) keyboardAway();          // (the tap itself still lands where it was: the page moves only after)
    var t = e.changedTouches[0], now = Date.now();
    var again = now - taps.t < TAP_MS && Math.hypot(t.clientX - taps.x, t.clientY - taps.y) < 24;
    taps = { n: again ? taps.n + 1 : 1, t: now, x: t.clientX, y: t.clientY };
    if (!phone && (taps.n === 2 || taps.n === 3)) {
      var x = t.clientX, y = t.clientY, n = taps.n, target = tc.target;
      setTimeout(function () { selectAt(target, x, y, n === 3); }, 0);        // after the tap has put the cursor
    }
  }, { passive: false, capture: true });
  document.addEventListener("touchcancel", function () {
    clearTimeout(holdTimer);
    touch = null;
    if (pinch) endPinch(false);
  }, { passive: true, capture: true });
  function endPinch(keep) {
    var p = pinch;
    pinch = null;
    if (keep && Math.abs(p.f - 1) > 0.05) post({ type: "text_scale", factor: Math.round(p.f * 100) / 100 });
    else rootStyle.setProperty("--co-size", p.size + "px");      // Chat Computer sends the new size (CO.layout)
  }
  // A double tap selects the word there, a triple tap the paragraph (in the message box: the line).
  function selectAt(target, x, y, paragraph) {
    if (target === input) {
      var v = input.value, at = input.selectionStart, a = at, b = at;
      if (paragraph) {
        a = v.lastIndexOf("\n", at - 1) + 1;
        b = v.indexOf("\n", at);
        if (b < 0) b = v.length;
      } else {
        while (a > 0 && /[\wÀ-￿'-]/.test(v.charAt(a - 1))) a--;
        while (b < v.length && /[\wÀ-￿'-]/.test(v.charAt(b))) b++;
      }
      input.setSelectionRange(a, b);
      return;
    }
    var msg = target.closest ? target.closest("#log > .you, #log > .ai") : null;
    if (!msg) return;
    var sel = window.getSelection();
    if (paragraph) {
      selectNode(target.closest("p, li, pre, blockquote, td") || msg.querySelector(".md") || msg);
      return;
    }
    var r = document.caretRangeFromPoint ? document.caretRangeFromPoint(x, y) : null;
    if (!r || r.startContainer.nodeType !== 3) return;
    var text = r.startContainer.textContent, s0 = r.startOffset, e0 = r.startOffset;
    while (s0 > 0 && /[\wÀ-￿'-]/.test(text.charAt(s0 - 1))) s0--;
    while (e0 < text.length && /[\wÀ-￿'-]/.test(text.charAt(e0))) e0++;
    if (s0 === e0) return;
    r.setStart(r.startContainer, s0);
    r.setEnd(r.startContainer, e0);
    sel.removeAllRanges();
    sel.addRange(r);
  }
  // A touchscreen alone: the page itself never zooms (Safari's own pinch, which its page can't switch off otherwise).
  ["gesturestart", "gesturechange", "gestureend"].forEach(function (name) {
    document.addEventListener(name, function (e) { if (touchOnly()) e.preventDefault(); }, { passive: false });
  });
  // (A phone's own menu for a held word isn't wanted in the conversation: ours comes instead.)
  document.addEventListener("contextmenu", function (e) {
    if (phone && e.target.closest && e.target.closest("#log")) e.preventDefault();
  });

  // ------------------------------------------------------------------ holding a message (a phone)
  // As in the Claude app: the screen behind dimmed (still seen, not blurred), the message lifted, and under it (over it where
  // there's no room under it, or by the finger for a message taller than the screen) what can be done with it: Copy,
  // Send to (another app: the phone's own share sheet), Select text, and Edit for yours or Read aloud for Chat
  // Computer's. A tap anywhere else puts it away.
  var holdMsg = null, toastTimer = 0;
  function openHold(msg, y) {
    closeHold();
    var mine = msg.classList.contains("you"), menu = $("hold-menu");
    menu.querySelector('[data-do="share"]').hidden = !navigator.share;
    menu.querySelector('[data-do="edit"]').hidden = !mine;
    menu.querySelector('[data-do="speak"]').hidden = mine;
    if (document.activeElement === input) input.blur();
    holdMsg = msg;
    msg.classList.add("held");
    body.classList.add("holding");
    try { if (navigator.vibrate) navigator.vibrate(8); } catch (e) {}
    var rem = remPx(), r = msg.getBoundingClientRect(), w = menu.offsetWidth, h = menu.offsetHeight;
    var pad = 0.75 * rem, gap = (mine ? 0.85 : 1.1) * rem, vh = window.innerHeight, top, from = "top";
    if (r.bottom + gap + h <= vh - pad) top = r.bottom + gap;
    else if (r.top - gap - h >= pad) { top = r.top - gap - h; from = "bottom"; }
    else top = Math.max(pad, Math.min(vh - pad - h, y + gap));
    var left = Math.max(pad, Math.min(window.innerWidth - pad - w, mine ? r.right - w : r.left));
    menu.style.top = Math.round(top) + "px";
    menu.style.left = Math.round(left) + "px";
    menu.style.transformOrigin = from + " " + (mine ? "right" : "left");
  }
  function closeHold() {
    if (!holdMsg) return;
    holdMsg.classList.remove("held");
    holdMsg = null;
    body.classList.remove("holding");
  }
  $("hold-shade").addEventListener("click", closeHold);
  $("hold-menu").addEventListener("click", function (e) {
    var b = e.target.closest ? e.target.closest("button") : null, msg = holdMsg;
    if (!b || !msg) return;
    closeHold();
    var text = saidIn(msg), k = b.dataset.do;
    if (k === "copy") { copyText(text, null); toast("Copied"); }
    else if (k === "share") { try { navigator.share({ text: text }).catch(function () {}); } catch (x) {} }
    else if (k === "select") openReader(msg);
    else if (k === "edit") editText(text);
    else if (k === "speak") post({ type: "speak", text: text });
  });
  // Select text: the message alone over the whole screen, its words to select as anywhere else on the phone.
  function openReader(msg) {
    var t = $("reader-text"), said = saidIn(msg);
    t.textContent = "";
    t.className = msg.classList.contains("you") ? "plain" : "";
    if (t.className) t.textContent = said;
    else { var md = el("md"); md.innerHTML = window.renderMarkdown(said); t.appendChild(md); }
    body.classList.add("reading");
    t.scrollTop = 0;
  }
  function closeReader() {
    body.classList.remove("reading");
    try { window.getSelection().removeAllRanges(); } catch (e) {}
  }
  $("reader-close").addEventListener("click", closeReader);
  function toast(text) {
    var t = $("toast");
    t.textContent = text;
    t.classList.add("on");
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { t.classList.remove("on"); }, 1400);
  }

  // Messages from the top of the screen (the iPad's Notification Center): where the sidebars aren't showing (a
  // screen too narrow for them, or none brought in yet: the middle filling the screen) it comes over the
  // conversation until a touch elsewhere; with them showing, it goes back to its newest.
  function hideMessages() { body.classList.remove("show-messages"); }
  function showMessages() {
    if (body.classList.contains("panel") && !full) msgList.scrollTop = 0;
    else body.classList.add("show-messages");
  }

  var users = {};
  window.CO = {
    user: function (id, text, final) {
      showLive("");
      stick = true;
      var u = users[id];
      if (!u) {
        u = users[id] = el("you enter", text);
        u.dataset.id = id;
        u.dataset.ts = String(now());
        log.appendChild(u);
        addActs(u);
        trim();
        if (mode === "chat") pin(); else jumpDown();      // to the top of the screen, the answer to come under it
      } else { u.textContent = text; follow(); }
    },
    // What's being heard, as it's heard.
    partial: function (text) {
      showLive(text);
      if (text) jumpDown(); else follow();
    },
    ai: function (id, delta) {
      var m = ensureAI(id);
      m.text += delta;
      if (!m.raf) m.raf = requestAnimationFrame(function () { paint(m); });
    },
    aiStart: function (id) { ensureAI(id); follow(); },
    aiDone: function (id) {
      var m = ai[id];
      if (!m) return;
      if (m.raf) cancelAnimationFrame(m.raf);
      paint(m);
      m.el.classList.remove("thinking");
      if (!m.text && !m.el.querySelector(".activity") && !m.md.firstChild) { m.el.remove(); trim(); }
      else { m.el._said = m.text || saidIn(m.el); lastActs(addActs(m.el)); }
      delete ai[id];
      follow();
    },
    // Buttons under an answer, from the page's host (the website's, where no model answers yet: download my model,
    // or sign in to an AI of yours): [{label, command, main}], each doing its command as "/" does its own. None:
    // they go (a model's here now).
    aiChoices: function (id, list) {
      var m = log.querySelector('.ai[data-id="' + String(id).replace(/["\\]/g, "") + '"]');
      if (!m) return;
      var was = m.querySelector(".offers");
      if (!list || !list.length) { if (was) was.remove(); return; }
      var row = was || m.appendChild(el("offers"));
      row.textContent = "";
      list.forEach(function (c) {
        var b = document.createElement("button");
        b.type = "button";
        b.className = c.main ? "main" : "";
        b.textContent = c.label;
        b.addEventListener("click", function () { post({ type: "command", id: c.command }); });
        row.appendChild(b);
      });
      follow();
    },
    activity: function (id, text) {
      var m = ensureAI(id);
      if (m.raf) { cancelAnimationFrame(m.raf); paint(m); }
      var a = el("activity", text);
      // Activity lines sit above the words they lead to.
      if (m.text) {
        m.el.appendChild(a);
        var md2 = el("md");
        m.el.appendChild(md2);
        m.md = md2; m.text = ""; m.cut = 0; m.tail = []; m.done = 0; m.drawn = 0; m.pieces = []; m.base = 0;
      } else m.el.insertBefore(a, m.md);
      follow();
    },
    notice: function (text, kind) {
      var n = el("notice" + (kind === "warn" ? " warn" : ""), text);
      log.appendChild(n);
      trim();
      follow();
    },
    state: function (name) {
      ["listening", "sleeping", "offline", "thinking", "speaking", "muted", "resting"].forEach(function (s) {
        body.classList.toggle(s, s === name);
      });
      // The microphone button, pressed while Chat Computer listens (awake: listening, thinking or speaking).
      micPressed(name === "listening" || name === "thinking" || name === "speaking");
    },
    // The microphone button pressed or not, where the page's host listens some other way (the website).
    listening: function (on) { micPressed(!!on); },
    // Which AI answers, for the button beside + (shown once this is called: the page's host offers it): {name, model,
    // icon} for one signed in with (its own icon, from its own site; the spark where it can't be had), or nothing for
    // Chat Computer's own model.
    ownAi: function (who) {
      var b = $("ai"), img = b.querySelector(".ai-icon"), on = !!(who && who.name);
      if (!img) { img = document.createElement("img"); img.className = "ai-icon"; img.alt = ""; b.appendChild(img); }
      body.classList.add("own-ai");
      b.title = on ? who.name + (who.model ? " (" + who.model + ")" : "") + " answers. Press to change" : "Sign in with your own AI";
      b.setAttribute("aria-label", b.title);
      b.classList.toggle("on", on);
      var url = on && typeof who.icon === "string" && /^https:\/\//.test(who.icon) && !iconFailed[who.icon] ? who.icon : "";
      b.classList.toggle("has-icon", !!url);
      if (!url) { img.removeAttribute("src"); return; }
      img.onerror = function () { iconFailed[url] = true; b.classList.remove("has-icon"); };
      img.referrerPolicy = "no-referrer";
      img.src = url;
    },
    // How loud what the microphone hears is, from 0 (quiet) to 1, while listening: the button's one bar.
    level: function (x) {
      x = Math.max(0, Math.min(1, +x || 0));
      $("mic").style.setProperty("--level", (0.18 + 0.82 * x).toFixed(3));
    },
    // Whether the microphone is on. (Not shown: the screen is the conversation; Chat Computer says so when it can't
    // hear.)
    mic: function (on) {},
    // Files added to the message by the computer's own file picker ({name, path}), and whether + uses this page's
    // browser's picker instead (the website and the app: CO.picked holds what was picked there).
    attach: function (files) {
      (files || []).forEach(function (f) {
        if (!f || !f.name) return;
        var key = f.path || f.id || f.name;
        if (!attached.some(function (a) { return (a.path || a.id || a.name) === key; })) {
          attached.push({ name: String(f.name), path: f.path ? String(f.path) : undefined, id: f.id ? String(f.id) : undefined });
        }
      });
      drawAttached();
      focusBox(true);
    },
    pickFiles: function (on) { pickHere = !!on; },
    // Messages pulled down from the top of the screen (the touchscreen's Notification Center).
    showMessages: showMessages,
    // "/": what on this computer matches (n says which search it answers: older answers are ignored).
    searchResults: function (n, q, items, note) { if (searching && n === searchN) drawResults(q || "", items, note); },
    // "content" while something is open in the middle (with its name, for the top of its frame), "chat" otherwise.
    mode: function (m, title) { setMode(m === "content" ? "content" : "chat", title); },
    // What the empty box says for a while ("Listening", "Getting my model ready: …"); nothing: its own hint again.
    hint: function (text) { hintNow = String(text || ""); fitHint(); },
    // What's open is called something else now (its page's title, once it's loaded).
    title: function (t) { if (mode === "content") $("frame-title").textContent = t || ""; },
    // Escape pressed where this page doesn't have the keys (what's open has them): as here (what's open closed).
    escape: function () { escape(); },
    // The sidebars both put away (the middle filling the screen), or back.
    toggleFull: function () { toggleFull(); },
    // What's open is drawn in this page (the browser demo), not laid over it by the window.
    openInPage: function (on) { openInPage = !!on; lastHole = ""; reportHole(); },
    history: function (items) {
      log.innerHTML = "";
      var row = null;
      (items || []).forEach(function (it) {
        var m = message(it);
        log.appendChild(m);
        var r = addActs(m);
        if (r && m.classList.contains("ai")) row = r;
      });
      lastActs(row);
      greet();                        // nothing said yet: the word logo, then Chat Computer's first words
      trim();
      jumpDown();
    },
    // Earlier messages put back above the conversation (from the transcript), then scrolled to the one asked for.
    earlier: function (items, id, ts) {
      var frag = document.createDocumentFragment(), day = "";
      (items || []).forEach(function (it) {
        var d = new Date(it.ts * 1000).toDateString();
        if (d !== day) { frag.appendChild(el("day", dayLabel(it.ts))); day = d; }
        var m = message(it);
        frag.appendChild(m);
        addActs(m);
      });
      var first = firstTs();
      if (day && first && new Date(first * 1000).toDateString() !== day) frag.appendChild(el("day", dayLabel(first)));
      log.insertBefore(frag, log.firstChild);
      marks();
      var target = findMessage(id, ts);
      if (target) reveal(target);
    },
    doneList: function (entries) { dones = (entries || []).slice(-DONE_MAX); drawDone(); },
    // The chats ({id, title, ts}, newest first) and which is on the screen.
    chats: function (list, current) { chats = (list || []).slice(0, 200); chatNow = String(current || ""); drawChats(); },
    // Which can be reverted and which have been, after a revert (or "undo that"): only the marks change.
    doneStates: function (states) {
      var changed = {};
      dones.forEach(function (e) { if (e.reverting) { delete e.reverting; changed[e.id] = e; } });   // (answered)
      (states || []).forEach(function (s) {
        var e = doneById[s.id];
        if (!e || !!e.revert === !!s.revert && !!e.reverted === !!s.reverted) return;
        if (s.revert) e.revert = true; else delete e.revert;
        if (s.reverted) e.reverted = true; else delete e.reverted;
        changed[e.id] = e;
      });
      for (var id in changed) redrawDone(changed[id]);
    },
    done: function (e) {
      var bottom = doneList.scrollHeight - doneList.scrollTop - doneList.clientHeight < 4;
      dones.push(e);
      doneById[e.id] = e;
      if (dones.length > DONE_MAX || labelsFor !== new Date().toDateString()) {
        dones = dones.slice(-DONE_MAX);
        drawDone();                 // the oldest goes, or a new day began ("Today" is yesterday now)
      } else {
        addDone(e);
        body.classList.add("has-done");
      }
      if (bottom) doneList.scrollTop = doneList.scrollHeight;
    },
    // Messages: all of them at once (with the connected accounts), or one new or changed.
    messagesList: function (items, accts) {
      msgs = {};
      (items || []).forEach(function (m) { msgs[m.id] = m; });
      accounts = accts || [];
      trimMessages();
      drawMessages();
    },
    message: function (m) {
      var old = msgs[m.id];
      if (old && old.pinned && !("pinned" in m)) m.pinned = old.pinned;
      msgs[m.id] = m;
      trimMessages();
      drawMessages();
    },
    accountForm: function (f) { showForm(f || null); },
    lock: function (info) { showLock(info || null); },
    // The name you go by (the first name) and the model that answers ("" where none does): for the
    // greeting under the logo. Only letters, spaces and a name's marks are kept, 40 at most.
    settings: function (name, model) {
      who = String(name || "").replace(/[^\p{L}\p{M}'’ .-]/gu, "").replace(/\s+/g, " ").trim().slice(0, 40);
      modelAnswers = !!model;
      regreet();
    },
    // Where you are, and its weather now when it's been looked up: {place, code, temp, day} (see hereGreetings);
    // pending while it's being looked up (the greeting waits for it a moment).
    here: function (info) {
      info = info || {};
      here = { place: String(info.place || "").replace(/[^\p{L}\p{M}\p{N}'’ .,()-]/gu, "").trim().slice(0, 60),
               code: typeof info.code === "number" ? info.code : null, temp: typeof info.temp === "number" ? info.temp : null,
               day: typeof info.day === "boolean" ? info.day : null, pending: !!info.pending };
      regreet();
    },
    // The greetings there are to choose from now: those for the time and day, and those for where you are.
    greetingsNow: function () { var at = new Date(); return { time: greetings(at), here: hereGreetings(at) }; },
    // Whether a greeting goes under the logo (when it doesn't, there's no need to look up where you are).
    greets: GREETING,
    // Which sidebars show: {left, right} (Chat Computer keeps the choice; see "the sidebars' switches").
    panels: function (s) {
      ["left", "right"].forEach(function (k) { if (s && k in s) shown[k] = !!s[k]; });
      applyPanels();
    },
    // Text size (1 rem) and column width for this screen (chatcomputer/ui/layout.py).
    layout: function (sizePx, columnPx) {
      rootStyle.setProperty("--co-size", sizePx + "px");
      rootStyle.setProperty("--co-column", columnPx + "px");
      fitPanel();
      fitInput();
      follow();
      reportHole();
    },
    // Page Up and Down, and the arrow keys, Home and End while the box is empty: reading back through the
    // conversation, or through what's open (the window scrolls that).
    scroll: function (how) {
      if (mode === "content") { post({ type: "scroll_open", how: how }); return; }
      var line = parseFloat(getComputedStyle(body).lineHeight) || 36;
      var by = { page_up: -0.85 * page.clientHeight, page_down: 0.85 * page.clientHeight, up: -3 * line, down: 3 * line }[how];
      if (how === "top") page.scrollTo({ top: 0, behavior: "smooth" });
      else if (how === "bottom") page.scrollTo({ top: page.scrollHeight, behavior: "smooth" });
      else if (by) page.scrollBy({ top: by, behavior: "smooth" });
    },
    clear: function () { log.innerHTML = ""; ai = {}; users = {}; greet(); trim(); jumpDown(); }
  };
  showLive("");
  body.classList.toggle("full", full);
  greet();
  fitPanel();
  fitInput();
  tick();
  window.CO.settings("", "");
  focusBox();
  post({ type: "ready" });
})();
